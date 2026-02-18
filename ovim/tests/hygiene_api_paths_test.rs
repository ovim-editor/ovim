use std::fs;
use std::path::{Path, PathBuf};

fn collect_shell_scripts(root: &Path, out: &mut Vec<PathBuf>) {
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
            collect_shell_scripts(&path, out);
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
fn test_shell_scripts_use_versioned_api_paths() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("ovim crate should live under repo root");

    let mut files = Vec::new();
    collect_shell_scripts(repo_root, &mut files);

    let forbidden = [
        "$API_URL/mode",
        "$API_URL/buffer",
        "$API_URL/cursor",
        "$API_URL/keys",
        "$API_URL/command",
        "$API_URL/snapshot",
        "$API_URL/health",
        "$API_URL/render",
        "$API_URL/lsp/status",
        "127.0.0.1:$PORT/mode",
        "127.0.0.1:$PORT/buffer",
        "127.0.0.1:$PORT/cursor",
        "127.0.0.1:$PORT/keys",
        "127.0.0.1:$PORT/command",
        "127.0.0.1:$PORT/snapshot",
        "127.0.0.1:$PORT/health",
        "127.0.0.1:$PORT/render",
        "127.0.0.1:$PORT/lsp/status",
    ];

    let mut hits = Vec::new();

    for path in files {
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        for (idx, line) in content.lines().enumerate() {
            if forbidden.iter().any(|needle| line.contains(needle)) {
                hits.push(format!(
                    "{}:{} uses unversioned API path: {}",
                    path.display(),
                    idx + 1,
                    line.trim()
                ));
            }
        }
    }

    assert!(
        hits.is_empty(),
        "Found unversioned API paths in shell scripts:\n{}",
        hits.join("\n")
    );
}
