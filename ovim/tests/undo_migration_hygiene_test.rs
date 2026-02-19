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
    let mut total_hits = 0usize;

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
            total_hits += 1;
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
    assert!(
        total_hits <= 5,
        "Expected add_change callsites to stay at or below 5, found {}",
        total_hits
    );
}

#[test]
fn test_pending_semantic_change_runtime_paths_are_constrained() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("ovim crate should live under repo root");
    let core_src = repo_root.join("ovim-core").join("src");

    let mut files = Vec::new();
    collect_rs_files(&core_src, &mut files);

    let mut set_hits = Vec::new();
    let mut take_hits = Vec::new();

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
            if line.contains("set_pending_semantic_change(") {
                set_hits.push(format!("{}:{}", relative, line_number + 1));
            }
            if line.contains("take_pending_semantic_change(") {
                take_hits.push(format!("{}:{}", relative, line_number + 1));
            }
        }
    }

    assert_eq!(
        set_hits.len(),
        1,
        "Expected exactly one set_pending_semantic_change site (definition only), found:\n{}",
        set_hits.join("\n")
    );
    assert!(
        set_hits[0].starts_with("ovim-core/src/editor/mod.rs:"),
        "set_pending_semantic_change should only be defined in editor/mod.rs, found:\n{}",
        set_hits.join("\n")
    );

    assert_eq!(
        take_hits.len(),
        2,
        "Expected exactly two take_pending_semantic_change sites (definition + insert-mode clear), found:\n{}",
        take_hits.join("\n")
    );
    for hit in &take_hits {
        let allowed = hit.starts_with("ovim-core/src/editor/mod.rs:")
            || hit.starts_with("ovim-core/src/editor/input/insert_mode.rs:");
        assert!(
            allowed,
            "Unexpected take_pending_semantic_change callsite: {}",
            hit
        );
    }
}
