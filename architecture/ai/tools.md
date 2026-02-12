# Tool Abstraction

A tool is a capability the AI can invoke to observe or mutate the editor environment. Tools have an input schema the model sees, a handler the editor runs, and scope requirements the runtime enforces.

## Tool Definition

```rust
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub required_scope: RequiredScope,
    pub side_effect: SideEffect,
    pub parameters: Vec<ToolParam>,
    pub handler: ToolHandler,
}

pub enum SideEffect {
    Read,       // Observation only, no state change
    Mutation,   // Changes editor state (buffer content, cursor, etc.)
    External,   // Side effects outside the editor (filesystem, shell, network)
}

pub struct ToolParam {
    pub name: String,
    pub param_type: ParamType,
    pub required: bool,
    pub description: String,
}

pub enum ParamType {
    String,
    Integer,
    Boolean,
    FilePath,       // Validated against scope at invocation time
    LineNumber,
    LineRange,      // (start_line, end_line)
    Json,           // Arbitrary JSON value
}

pub enum ToolHandler {
    Builtin(fn(&ToolInvocation) -> Result<ToolResult>),
    Lua(String),    // Name of a Lua function in the registry
}
```

The `FilePath` parameter type is special: it triggers scope validation before the handler runs. See [scopes.md](scopes.md) for the enforcement model.

## Built-in Tools

### Read Tools

| Name | Required Scope | Description |
|------|---------------|-------------|
| `read_selection` | `selection` | Read the current visual selection text |
| `read_file` | `file` | Read the current buffer content, optionally a line range |
| `read_diagnostics` | `file` | Get LSP diagnostics for the current file |
| `read_symbols` | `file` | Get document symbols (functions, types, etc.) from LSP |
| `read_ast` | `file` | Get tree-sitter AST nodes for a line range |
| `search_project` | `project` | Ripgrep search across project files |
| `list_files` | `project` | List files matching a glob pattern under project root |

### Edit Tools

| Name | Required Scope | Description |
|------|---------------|-------------|
| `edit_selection` | `selection` | Replace the current selection with new text |
| `edit_range` | `file` | Replace a line range in a file with new content |
| `edit_diff` | `file` | Apply a unified diff to a file |
| `edit_insert` | `file` | Insert text at a specific line |
| `edit_delete` | `file` | Delete a line range |
| `edit_search_replace` | `file` | Find and replace within a range |
| `edit_ast` | `file` | Transform a tree-sitter AST node by type and name |

### Tool Parameter Schemas

```
read_file:
  path:       FilePath (optional — defaults to current buffer)
  start_line: LineNumber (optional)
  end_line:   LineNumber (optional)

read_diagnostics:
  path:       FilePath (optional — defaults to current buffer)
  severity:   String (optional — "error" | "warning" | "info" | "hint")

edit_range:
  path:       FilePath (optional — defaults to current buffer)
  start_line: LineNumber (required)
  end_line:   LineNumber (required)
  content:    String (required — the replacement text)

edit_diff:
  path:       FilePath (optional — defaults to current buffer)
  diff:       String (required — unified diff format)

search_project:
  pattern:    String (required — regex pattern)
  glob:       String (optional — file filter, e.g. "*.rs")
  max_results: Integer (optional — default 20)
```

## Side Effect Classification

Side effects determine default permissions:

| Side Effect | What it does | Default permission (file scope) | Default permission (project+ scope) |
|------------|-------------|-------------------------------|-------------------------------------|
| `Read` | Observes state, no changes | `auto` | `auto` |
| `Mutation` | Changes buffer content | `auto` | `confirm` |
| `External` | Runs commands, touches filesystem | `confirm` | `confirm` |

These defaults can be overridden per-tool in the profile's `permissions` table.

## Tool Invocation Flow

```
1. Model produces a tool call: { name, arguments }
2. Look up ToolDefinition in registry
3. Validate: tool exists in profile's toolset
4. Validate: tool's required_scope fits within effective scope
5. Validate: each FilePath parameter is within scope
6. Check permission: auto → execute, confirm → prompt user, deny → error
7. Execute handler (Builtin or Lua)
8. Return result to model (for multi-turn) or apply edits (for single-shot)
```

If any validation step fails, the tool call returns an error message to the model (not a crash). The model can retry or explain what it tried to do.

## User-Defined Tools

Plugins and users register tools via Lua:

```lua
vim.ai.tools.register("cargo_test", {
  description = "Run cargo tests with optional filter pattern",
  scope = "shell",              -- needs shell access
  side_effect = "external",     -- runs a command
  params = {
    filter = { type = "string", optional = true, description = "Test name filter" },
    release = { type = "boolean", optional = true, description = "Use --release" },
  },
  handler = function(params)
    local cmd = "cargo test"
    if params.filter then cmd = cmd .. " " .. params.filter end
    if params.release then cmd = cmd .. " --release" end
    return vim.fn.system(cmd)
  end,
})
```

User tools go through the same validation pipeline as built-ins. The handler runs in the Lua context with access to `vim.fn.*` and `vim.api.*`.

### Overriding Built-ins

Registering a tool with the same name as a built-in replaces it:

```lua
-- Replace the built-in edit_diff with a custom implementation
vim.ai.tools.register("edit_diff", {
  description = "Apply edits using my custom diff format",
  scope = "file",
  side_effect = "mutation",
  params = {
    diff = { type = "string", description = "Custom diff format" },
  },
  handler = function(params)
    -- custom parsing logic
    return my_custom_apply(params.diff)
  end,
})
```

## Tool-to-JSON Schema Conversion

For models that support tool calling (function calling), each `ToolDefinition` is converted to a JSON schema at request time:

```json
{
  "name": "edit_range",
  "description": "Replace a line range in a file with new content",
  "parameters": {
    "type": "object",
    "properties": {
      "path": { "type": "string", "description": "File path (default: current buffer)" },
      "start_line": { "type": "integer", "description": "First line to replace (1-indexed)" },
      "end_line": { "type": "integer", "description": "Last line to replace (1-indexed)" },
      "content": { "type": "string", "description": "Replacement text" }
    },
    "required": ["start_line", "end_line", "content"]
  }
}
```

This conversion happens in the provider layer and is format-specific (OpenAI and Anthropic have slightly different tool schemas). The tool registry provides a provider-agnostic representation; the provider adapter converts it.

## Design Rationale

**Why separate tools from edit formats?** Tools are the structured interface (function calling). Edit formats are the unstructured interface (text parsing). Some models support tool calling, some don't. Some are better with diffs in prose, some with structured `edit_range` calls. The tool system handles both through the `edit_mode` profile setting (see [provider-layer.md](provider-layer.md)).

**Why `FilePath` as a special type?** So scope validation is automatic. A tool author writes `type = "filepath"` and gets scope checking for free. No manual path validation in handlers.

**Why allow overriding built-ins?** Experimentation. If someone finds a better way to represent diffs to a model, they can swap out `edit_diff` without forking the editor.
