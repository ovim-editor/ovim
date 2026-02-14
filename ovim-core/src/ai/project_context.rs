use super::config::ProjectContextConfig;
use std::path::{Path, PathBuf};

/// Load project context files according to `config`, starting from the
/// directory containing `file_path` (or the current working directory).
///
/// Returns the concatenated contents of all matched files, separated by
/// `\n---\n`. Returns an empty string when no files are found or the
/// feature is disabled.
pub fn load_project_context(config: &ProjectContextConfig, file_path: Option<&str>) -> String {
    if !config.enabled {
        return String::new();
    }

    let start_dir = file_path
        .map(Path::new)
        .and_then(|p| p.parent())
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));

    let repo_root = find_repo_root(&start_dir).unwrap_or_else(|| start_dir.clone());

    let dirs = if config.hierarchical {
        collect_dirs_root_to_start(&repo_root, &start_dir)
    } else {
        vec![start_dir]
    };

    let mut parts: Vec<String> = Vec::new();
    for dir in &dirs {
        for file_name in &config.files {
            let path = dir.join(file_name);
            if let Ok(content) = std::fs::read_to_string(&path) {
                let trimmed = content.trim();
                if !trimmed.is_empty() {
                    parts.push(trimmed.to_string());
                }
            }
        }
    }

    if parts.is_empty() {
        return String::new();
    }

    let joined = parts.join("\n---\n");

    // Rough char-to-token budget: budget * 4 chars
    let char_limit = config.budget.saturating_mul(4);
    if joined.len() > char_limit {
        let mut truncated = joined[..char_limit].to_string();
        truncated.push_str("\n[project context truncated]");
        truncated
    } else {
        joined
    }
}

/// Walk up from `start` looking for a `.git` directory.
fn find_repo_root(start: &Path) -> Option<PathBuf> {
    let mut current = start.to_path_buf();
    loop {
        if current.join(".git").exists() {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
}

/// Collect directories from `root` down to `start` (inclusive), in order.
/// If `start` is not under `root`, returns just `[start]`.
fn collect_dirs_root_to_start(root: &Path, start: &Path) -> Vec<PathBuf> {
    let Ok(root_canon) = std::fs::canonicalize(root) else {
        return vec![start.to_path_buf()];
    };
    let Ok(start_canon) = std::fs::canonicalize(start) else {
        return vec![start.to_path_buf()];
    };

    if !start_canon.starts_with(&root_canon) {
        return vec![start.to_path_buf()];
    }

    let mut dirs = vec![root_canon.clone()];
    let suffix = start_canon
        .strip_prefix(&root_canon)
        .unwrap_or(Path::new(""));
    let mut accum = root_canon;
    for component in suffix.components() {
        accum = accum.join(component);
        dirs.push(accum.clone());
    }
    dirs
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_load_project_context_finds_file() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(".ovim.md"), "# Project rules").unwrap();

        let config = ProjectContextConfig {
            files: vec![".ovim.md".to_string()],
            hierarchical: false,
            budget: 2000,
            enabled: true,
        };
        let result = load_project_context(&config, Some(dir.path().join("test.rs").to_str().unwrap()));
        assert!(result.contains("# Project rules"), "got: {result}");
    }

    #[test]
    fn test_load_project_context_hierarchical() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        fs::create_dir(&sub).unwrap();
        fs::write(dir.path().join(".ovim.md"), "root context").unwrap();
        fs::write(sub.join(".ovim.md"), "sub context").unwrap();

        let config = ProjectContextConfig {
            files: vec![".ovim.md".to_string()],
            hierarchical: true,
            budget: 2000,
            enabled: true,
        };
        // Create a .git dir so repo root is detected
        fs::create_dir(dir.path().join(".git")).unwrap();

        let file_in_sub = sub.join("test.rs");
        let result = load_project_context(&config, Some(file_in_sub.to_str().unwrap()));
        // Root should come before sub
        let root_pos = result.find("root context").expect("root context missing");
        let sub_pos = result.find("sub context").expect("sub context missing");
        assert!(root_pos < sub_pos, "root should appear before sub");
        assert!(result.contains("---"), "sections should be separated by ---");
    }

    #[test]
    fn test_load_project_context_budget_truncation() {
        let dir = tempfile::tempdir().unwrap();
        // Budget=1 means 4 chars max
        let content = "This is a long project context that should be truncated";
        fs::write(dir.path().join(".ovim.md"), content).unwrap();

        let config = ProjectContextConfig {
            files: vec![".ovim.md".to_string()],
            hierarchical: false,
            budget: 1, // 4 chars
            enabled: true,
        };
        let result = load_project_context(&config, Some(dir.path().join("t.rs").to_str().unwrap()));
        assert!(result.contains("[project context truncated]"), "got: {result}");
        assert!(result.len() < content.len() + 50);
    }

    #[test]
    fn test_load_project_context_disabled() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(".ovim.md"), "should not appear").unwrap();

        let config = ProjectContextConfig {
            files: vec![".ovim.md".to_string()],
            hierarchical: false,
            budget: 2000,
            enabled: false,
        };
        let result = load_project_context(&config, Some(dir.path().join("t.rs").to_str().unwrap()));
        assert!(result.is_empty());
    }

    #[test]
    fn test_load_project_context_no_files() {
        let dir = tempfile::tempdir().unwrap();
        let config = ProjectContextConfig {
            files: vec![".ovim.md".to_string()],
            hierarchical: false,
            budget: 2000,
            enabled: true,
        };
        let result = load_project_context(&config, Some(dir.path().join("t.rs").to_str().unwrap()));
        assert!(result.is_empty());
    }
}
