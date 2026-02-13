# ovim Issue Tracker

| ID | Status | Priority | Complexity | Description |
|----------|---------|----------|------------|-------------|
| OV-00003 | Pending | MEDIUM | Low | [MCP] `symbols` command fails with "Unknown tool: get_symbols" - should fall back to standard LSP textDocument/documentSymbol if MCP tool unavailable (cli/symbols.rs) |
| OV-00004 | Pending | MEDIUM | Low | [MCP] `diagnostics` command fails with "Unknown tool: get_diagnostics" - should fall back to standard LSP textDocument/publishDiagnostics cache (cli/diagnostics.rs) |
| OV-00005 | Pending | LOW | Low | [STATE] `snapshot` returns null for cursor_line, cursor_col, file fields - state not being serialized correctly (cli/snapshot.rs) |
| OV-00006 | Pending | TRIAGE | Medium | [LSP] `hover` returns null on valid symbol positions - investigate if this is ovim not forwarding request correctly or LSP response handling issue |
| OV-00007 | Pending | TRIAGE | Medium | [LSP] `find-references` returns empty array - investigate if ovim is correctly forwarding LSP request and parsing response |
| OV-00015 | Pending | MEDIUM | Medium | [PERF] Incremental wrap map invalidation only covers cursor line — [details](issue-docs/OV-00015-incremental-invalidation-cursor-only.md). (src/editor/mod.rs:ensure_wrap_map) |
| OV-00016 | Pending | LOW | Medium | [WRAP] No virtcol/curswant tracking for gj/gk — [details](issue-docs/OV-00016-virtcol-curswant.md). (src/editor/input/normal/pending_commands.rs) |
| OV-00019 | Triage | HIGH | N/A | When there is a wrapping line above the cursor in the current buffer and the textwidth option is set, there is a miscalculation causing the vertical cursor position to be incorrect (cursor is below input point). There seems to be other related bugs, so investigation so required. |
| OV-00023 | Pending | LOW | Low | [TEST] `bug_reproduction_test` segfaults (SIGSEGV signal 11) on both main and feature branches — likely mlua/LuaJIT FFI issue, pre-existing. |
| OV-00025 | Done | MEDIUM | Low | [CMD] Up/Down arrows in path completion don't update command line text — visual selection changes but command line stays the same, so Enter executes the Tab-selected entry, not the arrow-selected one. (src/editor/input/commands.rs) |
| OV-00046 | Pending | HIGH | High | [ARCH] `commands.rs` is 2K+ line monolith with duplicated handlers — `:bn`/`:bnext` handled in both top-level match AND else-if chain; `:set` has 400 lines of near-identical arms. Needs command table or macro. (src/commands.rs) |
| OV-00047 | Pending | MEDIUM | High | [ARCH] `lsp/server.rs` has 24 AtomicBool fields + 290 lines of capability boilerplate — adding a capability requires changes in 4 places. File is 1900+ lines. Replace with bitflags. (src/lsp/server.rs) |
| OV-00049 | Pending | LOW | Low | [WINDOW] Window focus overlap preference is dead code — `focus_left/right/up/down` originally had overlap preference checks (`vertical_overlap > 0`) but `|| true` made them no-ops. Now removed. To restore overlap preference, `focus_directional` sort should weight overlapping candidates higher (e.g., multi-key sort: overlap then distance). |
| OV-00054 | Done | TRIAGE | Medium | [LSP] TypeScript diagnostics not appearing — Fixed: `publish_diagnostics` client capability was missing from `TextDocumentClientCapabilities`. `typescript-language-server` checks `Boolean(publishDiagnostics)` and sets `diagnosticsSupport = false` when it's `None`, suppressing all diagnostic notifications. Added `PublishDiagnosticsClientCapabilities` with `related_information`, `tag_support`, and `version_support`. (ovim-core/src/lsp/server.rs:761) |
| OV-00055 | Pending | LOW | Low | [DEAD CODE] `Change::join_lines()` constructor has zero callers — superseded by `RepeatAction::JoinLines` + `Buffer::join_lines()`. The constructor (ovim-core/src/change.rs:269-285) and its match arms in `repeat()` and `undo_redo()` are dead code. (ovim-core/src/change.rs:269) |
| OV-00056 | Done | MEDIUM | Low | [PANIC] `wrap_map.as_mut().unwrap()` can panic if wrap_map is None — two unwrap calls on Option in `ensure_wrap_map`. If wrap_map initialization fails or is called before setup, these panic in production. Should use `if let Some(map)` or initialize defensively. (ovim-core/src/editor/mod.rs:714,720) |
| OV-00057 | Done | MEDIUM | Low | [LOGIC] Inconsistent cursor clamping between `validate_cursor_position()` and `clamp_cursor_col()` — `validate_cursor_position()` uses `col >= line_len` (clamps col 0 on 1-char line), while `clamp_cursor_col()` uses `col > 0 && col >= line_len` (preserves col 0). Both use grapheme_count. The `col > 0` guard in `clamp_cursor_col` is intentional (col 0 is always valid), so `validate_cursor_position` has the bug: it would clamp col 0 to col 0 on a 1-char line (no-op in practice, but semantically wrong — it enters the branch unnecessarily). (ovim-core/src/buffer/mod.rs:241, ovim-core/src/buffer/text_ops.rs:307) |
| OV-00058 | Done | HIGH | Medium | [FEATURE] Operator/motion gap: `b`, `e`, `B`, `E`, `h`, `0`, `^`, `W` now wired with d/c/y operators. All have RepeatAction variants for dot-repeat and undo. |
| OV-00059 | Pending | LOW | Low | [TEST] No unit tests for Buffer methods `toggle_char_at_cursor()`, `indent_lines_at()`, `dedent_lines_at()`, `clamp_cursor_col()` — these were extracted from Editor-level code and have integration test coverage via repeat_action_test and cursor_clamping_test, but no direct Buffer-level unit tests. (ovim-core/src/buffer/text_ops.rs) |
| OV-00060 | Pending | LOW | Low | [TEST] Missing `~` toggle case on empty line test — `toggle_char_at_cursor()` returns false on empty line but no test verifies this edge case. Also missing: undo/redo cycle tests for each RepeatAction variant (indent, dedent, toggle case, join). (ovim-core/src/buffer/text_ops.rs:325) |
| OV-00061 | Pending | MEDIUM | High | [ARCH] 11 files exceed 1.5k lines (CLAUDE.md says refactor at 3k, split before adding) — editor/mod.rs (1952), lsp/server.rs (1943), motions.rs (1898), commands.rs (1840), lsp/requests.rs (1815), change.rs (1769), subcommands.rs (1766), renderer/buffer.rs (1743), input/commands.rs (1636), helpers.rs (1611), operators.rs (1523). Several are approaching or past the 2k guideline. (multiple files) |
| OV-00062 | Pending | MEDIUM | High | [ARCH] Dual undo system mid-migration — Pattern A (`Change::insert/delete` + `add_change()`, 73 call sites) coexists with Pattern B (`record()` + `push_recorded_undo()` + `set_repeat_action()`, ~47 call sites). Pattern A is still needed for insert-mode undo grouping. The `Change` enum has dead variants (like `JoinLines`) from the migration. No clear boundary between which operations use which pattern. (ovim-core/src/change.rs, ovim-core/src/editor/change_tracking.rs) |
| OV-00063 | Done | HIGH | Medium | [UNDO] `pop_last_change` pops unrelated undo entry when delete phase produces no edits — Fixed: token-based undo stack safety (commit 5958688). |
| OV-00064 | Done | MEDIUM | Medium | [UNDO] save_point corrupted by pop_last_change — Fixed: token-based undo stack safety (commit 5958688). |
| OV-00065 | Done | LOW | Low | [UNDO] Missing validate_cursor_position in semantic change undo variants — Fixed: token-based undo stack safety (commit 5958688). |
| OV-00066 | Done | MEDIUM | Medium | [INDENT] expandtab setting now consulted by indent/dedent. |
| OV-00067 | Done | MEDIUM | Low | [INDENT] shiftwidth setting now used for indent operations. |
| OV-00068 | Done | LOW | Low | [INDENT] Empty lines now skipped during indent. |
| OV-00069 | Done | MEDIUM | Low | [INDENT] Cursor now positioned on start line at first non-blank after `>>`. |
| OV-00070 | Done | LOW | Low | [INDENT] Cursor now at first non-blank after `>>`. |
| OV-00071 | Done | LOW | Low | [INDENT] Cursor now moved to first non-blank after `<<`. |
| OV-00072 | Done | MEDIUM | Medium | [INDENT] Visual mode `=` now has undo support via `record()`. |
| OV-00073 | Done | LOW | Low | [INDENT] auto_indent_lines now uses char count for leading_len. |
| OV-00074 | Done | LOW | Low | [INDENT] Ctrl-T now respects expandtab setting. |
| OV-00075 | Done | HIGH | Medium | [SCROLL] `viewport_command_active` removed entirely (commit fbde4c2). |
| OV-00076 | Done | HIGH | Medium | [SCROLL] Ctrl-e/Ctrl-y now update buffer cursor (commit fbde4c2). |
| OV-00077 | Done | MEDIUM | Medium | [SCROLL] update_scroll_offset now uses focused window height (commit fbde4c2). |
| OV-00078 | Done | MEDIUM | Low | [SCROLL] sidescrolloff now clamped (commit fbde4c2). |
| OV-00079 | Done | MEDIUM | Low | [SCROLL] N/A — viewport_command_active removed entirely (commit fbde4c2). |
| OV-00080 | Done | HIGH | Low | [MOTION] `G` now clamped with `.min(max_line)`. |
| OV-00081 | Done | HIGH | Low | [MOTION] `gg` now clamped with `.min(max_line)`. |
| OV-00082 | Done | MEDIUM | Medium | [MOTION] `b` from col 0 now correct backward walk. |
| OV-00083 | Done | MEDIUM | Low | [MOTION] `$` now handles count (count-1 lines down). |
| OV-00084 | Done | LOW | Low | [MOTION] `_` now handles `count > 1`. |
| OV-00085 | Done | LOW | Low | [MOTION] `+` now uses `buffer.line_count()`. |
| OV-00086 | Done | MEDIUM | Medium | [MOTION] `%` doesn't search forward for bracket when cursor is not on one — Vim searches forward on current line for nearest bracket, then jumps to its match. ovim only works if cursor is already on a bracket. Also matches `<`/`>` which Vim doesn't. (ovim-core/src/editor/motions.rs:734-746) |
| OV-00087 | Done | MEDIUM | Low | [MOTION] `t` now returns false for no movement (`i - 1 > col`). |
| OV-00088 | Done | MEDIUM | Low | [MOTION] `]}` now skips cursor with `abs_pos + 1`. |
| OV-00089 | Done | MEDIUM | Medium | [MOTION] `ge` now has correct backward algorithm. |
| OV-00090 | Done | MEDIUM | Low | [PASTE] p/P count now implemented via `text.repeat(count)`. |
| OV-00091 | Done | MEDIUM | Low | [PASTE] P cursor now correctly positioned (`end_pos.1 - 1`). |
| OV-00092 | Done | MEDIUM | Medium | [PASTE] Visual paste now updates unnamed register. |
| OV-00093 | Done | MEDIUM | Medium | [PASTE] Visual-line now uses `paste_before`. |
| OV-00094 | Done | MEDIUM | Medium | [REGISTER] `delete_history` now `Vec<RegisterContent>`. |
| OV-00095 | Done | LOW | Low | [REGISTER] Named register ops now update unnamed register. |
| OV-00096 | Done | LOW | Low | [REGISTER] Uppercase append now updates RegisterType. |
| OV-00097 | Done | LOW | Low | [PASTE] Empty buffer paste edge case fixed. |
| OV-00098 | Done | LOW | Low | [PASTE] Last line paste trailing newline fixed. |
| OV-00099 | Done | HIGH | Medium | [EDIT] `:e!` does not clear undo history on reload — Fixed: `reload_from_disk()` calls `reset_derived_state()` which replaces ChangeManager. (ovim-core/src/buffer/file_io.rs) |
| OV-00100 | Done | HIGH | Medium | [EDIT] `:e` (bare) does not clear undo history on reload — Fixed: same path as OV-00099, `:e` calls `reload_from_disk()`. (ovim-core/src/commands.rs) |
| OV-00101 | Done | HIGH | Low | [EDIT] `:e filename` does not check for unsaved changes — Fixed: commands.rs checks `editor.is_modified()` before loading. (ovim-core/src/commands.rs) |
| OV-00102 | Done | MEDIUM | Low | [EDIT] `:e! filename` (force-edit different file) not supported — Fixed: commands.rs handles `e! ` prefix with tilde expansion. (ovim-core/src/commands.rs) |
| OV-00103 | Done | MEDIUM | Low | [EDIT] `reload_from_disk` does not reset fold state — Fixed: `reset_derived_state()` calls `fold_manager.delete_all()`. (ovim-core/src/buffer/mod.rs) |
| OV-00104 | Done | MEDIUM | Low | [EDIT] `reload_from_disk` does not increment version counter — Fixed: `reset_derived_state()` increments `self.version`. (ovim-core/src/buffer/mod.rs) |
| OV-00105 | Done | MEDIUM | Medium | [EDIT] `reload_from_disk` does not notify LSP of content change — Fixed: added `mark_buffer_modified_force_send()` after reload in `:e`, `:e!`, and `:checktime` handlers. (ovim-core/src/commands.rs) |
| OV-00106 | Done | MEDIUM | Low | [EDIT] Cursor clamping in `reload_from_disk` includes newline char — Fixed: uses `line_count()` + `validate_cursor_position()`. (ovim-core/src/buffer/file_io.rs) |
| OV-00107 | Done | MEDIUM | Medium | [EDIT] `:checktime` reload doesn't reset undo/cursor/folds — Fixed: `reload_if_changed()` calls `reset_derived_state()` + `validate_cursor_position()`. (ovim-core/src/buffer/file_io.rs) |
| OV-00108 | Done | LOW | Low | [EDIT] `reload_from_disk` does not reset git status/blame — Fixed: `reset_derived_state()` resets `git_status` and `git_blame`. (ovim-core/src/buffer/mod.rs) |
| OV-00109 | Done | LOW | Low | [EDIT] `reload_from_disk` does not clear highlight/cache state — Fixed: `reset_derived_state()` clears all highlight/cache fields. (ovim-core/src/buffer/mod.rs) |
| OV-00110 | Done | LOW | Low | [DEAD CODE] Duplicate `:e filename` handler in input/commands.rs — Fixed: confirmed no `:e` handler exists in input/commands.rs. (ovim-core/src/editor/input/commands.rs) |
| OV-00111 | Done | CRITICAL | Medium | [LSP] Incremental sync uses char count instead of UTF-16 code units — Fixed: extracted `char_col_to_utf16`/`utf16_to_char_col` into `lsp/position.rs`, updated `compute_simple_diff` to use UTF-16 positions. (ovim-core/src/lsp/position.rs, utils.rs) |
| OV-00112 | Done | HIGH | Low | [LSP] All response parsers silently swallow deserialization errors — Fixed: created `parse_lsp_response<T>()` helper that logs parse failures, replaced all 25 `.ok()` sites. (ovim-core/src/lsp/requests.rs) |
| OV-00113 | Done | HIGH | Medium | [LSP] Pending request leaked on early return after HashMap insertion — Fixed: reordered `request()` to insert pending entry AFTER precondition checks and channel send. (ovim-core/src/lsp/server.rs) |
| OV-00114 | Done | HIGH | Medium | [LSP] Reader now loops over headers until empty line, correctly handling multi-header responses per LSP spec. |
| OV-00115 | Done | HIGH | Medium | [LSP] Cancellation `.take()` silently drops pending responses of other variant types — Fixed: replaced `PendingLspResponse` enum with `PendingLspResponses` struct with typed Option fields per request type. (ovim-core/src/editor/lsp_state.rs, hover.rs, goto.rs, lsp_integration.rs) |
| OV-00116 | Done | HIGH | Low | [LSP] `send_lsp_changes_if_modified` can send didChange before didOpen — Fixed: added `did_open_sent` guard that returns early. (ovim-core/src/editor/lsp_integration.rs) |
| OV-00117 | Done | HIGH | Low | [LSP] `update_diagnostics` assigns cached count to itself — Fixed: compute count directly from fetched diagnostics instead of self-assignment. (ovim-core/src/editor/lsp_modules/diagnostics.rs) |
| OV-00118 | Done | HIGH | Low | [LSP] `expect()` panics reachable from user input in document symbols picker — Fixed: replaced `.expect()` with graceful error handling that returns early on scratch buffers. (ovim-core/src/editor/lsp_modules/references.rs) |
| OV-00119 | Won't Fix | HIGH | Medium | [LSP] Formatting edits returned unsorted — `apply_lsp_edits()` already sorts edits back-to-front before applying. No change needed. (ovim-core/src/lsp/requests.rs) |
| OV-00120 | Done | HIGH | Medium | [LSP] `did_save_broadcast` uses non-broadcast flush — Fixed: changed to `flush_pending_changes_broadcast`. (ovim-core/src/lsp/notifications.rs) |
| OV-00121 | Done | HIGH | Medium | [LSP] Broadcast flush increments version once per server — Fixed: increment version once before loop, use `send_did_change_with_version` for all servers. (ovim-core/src/lsp/notifications.rs) |
| OV-00122 | Done | HIGH | Medium | [LSP] start_server TOCTOU race — Fixed: added `starting_servers: DashSet<String>` guard to prevent concurrent initialization. (ovim-core/src/lsp/mod.rs) |
| OV-00123 | Done | HIGH | Low | [LSP] Supervisor backoff counter never resets — Fixed: reset `restarts = 0` after healthy runs (>60s uptime). (ovim-core/src/lsp/supervisor.rs) |
| OV-00124 | Done | HIGH | Low | [LSP] Server-initiated requests dropped when notification channel full — Fixed: enhanced backpressure logging for all dropped server-initiated requests. (ovim-core/src/lsp/notifications.rs) |
| OV-00125 | Done | MEDIUM | Low | [LSP] `shutdown()` does not drain pending_requests — Fixed: drain all pending requests with error before shutdown. (ovim-core/src/lsp/server.rs) |
| OV-00126 | Done | MEDIUM | Low | [LSP] Windows line endings cause off-by-one in incremental diff — Fixed: strip `\r` from lines after splitting. (ovim-core/src/lsp/utils.rs) |
| OV-00127 | Done | MEDIUM | Low | [LSP] Rename now handles documentChanges (workspace_edits.rs:56-130). |
| OV-00128 | Done | MEDIUM | Low | [LSP] No guard against double didOpen for same URI — Fixed: check `document_versions` before sending; log warning and return on duplicate. (ovim-core/src/lsp/notifications.rs) |
| OV-00129 | Won't Fix | MEDIUM | Low | [LSP] `workspace/applyEdit` responds success before edit is applied — By design, matches VS Code behavior of optimistic response. (ovim-core/src/lsp/notifications.rs) |
| OV-00130 | Done | MEDIUM | Low | [LSP] Notification listener JoinHandle dropped, task not cancellable — Fixed: store handle in `listener_handles: DashMap`, abort on `stop_server`. (ovim-core/src/lsp/mod.rs, notifications.rs) |
| OV-00131 | Done | MEDIUM | Low | [LSP] Diagnostics not cleaned up when server is stopped — Fixed: `stop_server()` now removes diagnostics per server and sets `diagnostics_changed` flag. (ovim-core/src/lsp/mod.rs) |
| OV-00132 | Blocked | MEDIUM | Low | [LSP] `type_hierarchy` capability hardcoded to false — Blocked on lsp-types upgrade. Added TODO comment. (ovim-core/src/lsp/server.rs) |
| OV-00133 | Done | MEDIUM | Low | [LSP] `set_capability_by_method` maps wrong method for executeCommand — Fixed: corrected to `workspace/executeCommand`. (ovim-core/src/lsp/server.rs) |
| OV-00134 | Done | MEDIUM | Low | [LSP] Diagnostic column comparison skips UTF-16 conversion — Fixed: use `utf16_to_char_col` for comparison. (ovim-core/src/editor/lsp_modules/diagnostics.rs) |
| OV-00135 | Done | MEDIUM | Low | [LSP] Supervisor "exponential" backoff is actually linear — Fixed: use `initial_backoff * 2^(restarts-1)` for true exponential backoff. (ovim-core/src/lsp/supervisor.rs) |
| OV-00136 | Done | LOW | Low | [LSP] LSP error code -32801 (ContentModified) not handled specially — Fixed: handle alongside -32800 (Cancelled) with debug log only. (ovim-core/src/lsp/server.rs) |
| OV-00137 | Done | LOW | Low | [LSP] `clear_lsp_state` does not clear hover_cache or abort pending responses — Fixed: abort all pending response tasks and clear `hover_cache` in `clear_lsp_state`. (ovim-core/src/editor/lsp_integration.rs) |
| OV-00138 | Pending | MEDIUM | High | [UNICODE] Grapheme vs char column systemic inconsistency — `clamp_cursor_col()` and `validate_cursor_position()` use `grapheme_count()` (visual characters), but `delete_range()`, `insert_text_at()`, `line_len()`, and all text_ops functions use `chars().count()` (Unicode scalar values). Cursor columns are clamped to grapheme bounds but passed as char indices to rope operations. For ASCII this is harmless, but multi-codepoint characters (emoji, combining marks) cause cursor misalignment. Most visible: `join_lines_impl` junction_col (text_ops.rs:269), `toggle_char_at_cursor` advance (text_ops.rs:365), `modify_number` end_col (text_ops.rs:452), search result columns (search.rs:115-197), text object boundaries (textobjects.rs). Highlighting has TODOs at highlighting.rs:175,192 acknowledging this. Fix requires choosing one column unit system and converting consistently. (ovim-core/src/buffer/text_ops.rs, ovim-core/src/buffer/mod.rs:240, ovim-core/src/buffer/mod.rs:306) |
| OV-00139 | Done | LOW | Low | [MOTION] `section_forward()` and `section_end_forward()` use `rope.len_lines()` instead of `buffer.line_count()` — Fixed: both now use `buffer.line_count()`. (ovim-core/src/editor/motions.rs) |
| OV-00140 | Done | LOW | Low | [VISUAL] `handle_yb` and `handle_y_big_b` missing `set_yank_flash_range()` — every other yank handler calls it but these two didn't, so yb/yB yanks had no visual flash feedback. Fixed: added flash calls. (ovim-core/src/editor/input/normal/operators.rs:1595,1630) |
| OV-00141 | Done | HIGH | Low | [LSP] Paste (p/P), toggle case (~), and other operations using `record()` directly never called `mark_buffer_modified()` — LSP `didChange` was never sent after paste, so diagnostics were never refreshed. Fixed: `push_recorded_undo()` now calls `mark_buffer_modified()` automatically. Also: `mark_buffer_modified()` now clears stale cached diagnostics to prevent wrong-line rendering. (ovim-core/src/editor/change_tracking.rs:135, ovim-core/src/editor/lsp_integration.rs:689) |
| OV-00142 | Done | HIGH | Low | [LSP] `repeat_last_change()` (dot-repeat) never called `mark_buffer_modified()` — both RepeatAction and Change-based paths pushed directly to undo stack, bypassing LSP notification. Dot-repeating any edit wouldn't trigger `didChange`. Fixed: added `self.mark_buffer_modified()` after both paths. (ovim-core/src/editor/change_tracking.rs:90,120) |
| OV-00143 | Done | MEDIUM | Low | [LSP] `accept_completion()` never called `mark_buffer_modified()` — accepting an LSP completion modified the buffer but the server was never notified via `didChange`. Fixed: added `mark_buffer_modified()` after `add_change()`. (ovim-core/src/editor/ui_features.rs:105) |
| OV-00144 | Done | MEDIUM | Low | [LSP] `confirm_substitute()` (interactive `:s///c`) never called `mark_buffer_modified()` — confirming a substitution modified the buffer but no `didChange` was sent. Fixed: added `mark_buffer_modified()` after `add_change()`. (ovim-core/src/editor/ui_features.rs:373) |
| OV-00145 | Done | MEDIUM | Low | [LSP] Visual block insert replay happened before `mark_buffer_modified()` in `exit_insert_mode()` — the first line's change triggered `didChange` but remaining lines' insertions were invisible to LSP. Fixed: moved `mark_buffer_modified()` after visual block replay. (ovim-core/src/editor/input/insert_mode.rs:262) |
| OV-00146 | Done | MEDIUM | Low | [NAV] Diagnostic navigation (`]d`/`[d`) hardcoded column 0 — `goto_next_diagnostic()` and `goto_prev_diagnostic()` extracted only `d.range.start.line` and ignored `d.range.start.character`. Fixed: extract both line and character, convert UTF-16 to char column via `utf16_to_col()`, compare position by (line, col) for same-line diagnostics. (ovim-core/src/editor/mod.rs:1688,1715) |
| OV-00147 | Done | MEDIUM | Low | [NAV] Live grep results hardcoded column 0 — `spawn_grep_search()` set `col: 0` in all `PickerResult`s. Fixed: use `matcher.find()` to get match byte offset within the line, convert to char column. Display format now includes column (`file:line:col`). (ovim-core/src/editor/grep.rs:131) |
| OV-00148 | Done | LOW | Low | [NAV] `PickerAction::OpenFile` missing `validate_cursor_position()` and `center_cursor_in_viewport()` — cursor could land on invalid position and viewport wouldn't scroll to show the match. `OpenFileWithTag` already had both calls. Fixed: added both after `set_position()`. (ovim-core/src/editor/picker_manager.rs:506) |
| OV-00149 | Done | HIGH | Medium | [LSP] Debounce timer flushes to single server, not broadcast — Fixed: `process_flush_requests()` now calls `flush_pending_changes_broadcast()` with language_id, broadcasting to all servers. (ovim-core/src/lsp/notifications.rs:1111) |
| OV-00150 | Done | HIGH | Medium | [PERF] Version lock held across `.await` in `send_did_change_immediate` — Fixed: lock is now acquired, version incremented, params built, then lock dropped before I/O. Both `send_did_change_immediate` and `flush_pending_changes_broadcast` follow this pattern. (ovim-core/src/lsp/notifications.rs:192-210) |
| OV-00151 | Done | HIGH | Medium | [PERF] `merge_diagnostics()` called 2-3x per render frame under Mutex — Fixed: added `merged_diagnostics_cache` that is checked first; cache invalidated on `set_diagnostics` and `stop_server`. (ovim-core/src/lsp/mod.rs:432-512) |
| OV-00152 | Pending | HIGH | Low | [PERF] Full document content cloned into debouncer on every keystroke — `ChangeDebouncer::new()` clones `text` and `old_text` (entire file content). For 100KB files, this is 200KB+ allocation per keystroke during rapid typing. (ovim-core/src/lsp/notifications.rs:358-371) |
| OV-00153 | Done | MEDIUM | Low | [LSP] 3 request methods bypass `parse_lsp_response` helper — Fixed: `document_symbols` and `workspace_symbols` already had logging added for both parse paths. `inlay_hints` already uses `parse_lsp_response`. `semantic_tokens_full` and `semantic_tokens_range` now use `parse_lsp_response` instead of bare `from_value`. (ovim-core/src/lsp/requests.rs) |
| OV-00154 | Done | MEDIUM | Low | [LSP] Health check uses `child.id()` instead of `try_wait()` — Fixed: now uses `child.try_wait()` to detect exited/zombie processes. (ovim-core/src/lsp/server.rs:1460-1472) |
| OV-00155 | Done | MEDIUM | Low | [LSP] Initialize timeout leaves orphaned pending request — Fixed: `initialize()` now catches timeout and transitions to `Failed` state, triggering cleanup of orphaned pending requests. (ovim-core/src/lsp/server.rs:687-716) |
| OV-00156 | Done | MEDIUM | Low | [LSP] `ShuttingDown` state not rejected in request gate — Fixed: `request()` now explicitly rejects `ShuttingDown` state with immediate error. (ovim-core/src/lsp/server.rs:1185-1189) |
| OV-00157 | Done | MEDIUM | Low | [LSP] `pending_completion` not cleared on buffer switch — Fixed: `clear_lsp_state()` now takes and aborts `pending_completion`. (ovim-core/src/editor/lsp_integration.rs:683-685) |
| OV-00158 | Pending | MEDIUM | Low | [PERF] Double serialization for message size validation — `request()` serializes message to check size against MAX_MESSAGE_SIZE, then `write_message()` serializes again for transmission. Every LSP request/notification is serialized twice. (ovim-core/src/lsp/server.rs:1127-1140) |
| OV-00159 | Pending | LOW | Low | [LSP] Capability flags use `Ordering::Relaxed` — all 24 AtomicBool capability caches use Relaxed ordering. No happens-before guarantee between init task writing and editor task reading. Safe on x86-64 but could cause stale reads on ARM. (ovim-core/src/lsp/server.rs:1456-1627) |
| OV-00160 | Pending | LOW | Low | [LSP] No state transition validation in server state machine — `transition_to()` accepts any state regardless of current state. Invalid transitions (Ready→Initializing, Terminated→Ready) are not prevented. (ovim-core/src/lsp/server.rs:947-970) |

## Bugs Filed Against Hyperion (if any)

These may belong in Hyperion's tracker after investigation:

| ID | Status | Priority | Description |
|----|--------|----------|-------------|
| HY-TRIAGE-01 | Triage | UNKNOWN | hover not returning info for method calls - may be unimplemented in hyperion-lsp |
| HY-TRIAGE-02 | Triage | UNKNOWN | find-references returning empty - may be unimplemented in hyperion-lsp |
| HY-TRIAGE-03 | Triage | UNKNOWN | goto-definition fails on DTO type references (AdminSqlRequest) - possibly classpath/dependency resolution |

## Notes

- OV-00003 and OV-00004 are MCP vs LSP protocol mismatch - ovim uses MCP, some LSPs don't implement those tools
- HY-TRIAGE items need investigation to determine if they're hyperion-lsp gaps or ovim request/response issues
- Closed issues moved to [CLOSED_ISSUES.md](CLOSED_ISSUES.md)
