use crate::codex::registry::{SkillInfo, SkillRegistry};
use crate::paths::resolve_command_path;
use std::path::Path;

const RUNNERS: &[&str] = &[
    "python", "python3", "bash", "zsh", "sh", "node", "deno", "ruby", "perl", "pwsh",
];

const SCRIPT_EXTENSIONS: &[&str] = &["py", "sh", "js", "ts", "rb", "pl", "ps1"];

pub fn detect_implicit_skills<'a>(
    command: &str,
    cwd: Option<&Path>,
    registry: &'a SkillRegistry,
) -> Vec<&'a SkillInfo> {
    let tokens = shlex::split(command).unwrap_or_else(|| {
        command
            .split_whitespace()
            .map(ToString::to_string)
            .collect()
    });
    if tokens.is_empty() || registry.is_empty() {
        return Vec::new();
    }

    let mut matches = Vec::new();

    for token in tokens.iter().filter(|token| candidate_path_token(token)) {
        let path = resolve_command_path(token, cwd);
        if let Some(skill) = registry.match_skill_file(&path) {
            push_unique(&mut matches, skill);
        }
    }

    for script_token in script_candidates(&tokens) {
        let path = resolve_command_path(script_token, cwd);
        if let Some(skill) = registry.match_script_path(&path) {
            push_unique(&mut matches, skill);
        }
    }

    matches
}

fn push_unique<'a>(matches: &mut Vec<&'a SkillInfo>, skill: &'a SkillInfo) {
    if !matches
        .iter()
        .any(|existing| existing.skill_path == skill.skill_path)
    {
        matches.push(skill);
    }
}

fn candidate_path_token(token: &str) -> bool {
    if token.starts_with('-') || token.contains('*') {
        return false;
    }
    token.ends_with("SKILL.md") || token.contains("/SKILL.md")
}

fn script_candidates(tokens: &[String]) -> Vec<&str> {
    let mut candidates = Vec::new();
    let first = command_basename(&tokens[0]);
    if RUNNERS.contains(&first.as_str()) {
        for token in tokens.iter().skip(1) {
            if token.starts_with('-') {
                continue;
            }
            if has_script_extension(token) {
                candidates.push(token.as_str());
                break;
            }
        }
    }

    if has_script_extension(&tokens[0]) {
        candidates.push(tokens[0].as_str());
    }

    candidates
}

fn command_basename(command: &str) -> String {
    command
        .rsplit('/')
        .next()
        .unwrap_or(command)
        .to_ascii_lowercase()
}

fn has_script_extension(token: &str) -> bool {
    let path = Path::new(token);
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            SCRIPT_EXTENSIONS
                .iter()
                .any(|candidate| extension.eq_ignore_ascii_case(candidate))
        })
        .unwrap_or(false)
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
    fn detects_skill_md_read() {
        let registry = registry();
        let matches = detect_implicit_skills(
            "sed -n '1,120p' /tmp/skills/diagnose/SKILL.md",
            None,
            &registry,
        );
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].name, "diagnose");
    }

    #[test]
    fn ignores_generic_find() {
        let registry = registry();
        let matches = detect_implicit_skills("find /tmp -name SKILL.md", None, &registry);
        assert!(matches.is_empty());
    }

    #[test]
    fn detects_script_runner() {
        let registry = registry();
        let matches = detect_implicit_skills(
            "python3 /tmp/skills/diagnose/scripts/check.py",
            None,
            &registry,
        );
        assert_eq!(matches.len(), 1);
    }
}
