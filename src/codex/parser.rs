use crate::codex::command_detection::detect_implicit_skills;
use crate::codex::registry::{SkillInfo, SkillRegistry};
use crate::events::{SessionState, SkillInvocation};
use crate::paths::resolve_command_path;
use crate::tags::extract_tag;
use chrono::Utc;
use serde_json::Value;
use std::path::Path;

pub fn parse_line(
    line: &str,
    source_file: &Path,
    source_offset: u64,
    source_line: u64,
    registry: &SkillRegistry,
    state: &mut SessionState,
) -> Result<Vec<SkillInvocation>, serde_json::Error> {
    let value: Value = serde_json::from_str(line)?;
    update_state(&value, state);

    let Some(event_type) = value.get("type").and_then(Value::as_str) else {
        return Ok(Vec::new());
    };
    if event_type != "response_item" {
        return Ok(Vec::new());
    }

    let timestamp = value
        .get("timestamp")
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .unwrap_or_else(|| Utc::now().to_rfc3339());

    let Some(payload) = value.get("payload") else {
        return Ok(Vec::new());
    };

    let turn_id = payload_turn_id(payload).or_else(|| state.turn_id.clone());
    let session_id = payload_session_id(payload).or_else(|| state.session_id.clone());

    match payload.get("type").and_then(Value::as_str) {
        Some("message") => Ok(parse_explicit_message(
            payload,
            source_file,
            source_offset,
            source_line,
            registry,
            session_id,
            turn_id,
            timestamp,
        )),
        Some("function_call") if is_exec_command_tool_call(payload) => Ok(parse_function_call(
            payload,
            source_file,
            source_offset,
            source_line,
            registry,
            state,
            session_id,
            turn_id,
            timestamp,
        )),
        Some("function_call") => Ok(Vec::new()),
        _ => Ok(Vec::new()),
    }
}

