use crate::ai::scope::{Capabilities, RequiredScope, ScopeContext};
use crate::ai::types::{DiagnosticFact, FileScope};
use crate::editor::grep;

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
    // Read tools
    registry.register(read_file_def());
    registry.register(read_selection_def());
    registry.register(read_diagnostics_def());
    registry.register(search_project_def());
    registry.register(list_files_def());
    // Mutation tools (dispatched via execute_mutation_tool, not execute_builtin)
    registry.register(edit_range_def());
    registry.register(insert_lines_def());
    registry.register(delete_lines_def());
}

/// Dispatch a built-in tool call by name.
///
/// Only handles read-only tools. Mutation tools (`edit_range`, `insert_lines`,
/// `delete_lines`) are dispatched via `execute_mutation_tool` which has `&mut Editor`.
pub fn execute_builtin(
    name: &str,
    args: &serde_json::Value,
    ctx: &ToolExecutionContext,
) -> ToolResult {
    match name {
        "read_file" => handle_read_file(args, ctx),
        "read_selection" => handle_read_selection(args, ctx),
        "read_diagnostics" => handle_read_diagnostics(args, ctx),
        "search_project" => handle_search_project(args, ctx),
        "list_files" => handle_list_files(args, ctx),
        "edit_range" | "insert_lines" | "delete_lines" => ToolResult::Error(format!(
            "'{name}' is a mutation tool — must be dispatched via execute_mutation_tool"
        )),
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

// ---------------------------------------------------------------------------
// search_project
// ---------------------------------------------------------------------------

fn search_project_def() -> ToolDefinition {
    ToolDefinition {
        name: "search_project".to_string(),
        description: "Search for a pattern across all project files (respects .gitignore)."
            .to_string(),
        required_scope: RequiredScope {
            file_scope: FileScope::Project,
            shell: false,
            network: false,
        },
        side_effect: SideEffect::Read,
        parameters: vec![
            ToolParam {
                name: "query".to_string(),
                param_type: ParamType::String,
                required: true,
                description: "Search pattern (regex or literal string).".to_string(),
            },
            ToolParam {
                name: "max_results".to_string(),
                param_type: ParamType::Integer,
                required: false,
                description: "Maximum number of results (default 50, max 200).".to_string(),
            },
        ],
    }
}

fn handle_search_project(args: &serde_json::Value, ctx: &ToolExecutionContext) -> ToolResult {
    let query = match args.get("query").and_then(|v| v.as_str()) {
        Some(q) if !q.is_empty() => q,
        _ => {
            return ToolResult::Error(
                "'query' parameter is required and must be non-empty".to_string(),
            )
        }
    };

    let max_results = args
        .get("max_results")
        .and_then(|v| v.as_u64())
        .map(|n| (n as usize).min(200))
        .unwrap_or(50);

    let project_root = match &ctx.scope_context.project_root {
        Some(root) => root.clone(),
        None => return ToolResult::Error("no project root available".to_string()),
    };

    let matches = grep::grep_search_sync(query, &project_root, max_results);

    if matches.is_empty() {
        return ToolResult::Success(format!("No matches found for '{query}'."));
    }

    let mut output = String::new();
    output.push_str(&format!(
        "Found {} match(es) for '{}':\n",
        matches.len(),
        query
    ));
    for m in &matches {
        output.push_str(&format!(
            "{}:{}:{}: {}\n",
            m.rel_path, m.line, m.col, m.content
        ));
    }
    ToolResult::Success(output)
}

// ---------------------------------------------------------------------------
// list_files
// ---------------------------------------------------------------------------

fn list_files_def() -> ToolDefinition {
    ToolDefinition {
        name: "list_files".to_string(),
        description: "List files in the project (respects .gitignore).".to_string(),
        required_scope: RequiredScope {
            file_scope: FileScope::Project,
            shell: false,
            network: false,
        },
        side_effect: SideEffect::Read,
        parameters: vec![
            ToolParam {
                name: "path".to_string(),
                param_type: ParamType::String,
                required: false,
                description: "Subdirectory relative to project root (default: root).".to_string(),
            },
            ToolParam {
                name: "max_results".to_string(),
                param_type: ParamType::Integer,
                required: false,
                description: "Maximum number of files to list (default 200, max 1000).".to_string(),
            },
        ],
    }
}

fn handle_list_files(args: &serde_json::Value, ctx: &ToolExecutionContext) -> ToolResult {
    let project_root = match &ctx.scope_context.project_root {
        Some(root) => root.clone(),
        None => return ToolResult::Error("no project root available".to_string()),
    };

    let max_results = args
        .get("max_results")
        .and_then(|v| v.as_u64())
        .map(|n| (n as usize).min(1000))
        .unwrap_or(200);

    let search_dir = if let Some(subpath) = args.get("path").and_then(|v| v.as_str()) {
        if subpath.contains("..") {
            return ToolResult::Error("path traversal (..) not allowed".to_string());
        }
        let candidate = project_root.join(subpath);
        // Validate it stays within project root
        let normalized = candidate.canonicalize().unwrap_or(candidate.clone());
        let root_normalized = project_root.canonicalize().unwrap_or(project_root.clone());
        if !normalized.starts_with(&root_normalized) {
            return ToolResult::Error("path is outside project root".to_string());
        }
        candidate
    } else {
        project_root.clone()
    };

    if !search_dir.is_dir() {
        return ToolResult::Error(format!("'{}' is not a directory", search_dir.display()));
    }

    let walker = grep::build_walker(&search_dir).build();

    let mut files = Vec::new();
    for entry in walker {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if entry.file_type().map_or(true, |ft| !ft.is_file()) {
            continue;
        }
        let rel_path = entry
            .path()
            .strip_prefix(&project_root)
            .unwrap_or(entry.path())
            .to_string_lossy()
            .to_string();
        files.push(rel_path);
        if files.len() >= max_results {
            break;
        }
    }

    if files.is_empty() {
        return ToolResult::Success("No files found.".to_string());
    }

    files.sort();
    let mut output = String::new();
    output.push_str(&format!("{} file(s):\n", files.len()));
    for f in &files {
        output.push_str(f);
        output.push('\n');
    }
    ToolResult::Success(output)
}

// ---------------------------------------------------------------------------
// Mutation tool definitions (dispatched via execute_mutation_tool)
// ---------------------------------------------------------------------------

pub(crate) fn edit_range_def() -> ToolDefinition {
    ToolDefinition {
        name: "edit_range".to_string(),
        description: "Replace lines start_line..end_line (inclusive) with new text.".to_string(),
        required_scope: RequiredScope {
            file_scope: FileScope::File,
            shell: false,
            network: false,
        },
        side_effect: SideEffect::Mutation,
        parameters: vec![
            ToolParam {
                name: "start_line".to_string(),
                param_type: ParamType::LineNumber,
                required: true,
                description: "First line to replace (1-indexed, inclusive).".to_string(),
            },
            ToolParam {
                name: "end_line".to_string(),
                param_type: ParamType::LineNumber,
                required: true,
                description: "Last line to replace (1-indexed, inclusive).".to_string(),
            },
            ToolParam {
                name: "new_text".to_string(),
                param_type: ParamType::String,
                required: true,
                description: "Replacement text (may contain newlines).".to_string(),
            },
        ],
    }
}

pub(crate) fn insert_lines_def() -> ToolDefinition {
    ToolDefinition {
        name: "insert_lines".to_string(),
        description: "Insert text after a specific line.".to_string(),
        required_scope: RequiredScope {
            file_scope: FileScope::File,
            shell: false,
            network: false,
        },
        side_effect: SideEffect::Mutation,
        parameters: vec![
            ToolParam {
                name: "after_line".to_string(),
                param_type: ParamType::LineNumber,
                required: true,
                description: "Line number to insert after (1-indexed, 0 = beginning of file)."
                    .to_string(),
            },
            ToolParam {
                name: "text".to_string(),
                param_type: ParamType::String,
                required: true,
                description: "Text to insert (may contain newlines).".to_string(),
            },
        ],
    }
}

pub(crate) fn delete_lines_def() -> ToolDefinition {
    ToolDefinition {
        name: "delete_lines".to_string(),
        description: "Delete a range of lines.".to_string(),
        required_scope: RequiredScope {
            file_scope: FileScope::File,
            shell: false,
            network: false,
        },
        side_effect: SideEffect::Mutation,
        parameters: vec![
            ToolParam {
                name: "start_line".to_string(),
                param_type: ParamType::LineNumber,
                required: true,
                description: "First line to delete (1-indexed, inclusive).".to_string(),
            },
            ToolParam {
                name: "end_line".to_string(),
                param_type: ParamType::LineNumber,
                required: true,
                description: "Last line to delete (1-indexed, inclusive).".to_string(),
            },
        ],
    }
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
                allow_mutations: true,
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

    #[test]
    fn mutation_tool_returns_error_via_execute_builtin() {
        let ctx = test_ctx("hello");
        let result = execute_builtin("edit_range", &serde_json::json!({}), &ctx);
        match result {
            ToolResult::Error(s) => assert!(s.contains("mutation tool")),
            ToolResult::Success(_) => panic!("expected error"),
        }
    }

    fn test_ctx_with_project(content: &str, project_root: PathBuf) -> ToolExecutionContext {
        ToolExecutionContext {
            buffer_content: content.to_string(),
            file_path: Some("test.rs".to_string()),
            cursor: (0, 0),
            selection: None,
            diagnostics: vec![],
            scope_context: ScopeContext {
                current_file: Some(PathBuf::from("test.rs")),
                project_root: Some(project_root),
            },
            capabilities: Capabilities {
                file_scope: FileScope::Project,
                shell: false,
                network: false,
                allow_mutations: true,
            },
        }
    }

    #[test]
    fn search_project_finds_matches() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "hello world\nfoo bar\n").unwrap();
        std::fs::write(root.join("b.txt"), "hello again\n").unwrap();

        let ctx = test_ctx_with_project("", root.to_path_buf());
        let result = execute_builtin(
            "search_project",
            &serde_json::json!({"query": "hello"}),
            &ctx,
        );
        match result {
            ToolResult::Success(s) => {
                assert!(s.contains("2 match(es)"));
                assert!(s.contains("a.txt"));
                assert!(s.contains("b.txt"));
            }
            ToolResult::Error(e) => panic!("expected success, got error: {e}"),
        }
    }

    #[test]
    fn search_project_respects_max_results() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // Create a file with many matching lines
        let content: String = (0..50).map(|i| format!("needle {i}\n")).collect();
        std::fs::write(root.join("many.txt"), &content).unwrap();

        let ctx = test_ctx_with_project("", root.to_path_buf());
        let result = execute_builtin(
            "search_project",
            &serde_json::json!({"query": "needle", "max_results": 5}),
            &ctx,
        );
        match result {
            ToolResult::Success(s) => {
                // Header line also contains "match(es)" so count lines with file path format
                let match_count = s.lines().filter(|l| l.starts_with("many.txt:")).count();
                assert_eq!(match_count, 5, "got {match_count} matches, expected 5");
            }
            ToolResult::Error(e) => panic!("expected success, got error: {e}"),
        }
    }

    #[test]
    fn search_project_empty_query_error() {
        let result = execute_builtin(
            "search_project",
            &serde_json::json!({"query": ""}),
            &test_ctx_with_project("", PathBuf::from("/")),
        );
        match result {
            ToolResult::Error(s) => assert!(s.contains("required")),
            ToolResult::Success(_) => panic!("expected error"),
        }
        // Also test missing query
        let result = execute_builtin(
            "search_project",
            &serde_json::json!({}),
            &test_ctx_with_project("", PathBuf::from("/")),
        );
        match result {
            ToolResult::Error(s) => assert!(s.contains("required")),
            ToolResult::Success(_) => panic!("expected error"),
        }
    }

    #[test]
    fn list_files_returns_project_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("Cargo.toml"), "").unwrap();
        std::fs::write(root.join("src/main.rs"), "").unwrap();
        std::fs::write(root.join("src/lib.rs"), "").unwrap();

        let ctx = test_ctx_with_project("", root.to_path_buf());
        let result = execute_builtin("list_files", &serde_json::json!({}), &ctx);
        match result {
            ToolResult::Success(s) => {
                assert!(s.contains("3 file(s)"));
                assert!(s.contains("Cargo.toml"));
                assert!(s.contains("src/main.rs"));
                assert!(s.contains("src/lib.rs"));
            }
            ToolResult::Error(e) => panic!("expected success, got error: {e}"),
        }
    }

    #[test]
    fn list_files_path_traversal_blocked() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx_with_project("", dir.path().to_path_buf());
        let result = execute_builtin("list_files", &serde_json::json!({"path": "../etc"}), &ctx);
        match result {
            ToolResult::Error(s) => assert!(s.contains("traversal")),
            ToolResult::Success(_) => panic!("expected error for path traversal"),
        }
    }
}
