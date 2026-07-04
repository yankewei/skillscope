use crate::claude::parser::parse_line;
use crate::codex::registry::SkillRegistry;
use crate::codex::scan::ScanResult;
use crate::config::Config;
use crate::db::Database;
use crate::error::Result;
use crate::jsonl_cursor;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

pub fn scan_all(db: &mut Database, config: &Config, rescan: bool) -> Result<ScanResult> {
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
    for file in project_files(config)? {
        let file_result = scan_file(db, &file, registry, rescan)?;
        result.files_scanned += 1;
        result.events_inserted += file_result.events_inserted;
        result.errors += file_result.errors;
        result.events.extend(file_result.events);
    }
    Ok(result)
}

pub fn project_files(config: &Config) -> Result<Vec<PathBuf>> {
    let projects_dir = config.claude_projects_dir();
    if !projects_dir.exists() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    for entry in WalkDir::new(projects_dir).follow_links(false) {
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
        |_| (),
        |line, source_file, source_offset, source_line, _| {
            parse_line(line, source_file, source_offset, source_line, registry)
        },
        |_, _| {},
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use std::fs;
    use tempfile::TempDir;

    struct Fixture {
        _tmp: TempDir,
        config: Config,
        transcript_file: PathBuf,
        db_path: PathBuf,
    }

    fn fixture() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        let claude_home = root.join(".claude");
        let project_dir = claude_home.join("projects/project-one");
        fs::create_dir_all(&project_dir).unwrap();
        let transcript_file = project_dir.join("session.jsonl");
        let db_path = tmp.path().join("skillscope.sqlite");
        Fixture {
            _tmp: tmp,
            config: Config {
                codex_home: root.join(".codex"),
                claude_home,
                agents_home: root.join(".agents"),
                db_path: db_path.clone(),
            },
            transcript_file,
            db_path,
        }
    }

    #[test]
    fn scan_detects_claude_skill_tool_use_incrementally() {
        let fixture = fixture();
        let event = r#"{"type":"assistant","timestamp":"2026-07-02T00:00:00Z","sessionId":"session_1","message":{"role":"assistant","content":[{"type":"tool_use","id":"toolu_1","name":"Skill","input":{"skill":"think","args":"private"}}]}}"#;
        fs::write(&fixture.transcript_file, format!("{event}\n")).unwrap();

        let mut db = Database::open(&fixture.db_path).unwrap();
        db.init().unwrap();
        let first = scan_all(&mut db, &fixture.config, false).unwrap();
        assert_eq!(first.events_inserted, 1);
        assert_eq!(first.events[0].runtime, "claude_code");
        assert_eq!(first.events[0].skill_name, "think");

        let second = scan_all(&mut db, &fixture.config, false).unwrap();
        assert_eq!(second.events_inserted, 0);
        assert!(second.events.is_empty());
    }

    #[test]
    fn scan_persists_same_line_repeated_skill_tool_uses() {
        let fixture = fixture();
        let event = r#"{"type":"assistant","timestamp":"2026-07-02T00:00:00Z","sessionId":"session_1","message":{"role":"assistant","content":[{"type":"tool_use","id":"toolu_1","name":"Skill","input":{"skill":"think"}},{"type":"tool_use","id":"toolu_2","name":"Skill","input":{"skill":"think"}}]}}"#;
        fs::write(&fixture.transcript_file, format!("{event}\n")).unwrap();

        let mut db = Database::open(&fixture.db_path).unwrap();
        db.init().unwrap();
        let first = scan_all(&mut db, &fixture.config, false).unwrap();
        assert_eq!(first.events_inserted, 2);
        assert_eq!(first.events.len(), 2);
        assert_ne!(first.events[0].id, first.events[1].id);

        let second = scan_all(&mut db, &fixture.config, false).unwrap();
        assert_eq!(second.events_inserted, 0);
    }
}