fn update_state(value: &Value, state: &mut SessionState) {
    match value.get("type").and_then(Value::as_str) {
        Some("session_meta") => {
            if let Some(payload) = value.get("payload") {
                state.session_id = payload
                    .get("session_id")
                    .or_else(|| payload.get("id"))
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
                    .or_else(|| state.session_id.clone());
            }
        }
        Some("turn_context") => {
            if let Some(payload) = value.get("payload") {
                state.turn_id = payload
                    .get("turn_id")
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
                    .or_else(|| state.turn_id.clone());
                state.cwd = payload
                    .get("cwd")
                    .and_then(Value::as_str)
                    .map(std::path::PathBuf::from)
                    .or_else(|| state.cwd.clone());
            }
        }
        _ => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn parse_explicit_message(
    payload: &Value,
    source_file: &Path,
    source_offset: u64,
    source_line: u64,
    registry: &SkillRegistry,
    session_id: Option<String>,
    turn_id: Option<String>,
    timestamp: String,
) -> Vec<SkillInvocation> {
    if payload.get("role").and_then(Value::as_str) != Some("user") {
        return Vec::new();
    }

    let mut events = Vec::new();
    for text in content_texts(payload) {
        for block in skill_blocks(text) {
            let Some(path) = extract_tag(block, "path") else {
                continue;
            };
            let command_path = resolve_command_path(&path, None);
            let Some(skill) = registry.match_skill_file(&command_path) else {
                continue;
            };
            events.push(event_for_skill(
                "explicit_skill_injection",
                "explicit",
                skill,
                source_file,
                source_offset,
                source_line,
                None,
                session_id.clone(),
                turn_id.clone(),
                timestamp.clone(),
                1.0,
            ));
        }
    }
    events
}

#[allow(clippy::too_many_arguments)]
fn parse_function_call(
    payload: &Value,
    source_file: &Path,
    source_offset: u64,
    source_line: u64,
    registry: &SkillRegistry,
    state: &SessionState,
    session_id: Option<String>,
    turn_id: Option<String>,
    timestamp: String,
) -> Vec<SkillInvocation> {
    let Some(command) = command_from_arguments(payload) else {
        return Vec::new();
    };
    let call_id = payload
        .get("call_id")
        .or_else(|| payload.get("id"))
        .and_then(Value::as_str)
        .map(ToString::to_string);

    detect_implicit_skills(&command, state.cwd.as_deref(), registry)
        .into_iter()
        .map(|skill| {
            event_for_skill(
                "implicit_skill_command",
                "implicit",
                skill,
                source_file,
                source_offset,
                source_line,
                call_id.clone(),
                session_id.clone(),
                turn_id.clone(),
                timestamp.clone(),
                0.9,
            )
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn event_for_skill(
    trigger_source: &str,
    invocation_type: &str,
    skill: &SkillInfo,
    source_file: &Path,
    source_offset: u64,
    source_line: u64,
    tool_call_id: Option<String>,
    session_id: Option<String>,
    turn_id: Option<String>,
    timestamp: String,
    confidence: f64,
) -> SkillInvocation {
    SkillInvocation::new(
        trigger_source,
        invocation_type,
        skill.name.clone(),
        Some(skill.skill_path.to_string_lossy().into_owned()),
        Some(skill.scope.clone()),
        skill.plugin_id.clone(),
        session_id,
        turn_id,
        source_file,
        source_offset,
        source_line,
        tool_call_id,
        timestamp,
        confidence,
    )
}

fn content_texts(payload: &Value) -> Vec<&str> {
    let Some(content) = payload.get("content") else {
        return Vec::new();
    };
    match content {
        Value::String(text) => vec![text.as_str()],
        Value::Array(items) => items
            .iter()
            .filter_map(|item| {
                item.get("text")
                    .or_else(|| item.get("input_text"))
                    .and_then(Value::as_str)
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn skill_blocks(text: &str) -> Vec<&str> {
    let mut blocks = Vec::new();
    let mut remainder = text;
    while let Some(start) = remainder.find("<skill>") {
        let after_start = &remainder[start..];
        let Some(end) = after_start.find("</skill>") else {
            break;
        };
        let block = &after_start[..end + "</skill>".len()];
        blocks.push(block);
        remainder = &after_start[end + "</skill>".len()..];
    }
    blocks
}

fn command_from_arguments(payload: &Value) -> Option<String> {
    let arguments = payload.get("arguments")?;
    let object = match arguments {
        Value::String(text) => serde_json::from_str::<Value>(text).ok()?,
        Value::Object(_) => arguments.clone(),
        _ => return None,
    };
    object
        .get("cmd")
        .or_else(|| object.get("command"))
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

fn is_exec_command_tool_call(payload: &Value) -> bool {
    payload.get("name").and_then(Value::as_str) == Some("exec_command")
}

fn payload_turn_id(payload: &Value) -> Option<String> {
    payload
        .get("turn_id")
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .or_else(|| {
            payload
                .get("internal_chat_message_metadata_passthrough")
                .and_then(|metadata| metadata.get("turn_id"))
                .and_then(Value::as_str)
                .map(ToString::to_string)
        })
}

fn payload_session_id(payload: &Value) -> Option<String> {
    payload
        .get("session_id")
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex::registry::{SkillInfo, SkillRegistry};
    use std::path::PathBuf;

    fn registry() -> SkillRegistry {
        SkillRegistry::from_skills(vec![SkillInfo {
            name: "diagnose".to_string(),
            skill_path: PathBuf::from("/tmp/skills/diagnose/SKILL.md"),
            skill_dir: PathBuf::from("/tmp/skills/diagnose"),
            scripts_dir: PathBuf::from("/tmp/skills/diagnose/scripts"),
            scope: "agent".to_string(),
            plugin_id: None,
            plugin_name: None,
        }])
    }

    #[test]
    fn parses_explicit_skill_message() {
        let line = r#"{"timestamp":"2026-07-02T00:00:00Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<skill>\n<name>diagnose</name>\n<path>/tmp/skills/diagnose/SKILL.md</path>\n</skill>"}]}}"#;
        let mut state = SessionState::default();
        let events = parse_line(
            line,
            Path::new("/tmp/session.jsonl"),
            0,
            1,
            &registry(),
            &mut state,
        )
        .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].invocation_type, "explicit");
    }

    #[test]
    fn ignores_plain_text_mention() {
        let line = r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"please use diagnose"}]}}"#;
        let mut state = SessionState::default();
        let events = parse_line(
            line,
            Path::new("/tmp/session.jsonl"),
            0,
            1,
            &registry(),
            &mut state,
        )
        .unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn parses_implicit_skill_from_exec_command() {
        let line = r#"{"timestamp":"2026-07-02T00:00:01Z","type":"response_item","payload":{"type":"function_call","name":"exec_command","call_id":"call_1","arguments":"{\"cmd\":\"sed -n '1,120p' /tmp/skills/diagnose/SKILL.md\"}"}}"#;
        let mut state = SessionState::default();
        let events = parse_line(
            line,
            Path::new("/tmp/session.jsonl"),
            0,
            1,
            &registry(),
            &mut state,
        )
        .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].invocation_type, "implicit");
        assert_eq!(events[0].tool_call_id.as_deref(), Some("call_1"));
    }

    #[test]
    fn ignores_non_exec_function_call_with_command_argument() {
        let line = r#"{"timestamp":"2026-07-02T00:00:01Z","type":"response_item","payload":{"type":"function_call","name":"other_tool","call_id":"call_1","arguments":"{\"command\":\"sed -n '1,120p' /tmp/skills/diagnose/SKILL.md\"}"}}"#;
        let mut state = SessionState::default();
        let events = parse_line(
            line,
            Path::new("/tmp/session.jsonl"),
            0,
            1,
            &registry(),
            &mut state,
        )
        .unwrap();
        assert!(events.is_empty());
    }
}
