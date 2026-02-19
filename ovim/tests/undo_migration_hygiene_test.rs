use std::fs;
use std::path::{Path, PathBuf};

fn collect_rs_files(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, out);
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

#[test]
fn test_add_change_callsites_are_infrastructure_only() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("ovim crate should live under repo root");
    let core_src = repo_root.join("ovim-core").join("src");

    let mut files = Vec::new();
    collect_rs_files(&core_src, &mut files);

    let allowed_files = ["ovim-core/src/change.rs", "ovim-core/src/editor/mod.rs"];
    let mut violations = Vec::new();

    for path in files {
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        let relative = path
            .strip_prefix(repo_root)
            .expect("core source file should be under repo root")
            .to_string_lossy()
            .replace('\\', "/");

        for (line_number, line) in content.lines().enumerate() {
            if !line.contains("add_change(") {
                continue;
            }
            if !allowed_files.contains(&relative.as_str()) {
                violations.push(format!(
                    "{}:{} contains forbidden add_change callsite: {}",
                    relative,
                    line_number + 1,
                    line.trim()
                ));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "Pattern A add_change callsites escaped infrastructure-only files:\n{}",
        violations.join("\n")
    );
}
