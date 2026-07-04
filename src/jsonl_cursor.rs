use crate::db::{Database, ParsedFile};
use crate::error::Result;
use crate::events::SkillInvocation;
use crate::paths::normalize_for_compare;
use chrono::{DateTime, Utc};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

pub struct FileScanResult {
    pub events_inserted: u64,
    pub errors: u64,
    pub events: Vec<SkillInvocation>,
}

pub fn scan_file<State, InitState, ParseLine, PersistState>(
    db: &mut Database,
    path: &Path,
    rescan: bool,
    init_state: InitState,
    mut parse_line: ParseLine,
    persist_state: PersistState,
) -> Result<FileScanResult>
where
    InitState: FnOnce(&ParsedFile) -> State,
    ParseLine: FnMut(
        &str,
        &Path,
        u64,
        u64,
        &mut State,
    ) -> std::result::Result<Vec<SkillInvocation>, serde_json::Error>,
    PersistState: FnOnce(&mut ParsedFile, State),
{
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
    let mut state = init_state(&parsed);
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

        match parse_line(trimmed, path, source_offset, current_line, &mut state) {
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
    persist_state(&mut parsed, state);
    db.upsert_parsed_file(&parsed)?;

    Ok(result)
}
