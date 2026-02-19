# Undo Migration Handoff Notes

## Context
Undo/repeat migration work was continued on `main` and committed in scoped slices.
Recent slices removed all `add_change` callsites from `input/commands.rs` and migrated text-object case operators to semantic `RepeatAction`.
`PRIORITIES.md` now reports 5 `add_change` callsites in `ovim-core/src` (infrastructural only), and visual-block `c` merge now redeems delete undo by token.

## Branch + Commits
Current branch: `main`

Recent undo-migration commits:
1. `62ebc5f` - Migrate visual delete dot-repeat to RepeatAction
2. `7da245a` - Migrate visual-block change dot-repeat to semantic action
3. `43f1784` - Make LSP ResourceOp workspace edits undoable
4. `b07ad91` - Migrate substitute-confirm edits to recorded undo
5. `e3277fc` - Migrate text-object changes to PendingChangeRepeat
6. `3ca93ee` - Fix change repeat insert point for C/c$ and text objects
7. `1e5733c` - Migrate completion accept undo to recorded edits
8. `e7871a6` - Tighten text-object change repeat contract
9. `f8c8165` - Add ci(paren) dot-repeat undo regression test
10. `be4878b` - Migrate text-object case ops to semantic repeat
11. `5e1419b` - Record :global delete undo via Pattern B
12. `ae38ea7` - Record ranged Ex delete undo via Pattern B
13. `2db973d` - Record shell filter undo via Pattern B
14. `67192db` - Migrate remaining Ex command undo paths
15. `d473ae3` - Refactor insert helper undo paths to apply_change
16. `bace2ba` - Token-harden visual-block change undo merge
17. `423d936` - Migrate cw change path to tokenized repeat flow
18. `a32732d` - Migrate cgn change flow off pending semantic merge
19. `cdd2d5e` - Migrate replace-mode `R` to recorded undo and repeat action
20. `684a0b3` - Remove insert-mode add_change callsites
21. `6d85c16` - Add open-line undo isolation macro regressions
22. `4323c97` - Add cgn undo isolation macro regression
23. `b000ea2` - Add cw no-insert undo isolation macro regression
24. `61a6669` - Add undo redo isolation regressions for o O cw cgn
25. `2b07e49` - Add replace mode isolation regressions and migration hygiene guard
26. `aa833aa` - Add cc and C undo redo isolation regressions
27. `0c2349a` - Add no-insert cc C undo redo isolation tests
28. `da5f26e` - Sync undo repeat architecture docs with migrated flows

## What Landed

### A) Visual delete dot-repeat uses semantic RepeatAction (char/line/block)
Files:
- `/Users/adrian/Projects/ovim/ovim-core/src/editor/input/helpers.rs`
- `/Users/adrian/Projects/ovim/ovim-core/src/repeat_action.rs`
- `/Users/adrian/Projects/ovim/ovim/tests/dot_repeat_test.rs`
- `/Users/adrian/Projects/ovim/ovim/tests/visual_block_mode_test.rs`

### B) Visual block change (`Ctrl-V ... c ... .`) migrated to semantic repeat
Files:
- `/Users/adrian/Projects/ovim/ovim-core/src/editor/input/visual_mode.rs`
- `/Users/adrian/Projects/ovim/ovim-core/src/editor/input/insert_mode.rs`
- `/Users/adrian/Projects/ovim/ovim-core/src/editor/editing_state.rs`
- `/Users/adrian/Projects/ovim/ovim-core/src/editor/mod.rs`
- `/Users/adrian/Projects/ovim/ovim-core/src/repeat_action.rs`
- `/Users/adrian/Projects/ovim/ovim/tests/visual_block_mode_test.rs`

### C) LSP workspace `ResourceOp` undo integration (create/rename/delete)
Files:
- `/Users/adrian/Projects/ovim/ovim-core/src/change.rs`
  - Added `Change::ResourceOp` snapshot variant for filesystem undo/redo.
- `/Users/adrian/Projects/ovim/ovim-core/src/editor/lsp_modules/workspace_edits.rs`
  - `apply_resource_op` now snapshots before/after and pushes undo entries.
