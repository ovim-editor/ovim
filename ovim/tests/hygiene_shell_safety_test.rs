use std::fs;
use std::path::{Path, PathBuf};

fn collect_shell_files(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if matches!(name, ".git" | "target") {
                continue;
            }
            collect_shell_files(&path, out);
            continue;
        }

        if path.extension().and_then(|e| e.to_str()) == Some("sh")
            || path.file_name().and_then(|n| n.to_str()) == Some("ovim-ctl")
            || path.file_name().and_then(|n| n.to_str()) == Some("send-cmd")
        {
            out.push(path);
        }
    }
}

#[test]
fn test_shell_scripts_avoid_broad_tmp_deletes() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("ovim crate should live under repo root");

    let mut files = Vec::new();
    collect_shell_files(repo_root, &mut files);

    let forbidden_patterns = [
        "rm -rf /tmp/*",
        "rm -rf /tmp/",
        "rm -rf /tmp/ovim_test_*",
        "rm -rf /tmp/ovim-*",
    ];

    let mut violations = Vec::new();
    for path in files {
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };

        for (idx, line) in content.lines().enumerate() {
            let trimmed = line.trim();
            if forbidden_patterns
                .iter()
                .any(|pattern| trimmed.contains(pattern))
            {
                violations.push(format!(
                    "{}:{} has broad /tmp delete: {}",
                    path.display(),
                    idx + 1,
                    trimmed
                ));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "Found broad tmp delete patterns:\n{}",
        violations.join("\n")
    );
}
