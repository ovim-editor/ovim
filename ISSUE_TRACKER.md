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
| OV-00033 | Fixed | HIGH | Medium | [NUM] `format_number` produces two's complement for negative hex/bin/oct — Fixed: use unsigned_abs() with sign prefix for hex/bin/oct formatting. |
| OV-00034 | Fixed | HIGH | Medium | [NUM] Hex digit scanning only checks `is_ascii_digit()` — Fixed: detect hex context and use is_ascii_hexdigit() for scanning when cursor is on a-f within 0x-prefixed numbers. |
| OV-00035 | Fixed | HIGH | Medium | [NAV] `sentence_backward` can panic on out-of-bounds col — Fixed: clamp col to chars.len()-1 and handle empty lines gracefully. |
| OV-00036 | Won't Fix | MEDIUM | Low | [NUM] `g Ctrl-A` first line gets zero delta — Investigation shows ovim matches Vim behavior: first line +0, second +1, etc. Existing tests confirm this. Not a bug. |
| OV-00037 | Fixed | MEDIUM | Low | [NUM] Ctrl-A/X searches backward for numbers, Vim only searches forward — Fixed: removed backward search branch; now searches forward-only when cursor is not on a digit. |
| OV-00038 | Fixed | MEDIUM | Low | [PASTE] Line paste doesn't reposition cursor to first non-blank — Fixed: after linewise p/P, cursor now moves to first non-blank character of the pasted line. |
| OV-00039 | Fixed | MEDIUM | Medium | [ENCODING] `reload_if_changed` ignores file encoding — Fixed: both reload_if_changed and reload_from_disk now use FileEncoding::detect + decode instead of hardcoded UTF-8. |
| OV-00040 | Fixed | MEDIUM | Low | [SEARCH] `search_next` forward wraps to col 0 of same line — Fixed: when cursor_col+1 >= line_len, now advances to next line (modulo line count) instead of staying on same line. |
| OV-00041 | Fixed | MEDIUM | Medium | [UNDO] Visual mode `~` creates N separate undo entries — Fixed: all per-line changes collected into a single outer composite before add_change. |
| OV-00042 | Fixed | MEDIUM | Medium | [UNDO] Visual block `r` creates N separate undo entries — Fixed: same pattern as OV-00041, collected into single composite. |
| OV-00043 | Fixed | LOW | Low | [WINDOW] Directional focus `|| true` makes overlap preference a no-op — Fixed: removed || true hack; direction-only filter with min_by_key(distance) for selection. |
| OV-00044 | Fixed | MEDIUM | Low | [LSP] No Content-Length validation before allocation in LSP reader — Fixed: bounds check against MAX_MESSAGE_SIZE before allocation. |
| OV-00045 | Won't Fix | LOW | Medium | [SEARCH] Backward search at col=0 produces empty search range — Investigation shows find_backward correctly falls through to previous lines when from_col=0. Not a bug in practice. |
| OV-00046 | Pending | HIGH | High | [ARCH] `commands.rs` is 2K+ line monolith with duplicated handlers — `:bn`/`:bnext` handled in both top-level match AND else-if chain; `:set` has 400 lines of near-identical arms. Needs command table or macro. (src/commands.rs) |
| OV-00047 | Pending | MEDIUM | High | [ARCH] `lsp/server.rs` has 24 AtomicBool fields + 290 lines of capability boilerplate — adding a capability requires changes in 4 places. File is 1900+ lines. Replace with bitflags. (src/lsp/server.rs) |
| OV-00048 | Fixed | MEDIUM | Medium | [RENDER] ANSI escape codes in buffer content interpreted by terminal — control chars (0x00-0x1F excl tab/newline, and 0x7F) now rendered as caret notation (e.g. `^[` for ESC, `^@` for NUL) with SpecialKey highlight group. Buffer stores original bytes untouched for round-trip fidelity. |
| OV-00049 | Pending | LOW | Low | [WINDOW] Window focus overlap preference is dead code — `focus_left/right/up/down` originally had overlap preference checks (`vertical_overlap > 0`) but `|| true` made them no-ops. Now removed. To restore overlap preference, `focus_directional` sort should weight overlapping candidates higher (e.g., multi-key sort: overlap then distance). |
| OV-00050 | Fixed | HIGH | Medium | [RENDER] `slice_horizontal_viewport` uses char count instead of display width — Fixed: rewrote to walk chars by display width, made h_offset display-column-based throughout (ensure_cursor_visible_horizontal, scroll-to-edge functions, shift_highlights_for_viewport). (ovim/src/ui/renderer/buffer.rs, ovim-core/src/editor/mod.rs, window_viewport.rs) |
| OV-00051 | Fixed | HIGH | Medium | [RENDER] Visual selection and diagnostic underlines misaligned on lines with tabs — Fixed: added char_mapping to expand_tabs_with_mapping, remap visual selection/bracket/diagnostic coordinates from original-text to expanded-text space before rendering. Diagnostic UTF-16 offsets properly converted to char indices first. (ovim/src/ui/renderer/buffer.rs, helpers.rs) |

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
