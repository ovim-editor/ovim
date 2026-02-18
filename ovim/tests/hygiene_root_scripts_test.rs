use std::fs;
use std::path::Path;

#[test]
fn test_root_scripts_use_portable_strict_bash() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("ovim crate should live under repo root");

    let scripts = [
        "ovim-ctl",
        "send-cmd",
        "build-and-sign.sh",
        "verify_edge_cases.sh",
        "test_vim_behavior.sh",
        "test_rust_analyzer.sh",
        "test_rust_analyzer_v2.sh",
        "test_rust_analyzer_v3.sh",
        "test_rust_analyzer_final.sh",
        "test_simple_ra.sh",
        "test_ra_workspace.sh",
    ];

    let mut violations = Vec::new();
    for script in scripts {
        let path = repo_root.join(script);
        let Ok(content) = fs::read_to_string(&path) else {
            violations.push(format!("missing script: {}", path.display()));
            continue;
        };

        let mut lines = content.lines();
        let shebang = lines.next().unwrap_or_default().trim();
        if shebang != "#!/usr/bin/env bash" {
            violations.push(format!(
                "{} has non-portable shebang: {}",
                path.display(),
                shebang
            ));
        }

        if !content.contains("set -euo pipefail") {
            violations.push(format!(
                "{} missing strict mode: set -euo pipefail",
                path.display()
            ));
        }
    }

    assert!(
        violations.is_empty(),
        "Root script hygiene violations:\n{}",
        violations.join("\n")
    );
}
