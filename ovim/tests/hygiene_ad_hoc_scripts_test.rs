use std::fs;
use std::path::Path;

#[test]
fn test_ad_hoc_shell_scripts_are_portable_and_strict() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("ovim crate should live under repo root");
    let ad_hoc_dir = repo_root.join("ad-hoc-tests");

    let entries = fs::read_dir(&ad_hoc_dir).expect("ad-hoc-tests should exist");
    let mut violations = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("sh") {
            continue;
        }

        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        let shebang = content.lines().next().unwrap_or_default().trim();

        if shebang == "#!/bin/bash" {
            violations.push(format!("{} uses non-portable bash shebang", path.display()));
        }

        if (shebang == "#!/usr/bin/env bash" || shebang == "#!/usr/bin/env zsh")
            && !content.contains("set -euo pipefail")
        {
            violations.push(format!(
                "{} missing strict mode: set -euo pipefail",
                path.display()
            ));
        }
    }

    assert!(
        violations.is_empty(),
        "ad-hoc script hygiene violations:\n{}",
        violations.join("\n")
    );
}
