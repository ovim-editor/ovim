# CLI Restructure Plan

## Motivation

The current CLI has ~30 flat subcommands, ambiguous file-vs-subcommand parsing, session-centric design that doesn't match how agents actually work, and MCP complexity that could be replaced with simpler integration. This plan restructures the CLI to be cleaner, more discoverable, and agent-friendly.

## Design Principles

1. **File-addressed by default, session-addressed when opted in.** Agents want `file:line:col`, not session management.
2. **Sessions are opt-in.** TUI mode doesn't register a session. Users start sessions explicitly via `:session start NAME`.
3. **Group related commands.** LSP commands under `ovim lsp`, session management under `ovim session`.
4. **`--` support.** `ovim -- buffer` opens a file named "buffer" instead of running the subcommand.
5. **Drop the `mcp` debugging subcommand.** Keep `mcp-server` and `install` for tool integration.

## Target CLI Structure

```
# Editor mode
ovim [FILE[:LINE[:COL]]] [--headless --session NAME] [--dimension WxH] [--render]
ovim -- buffer                          # open file named "buffer"

# File operations (stateless — no session needed)
ovim edit <FILE> --old TEXT --new TEXT [--line N]
ovim insert <FILE> --after N|--before N --text TEXT
ovim delete-lines <FILE> --from N --to N
ovim read-lines <FILE> --from N --to N [--json]

# Session control (require explicit -s SESSION)
ovim send <KEYS> -s SESSION
ovim exec <COMMAND> -s SESSION
ovim context -s SESSION
ovim buffer -s SESSION
ovim snapshot [-s SESSION] [--format json|pretty]
ovim search <PATTERN> -s SESSION
ovim next-match -s SESSION

# LSP commands (grouped, require -s SESSION)
ovim lsp status -s SESSION
ovim lsp hover -s SESSION
ovim lsp definition -s SESSION
ovim lsp references -s SESSION
ovim lsp diagnostics -s SESSION
ovim lsp symbols -s SESSION
ovim lsp outline -s SESSION
ovim lsp symbol <QUERY> -s SESSION
ovim lsp trace -s SESSION
ovim lsp wait [--timeout MS] -s SESSION
ovim lsp check <FILE> [--verbose]           # standalone, no session
ovim lsp languages [--verbose]              # standalone, no session

# Session management (grouped)
ovim session list
ovim session kill -s SESSION
ovim session health -s SESSION
ovim session cleanup [--max-age DAYS] [--dry-run]

# Integration (infrequent)
ovim mcp-server [--workspace DIR]
ovim install [claude|cursor|all] [--workspace DIR] [--show-config]
```

### Future: dual addressing

Most commands could accept *either* `-s SESSION` or `FILE[:LINE[:COL]]`. For now, file operations work on disk and session operations go through HTTP. The CLI grammar supports both from the start so it doesn't change later.

```
# Today: session-only
ovim lsp hover -s myproject

# Future: file-addressed (spins up LSP, queries, exits)
ovim lsp hover src/main.rs:42:10
```

## Changes

### Phase 1: Restructure CLI argument parsing

**File: `ovim/src/cli.rs`**

1. **Remove `global = true` from `file` arg.** The file argument is only meaningful in editor mode. Making it global silently captures `ovim send "keys" myfile.txt` without error.

2. **Support `FILE:LINE:COL` parsing.** Parse the file argument to extract optional line and column:
   ```rust
   pub struct FileArg {
       pub path: String,
       pub line: Option<usize>,  // 1-indexed
       pub col: Option<usize>,   // 1-indexed
   }
   ```
   Parser: split on `:` from the right, try to parse trailing segments as numbers. If they're not numbers, treat the whole thing as a path (handles Windows paths and colons in filenames gracefully).

3. **Nest LSP subcommands** under a `Lsp` variant:
   ```rust
   Lsp {
       #[command(subcommand)]
       command: LspCommand,
   }
   ```
   Where `LspCommand` is a new enum containing: `Status`, `Hover`, `Definition`, `References`, `Diagnostics`, `Symbols`, `Outline`, `Symbol { query }`, `Trace`, `Wait`, `Check { file }`, `Languages`.

4. **Nest session subcommands** under a `Session` variant:
   ```rust
   Session {
       #[command(subcommand)]
       command: SessionCommand,
   }
   ```
   Where `SessionCommand` is: `List`, `Kill`, `Health`, `Cleanup`.

