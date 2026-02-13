use crate::ai::scope::{Capabilities, RequiredScope, ScopeContext};
use crate::ai::types::{DiagnosticFact, FileScope};

use super::{ParamType, SideEffect, ToolDefinition, ToolParam, ToolRegistry, ToolResult};

/// Everything a tool handler needs from the editor (read-only snapshot).
#[derive(Debug, Clone)]
pub struct ToolExecutionContext {
    pub buffer_content: String,
    pub file_path: Option<String>,
    pub cursor: (usize, usize),
    /// (start_line, start_col, end_line, end_col) — 0-indexed.
    pub selection: Option<(usize, usize, usize, usize)>,
    pub diagnostics: Vec<DiagnosticFact>,
    pub scope_context: ScopeContext,
    pub capabilities: Capabilities,
}

/// Register all built-in tools into the registry.
pub fn register_builtins(registry: &mut ToolRegistry) {
    registry.register(read_file_def());
    registry.register(read_selection_def());
    registry.register(read_diagnostics_def());
}

/// Dispatch a built-in tool call by name.
pub fn execute_builtin(
    name: &str,
    args: &serde_json::Value,
    ctx: &ToolExecutionContext,
) -> ToolResult {
    match name {
        "read_file" => handle_read_file(args, ctx),
        "read_selection" => handle_read_selection(args, ctx),
        "read_diagnostics" => handle_read_diagnostics(args, ctx),
        _ => ToolResult::Error(format!("unknown built-in tool: {name}")),
    }
}

// ---------------------------------------------------------------------------
// read_file
// ---------------------------------------------------------------------------

fn read_file_def() -> ToolDefinition {
    ToolDefinition {
        name: "read_file".to_string(),
        description: "Read the current buffer content, optionally a line range.".to_string(),
        required_scope: RequiredScope {
            file_scope: FileScope::File,
            shell: false,
            network: false,
        },
        side_effect: SideEffect::Read,
        parameters: vec![
            ToolParam {
                name: "start_line".to_string(),
                param_type: ParamType::LineNumber,
                required: false,
                description: "First line to read (1-indexed, inclusive).".to_string(),
            },
            ToolParam {
                name: "end_line".to_string(),
                param_type: ParamType::LineNumber,
                required: false,
                description: "Last line to read (1-indexed, inclusive).".to_string(),
            },
        ],
    }
}

fn handle_read_file(args: &serde_json::Value, ctx: &ToolExecutionContext) -> ToolResult {
    let lines: Vec<&str> = ctx.buffer_content.lines().collect();
    let total = lines.len();

    if total == 0 {
        return ToolResult::Success("[empty buffer]".to_string());
    }

    let start = args
        .get("start_line")
        .and_then(|v| v.as_u64())
        .map(|n| n.saturating_sub(1) as usize)
        .unwrap_or(0);
    let end = args
        .get("end_line")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize)
        .unwrap_or(total);

    let start = start.min(total);
    let end = end.min(total);
    if start >= end {
        return ToolResult::Success("[empty range]".to_string());
    }

    let mut output = String::new();
    let file_label = ctx.file_path.as_deref().unwrap_or("[No Name]");
    output.push_str(&format!(
        "File: {} (lines {}-{} of {})\n",
        file_label,
        start + 1,
        end,
        total
    ));
    for (i, line) in lines[start..end].iter().enumerate() {
        output.push_str(&format!("{:>4} | {}\n", start + i + 1, line));
    }
    ToolResult::Success(output)
}

// ---------------------------------------------------------------------------
// read_selection
// ---------------------------------------------------------------------------

fn read_selection_def() -> ToolDefinition {
    ToolDefinition {
        name: "read_selection".to_string(),
        description: "Read the current or most recent visual selection.".to_string(),
        required_scope: RequiredScope {
            file_scope: FileScope::File,
            shell: false,
            network: false,
        },
        side_effect: SideEffect::Read,
        parameters: vec![],
    }
}

fn handle_read_selection(_args: &serde_json::Value, ctx: &ToolExecutionContext) -> ToolResult {
    let Some((start_line, start_col, end_line, end_col)) = ctx.selection else {
        return ToolResult::Error("no active selection".to_string());
    };

    let lines: Vec<&str> = ctx.buffer_content.lines().collect();
    if lines.is_empty() || start_line >= lines.len() {
        return ToolResult::Error("selection out of range".to_string());
    }

    let end_line = end_line.min(lines.len().saturating_sub(1));
    let mut output = String::new();
    output.push_str(&format!(
        "Selection: lines {}-{}\n",
        start_line + 1,
        end_line + 1
    ));

    for i in start_line..=end_line {
        let line = lines[i];
        let slice = if i == start_line && i == end_line {
            // Single line selection
            let sc = start_col.min(line.len());
            let ec = end_col.min(line.len());
            &line[sc..ec]
        } else if i == start_line {
            let sc = start_col.min(line.len());
            &line[sc..]
        } else if i == end_line {
            let ec = end_col.min(line.len());
            &line[..ec]
        } else {
            line
        };
        output.push_str(&format!("{:>4} | {}\n", i + 1, slice));
    }
    ToolResult::Success(output)
}

// ---------------------------------------------------------------------------
// read_diagnostics
// ---------------------------------------------------------------------------

