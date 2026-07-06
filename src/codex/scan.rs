use crate::codex::parser::parse_line;
use crate::codex::registry::SkillRegistry;
use crate::config::Config;
use crate::db::{Database, ParsedFile};
use crate::error::Result;
use crate::events::{SessionState, SkillInvocation};
use crate::jsonl_cursor;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct ScanResult {
    pub files_scanned: u64,
    pub events_inserted: u64,
    pub errors: u64,
    pub events: Vec<SkillInvocation>,
}

#[cfg(test)]
fn scan_all(db: &mut Database, config: &Config, rescan: bool) -> Result<ScanResult> {
    let registry = SkillRegistry::scan(config)?;
    scan_all_with_registry(db, config, &registry, rescan)
}

pub fn scan_all_with_registry(
    db: &mut Database,
    config: &Config,
    registry: &SkillRegistry,
    rescan: bool,
) -> Result<ScanResult> {
    let mut result = ScanResult::default();
    for file in session_files(config)? {
        let file_result = scan_file(db, &file, registry, rescan)?;
        result.files_scanned += 1;
        result.events_inserted += file_result.events_inserted;
        result.errors += file_result.errors;
        result.events.extend(file_result.events);
    }
    Ok(result)
}

pub fn session_files(config: &Config) -> Result<Vec<PathBuf>> {
    let sessions_dir = config.sessions_dir();
    if !sessions_dir.exists() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    for entry in WalkDir::new(sessions_dir).follow_links(false) {
        let entry = entry?;
        if entry.file_type().is_file()
            && entry
                .path()
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext == "jsonl")
        {
            files.push(entry.path().to_path_buf());
        }
    }
    files.sort();
    Ok(files)
}

pub type FileScanResult = jsonl_cursor::FileScanResult;

pub fn scan_file(
    db: &mut Database,
    path: &Path,
    registry: &SkillRegistry,
    rescan: bool,
) -> Result<FileScanResult> {
    jsonl_cursor::scan_file(
        db,
        path,
        rescan,
        session_state_from_parsed,
        |line, source_file, source_offset, source_line, state| {
            parse_line(
                line,
                source_file,
                source_offset,
                source_line,
                registry,
                state,
            )
        },
        |parsed, state| {
            parsed.session_id = state.session_id;
            parsed.turn_id = state.turn_id;
            parsed.cwd = state.cwd.map(|cwd| cwd.to_string_lossy().into_owned());
        },
    )
}

fn session_state_from_parsed(parsed: &ParsedFile) -> SessionState {
    SessionState {
        session_id: parsed.session_id.clone(),
        turn_id: parsed.turn_id.clone(),
        cwd: parsed.cwd.as_ref().map(PathBuf::from),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use std::fs;
    use tempfile::TempDir;

    struct Fixture {
        _tmp: TempDir,
        root: PathBuf,
        config: Config,
        session_file: PathBuf,
        db_path: PathBuf,
        skill_path: PathBuf,
    }

    fn fixture() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        let codex_home = root.join(".codex");
        let agents_home = root.join(".agents");
        let session_dir = codex_home.join("sessions/2026/07/02");
        fs::create_dir_all(&session_dir).unwrap();
        let skill_dir = agents_home.join("skills/diagnose");
        fs::create_dir_all(skill_dir.join("scripts")).unwrap();
        fs::write(skill_dir.join("SKILL.md"), "---\nname: diagnose\n---\n").unwrap();
        fs::write(skill_dir.join("scripts/check.py"), "print('ok')\n").unwrap();
        let session_file = session_dir.join("session.jsonl");
        let db_path = tmp.path().join("skillscope.sqlite");
        Fixture {
            _tmp: tmp,
            root: root.clone(),
            config: Config {
                codex_home,
                claude_home: root.join(".claude"),
                agents_home,
                db_path: db_path.clone(),
            },
            session_file,
            db_path,
            skill_path: skill_dir.join("SKILL.md"),
        }
    }

    #[test]
    fn scan_is_incremental_and_deduplicated() {
        let fixture = fixture();
        let explicit = format!(
            r#"{{"timestamp":"2026-07-02T00:00:00Z","type":"response_item","payload":{{"type":"message","role":"user","content":[{{"type":"input_text","text":"<skill>\n<name>diagnose</name>\n<path>{}</path>\n</skill>"}}]}}}}"#,
            fixture.skill_path.to_string_lossy()
        );
        let implicit = format!(
            r#"{{"timestamp":"2026-07-02T00:00:01Z","type":"response_item","payload":{{"type":"function_call","name":"exec_command","call_id":"call_1","arguments":"{{\"cmd\":\"sed -n '1,120p' {}\"}}"}}}}"#,
            fixture.skill_path.to_string_lossy()
        );
        fs::write(&fixture.session_file, format!("{explicit}\n{implicit}\n")).unwrap();

        let mut db = Database::open(&fixture.db_path).unwrap();
        db.init().unwrap();
        let first = scan_all(&mut db, &fixture.config, false).unwrap();
        assert_eq!(first.events_inserted, 2);
        assert_eq!(first.events.len(), 2);
        assert_eq!(first.events[0].invocation_type, "explicit");
        assert_eq!(first.events[1].invocation_type, "implicit");

        let second = scan_all(&mut db, &fixture.config, false).unwrap();
        assert_eq!(second.events_inserted, 0);
        assert!(second.events.is_empty());
    }

    #[test]
    fn scan_waits_for_complete_jsonl_line() {
        let fixture = fixture();
        let explicit = format!(
            r#"{{"timestamp":"2026-07-02T00:00:00Z","type":"response_item","payload":{{"type":"message","role":"user","content":[{{"type":"input_text","text":"<skill>\n<name>diagnose</name>\n<path>{}</path>\n</skill>"}}]}}}}"#,
            fixture.skill_path.to_string_lossy()
        );
        fs::write(&fixture.session_file, explicit.as_bytes()).unwrap();

        let mut db = Database::open(&fixture.db_path).unwrap();
        db.init().unwrap();
        let first = scan_all(&mut db, &fixture.config, false).unwrap();
        assert_eq!(first.events_inserted, 0);

        fs::write(&fixture.session_file, format!("{explicit}\n")).unwrap();
        let second = scan_all(&mut db, &fixture.config, false).unwrap();
        assert_eq!(second.events_inserted, 1);
    }

    #[test]
    fn incremental_scan_preserves_cwd_for_relative_script_detection() {
        let fixture = fixture();
        let context = format!(
            r#"{{"type":"turn_context","payload":{{"turn_id":"turn_1","cwd":"{}"}}}}"#,
            fixture.root.to_string_lossy()
        );
        fs::write(&fixture.session_file, format!("{context}\n")).unwrap();

        let mut db = Database::open(&fixture.db_path).unwrap();
        db.init().unwrap();
        let first = scan_all(&mut db, &fixture.config, false).unwrap();
        assert_eq!(first.events_inserted, 0);

        let implicit = r#"{"timestamp":"2026-07-02T00:00:01Z","type":"response_item","payload":{"type":"function_call","name":"exec_command","call_id":"call_1","arguments":"{\"cmd\":\"python3 .agents/skills/diagnose/scripts/check.py\"}"}}"#;
        fs::write(&fixture.session_file, format!("{context}\n{implicit}\n")).unwrap();

        let second = scan_all(&mut db, &fixture.config, false).unwrap();
        assert_eq!(second.events_inserted, 1);
    }
}
