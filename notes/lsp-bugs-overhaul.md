# LSP Bugs Overhaul — Full Status

Comprehensive tracking of the bug fix sprint that started with 27 LSP issues (OV-00111–OV-00137) and expanded to cover the entire issue backlog.

## Phase Summary

| Phase | Description | Issues | Status |
|-------|-------------|--------|--------|
| LSP Sprint | LSP client bugs (OV-00111–OV-00137) | 23 fixed, 2 won't-fix, 1 blocked, 0 remaining | **Done** |
| Housekeeping | Mark silently-fixed issues as Done | ~35 issues | **Done** |
| Phase 1 | Operator motion gap + LSP reader | OV-00058, OV-00114 | **Done** |
| Phase 2 | Stability fixes | OV-00056, OV-00057, OV-00086, OV-00025 | **Done** |
| Phase 3 | Wrapping/cursor investigation | OV-00019, OV-00015 | Planned |
| Phase 4 | Architecture cleanup | OV-00046, OV-00055, OV-00062 | Planned |
| Phase 5 | Triage investigation | OV-00054, OV-00006, OV-00007 | Planned |
| Phase 6 | Polish | OV-00005, OV-00016, OV-00023, OV-00047, OV-00049, OV-00059–OV-00061 | Ongoing |

---

## LSP Sprint (Complete)

27 issues filed from systematic LSP bug hunt. 23 fixed, 2 won't-fix, 1 blocked on dependency, 0 remaining.

### Fixed (23)

| ID | Fix Summary |
|----|-------------|
| OV-00111 | Incremental sync: char→UTF-16 code units (position.rs) |
| OV-00112 | Response parsing: `.ok()` → `parse_lsp_response<T>()` (25 sites) |
| OV-00113 | Pending request leak: reorder insert after precondition checks |
| OV-00115 | Cancellation: `PendingLspResponse` enum → typed `PendingLspResponses` struct |
| OV-00116 | didChange before didOpen: added `did_open_sent` guard |
| OV-00117 | Diagnostics self-assignment: compute count from fetched data |
| OV-00118 | expect() panics: replaced with graceful error handling |
| OV-00120 | did_save broadcast: switched to `flush_pending_changes_broadcast` |
| OV-00121 | Broadcast version: increment once before server loop |
| OV-00122 | start_server TOCTOU: added `starting_servers: DashSet` guard |
| OV-00123 | Supervisor backoff: reset counter after healthy runs |
| OV-00124 | Dropped requests: enhanced backpressure logging |
| OV-00125 | shutdown(): drain pending_requests before shutdown |
| OV-00126 | Windows line endings: strip `\r` from lines |
| OV-00127 | Rename: handle `documentChanges` (workspace_edits.rs) |
| OV-00128 | Double didOpen: check `document_versions` before sending |
| OV-00130 | Listener handle: store in `listener_handles: DashMap`, abort on stop |
| OV-00131 | Diagnostics cleanup: remove per-server diagnostics on stop |
| OV-00133 | executeCommand capability: corrected method mapping |
| OV-00134 | Diagnostic column: use `utf16_to_char_col` for comparison |
| OV-00135 | Supervisor backoff: true exponential (`initial * 2^(n-1)`) |
| OV-00136 | ContentModified (-32801): handled alongside Cancelled |
| OV-00137 | clear_lsp_state: abort pending responses, clear hover_cache |

### Won't Fix (2)

| ID | Reason |
|----|--------|
| OV-00119 | `apply_lsp_edits()` already sorts edits back-to-front |
| OV-00129 | Optimistic response matches VS Code behavior (by design) |

### Blocked (1)

| ID | Reason |
|----|--------|
| OV-00132 | type_hierarchy capability: blocked on lsp-types upgrade |

### Remaining (0)

All LSP sprint issues resolved. OV-00114 was fixed in Phase 1 (commit 286e1a6).

---

## Housekeeping: Silently Fixed Issues (~35)

These issues were fixed through incremental improvements but never marked Done in the tracker.

### Undo stack safety (commit 5958688)
- **OV-00063**: pop_last_change pops unrelated entry → token-based approach
- **OV-00064**: save_point corrupted → generation counter
- **OV-00065**: Missing validate_cursor_position in undo variants

### Viewport/scroll unification (commit fbde4c2)
- **OV-00075**: viewport_command_active never reset → removed entirely
- **OV-00076**: Ctrl-e/Ctrl-y don't update buffer cursor → now update
- **OV-00077**: Wrong viewport_height for splits → window-level height
- **OV-00078**: sidescrolloff not clamped → now clamped
- **OV-00079**: undo doesn't reset viewport flag → flag removed

