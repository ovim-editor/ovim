# ovim Issue Tracker

| ID | Status | Priority | Complexity | Description |
|----------|---------|----------|------------|-------------|
| OV-00001 | Fixed | HIGH | Medium | [NAV] Ex command `:N` doesn't update cursor position - Fixed: Added line-number command parsing to src/commands.rs |
| OV-00002 | Fixed | HIGH | Medium | [NAV] Normal mode `gg` doesn't update cursor - Fixed: Code analysis confirmed implementation correct; also fixed client error handling in src/client.rs |
| OV-00003 | Pending | MEDIUM | Low | [MCP] `symbols` command fails with "Unknown tool: get_symbols" - should fall back to standard LSP textDocument/documentSymbol if MCP tool unavailable (cli/symbols.rs) |
| OV-00004 | Pending | MEDIUM | Low | [MCP] `diagnostics` command fails with "Unknown tool: get_diagnostics" - should fall back to standard LSP textDocument/publishDiagnostics cache (cli/diagnostics.rs) |
| OV-00005 | Pending | LOW | Low | [STATE] `snapshot` returns null for cursor_line, cursor_col, file fields - state not being serialized correctly (cli/snapshot.rs) |
| OV-00006 | Pending | TRIAGE | Medium | [LSP] `hover` returns null on valid symbol positions - investigate if this is ovim not forwarding request correctly or LSP response handling issue |
| OV-00007 | Pending | TRIAGE | Medium | [LSP] `find-references` returns empty array - investigate if ovim is correctly forwarding LSP request and parsing response |
| OV-00008 | Fixed | HIGH | Medium | [WRAP] Display column vs character column mismatch in wrap scroll offset - Fixed: replaced substring+display_width hack with char_col_to_display_col in update_scroll_offset (src/editor/mod.rs) |
| OV-00009 | Fixed | MEDIUM | Medium | [PERF] ensure_wrap_map rebuilds entire map on every buffer edit - Fixed: use invalidate_line for O(1) single-line updates when only buffer version changed (src/editor/mod.rs:ensure_wrap_map) |
| OV-00010 | Fixed | HIGH | Low | [RENDER] Cursor positioning ignores horizontal_offset in nowrap mode - Fixed: subtract horizontal_offset from display_col in both nowrap branches of set_cursor_position (src/ui/renderer/core.rs) |
| OV-00011 | Fixed | MEDIUM | Medium | [WRAP] gj/gk use character column instead of display column - Fixed: convert char col to display col before wrap map ops, convert back to char col for target position (src/editor/input/normal/pending_commands.rs) |
| OV-00012 | Fixed | MEDIUM | Low | [CMD] Verify :set wrap / :set nowrap command wiring - Already wired up at commands.rs:1787-1802, no code change needed |
| OV-00013 | Fixed | LOW | Medium | [WRAP] split_line_into_rows doesn't handle wide characters at wrap boundaries - Fixed: use UnicodeWidthChar for display width tracking, pad and push wide chars to next row at boundaries (src/ui/renderer/buffer.rs) |
| OV-00014 | Pending | HIGH | Medium | [WRAP] WrapMap compute_visual_lines disagrees with renderer for wide chars — [details](issue-docs/OV-00014-wrapmap-wide-char-mismatch.md). (src/editor/wrap_map.rs, src/ui/renderer/buffer.rs) |
| OV-00015 | Pending | MEDIUM | Medium | [PERF] Incremental wrap map invalidation only covers cursor line — [details](issue-docs/OV-00015-incremental-invalidation-cursor-only.md). (src/editor/mod.rs:ensure_wrap_map) |
| OV-00016 | Pending | LOW | Medium | [WRAP] No virtcol/curswant tracking for gj/gk — [details](issue-docs/OV-00016-virtcol-curswant.md). (src/editor/input/normal/pending_commands.rs) |
| OV-00017 | Pending | LOW | Low | [CLEANUP] Remove delegate display_width in editor/mod.rs — [details](issue-docs/OV-00017-remove-delegate-display-width.md). (src/editor/mod.rs) |
| OV-00018 | Fixed | MEDIUM | High | [UX] Tab completion for file paths in command mode (:e, :tabe, :sp, etc.) — implemented with popup UI, Tab/BackTab cycling, prefix filtering. See OV-00025 for remaining arrow key issue. |
| OV-00019 | Triage | HIGH | N/A | When there is a wrapping line above the cursor in the current buffer and the textwidth option is set, there is a miscalculation causing the vertical cursor position to be incorrect (cursor is below input point). There seems to be other related bugs, so investigation so required. |
| OV-00020 | Fixed | LOW | Low | [CLEANUP] Dead `sign_width > 0` branch in gutter layout — always-true conditional with unreachable else. Fixed: removed dead branch, added SIGN_WIDTH/GUTTER_SPACING constants. |
| OV-00021 | Fixed | LOW | Low | [CLEANUP] `render_hover_window` takes 10 parameters — collapsed layout+viewport into `OverlayContext`. Also applied to `render_completion_menu` and `set_cursor_position`. |
| OV-00022 | Fixed | MEDIUM | Medium | [CLEANUP] `render_to_frame` is 218-line god method — decomposed into `clear_frame`, `compute_frame_layout`, `render_buffer_area`, `render_status_area`, `render_overlays` with `FrameAreas` struct. |
| OV-00023 | Pending | LOW | Low | [TEST] `bug_reproduction_test` segfaults (SIGSEGV signal 11) on both main and feature branches — likely mlua/LuaJIT FFI issue, pre-existing. |
| OV-00024 | Fixed | MEDIUM | Low | [CMD] Tab completion skips first entry when popup was already visible from typing — first Tab called `select_next()` before `accept()`. Fixed: added `tab_accepted` flag to distinguish typing-triggered popup from Tab-triggered. |
| OV-00025 | Pending | MEDIUM | Low | [CMD] Up/Down arrows in path completion don't update command line text — visual selection changes but command line stays the same, so Enter executes the Tab-selected entry, not the arrow-selected one. (src/editor/input/commands.rs) |
| OV-00026 | Fixed | HIGH | Medium | [VISUAL] V<C-u> and V<C-d> broken — half-page scrolling in visual modes was not implemented. Fixed: added Ctrl-D/Ctrl-U handlers to visual_mode.rs. |
| OV-00027 | Fixed | MEDIUM | Medium | [FILETREE] No scroll offset — file tree had no viewport scrolling, selection could go off-screen in large trees. Fixed: added scroll_offset + ensure_visible() to FileTree. |
| OV-00028 | Fixed | MEDIUM | Low | [FILETREE] Selection not clamped after directory collapse — collapsing a directory above selected item could leave selected_index out of bounds. Fixed: clamp in rebuild_flattened(). |
| OV-00029 | Fixed | LOW | Low | [FILETREE] Refresh loses expansion state — refresh() called open() which reset all expansion. Fixed: collect/restore expanded paths across refresh. |
| OV-00030 | Fixed | LOW | Low | [FILETREE] Dead `<Space>e` toggle code in legacy leader handler — leader.rs maps `<Space>e` to diagnostics, legacy handler was unreachable. Fixed: removed dead code. |
| OV-00031 | Fixed | MEDIUM | Medium | [FILETREE] `-` key was pure toggle, no reveal — pressing `-` didn't reveal current file in tree. Fixed: new semantics with reveal_path() and file_tree_reveal option. |
| OV-00032 | Fixed | LOW | Low | [FILETREE] `h` key only collapsed dirs, no parent navigation — on files or collapsed dirs, `h` did nothing. Fixed: navigate_to_parent() fallback. |
| OV-00033 | Pending | HIGH | Medium | [NUM] `format_number` produces two's complement for negative hex/bin/oct — Ctrl-X on `0x01` yields `0xffffffffffffffff` instead of `-0x1`. Rust's `format!("{:x}")` gives unsigned representation for negative i64. (src/editor/input/numbers.rs:387-412) |
| OV-00034 | Pending | HIGH | Medium | [NUM] Hex digit scanning only checks `is_ascii_digit()` — `0xff` with cursor on `f` fails to find the number; cursor on `0` extracts just `"0"` destroying the hex prefix. Needs `is_ascii_hexdigit()` and hex-aware prefix handling. (src/editor/input/numbers.rs:170,207-210) |
| OV-00035 | Pending | HIGH | Medium | [NAV] `sentence_backward` can panic on out-of-bounds col — if cursor column exceeds line length (e.g., `$` then `k` to shorter line), pressing `(` indexes into chars without clamping. (src/editor/motions.rs:1044-1045) |
| OV-00036 | Pending | MEDIUM | Low | [NUM] `g Ctrl-A` first line gets zero delta — `line_offset = line_idx - start_line` is 0 for first line, so first number is unchanged. Vim gives +1,+2,+3; ovim gives +0,+1,+2. (src/editor/input/numbers.rs:35-37) |
| OV-00037 | Pending | MEDIUM | Low | [NUM] Ctrl-A/X searches backward for numbers, Vim only searches forward — when cursor isn't on a digit, code searches backward first then forward; Vim searches forward-only on the current line. (src/editor/input/numbers.rs:216-261) |
| OV-00038 | Pending | MEDIUM | Low | [PASTE] Line paste doesn't reposition cursor to first non-blank — after `yy` then `p`, cursor is left at end of inserted text instead of first non-blank of pasted line. (src/editor/input/helpers.rs:494-501) |
| OV-00039 | Pending | MEDIUM | Medium | [ENCODING] `reload_if_changed` ignores file encoding — reload assumes UTF-8 via `String::from_utf8()` instead of using encoding detection from initial load path. Non-UTF-8 files fail to reload. (src/buffer/file_io.rs:411-458) |
| OV-00040 | Pending | MEDIUM | Low | [SEARCH] `search_next` forward wraps to col 0 of same line — when cursor is at end of line, `search_col=0` but `cursor_line` not advanced, can re-find same match or jump backward on line. (src/editor/search_manager.rs:109-125) |
| OV-00041 | Pending | MEDIUM | Medium | [UNDO] Visual mode `~` creates N separate undo entries — multi-line case toggle pushes per-line changes individually instead of a single composite. `u` only reverts one line. (src/editor/input/visual_mode.rs:831-889) |
| OV-00042 | Pending | MEDIUM | Medium | [UNDO] Visual block `r` creates N separate undo entries — same pattern as OV-00041, visual block replace loops per-line with individual add_change calls. (src/editor/input/visual_mode.rs:66-117) |
| OV-00043 | Pending | LOW | Low | [WINDOW] Directional focus `|| true` makes overlap preference a no-op — `focus_left/right/up/down` all have `vertical_overlap(candidate) > 0 \|\| true`, so overlap is never used as a tiebreaker. Wrong window chosen in complex splits. (src/editor/window.rs:736-789) |
| OV-00044 | Pending | MEDIUM | Low | [LSP] No Content-Length validation before allocation in LSP reader — malicious/buggy LSP sending huge Content-Length causes unchecked `vec![0u8; content_length]` allocation, potential OOM. MAX_MESSAGE_SIZE only checked for outgoing. (src/lsp/server.rs:498-522) |
| OV-00045 | Pending | LOW | Medium | [SEARCH] Backward search at col=0 produces empty search range — `find_backward` with `from_col=0` takes 0 chars, skipping any match at column 0 on current line. (src/editor/search_manager.rs:120-125) |
| OV-00046 | Pending | HIGH | High | [ARCH] `commands.rs` is 2K+ line monolith with duplicated handlers — `:bn`/`:bnext` handled in both top-level match AND else-if chain; `:set` has 400 lines of near-identical arms. Needs command table or macro. (src/commands.rs) |
| OV-00047 | Pending | MEDIUM | High | [ARCH] `lsp/server.rs` has 24 AtomicBool fields + 290 lines of capability boilerplate — adding a capability requires changes in 4 places. File is 1900+ lines. Replace with bitflags. (src/lsp/server.rs) |

## Bugs Filed Against Hyperion (if any)

These may belong in Hyperion's tracker after investigation:

| ID | Status | Priority | Description |
|----|--------|----------|-------------|
| HY-TRIAGE-01 | Triage | UNKNOWN | hover not returning info for method calls - may be unimplemented in hyperion-lsp |
| HY-TRIAGE-02 | Triage | UNKNOWN | find-references returning empty - may be unimplemented in hyperion-lsp |
| HY-TRIAGE-03 | Triage | UNKNOWN | goto-definition fails on DTO type references (AdminSqlRequest) - possibly classpath/dependency resolution |

## Notes

- OV-00001 fixed by adding goto-line parsing to commands.rs; OV-00002 fixed by improving client error handling
- OV-00003 and OV-00004 are MCP vs LSP protocol mismatch - ovim uses MCP, some LSPs don't implement those tools
- HY-TRIAGE items need investigation to determine if they're hyperion-lsp gaps or ovim request/response issues
