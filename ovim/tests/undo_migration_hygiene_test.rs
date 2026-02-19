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
fn test_pending_semantic_change_apis_are_removed() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("ovim crate should live under repo root");
    let core_src = repo_root.join("ovim-core").join("src");

    let mut files = Vec::new();
    collect_rs_files(&core_src, &mut files);

    let mut legacy_hits = Vec::new();

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
            if line.contains("set_pending_semantic_change(")
                || line.contains("take_pending_semantic_change(")
                || line.contains("pending_semantic_change")
                || line.contains("PendingSemanticChange")
            {
                legacy_hits.push(format!("{}:{}", relative, line_number + 1));
            }
        }
    }

    assert!(
        legacy_hits.is_empty(),
        "Legacy pending semantic-change APIs/state should be removed, found:\n{}",
        legacy_hits.join("\n")
    );
}

#[test]
fn test_char_awaiting_commands_stay_on_input_state_path() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("ovim crate should live under repo root");

    let normal_mod = repo_root
        .join("ovim-core/src/editor/input/normal/mod.rs")
        .to_string_lossy()
        .to_string();
    let visual_mode = repo_root
        .join("ovim-core/src/editor/input/visual_mode.rs")
        .to_string_lossy()
        .to_string();
    let pending_commands = repo_root
        .join("ovim-core/src/editor/input/normal/pending_commands.rs")
        .to_string_lossy()
        .to_string();

    let normal_mod_src = fs::read_to_string(&normal_mod).expect("read normal/mod.rs");
    let visual_mode_src = fs::read_to_string(&visual_mode).expect("read visual_mode.rs");
    let pending_src = fs::read_to_string(&pending_commands).expect("read pending_commands.rs");

    // r/m/'/` should use InputState::AwaitingChar in normal mode, not pending_command.
    for forbidden in [
        "set_pending_command('r')",
        "set_pending_command('m')",
        "set_pending_command('\\'')",
        "set_pending_command('`')",
    ] {
        assert!(
            !normal_mod_src.contains(forbidden),
            "Found legacy pending_command setup for awaiting-char command in normal/mod.rs: {}",
            forbidden
        );
    }

    // Visual-block r{char} should use InputState::AwaitingChar::Replace.
    assert!(
        !visual_mode_src.contains("set_pending_command('r')"),
        "Found legacy pending_command setup for visual-block replace in visual_mode.rs"
    );

    // pending_commands.rs should not carry first-key handlers for these commands anymore.
    for forbidden in [
        "('r', KeyCode::Char(",
        "('m', KeyCode::Char(",
        "('\'', KeyCode::Char(",
        "('`', KeyCode::Char(",
    ] {
        assert!(
            !pending_src.contains(forbidden),
            "Found legacy pending_commands arm for awaiting-char command: {}",
            forbidden
        );
    }

    // visual_mode.rs should not carry a pending-command replace arm either.
    assert!(
        !visual_mode_src.contains("('r', KeyCode::Char("),
        "Found legacy pending-command replace arm in visual_mode.rs"
    );
}