### Motion contracts (various commits)
- **OV-00080**: G clamped with `.min(max_line)`
- **OV-00081**: gg clamped with `.min(max_line)`
- **OV-00082**: b from col 0: correct backward walk
- **OV-00083**: $ with count: count-1 lines down
- **OV-00084**: _ with count: `count > 1` handling
- **OV-00085**: + uses `buffer.line_count()`
- **OV-00087**: t returns false for no movement
- **OV-00088**: ]} skips cursor char
- **OV-00089**: ge: correct backward algorithm

### Register type fidelity
- **OV-00094**: `delete_history` → `Vec<RegisterContent>`
- **OV-00095**: Named register ops update unnamed
- **OV-00096**: Uppercase append updates type

### Indentation option wiring
- **OV-00066**: expandtab consulted
- **OV-00067**: shiftwidth used
- **OV-00068**: Empty lines skipped
- **OV-00069**: Cursor on start line
- **OV-00070**: First non-blank positioning
- **OV-00071**: Dedent cursor positioning
- **OV-00072**: Visual `=` undo via `record()`
- **OV-00073**: Char count for leading_len
- **OV-00074**: Ctrl-T respects expandtab

### Paste fixes
- **OV-00090**: Count via `text.repeat(count)`
- **OV-00091**: P cursor corrected
- **OV-00092**: Visual paste updates unnamed register
- **OV-00093**: Visual-line uses paste_before
- **OV-00097**: Empty buffer paste fixed
- **OV-00098**: Last line trailing newline fixed

---

## Genuinely Remaining Issues

### Phase 1: Operator Motion Gap + LSP Reader (Done)

| ID | Priority | Description |
|----|----------|-------------|
| OV-00058 | HIGH | ~~Operator/motion gap~~ — All 8 motions wired with d/c/y operators + RepeatAction |
| OV-00114 | HIGH | ~~LSP reader single header~~ — Multi-header loop per LSP spec |

### Phase 2: Stability Fixes (Done)

| ID | Priority | Description |
|----|----------|-------------|
| OV-00056 | MEDIUM | ~~wrap_map unwrap panics~~ → `if let Some(map)` |
| OV-00057 | MEDIUM | ~~Cursor clamping inconsistency~~ → add `col > 0` guard |
| OV-00086 | MEDIUM | ~~% matches `<`/`>`~~ — removed angle brackets from `is_bracket()` |
| OV-00025 | MEDIUM | ~~Path completion arrows don't update command line text~~ — accept + set_command_line after select |

### Phase 3: Wrapping/Cursor Investigation

| ID | Priority | Description |
|----|----------|-------------|
| OV-00019 | HIGH | Wrapping line vertical cursor miscalculation |
| OV-00015 | MEDIUM | Incremental wrap map invalidation only covers cursor line |

### Phase 4: Architecture Cleanup

| ID | Priority | Description |
|----|----------|-------------|
| OV-00046 | HIGH | commands.rs 2K+ line monolith → command table/macro |
| OV-00055 | LOW | Dead `Change::join_lines()` removal |
| OV-00062 | MEDIUM | Dual undo system documentation |

### Phase 5: Triage Investigation

| ID | Priority | Description |
|----|----------|-------------|
| OV-00054 | TRIAGE | TypeScript diagnostics not appearing |
| OV-00006 | TRIAGE | hover returns null on valid positions |
| OV-00007 | TRIAGE | find-references returns empty |

### Phase 6: Polish (Ongoing, Low Priority)

| ID | Priority | Description |
|----|----------|-------------|
| OV-00005 | LOW | Snapshot returns null for cursor/file fields |
| OV-00016 | LOW | No virtcol/curswant for gj/gk |
| OV-00023 | LOW | bug_reproduction_test segfaults (mlua/LuaJIT) |
| OV-00047 | MEDIUM | lsp/server.rs capability boilerplate → bitflags |
| OV-00049 | LOW | Window focus overlap preference is dead code |
| OV-00059 | LOW | Missing unit tests for Buffer extract methods |
| OV-00060 | LOW | Missing ~ edge case tests |
| OV-00061 | MEDIUM | 11 files exceed 1.5k lines |

### Not Tracked Here

| ID | Status | Description |
|----|--------|-------------|
| OV-00003 | Pending | MCP symbols fallback |
| OV-00004 | Pending | MCP diagnostics fallback |
| OV-00132 | Blocked | type_hierarchy (lsp-types upgrade) |

---

## Statistics

- **Total issues filed**: 101 (OV-00003 through OV-00137, with gaps)
- **Done**: ~76 (23 LSP sprint + 12 buffer/edit + ~35 housekeeping + 2 Phase 1 + 4 Phase 2)
- **Won't Fix**: 2
- **Blocked**: 1
- **Genuinely remaining**: ~15
  - HIGH: 2 (OV-00019, OV-00046)
  - MEDIUM: 3
  - LOW: 7
  - TRIAGE: 3
