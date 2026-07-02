use crate::config::Config;
use crate::error::Result;
use crate::paths::{normalize_for_compare, path_to_key};
use serde::Serialize;
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
}

#[derive(Debug, Clone, Default)]
pub struct SkillRegistry {
    skills: Vec<SkillInfo>,
    by_skill_path: HashMap<String, usize>,
}

impl SkillRegistry {
    pub fn scan(config: &Config) -> Result<Self> {
        let mut registry = Self::default();
        registry.scan_root(&config.codex_home.join("skills"), "user", None)?;
        registry.scan_root(&config.agents_home.join("skills"), "agent", None)?;
        registry.scan_plugin_cache(&config.codex_home.join("plugins").join("cache"))?;
        Ok(registry)
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
            });
        }
        Ok(())
    }

    fn scan_plugin_cache(&mut self, root: &Path) -> Result<()> {
        if !root.exists() {
            return Ok(());
        }
        for entry in WalkDir::new(root).follow_links(false) {
            let entry = entry?;
            if !entry.file_type().is_file() || entry.file_name() != "SKILL.md" {
                continue;
            }
            if !entry.path().components().any(|c| c.as_os_str() == "skills") {
                continue;
            }
            let plugin_id = plugin_id_from_cache_path(root, entry.path());
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
                scope: "plugin".to_string(),
                plugin_id,
            });
        }
        Ok(())
    }

    fn push(&mut self, skill: SkillInfo) {
        let key = path_to_key(&skill.skill_path);
        if self.by_skill_path.contains_key(&key) {
            return;
        }
        self.by_skill_path.insert(key, self.skills.len());
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
