use crate::events::SkillInvocation;
use chrono::Utc;
use serde_json::Value;
use std::path::Path;

pub fn parse_line(
    line: &str,
    source_file: &Path,
    source_offset: u64,
    source_line: u64,
) -> Result<Vec<SkillInvocation>, serde_json::Error> {
    let value: Value = serde_json::from_str(line)?;
    let timestamp = value
        .get("timestamp")
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .unwrap_or_else(|| Utc::now().to_rfc3339());
    let session_id = value
        .get("sessionId")
        .and_then(Value::as_str)
        .map(ToString::to_string);

    let Some(message) = value.get("message") else {
        return Ok(Vec::new());
    };
    if message.get("role").and_then(Value::as_str) != Some("assistant") {
        return Ok(Vec::new());
    }

    let Some(content) = message.get("content").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };

    let mut events = Vec::new();
    for item in content {
        if item.get("type").and_then(Value::as_str) != Some("tool_use") {
            continue;
        }
        if item.get("name").and_then(Value::as_str) != Some("Skill") {
            continue;
        }
        let Some(skill_name) = item
            .get("input")
            .and_then(|input| input.get("skill"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|skill| !skill.is_empty())
            .map(ToString::to_string)
        else {
            continue;
        };
        let tool_call_id = item
            .get("id")
            .and_then(Value::as_str)
            .map(ToString::to_string);
        events.push(SkillInvocation::new_for_runtime(
            "claude_code",
            "claude_project_jsonl",
            "claude_skill_tool",
            "skill",
            skill_name,
            None,
            None,
            None,
            session_id.clone(),
            None,
            source_file,
            source_offset,
            source_line,
            tool_call_id,
            timestamp.clone(),
            1.0,
        ));
    }

    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_claude_skill_tool_use() {
        let line = r#"{"type":"assistant","timestamp":"2026-07-02T00:00:00Z","sessionId":"session_1","message":{"role":"assistant","content":[{"type":"tool_use","id":"toolu_1","name":"Skill","input":{"skill":"think","args":"do not persist me"}}]}}"#;
        let events = parse_line(line, Path::new("/tmp/claude.jsonl"), 0, 1).unwrap();

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].runtime, "claude_code");
        assert_eq!(events[0].source, "claude_project_jsonl");
        assert_eq!(events[0].trigger_source, "claude_skill_tool");
        assert_eq!(events[0].invocation_type, "skill");
        assert_eq!(events[0].skill_name, "think");
        assert_eq!(events[0].tool_call_id.as_deref(), Some("toolu_1"));
        assert_eq!(events[0].session_id.as_deref(), Some("session_1"));
    }

    #[test]
    fn ignores_plain_skill_mentions() {
        let line = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"I should use a Skill here."}]}}"#;
        let events = parse_line(line, Path::new("/tmp/claude.jsonl"), 0, 1).unwrap();

        assert!(events.is_empty());
    }
}
