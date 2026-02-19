use std::fs;
use std::path::{Path, PathBuf};

fn collect_files(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if matches!(name, ".git" | "target" | "node_modules") {
                continue;
            }
            collect_files(&path, out);
            continue;
        }

        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if matches!(
            ext,
            "rs" | "md" | "toml" | "sh" | "lua" | "py" | "yaml" | "yml"
        ) || matches!(file_name, "ovim-ctl" | "send-cmd")
        {
            out.push(path);
        }
    }
}

fn contains_machine_specific_path(line: &str) -> bool {
    let forbidden_literals = [
        "/Users/adrian/",
        "/home/adrian/",
        "C:\\Users\\adrian\\",
        "~/Personal/",
    ];
    if forbidden_literals
        .iter()
        .any(|needle| line.contains(needle))
    {
        return true;
    }

    // Catch hardcoded macOS home paths like /Users/<name>/... while allowing placeholders.
    if let Some(start) = line.find("/Users/") {
        let rest = &line[start + "/Users/".len()..];
        if let Some((user, _tail)) = rest.split_once('/') {
            let is_placeholder = matches!(
                user,
                "user" | "USER" | "username" | "<user>" | "<username>" | "{user}" | "{username}"
            );
            let looks_like_user = !user.is_empty()
                && user
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'));
            if looks_like_user && !is_placeholder {
                return true;
            }
        }
    }

    false
}

#[test]
fn test_repo_has_no_machine_specific_path_leaks() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("ovim crate should live under repo root");

    let mut files = Vec::new();
    collect_files(repo_root, &mut files);

    let mut hits = Vec::new();

    for path in files {
        if path.file_name().and_then(|n| n.to_str()) == Some("hygiene_paths_test.rs") {
            continue;
        }
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        for (idx, line) in content.lines().enumerate() {
            if contains_machine_specific_path(line) {
                hits.push(format!(
                    "{}:{} contains machine-specific path: {}",
                    path.display(),
                    idx + 1,
                    line.trim()
                ));
            }
        }
    }

    assert!(
        hits.is_empty(),
        "Found machine-specific path leaks:\n{}",
        hits.join("\n")
    );
}