- `/Users/adrian/Projects/ovim/ovim/tests/lsp_applied_edits_sync_test.rs`
  - Added resource-op undo/redo tests for create, rename, delete.

### D) Substitute-confirm + text-object change migration
Files:
- `/Users/adrian/Projects/ovim/ovim-core/src/editor/ui_features.rs`
  - Confirmed substitutions now use `record()` + `push_recorded_undo()`.
- `/Users/adrian/Projects/ovim/ovim-core/src/editor/input/commands.rs`
  - Command Enter handling preserves `SubstituteConfirm` mode transitions.
- `/Users/adrian/Projects/ovim/ovim-core/src/editor/input/normal/text_objects.rs`
  - Change-operator text objects now use `PendingChangeRepeat` path.
  - Contract tightened so text-object operators always carry concrete `TextObjectType`.
- `/Users/adrian/Projects/ovim/ovim-core/src/repeat_action.rs`
  - Fixed `RepeatAction::Change` insert-point behavior for `C/c$` vs text-objects.

### E) Completion accept migration
Files:
- `/Users/adrian/Projects/ovim/ovim-core/src/editor/ui_features.rs`
  - `accept_completion()` now uses recorded undo (`record` + `push_recorded_undo`) instead of manual composite `add_change`.
- `/Users/adrian/Projects/ovim/ovim/tests/completion_menu_test.rs`
  - Added macro-flow undo/redo regression for completion accept.

### F) Plan docs updated
File:
- `/Users/adrian/Projects/ovim/PRIORITIES.md`
  - `add_change` snapshot was reduced from 14 and is now 5 in `ovim-core/src`.
  - Added notes for command-mode migration slices and new macro regressions.

### G) Text-object case operators migrated
Files:
- `/Users/adrian/Projects/ovim/ovim-core/src/repeat_action.rs`
  - Added `CaseTransform` + `RepeatAction::ChangeCaseTextObject`.
- `/Users/adrian/Projects/ovim/ovim-core/src/editor/input/normal/text_objects.rs`
  - `gu/gU/g~` text-object paths now use `record_operation()` + semantic repeat.
- `/Users/adrian/Projects/ovim/ovim/tests/dot_repeat_test.rs`
  - Added `test_dot_repeat_guiw_semantic_undo_granularity_macro_flow`.

### H) Ex command mutation paths migrated off `add_change`
Files:
- `/Users/adrian/Projects/ovim/ovim-core/src/editor/input/commands.rs`
  - `:global ... d`, ranged `:d`, shell filter (`:%!cmd` / `:.!cmd`), `:r !cmd`, `:sort`, `:copy`, and `:move` now use recorded undo paths.
- `/Users/adrian/Projects/ovim/ovim/tests/command_global_test.rs`
  - Added macro undo/redo flow for `:global` delete.
- `/Users/adrian/Projects/ovim/ovim/tests/command_mode_test.rs`
  - Added macro undo/redo flows for ranged delete, `:sort`, `:copy`, and `:move`.
- `/Users/adrian/Projects/ovim/ovim/tests/shell_commands_test.rs`
  - Added macro undo/redo flow for `:%!sort`.

### I) Insert helper migration slice
Files:
- `/Users/adrian/Projects/ovim/ovim-core/src/editor/input/helpers.rs`
  - `Ctrl-W/U/T/D` insert-mode helper edits now go through `apply_change_and_record()` instead of direct buffer mutation + manual `add_change`.
- `/Users/adrian/Projects/ovim/ovim/tests/ctrl_commands_test.rs`
  - Added macro undo/redo flow coverage for `Ctrl-W/U/T/D` insert-mode behavior.
- `/Users/adrian/Projects/ovim/PRIORITIES.md`
  - Remaining `add_change` snapshot updated to 10 in that slice (currently 5).

### J) Visual-block change merge token hardening
Files:
- `/Users/adrian/Projects/ovim/ovim-core/src/editor/editing_state.rs`
  - Added `pending_visual_block_change_delete_token` to hold delete-phase undo token.