fn read_diagnostics_def() -> ToolDefinition {
    ToolDefinition {
        name: "read_diagnostics".to_string(),
        description: "Get LSP diagnostics for the current file.".to_string(),
        required_scope: RequiredScope {
            file_scope: FileScope::File,
            shell: false,
            network: false,
        },
        side_effect: SideEffect::Read,
        parameters: vec![],
    }
}

fn handle_read_diagnostics(_args: &serde_json::Value, ctx: &ToolExecutionContext) -> ToolResult {
    if ctx.diagnostics.is_empty() {
        return ToolResult::Success("No diagnostics.".to_string());
    }

    let mut output = String::new();
    let file_label = ctx.file_path.as_deref().unwrap_or("[No Name]");
    output.push_str(&format!(
        "Diagnostics for {} ({} total):\n",
        file_label,
        ctx.diagnostics.len()
    ));
    for d in &ctx.diagnostics {
        let severity = d.severity.as_deref().unwrap_or("unknown");
        output.push_str(&format!(
            "  Line {}: [{}] {} (col {}-{})\n",
            d.line + 1,
            severity,
            d.message,
            d.start_character,
            d.end_character,
        ));
    }
    ToolResult::Success(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn test_ctx(content: &str) -> ToolExecutionContext {
        ToolExecutionContext {
            buffer_content: content.to_string(),
            file_path: Some("test.rs".to_string()),
            cursor: (0, 0),
            selection: None,
            diagnostics: vec![],
            scope_context: ScopeContext {
                current_file: Some(PathBuf::from("test.rs")),
                project_root: Some(PathBuf::from("/")),
            },
            capabilities: Capabilities {
                file_scope: FileScope::File,
                shell: false,
                network: false,
            },
        }
    }

    #[test]
    fn read_file_full() {
        let ctx = test_ctx("line 1\nline 2\nline 3");
        let result = execute_builtin("read_file", &serde_json::json!({}), &ctx);
        match result {
            ToolResult::Success(s) => {
                assert!(s.contains("line 1"));
                assert!(s.contains("line 3"));
                assert!(s.contains("lines 1-3 of 3"));
            }
            ToolResult::Error(e) => panic!("expected success, got error: {e}"),
        }
    }

    #[test]
    fn read_file_range() {
        let ctx = test_ctx("a\nb\nc\nd\ne");
        let result = execute_builtin(
            "read_file",
            &serde_json::json!({"start_line": 2, "end_line": 4}),
            &ctx,
        );
        match result {
            ToolResult::Success(s) => {
                assert!(s.contains("lines 2-4 of 5"));
                assert!(!s.contains("| a"));
                assert!(s.contains("| b"));
                assert!(s.contains("| d"));
                assert!(!s.contains("| e"));
            }
            ToolResult::Error(e) => panic!("expected success, got error: {e}"),
        }
    }

    #[test]
    fn read_file_empty() {
        let ctx = test_ctx("");
        let result = execute_builtin("read_file", &serde_json::json!({}), &ctx);
        match result {
            ToolResult::Success(s) => assert!(s.contains("[empty buffer]")),
            ToolResult::Error(e) => panic!("expected success, got error: {e}"),
        }
    }

    #[test]
    fn read_selection_no_selection() {
        let ctx = test_ctx("hello world");
        let result = execute_builtin("read_selection", &serde_json::json!({}), &ctx);
        match result {
            ToolResult::Error(s) => assert!(s.contains("no active selection")),
            ToolResult::Success(_) => panic!("expected error"),
        }
    }

    #[test]
    fn read_selection_single_line() {
        let mut ctx = test_ctx("hello world\nsecond line");
        ctx.selection = Some((0, 6, 0, 11));
        let result = execute_builtin("read_selection", &serde_json::json!({}), &ctx);
        match result {
            ToolResult::Success(s) => {
                assert!(s.contains("world"));
            }
            ToolResult::Error(e) => panic!("expected success, got error: {e}"),
        }
    }

    #[test]
    fn read_diagnostics_empty() {
        let ctx = test_ctx("fn main() {}");
        let result = execute_builtin("read_diagnostics", &serde_json::json!({}), &ctx);
        match result {
            ToolResult::Success(s) => assert!(s.contains("No diagnostics")),
            ToolResult::Error(e) => panic!("expected success, got error: {e}"),
        }
    }

    #[test]
    fn read_diagnostics_with_items() {
        let mut ctx = test_ctx("fn main() {}");
        ctx.diagnostics = vec![DiagnosticFact {
            message: "unused variable".to_string(),
            severity: Some("warning".to_string()),
            line: 0,
            start_character: 4,
            end_character: 8,
        }];
        let result = execute_builtin("read_diagnostics", &serde_json::json!({}), &ctx);
        match result {
            ToolResult::Success(s) => {
                assert!(s.contains("unused variable"));
                assert!(s.contains("[warning]"));
                assert!(s.contains("1 total"));
            }
            ToolResult::Error(e) => panic!("expected success, got error: {e}"),
        }
    }

    #[test]
    fn unknown_tool_returns_error() {
        let ctx = test_ctx("hello");
        let result = execute_builtin("nonexistent", &serde_json::json!({}), &ctx);
        match result {
            ToolResult::Error(s) => assert!(s.contains("unknown built-in tool")),
            ToolResult::Success(_) => panic!("expected error"),
        }
    }
}
