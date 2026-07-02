use std::path::{Component, Path, PathBuf};

pub fn expand_tilde(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if s == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    }
    if let Some(rest) = s.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    path.to_path_buf()
}

pub fn normalize_for_compare(path: &Path) -> PathBuf {
    let expanded = expand_tilde(path);
    if let Ok(canonical) = expanded.canonicalize() {
        return canonical;
    }
    lexical_normalize(&expanded)
}

pub fn resolve_command_path(token: &str, cwd: Option<&Path>) -> PathBuf {
    let path = expand_tilde(&PathBuf::from(token));
    if path.is_absolute() {
        normalize_for_compare(&path)
    } else if let Some(cwd) = cwd {
        normalize_for_compare(&cwd.join(path))
    } else {
        normalize_for_compare(&path)
    }
}

pub fn path_to_key(path: &Path) -> String {
    normalize_for_compare(path).to_string_lossy().into_owned()
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                result.pop();
            }
            other => result.push(other.as_os_str()),
        }
    }
    result
}