- `/Users/adrian/Projects/ovim/ovim-core/src/editor/input/helpers.rs`
  - Added `delete_visual_selection_with_token()` so visual delete can return the recorded undo token.
- `/Users/adrian/Projects/ovim/ovim-core/src/editor/input/visual_mode.rs`
  - `Ctrl-V ... c ...` now stores delete token for insert-exit merge instead of relying on stack-top assumptions.
- `/Users/adrian/Projects/ovim/ovim-core/src/editor/input/insert_mode.rs`
  - Visual-block replay merge now redeems delete entry via `pop_by_token()` (no blind `pop_last_change()`).
- `/Users/adrian/Projects/ovim/ovim/tests/visual_block_mode_test.rs`
  - Added macro regressions for undo isolation after unrelated prior changes:
    - `test_ctrl_v_change_undo_does_not_consume_prior_change_macro_flow`
    - `test_ctrl_v_insert_undo_does_not_consume_prior_change_macro_flow`

### K) `cw` tokenized pending-change migration
Files:
- `/Users/adrian/Projects/ovim/ovim-core/src/editor/input/normal/operators.rs`
  - `handle_cw` now records delete edits with `record()` and stores a delete token in `PendingChangeRepeat`.
- `/Users/adrian/Projects/ovim/ovim-core/src/repeat_action.rs`
  - Added `RepeatAction::DeleteWordChange { count }` for `cw` delete-phase semantics (`word_end_forward_prefer_current` + inclusive delete).
- `/Users/adrian/Projects/ovim/ovim/tests/dot_repeat_test.rs`
  - Added macro regression: `test_dot_repeat_cw_semantic_undo_granularity_macro_flow`.
  - Added macro regression: `test_cw_esc_undo_does_not_consume_prior_change_macro_flow`.
  - Added macro regression: `test_cw_esc_undo_redo_isolation_macro_flow`.

### L) `cgn/cgN` migration off pending semantic merge
Files:
- `/Users/adrian/Projects/ovim/ovim-core/src/editor/input/normal/pending_commands.rs`
  - `Operator::Change` on `gn/gN` now stores `PendingChangeRepeat` with a delete token (`DeleteSearchMatch`) instead of popping semantic `Change` payloads.
- `/Users/adrian/Projects/ovim/ovim-core/src/repeat_action.rs`
  - Added `RepeatAction::DeleteSearchMatch { search_pattern, search_forward }`.
  - `RepeatAction::Change` now preserves delete-resolved insertion position for `DeleteSearchMatch` (same treatment as text-object deletes).
- `/Users/adrian/Projects/ovim/ovim-core/src/editor/input/insert_mode.rs`
  - Removed runtime `PendingSemanticChange` merge branch; insert-exit now uses tokenized pending-change path only and clears stale semantic state defensively.
- `/Users/adrian/Projects/ovim/ovim/tests/visual_mode_test.rs`
  - Added macro regression: `test_cgn_esc_undo_does_not_consume_prior_change_macro_flow`.
  - Added macro regression: `test_cgn_esc_undo_redo_isolation_macro_flow`.

### M) Replace-mode (`R`) migration off semantic add_change
Files:
- `/Users/adrian/Projects/ovim/ovim-core/src/editor/input/normal/mode_transitions.rs`
  - Entering `R` now starts replace-mode change building at cursor.
- `/Users/adrian/Projects/ovim/ovim-core/src/editor/input/replace_mode.rs`
  - Replace typing/backspace mutations now use `apply_change_and_record()` into replace-mode builder.
  - On `<Esc>`, non-empty replace sessions finalize recorded undo and set `RepeatAction::ReplaceMode`; empty backspaced sessions discard builder.
- `/Users/adrian/Projects/ovim/ovim-core/src/repeat_action.rs`
  - Added `RepeatAction::ReplaceMode { replacements }`.
