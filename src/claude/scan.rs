use crate::claude::parser::parse_line;
use crate::codex::scan::ScanResult;
use crate::config::Config;
use crate::db::{Database, ParsedFile};
use crate::error::Result;
use crate::events::SkillInvocation;
use crate::paths::normalize_for_compare;
use chrono::{DateTime, Utc};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

pub fn scan_all(db: &mut Database, config: &Config, rescan: bool) -> Result<ScanResult> {
    let mut result = ScanResult::default();
    for file in project_files(config)? {
        let file_result = scan_file(db, &file, rescan)?;
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

struct FileScanResult {
    events_inserted: u64,
    errors: u64,
    events: Vec<SkillInvocation>,
}

fn scan_file(db: &mut Database, path: &Path, rescan: bool) -> Result<FileScanResult> {
    let metadata = fs::metadata(path)?;
    let file_size = metadata.len();
    let modified_at = metadata
        .modified()
        .ok()
        .map(DateTime::<Utc>::from)
        .map(|value| value.to_rfc3339());

    let mut parsed = db.parsed_file(path)?.unwrap_or_else(|| ParsedFile {
        path: path.to_string_lossy().into_owned(),
        canonical_path: Some(normalize_for_compare(path).to_string_lossy().into_owned()),
        ..ParsedFile::default()
    });

    if rescan || file_size < parsed.byte_offset {
        parsed.byte_offset = 0;
        parsed.line_number = 0;
        parsed.partial_line.clear();
        parsed.session_id = None;
        parsed.turn_id = None;
        parsed.cwd = None;
    }

    let start_offset = parsed.byte_offset;
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(start_offset))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;

    let (complete_bytes, partial_bytes) = match bytes.iter().rposition(|byte| *byte == b'\n') {
        Some(index) => bytes.split_at(index + 1),
        None => (&[][..], bytes.as_slice()),
    };

    let mut current_offset = start_offset;
    let mut current_line = parsed.line_number;
    let mut result = FileScanResult {
        events_inserted: 0,
        errors: 0,
        events: Vec::new(),
    };
    let mut last_error = None;

    for line_bytes in complete_bytes.split_inclusive(|byte| *byte == b'\n') {
        let source_offset = current_offset;
        current_offset += line_bytes.len() as u64;
        current_line += 1;

        let line = String::from_utf8_lossy(line_bytes);
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.trim().is_empty() {
            continue;
        }

        match parse_line(trimmed, path, source_offset, current_line) {
            Ok(events) => {
                for event in events {
                    if db.insert_invocation(&event)? {
                        result.events_inserted += 1;
                        result.events.push(event);
                    }
                }
            }
            Err(err) => {
                result.errors += 1;
                last_error = Some(format!("line {current_line}: {err}"));
            }
        }
    }

    parsed.file_size = file_size;
    parsed.modified_at = modified_at;
    parsed.byte_offset = current_offset;
    parsed.line_number = current_line;
    parsed.partial_line = String::from_utf8_lossy(partial_bytes).into_owned();
    parsed.canonical_path = Some(normalize_for_compare(path).to_string_lossy().into_owned());
    parsed.fingerprint = Some(format!("size:{file_size}:offset:{current_offset}"));
    parsed.last_error = last_error;
    db.upsert_parsed_file(&parsed)?;

    Ok(result)
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
}
