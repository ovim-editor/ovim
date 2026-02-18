use std::fs;
use std::path::{Path, PathBuf};

fn collect_rust_test_files(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rust_test_files(&path, out);
            continue;
        }

        if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

#[test]
fn test_ignored_tests_have_explicit_reason() {
    let tests_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut files = Vec::new();
    collect_rust_test_files(&tests_root, &mut files);

    let mut violations = Vec::new();

    for path in files {
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };

        for (line_idx, line) in content.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.starts_with("#[ignore]") {
                violations.push(format!(
                    "{}:{} has bare #[ignore]; use #[ignore = \"reason\"]",
                    path.display(),
                    line_idx + 1
                ));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "Found ignored tests without reasons:\n{}",
        violations.join("\n")
    );
}