- `/Users/adrian/Projects/ovim/ovim/tests/dot_repeat_test.rs`
  - Added macro regressions:
    - `test_dot_repeat_R_semantic_undo_granularity_macro_flow`
    - `test_replace_mode_backspace_to_empty_does_not_create_undo_entry_macro_flow`
    - `test_R_esc_undo_redo_isolation_macro_flow`
    - `test_replace_mode_backspace_to_empty_undo_redo_isolation_macro_flow`

### O) Open-line undo isolation regressions (`o` / `O`)
Files:
- `/Users/adrian/Projects/ovim/ovim/tests/dot_repeat_test.rs`
  - Added macro regressions:
    - `test_o_esc_undo_does_not_consume_prior_change_macro_flow`
    - `test_uppercase_o_esc_undo_does_not_consume_prior_change_macro_flow`
    - `test_o_esc_undo_redo_isolation_macro_flow`
    - `test_uppercase_o_esc_undo_redo_isolation_macro_flow`

### Q) Change-operator isolation regressions (`cc` / `C`)
Files:
- `/Users/adrian/Projects/ovim/ovim/tests/dot_repeat_test.rs`
  - Added macro regressions:
    - `test_cc_esc_undo_redo_isolation_macro_flow`
    - `test_C_esc_undo_redo_isolation_macro_flow`
    - `test_cc_esc_no_insert_undo_redo_isolation_macro_flow`
    - `test_C_esc_no_insert_undo_redo_isolation_macro_flow`

### N) Insert-mode finalization callsite cleanup
Files:
- `/Users/adrian/Projects/ovim/ovim-core/src/editor/input/insert_mode.rs`
  - Replaced final composite pushes (`pending_change_repeat` merge and visual-block replay) with direct `ChangeManager::push_change(...)`.
  - This removes all runtime `add_change` usage from insert mode while preserving existing undo/repeat behavior.

### P) Migration hygiene guard (infrastructure-only `add_change` usage)
Files:
- `/Users/adrian/Projects/ovim/ovim/tests/undo_migration_hygiene_test.rs`
  - Added regression to fail if `add_change(...)` appears in `ovim-core/src` outside infrastructure files (`change.rs`, `editor/mod.rs`).
  - Added callsite cap assertion (`<= 5`) to catch infrastructure-side regression growth.

### R) Architecture docs synced with migrated boundaries
Files:
- `/Users/adrian/Projects/ovim/ovim-core/src/change.rs`
  - Updated top-level Pattern A/B guide to reflect current migration state (semantic repeat ownership for `cw/cgn/cc/C/R/o/O` paths).
- `/Users/adrian/Projects/ovim/ovim-core/src/repeat_action.rs`
  - Updated `RepeatAction` overview comment to clarify Pattern B covers semantic repeat flows that may pass through insert mode.

