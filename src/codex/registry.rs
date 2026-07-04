use crate::config::Config;
use crate::error::Result;
use crate::paths::{normalize_for_compare, path_to_key};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize)]
pub struct SkillInfo {
    pub name: String,
    pub skill_path: PathBuf,
    pub skill_dir: PathBuf,
    pub scripts_dir: PathBuf,
    pub scope: String,
    pub plugin_id: Option<String>,
    pub plugin_name: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct SkillRegistry {
    skills: Vec<SkillInfo>,
    by_skill_path: HashMap<String, usize>,
    by_claude_user_command: HashMap<String, usize>,
    by_claude_plugin_command: HashMap<String, usize>,
    diagnostics: Vec<String>,
}

impl SkillRegistry {
    pub fn scan(config: &Config) -> Result<Self> {
        let mut registry = Self::default();
        registry.scan_root(&config.codex_home.join("skills"), "user", None)?;
        registry.scan_root(&config.agents_home.join("skills"), "agent", None)?;
        registry.scan_plugin_cache(&config.codex_home.join("plugins").join("cache"), "plugin")?;
        registry.scan_root(&config.claude_home.join("skills"), "claude_user", None)?;
        registry.scan_plugin_cache(
            &config.claude_home.join("plugins").join("cache"),
            "claude_plugin",
        )?;
        Ok(registry)
    }

    pub fn match_claude_slash_command(&self, command: &str) -> Option<&SkillInfo> {
        command
            .contains(':')
            .then(|| self.by_claude_plugin_command.get(command))
            .flatten()
            .or_else(|| self.by_claude_user_command.get(command))
            .and_then(|index| self.skills.get(*index))
    }

    #[cfg(test)]
    pub fn from_skills(skills: Vec<SkillInfo>) -> Self {
        let mut registry = Self::default();
        for skill in skills {
            registry.push(skill);
        }
        registry
    }

    pub fn len(&self) -> usize {
        self.skills.len()
    }

    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }

    pub fn diagnostics(&self) -> &[String] {
        &self.diagnostics
    }

    pub fn match_skill_file(&self, path: &Path) -> Option<&SkillInfo> {
        let key = path_to_key(path);
        self.by_skill_path
            .get(&key)
            .and_then(|index| self.skills.get(*index))
    }

    pub fn match_script_path(&self, path: &Path) -> Option<&SkillInfo> {
        let normalized = normalize_for_compare(path);
        self.skills.iter().find(|skill| {
            let scripts_dir = normalize_for_compare(&skill.scripts_dir);
            normalized.starts_with(scripts_dir)
        })
    }

    fn scan_root(&mut self, root: &Path, scope: &str, plugin_id: Option<String>) -> Result<()> {
        if !root.exists() {
            return Ok(());
        }
        for entry in WalkDir::new(root).follow_links(false) {
            let entry = entry?;
            if !entry.file_type().is_file() || entry.file_name() != "SKILL.md" {
                continue;
            }
            let skill_path = normalize_for_compare(entry.path());
            let Some(skill_dir) = skill_path.parent().map(Path::to_path_buf) else {
                continue;
            };
            let name = read_skill_name(&skill_path).unwrap_or_else(|| {
                skill_dir
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "unknown".to_string())
            });
            self.push(SkillInfo {
                name,
                skill_path,
                scripts_dir: skill_dir.join("scripts"),
                skill_dir,
                scope: scope.to_string(),
                plugin_id: plugin_id.clone(),
                plugin_name: None,
            });
        }
        Ok(())
    }

    fn scan_plugin_cache(&mut self, root: &Path, scope: &str) -> Result<()> {
        if !root.exists() {
            return Ok(());
        }
        let mut plugin_names: HashMap<String, Option<String>> = HashMap::new();
        for entry in WalkDir::new(root).follow_links(false) {
            let entry = entry?;
            if !entry.file_type().is_file() || entry.file_name() != "SKILL.md" {
                continue;
            }
            if !entry.path().components().any(|c| c.as_os_str() == "skills") {
                continue;
            }
            let plugin_id = plugin_id_from_cache_path(root, entry.path());
            let plugin_name = if scope == "claude_plugin" {
                plugin_id.as_ref().and_then(|id| {
                    if !plugin_names.contains_key(id) {
                        let name = match plugin_name_for_id(root, id) {
                            Ok(name) => Some(name),
                            Err(diagnostic) => {
                                self.diagnostics.push(diagnostic);
                                None
                            }
                        };
                        plugin_names.insert(id.clone(), name);
                    }
                    plugin_names.get(id).cloned().flatten()
                })
            } else {
                None
            };
            let skill_path = normalize_for_compare(entry.path());
            let Some(skill_dir) = skill_path.parent().map(Path::to_path_buf) else {
                continue;
            };
            let name = read_skill_name(&skill_path).unwrap_or_else(|| {
                skill_dir
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "unknown".to_string())
            });
            self.push(SkillInfo {
                name,
                skill_path,
                scripts_dir: skill_dir.join("scripts"),
                skill_dir,
                scope: scope.to_string(),
                plugin_id,
                plugin_name,
            });
        }
        Ok(())
    }

    fn push(&mut self, skill: SkillInfo) {
        let key = path_to_key(&skill.skill_path);
        if self.by_skill_path.contains_key(&key) {
            return;
        }
        let index = self.skills.len();
        self.by_skill_path.insert(key, index);
        match skill.scope.as_str() {
            "agent" | "claude_user" => {
                self.by_claude_user_command
                    .entry(skill.name.clone())
                    .or_insert(index);
            }
            "claude_plugin" => {
                if let Some(plugin_name) = &skill.plugin_name {
                    self.by_claude_plugin_command
                        .entry(format!("{plugin_name}:{}", skill.name))
                        .or_insert(index);
                }
            }
            _ => {}
        }
        self.skills.push(skill);
    }
}

