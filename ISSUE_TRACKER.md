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
| OV-00026 | Pending | HIGH | Medium | [VISUAL] V<C-u> and V<C-d> broken — half-page scrolling in visual line mode doesn't work correctly. Needs investigation. (src/editor/input/) |

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
