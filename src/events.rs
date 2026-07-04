use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SkillInvocation {
    pub id: String,
    pub runtime: String,
    pub source: String,
    pub trigger_source: String,
    pub invocation_type: String,
    pub skill_name: String,
    pub skill_path: Option<String>,
    pub skill_scope: Option<String>,
    pub plugin_id: Option<String>,
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub source_file: String,
    pub source_offset: u64,
    pub source_line: u64,
    pub tool_call_id: Option<String>,
    pub timestamp: String,
    pub confidence: f64,
}

impl SkillInvocation {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        trigger_source: &str,
        invocation_type: &str,
        skill_name: String,
        skill_path: Option<String>,
        skill_scope: Option<String>,
        plugin_id: Option<String>,
        session_id: Option<String>,
        turn_id: Option<String>,
        source_file: &Path,
        source_offset: u64,
        source_line: u64,
        tool_call_id: Option<String>,
        timestamp: String,
        confidence: f64,
    ) -> Self {
        Self::new_for_runtime(
            "codex",
            "codex_session_jsonl",
            trigger_source,
            invocation_type,
            skill_name,
            skill_path,
            skill_scope,
            plugin_id,
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

    #[allow(clippy::too_many_arguments)]
    pub fn new_for_runtime(
        runtime: &str,
        source: &str,
        trigger_source: &str,
        invocation_type: &str,
        skill_name: String,
        skill_path: Option<String>,
        skill_scope: Option<String>,
        plugin_id: Option<String>,
        session_id: Option<String>,
        turn_id: Option<String>,
        source_file: &Path,
        source_offset: u64,
        source_line: u64,
        tool_call_id: Option<String>,
        timestamp: String,
        confidence: f64,
    ) -> Self {
        let skill_key = skill_path.clone().unwrap_or_else(|| skill_name.clone());
        let tool_key = tool_call_id
            .as_ref()
            .map(|value| format!(":{value}"))
            .unwrap_or_default();
        let source_file_string = source_file.to_string_lossy().into_owned();
        Self {
            id: format!(
                "{runtime}:{source_file_string}:{source_offset}:{trigger_source}:{skill_key}{tool_key}"
            ),
            runtime: runtime.to_string(),
            source: source.to_string(),
            trigger_source: trigger_source.to_string(),
            invocation_type: invocation_type.to_string(),
            skill_name,
            skill_path,
            skill_scope,
            plugin_id,
            session_id,
            turn_id,
            source_file: source_file_string,
            source_offset,
            source_line,
            tool_call_id,
            timestamp,
            confidence,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct SessionState {
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub cwd: Option<std::path::PathBuf>,
}