## Tests Run (Passing)
- `cargo test -p ovim --test visual_block_mode_test -- --nocapture`
- `cargo test -p ovim --test dot_repeat_test test_dot_after_visual_delete_macro_flow -- --nocapture`
- `cargo test -p ovim --test dot_repeat_test test_dot_after_visual_line_delete_undo_granularity_macro_flow -- --nocapture`
- `cargo test -p ovim --test lsp_applied_edits_sync_test -- --nocapture`
- `cargo test -p ovim --test lsp_document_sync_undo_test -- --nocapture`
- `cargo test -p ovim --test visual_block_mode_test test_ctrl_v_change_dot_repeat_macro_flow -- --nocapture`
- `cargo test -p ovim --test command_mode_test -- --nocapture`
- `cargo test -p ovim --test change_operations_test --test dot_repeat_test --test case_change_test -- --nocapture`
- `cargo test -p ovim --test completion_menu_test -- --nocapture`
- `cargo test -p ovim --test dot_repeat_test test_dot_repeat_ci_paren_undo_granularity_macro_flow -- --nocapture`
- `cargo test -p ovim --test undo_repeat_coverage_test -- --nocapture`
- `cargo test -p ovim --test dot_repeat_test test_dot_repeat_guiw_semantic_undo_granularity_macro_flow -- --nocapture`
- `cargo test -p ovim --test dot_repeat_test -- --nocapture`
- `cargo test -p ovim --test command_global_test -- --nocapture`
- `cargo test -p ovim --test command_mode_test -- --nocapture`
- `cargo test -p ovim --test shell_commands_test -- --nocapture`
- `cargo test -p ovim --test ctrl_commands_test -- --nocapture`
- `cargo test -p ovim --test visual_block_mode_test -- --nocapture`
- `cargo test -p ovim --test undo_repeat_coverage_test -- --nocapture`
- `cargo test -p ovim --test dot_repeat_test -- --nocapture`
- `cargo test -p ovim --test change_operations_test -- --nocapture`
- `cargo test -p ovim --test visual_mode_test -- --nocapture`
- `cargo test -p ovim --test visual_mode_test -- --nocapture` (after adding cgn undo isolation macro)
- `cargo test -p ovim --test replace_mode_test -- --nocapture`
- `cargo test -p ovim --test dot_repeat_test -- --nocapture`
- `cargo test -p ovim --test dot_repeat_test -- --nocapture` (after adding open-line undo isolation macros)
- `cargo test -p ovim --test dot_repeat_test undo_redo_isolation_macro_flow -- --nocapture`
- `cargo test -p ovim --test visual_mode_test test_cgn_esc_undo_redo_isolation_macro_flow -- --nocapture`
- `cargo test -p ovim --test dot_repeat_test -- --nocapture` (after adding open-line/cw undo+redo isolation macros)
- `cargo test -p ovim --test visual_mode_test -- --nocapture` (after adding cgn undo+redo isolation macro)
- `cargo test -p ovim --test dot_repeat_test test_R_esc_undo_redo_isolation_macro_flow -- --nocapture`
- `cargo test -p ovim --test dot_repeat_test test_replace_mode_backspace_to_empty_undo_redo_isolation_macro_flow -- --nocapture`
- `cargo test -p ovim --test undo_migration_hygiene_test -- --nocapture`
- `cargo test -p ovim --test dot_repeat_test esc_undo_redo_isolation_macro_flow -- --nocapture`
- `cargo test -p ovim --test dot_repeat_test no_insert_undo_redo_isolation_macro_flow -- --nocapture`
- `cargo test -p ovim --test dot_repeat_test -- --nocapture` (after adding no-insert `cc/C` isolation macros)
- `cargo test -p ovim --test undo_migration_hygiene_test -- --nocapture` (after architecture doc sync)
- `cargo test -p ovim --test undo_migration_hygiene_test -- --nocapture` (after adding add_change callsite cap assertion)

## Current Workspace Safety Notes
There are unrelated in-progress edits from another agent. Do not revert them.
Observed modified files include:
- `/Users/adrian/Projects/ovim/ovim-core/src/editor/ai_chat_tools.rs`
- `/Users/adrian/Projects/ovim/ovim-core/src/editor/ai_integration.rs`
- `/Users/adrian/Projects/ovim/ovim-core/src/editor/lsp_integration.rs`
- `/Users/adrian/Projects/ovim/ovim-core/src/editor/lsp_modules/workspace_edits.rs`
- `/Users/adrian/Projects/ovim/ovim-core/src/editor/mod.rs`
- `/Users/adrian/Projects/ovim/ovim/src/ui/renderer/dashboard.rs`
- `/Users/adrian/Projects/ovim/ovim/tests/hygiene_ad_hoc_scripts_test.rs`
- `/Users/adrian/Projects/ovim/ovim/tests/hygiene_paths_test.rs`
- `/Users/adrian/Projects/ovim/ovim/tests/java_comment_test.rs`
- `/Users/adrian/Projects/ovim/ovim/tests/lsp_multi_file_test.rs`
- `/Users/adrian/Projects/ovim/ovim/tests/syntax_test.rs`

## Suggested Continuation
If continuing migration work, likely next slice is:
1. Decide whether to keep or retire `add_change` infrastructure (`Editor::add_change` + `ChangeManager::add_change`) now that all runtime migration callsites are gone outside infrastructure.
2. Keep commits path-scoped and avoid touching dirty AI files listed above.
