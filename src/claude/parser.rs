use crate::codex::registry::{SkillInfo, SkillRegistry};
use crate::events::SkillInvocation;
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
    if message.get("role").and_then(Value::as_str) == Some("user") {
        return Ok(parse_explicit_slash_command(
            message,
            source_file,
            source_offset,
            source_line,
            registry,
            session_id,
            timestamp,
        ));
    }
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

fn parse_explicit_slash_command(
    message: &Value,
    source_file: &Path,
    source_offset: u64,
    source_line: u64,
    registry: &SkillRegistry,
    session_id: Option<String>,
    timestamp: String,
) -> Vec<SkillInvocation> {
    let Some(command_name) = command_texts(message)
        .into_iter()
        .find_map(extract_command_name)
    else {
        return Vec::new();
    };
    let Some(skill) = registry.match_claude_slash_command(&command_name) else {
        return Vec::new();
    };

    vec![event_for_slash_skill(
        skill,
        source_file,
        source_offset,
        source_line,
        session_id,
        timestamp,
    )]
}

#[allow(clippy::too_many_arguments)]
fn event_for_slash_skill(
    skill: &SkillInfo,
    source_file: &Path,
    source_offset: u64,
    source_line: u64,
    session_id: Option<String>,
    timestamp: String,
) -> SkillInvocation {
    SkillInvocation::new_for_runtime(
        "claude_code",
        "claude_project_jsonl",
        "claude_slash_command",
        "explicit",
        skill.name.clone(),
        Some(skill.skill_path.to_string_lossy().into_owned()),
        Some(skill.scope.clone()),
        skill.plugin_id.clone(),
        session_id,
        None,
        source_file,
        source_offset,
        source_line,
        None,
        timestamp,
        1.0,
    )
}

fn command_texts(message: &Value) -> Vec<&str> {
    match message.get("content") {
        Some(Value::String(text)) => vec![text.as_str()],
        Some(Value::Array(items)) => items
            .iter()
            .filter(|item| item.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|item| item.get("text").and_then(Value::as_str))
            .collect(),
        _ => Vec::new(),
    }
}

fn extract_command_name(text: &str) -> Option<String> {
    extract_tag(text, "command-name")
        .and_then(normalize_command_name)
        .or_else(|| extract_tag(text, "command-message").and_then(normalize_command_name))
}