5. **Add `FILE` positional arg to file-operation commands.** Currently `edit`, `insert`, `delete-lines`, `read-lines` only have `--session`. Add a required `file` positional:
   ```rust
   Edit {
       /// File to edit
       file: String,
       #[arg(long)]
       line: Option<usize>,
       #[arg(long)]
       old: String,
       #[arg(long)]
       new: String,
   }
   ```

6. **Make `-s SESSION` required** (not optional) for session-addressed commands. Remove auto-discovery from the subcommand path. Commands that need a session must say which one.

7. **Drop the `Mcp` subcommand** (raw JSON-RPC debugging). Keep `McpServer` and `Install`.

8. **Rename `McpServer` to `MpcServer`** — actually keep as `McpServer` but the CLI name stays `mcp-server`.

### Phase 2: Make sessions opt-in

**File: `ovim/src/main.rs`**

1. **TUI mode: don't register a session by default.** Remove the auto-generated `tui_{random}_{timestamp}` session and the `session_info.write()` call. The API server still starts (needed for internal communication), but no session file is written.

2. **Headless mode: require `--session NAME`.** Remove the `"default"` fallback. If you want headless mode, you must name your session. Error message: `"--headless requires --session NAME"`.

3. **Remove `SessionGuard` from TUI mode.** No session file = no cleanup needed.

4. **Keep signal handlers for headless mode only.** They clean up the session file on SIGINT/SIGTERM.

**File: `ovim-core/src/session.rs`**

5. **Remove `auto_discover()`.** With sessions being explicit, auto-discovery is unnecessary. `resolve_session()` in subcommands becomes:
   ```rust
   fn resolve_session(session_name: &str) -> Result<SessionInfo> {
       SessionInfo::read(session_name)
   }
   ```

6. **Remove `get_default()`.** No default session concept.

7. **Keep `list_all()`** — still needed for `ovim session list`.

### Phase 3: Add `:session start/stop` ex commands

**File: `ovim-core/src/commands.rs`**

Add to the command dispatch:

```rust
cmd if cmd.starts_with("session") => {
    let subcmd = cmd.strip_prefix("session").unwrap_or("").trim();
    match subcmd {
        "" | "list" => {
            // Show active sessions in scratch buffer
        }
        s if s.starts_with("start ") => {
            let name = s["start ".len()..].trim();
            // Validate name, start HTTP session, write session file
            // Need: access to the API server port from Editor
            // Store port in Editor during startup for this purpose
        }
        "stop" => {
            // Delete session file, clear session state
            // API server keeps running (internal), just unregisters externally
        }
        _ => error
    }
}
```

**Key design decision:** The API server already runs in both TUI and headless mode. `:session start NAME` just writes the session file that makes it discoverable. `:session stop` deletes the file. The server doesn't stop — it's still needed for internal communication. This makes start/stop cheap and safe.

**File: `ovim-core/src/editor/mod.rs`**

Add fields to `Editor`:
```rust
pub api_port: Option<u16>,              // Set during startup
pub active_session: Option<String>,     // Set by :session start
```

**File: `ovim/src/main.rs`**

After API server starts, store port in editor:
```rust
editor.api_port = Some(port);
```

### Phase 4: Implement file-addressed operations

**File: `ovim/src/subcommands.rs`**

The file-operation commands (`edit`, `insert`, `delete-lines`, `read-lines`) currently go through the HTTP API. Change them to work directly on files:

1. **`read-lines FILE --from N --to N`**: Read file, extract line range, print with line numbers. No session needed.

2. **`edit FILE --old TEXT --new TEXT [--line N]`**: Read file, find text, replace, write file. Atomic write (write to .tmp, rename).

3. **`insert FILE --after N|--before N --text TEXT`**: Read file, insert lines, write file.

4. **`delete-lines FILE --from N --to N`**: Read file, remove lines, write file.

These are simple file operations. No ropey, no editor instance. Just `std::fs::read_to_string`, manipulate lines, `std::fs::write`.

**Error cases:**
- File doesn't exist → error
- `--old` text not found → error with context (show nearby lines)
- `--old` text matches multiple times (without `--line`) → error listing matches
- Line numbers out of range → error

### Phase 5: Update help text and documentation

1. **Top-level `--help`**: Lead with editor usage and file operations. Session stuff is secondary.

