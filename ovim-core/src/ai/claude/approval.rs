//! Human-readable Claude approval details. The original input remains untouched
//! for the SDK response; both frontends consume this shared presentation.
use serde_json::Value;

pub(crate) fn permission_summary(name: &str, input: &Value, reason: &str) -> String {
    let description = input
        .get("description")
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty());
    let command = input
        .get("command")
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty());
    let mut sections = vec![format!("Claude Code: {name}")];
    if let Some(description) = description {
        sections.push(description.to_owned());
    }
    if let Some(command) = command {
        sections.push(format!("Command:\n{command}"));
    }
    if !reason.trim().is_empty() && description != Some(reason) {
        sections.push(format!("Reason: {reason}"));
    }
    // Preserve remaining arguments as labeled text, including unknown MCP
    // inputs, so presentation never hides the operation being approved.
    let mut details = Vec::new();
    if let Some(fields) = input.as_object() {
        for (key, value) in fields {
            if (key == "description" && description.is_some())
                || (key == "command" && command.is_some())
            {
                continue;
            }
            append_detail(&mut details, &key.replace('_', " "), value, 0);
        }
    } else {
        append_detail(&mut details, "Input", input, 0);
    }
    if !details.is_empty() {
        sections.push(details.join("\n"));
    }
    sections.push("Approval applies to this invocation only.".into());
    sections.join("\n\n")
}

fn append_detail(lines: &mut Vec<String>, label: &str, value: &Value, depth: usize) {
    let indent = "  ".repeat(depth);
    match value {
        Value::Object(fields) if !fields.is_empty() => {
            lines.push(format!("{indent}{label}:"));
            for (key, value) in fields {
                append_detail(lines, &key.replace('_', " "), value, depth + 1);
            }
        }
        Value::Array(items) if !items.is_empty() => {
            lines.push(format!("{indent}{label}:"));
            for (index, value) in items.iter().enumerate() {
                append_detail(lines, &(index + 1).to_string(), value, depth + 1);
            }
        }
        _ => {
            let text = match value {
                Value::String(text) => text.clone(),
                Value::Null => "Not supplied".into(),
                Value::Object(_) | Value::Array(_) => "Empty".into(),
                _ => value.to_string(),
            };
            if text.contains('\n') {
                lines.push(format!("{indent}{label}:"));
                lines.extend(text.split('\n').map(|line| format!("{indent}  {line}")));
            } else {
                lines.push(format!("{indent}{label}: {text}"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn shell_approval_shows_exact_command_and_description_without_json() {
        let command = "cd 'norsk 🦦'\nprintf '%s\\n' \"$VALUE\" && npm test";
        let input =
            json!({"command": command, "description": "Run project tests", "timeout": 120000});
        let summary = permission_summary("Bash", &input, "Shell access needs approval");
        assert_eq!(summary, format!("Claude Code: Bash\n\nRun project tests\n\nCommand:\n{command}\n\nReason: Shell access needs approval\n\ntimeout: 120000\n\nApproval applies to this invocation only."));
        assert_eq!(input["command"], command);
    }

    #[test]
    fn missing_description_uses_reason_and_duplicate_reason_is_not_repeated() {
        let summary = permission_summary("Bash", &json!({"command":"npm test"}), "Run tests");
        assert!(summary.contains("Command:\nnpm test\n\nReason: Run tests"));
        let summary = permission_summary(
            "Bash",
            &json!({"command":"npm test", "description":"Run tests"}),
            "Run tests",
        );
        assert_eq!(summary.matches("Run tests").count(), 1);
    }

    #[test]
    fn non_shell_approvals_preserve_paths_changes_and_nested_arguments() {
        let summary = permission_summary(
            "Edit",
            &json!({"file_path":"src/main.rs", "old_string":"one\ntwo", "new_string":"tre 🦦", "replace_all":false}),
            "Approve edit",
        );
        assert!(summary.contains("file path: src/main.rs"));
        assert!(summary.contains("old string:\n  one\n  two"));
        assert!(summary.contains("new string: tre 🦦"));
        assert!(summary.contains("replace all: false"));
        let summary = permission_summary(
            "mcp__custom__request",
            &json!({"targets":[{"path":"/tmp/a", "recursive":true}], "options":{}, "note":null}),
            "",
        );
        assert!(summary.contains("targets:\n  1:\n    path: /tmp/a\n    recursive: true"));
        assert!(summary.contains("options: Empty"));
        assert!(summary.contains("note: Not supplied"));
    }

    #[test]
    fn invalid_command_shape_remains_visible_for_review() {
        let summary = permission_summary(
            "Bash",
            &json!({"command":["unexpected"], "description":42}),
            "Invalid input",
        );
        assert!(summary.contains("command:\n  1: unexpected"));
        assert!(summary.contains("description: 42"));
    }
}