fn normalize_command_name(command: String) -> Option<String> {
    let command = command.trim().trim_start_matches('/').trim().to_string();
    if command.is_empty() {
        None
    } else {
        Some(command)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex::registry::SkillInfo;
    use std::path::PathBuf;

    fn plugin_skill(name: &str, plugin_name: &str, plugin_id: &str) -> SkillInfo {
        SkillInfo {
            name: name.to_string(),
            skill_path: PathBuf::from(format!(
                "/tmp/claude-plugins/{plugin_id}/skills/{name}/SKILL.md"
            )),
            skill_dir: PathBuf::from(format!("/tmp/claude-plugins/{plugin_id}/skills/{name}")),
            scripts_dir: PathBuf::from(format!(
                "/tmp/claude-plugins/{plugin_id}/skills/{name}/scripts"
            )),
            scope: "claude_plugin".to_string(),
            plugin_id: Some(plugin_id.to_string()),
            plugin_name: Some(plugin_name.to_string()),
        }
    }

    fn user_skill(name: &str, scope: &str) -> SkillInfo {
        SkillInfo {
            name: name.to_string(),
            skill_path: PathBuf::from(format!("/tmp/skills/{name}/SKILL.md")),
            skill_dir: PathBuf::from(format!("/tmp/skills/{name}")),
            scripts_dir: PathBuf::from(format!("/tmp/skills/{name}/scripts")),
            scope: scope.to_string(),
            plugin_id: None,
            plugin_name: None,
        }
    }

    #[test]
    fn parses_claude_skill_tool_use() {
        let registry = SkillRegistry::from_skills(vec![]);
        let line = r#"{"type":"assistant","timestamp":"2026-07-02T00:00:00Z","sessionId":"session_1","message":{"role":"assistant","content":[{"type":"tool_use","id":"toolu_1","name":"Skill","input":{"skill":"think","args":"do not persist me"}}]}}"#;
        let events = parse_line(line, Path::new("/tmp/claude.jsonl"), 0, 1, &registry).unwrap();

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
    fn parses_explicit_plugin_skill_slash_command() {
        let registry = SkillRegistry::from_skills(vec![plugin_skill(
            "rc-glab",
            "rc-shared",
            "rightcapital/rc-shared/1.0.4",
        )]);
        let line = r#"{"type":"user","timestamp":"2026-07-03T05:27:38.130Z","sessionId":"session_1","message":{"role":"user","content":"<command-message>rc-shared:rc-glab</command-message>\n<command-name>/rc-shared:rc-glab</command-name>\n<command-args>do not persist me</command-args>"}}"#;
        let events = parse_line(line, Path::new("/tmp/claude.jsonl"), 0, 1, &registry).unwrap();

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].runtime, "claude_code");
        assert_eq!(events[0].trigger_source, "claude_slash_command");
        assert_eq!(events[0].invocation_type, "explicit");
        assert_eq!(events[0].skill_name, "rc-glab");
        assert_eq!(events[0].skill_scope.as_deref(), Some("claude_plugin"));
        assert_eq!(
            events[0].plugin_id.as_deref(),
            Some("rightcapital/rc-shared/1.0.4")
        );
        assert_eq!(events[0].session_id.as_deref(), Some("session_1"));
    }

    #[test]
    fn parses_explicit_user_skill_slash_command() {
        let registry = SkillRegistry::from_skills(vec![user_skill("handoff", "agent")]);
        let line = r#"{"type":"user","timestamp":"2026-07-03T05:27:38.130Z","sessionId":"session_1","message":{"role":"user","content":"<command-message>handoff</command-message>\n<command-name>/handoff</command-name>"}}"#;
        let events = parse_line(line, Path::new("/tmp/claude.jsonl"), 0, 1, &registry).unwrap();

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].trigger_source, "claude_slash_command");
        assert_eq!(events[0].invocation_type, "explicit");
        assert_eq!(events[0].skill_name, "handoff");
        assert_eq!(events[0].skill_scope.as_deref(), Some("agent"));
    }

    #[test]
    fn ignores_non_skill_slash_command() {
        let registry = SkillRegistry::from_skills(vec![]);
        let line = r#"{"type":"user","message":{"role":"user","content":"<command-message>plugin</command-message>\n<command-name>/plugin</command-name>"}}"#;
        let events = parse_line(line, Path::new("/tmp/claude.jsonl"), 0, 1, &registry).unwrap();

        assert!(events.is_empty());
    }

    #[test]
    fn ignores_unregistered_skill_slash_command() {
        let registry = SkillRegistry::from_skills(vec![]);
        let line = r#"{"type":"user","message":{"role":"user","content":"<command-message>my-custom-prompt</command-message>\n<command-name>/my-custom-prompt</command-name>"}}"#;
        let events = parse_line(line, Path::new("/tmp/claude.jsonl"), 0, 1, &registry).unwrap();

        assert!(events.is_empty());
    }

    #[test]
    fn ignores_plain_skill_mentions() {
        let registry = SkillRegistry::from_skills(vec![]);
        let line = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"I should use a Skill here."}]}}"#;
        let events = parse_line(line, Path::new("/tmp/claude.jsonl"), 0, 1, &registry).unwrap();

        assert!(events.is_empty());
    }

    #[test]
    fn parses_slash_command_from_text_content_array() {
        let registry = SkillRegistry::from_skills(vec![user_skill("handoff", "agent")]);
        let line = r#"{"type":"user","timestamp":"2026-07-03T05:27:38.130Z","sessionId":"session_1","message":{"role":"user","content":[{"type":"text","text":"<command-name>/handoff</command-name>\n<command-message>handoff</command-message>"}]}}"#;
        let events = parse_line(line, Path::new("/tmp/claude.jsonl"), 0, 1, &registry).unwrap();

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].skill_name, "handoff");
    }

    #[test]
    fn ignores_command_tags_echoed_in_tool_result_arrays() {
        let registry = SkillRegistry::from_skills(vec![user_skill("handoff", "agent")]);
        let line = r#"{"type":"user","timestamp":"2026-07-03T05:27:38.130Z","sessionId":"session_1","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"tool_1","content":"<command-name>/handoff</command-name>\n<command-message>handoff</command-message>"}]}}"#;
        let events = parse_line(line, Path::new("/tmp/claude.jsonl"), 0, 1, &registry).unwrap();

        assert!(events.is_empty());
    }

    #[test]
    fn empty_command_name_falls_back_to_command_message() {
        let registry = SkillRegistry::from_skills(vec![user_skill("handoff", "agent")]);
        let line = r#"{"type":"user","timestamp":"2026-07-03T05:27:38.130Z","sessionId":"session_1","message":{"role":"user","content":"<command-name></command-name>\n<command-message>handoff</command-message>"}}"#;
        let events = parse_line(line, Path::new("/tmp/claude.jsonl"), 0, 1, &registry).unwrap();

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].skill_name, "handoff");
    }

    #[test]
    fn same_line_skill_tool_uses_have_distinct_ids() {
        let registry = SkillRegistry::from_skills(vec![]);
        let line = r#"{"type":"assistant","timestamp":"2026-07-02T00:00:00Z","sessionId":"session_1","message":{"role":"assistant","content":[{"type":"tool_use","id":"toolu_1","name":"Skill","input":{"skill":"think"}},{"type":"tool_use","id":"toolu_2","name":"Skill","input":{"skill":"think"}}]}}"#;
        let events = parse_line(line, Path::new("/tmp/claude.jsonl"), 42, 1, &registry).unwrap();

        assert_eq!(events.len(), 2);
        assert_ne!(events[0].id, events[1].id);
    }
}