2. **Subcommand grouping in help**: clap supports `#[command(next_help_heading = "...")]` for visual grouping in `--help` output. Use it:
   ```
   Usage: ovim [FILE[:LINE[:COL]]] [OPTIONS]
          ovim <COMMAND>

   Commands:
     Editor Operations:
       edit          Replace text in a file
       insert        Insert text into a file
       delete-lines  Delete lines from a file
       read-lines    Read lines from a file

     Session Control:
       send          Send key sequence to a session
       exec          Execute ex command in a session
       context       Get context window from a session
       buffer        Get buffer content from a session
       ...

     LSP:
       lsp           Language Server Protocol commands

     Session Management:
       session       Manage ovim sessions

     Integration:
       mcp-server    Start MCP stdio server
       install       Install ovim for editor integration
   ```

3. **Update `CLAUDE.md`**: Reflect new CLI structure. Lead with file operations for agent interface.

4. **Update `user-docs/`**: If any user-facing docs reference old subcommand names.

### Phase 6: Clean up dead code

1. **Remove `OvimClient` methods** that are no longer called (the file-operation ones that used HTTP).
2. **Remove API endpoints** for edit/insert/delete-lines/read-lines if they become file-only operations. Or keep them for session-addressed use — TBD based on whether dual addressing is implemented now or later.
3. **Remove auto-discovery code paths** in `session.rs`.
4. **Remove the `EditorArgs` struct** if no longer needed after `global = true` cleanup.

## Execution Order

The phases above are presented for understanding. Execution order to minimize breakage:

1. **Phase 1 (CLI restructure)** — biggest diff, mostly mechanical. Nest subcommands, add file args. Everything still compiles, behavior unchanged except command syntax.
2. **Phase 4 (file-addressed operations)** — implement direct file I/O for edit/insert/delete-lines/read-lines. Can coexist with session-addressed versions during transition.
3. **Phase 2 (sessions opt-in)** — remove auto-registration. This is the behavioral change. Do it after the file operations work so agents have a migration path.
4. **Phase 3 (`:session start/stop`)** — add ex commands. Depends on Phase 2's design.
5. **Phase 5 (docs)** — update after behavior is settled.
6. **Phase 6 (cleanup)** — remove dead code last.

## What NOT to Do

- **Don't remove the HTTP API server from TUI mode.** It's still useful for internal communication and will be needed when `:session start` re-enables external access.
- **Don't break headless mode.** It's the foundation for CI/testing workflows. Just require `--session NAME`.
- **Don't add file-addressed LSP commands yet.** That requires spinning up an LSP server on the fly, which is complex. Mark it as future work.
- **Don't remove `install` or `mcp-server`.** Claude Desktop and Cursor need MCP stdio. The CLI subcommands are for Claude Code and direct use.
- **Don't try to make file operations go through the editor.** Direct file I/O is simpler, faster, and what agents actually need. The edit ledger / history layer can wrap around this later without changing the interface.

## Migration Notes

Existing users of the CLI will need to update:

| Old | New |
|-----|-----|
| `ovim sessions` | `ovim session list` |
| `ovim kill` | `ovim session kill -s NAME` |
| `ovim health` | `ovim session health -s NAME` |
| `ovim cleanup` | `ovim session cleanup` |
| `ovim lsp-status -s X` | `ovim lsp status -s X` |
| `ovim goto-definition -s X` | `ovim lsp definition -s X` |
| `ovim find-references -s X` | `ovim lsp references -s X` |
| `ovim hover -s X` | `ovim lsp hover -s X` |
| `ovim diagnostics -s X` | `ovim lsp diagnostics -s X` |
| `ovim symbols -s X` | `ovim lsp symbols -s X` |
| `ovim outline -s X` | `ovim lsp outline -s X` |
| `ovim symbol QUERY -s X` | `ovim lsp symbol QUERY -s X` |
| `ovim trace -s X` | `ovim lsp trace -s X` |
| `ovim wait-lsp -s X` | `ovim lsp wait -s X` |
| `ovim check-lsp FILE` | `ovim lsp check FILE` |
| `ovim list-languages` | `ovim lsp languages` |
| `ovim mcp METHOD PARAMS` | removed (use curl) |
| `ovim edit --old X --new Y -s S` | `ovim edit FILE --old X --new Y` |
| `ovim send "keys"` (auto-discover) | `ovim send "keys" -s NAME` (explicit) |