fn read_skill_name(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    if !text.starts_with("---") {
        return None;
    }
    let mut lines = text.lines();
    lines.next()?;
    for line in lines {
        if line.trim() == "---" {
            break;
        }
        let trimmed = line.trim();
        if let Some(value) = trimmed.strip_prefix("name:") {
            return Some(
                value
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'')
                    .to_string(),
            );
        }
    }
    None
}

fn plugin_id_from_cache_path(root: &Path, skill_path: &Path) -> Option<String> {
    let relative = skill_path.strip_prefix(root).ok()?;
    let mut parts = Vec::new();
    for component in relative.components() {
        let value = component.as_os_str().to_string_lossy();
        if value == "skills" {
            break;
        }
        parts.push(value.to_string());
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    }
}

fn plugin_name_for_id(root: &Path, plugin_id: &str) -> std::result::Result<String, String> {
    let manifest = root
        .join(plugin_id)
        .join(".claude-plugin")
        .join("plugin.json");
    let text = fs::read_to_string(&manifest)
        .map_err(|err| format!("could not read {}: {err}", manifest.display()))?;
    let value: Value = serde_json::from_str(&text)
        .map_err(|err| format!("could not parse {}: {err}", manifest.display()))?;
    value
        .get("name")
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .ok_or_else(|| format!("missing string name in {}", manifest.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use std::fs;

    fn skill(name: &str, scope: &str) -> SkillInfo {
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

    fn plugin_skill(plugin_name: &str, name: &str) -> SkillInfo {
        SkillInfo {
            name: name.to_string(),
            skill_path: PathBuf::from(format!("/tmp/plugins/{plugin_name}/skills/{name}/SKILL.md")),
            skill_dir: PathBuf::from(format!("/tmp/plugins/{plugin_name}/skills/{name}")),
            scripts_dir: PathBuf::from(format!("/tmp/plugins/{plugin_name}/skills/{name}/scripts")),
            scope: "claude_plugin".to_string(),
            plugin_id: Some(plugin_name.to_string()),
            plugin_name: Some(plugin_name.to_string()),
        }
    }

    #[test]
    fn claude_slash_command_falls_back_to_user_skill_with_colon_name() {
        let registry = SkillRegistry::from_skills(vec![skill("foo:bar", "agent")]);

        let matched = registry.match_claude_slash_command("foo:bar").unwrap();

        assert_eq!(matched.name, "foo:bar");
        assert_eq!(matched.scope, "agent");
    }

    #[test]
    fn claude_slash_command_prefers_plugin_match_when_present() {
        let registry =
            SkillRegistry::from_skills(vec![skill("foo:bar", "agent"), plugin_skill("foo", "bar")]);

        let matched = registry.match_claude_slash_command("foo:bar").unwrap();

        assert_eq!(matched.name, "bar");
        assert_eq!(matched.scope, "claude_plugin");
    }

    #[test]
    fn claude_plugin_manifest_errors_are_reported_as_diagnostics() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let skill_dir = root.join(".claude/plugins/cache/acme/bad/1.0/skills/review");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(skill_dir.join("SKILL.md"), "---\nname: review\n---\n").unwrap();
        let config = Config {
            codex_home: root.join(".codex"),
            claude_home: root.join(".claude"),
            agents_home: root.join(".agents"),
            db_path: root.join("skillscope.sqlite"),
        };

        let registry = SkillRegistry::scan(&config).unwrap();

        assert_eq!(registry.len(), 1);
        assert!(registry.match_claude_slash_command("bad:review").is_none());
        assert!(registry
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.contains("could not read")));
    }
}
