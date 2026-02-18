# Undo Migration Handoff Notes

## Context
Undo/repeat migration work was continued on `main` and committed in scoped slices.
Recent slices removed all `add_change` callsites from `input/commands.rs` and migrated text-object case operators to semantic `RepeatAction`.
`PRIORITIES.md` now reports 14 `add_change` callsites in `ovim-core/src` (all in insert/helpers infrastructure + replace + internals).

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
  - `add_change` snapshot now reflects recent reductions (currently 14 in `ovim-core/src`).
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

## Current Workspace Safety Notes
There are unrelated in-progress edits from another agent. Do not revert them.
Observed modified files include:
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
1. Evaluate whether any `input/helpers.rs` insert-mode callsites can be safely converted without breaking insert-session composition.
2. Keep commits path-scoped and avoid touching dirty AI files listed above.
