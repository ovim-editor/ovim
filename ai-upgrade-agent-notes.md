# AI Chat Upgrade Handoff Notes

## Context
User request:
1. Fix incorrect permissions checks for edits in current file.
2. Add a bash tool with whitelisted commands available by default.
3. Add dedicated UI for permission requests (current bottom-bar location is too easy to miss).
4. Reflect on where this and similar popups should live, then implement.
5. Investigate codebase thoroughly and commit regularly.

## Important Workspace State
The worktree is heavily dirty from concurrent editor work. Do **not** revert unrelated files.
Current modified files include many unrelated ones (tests/editor). Only AI-chat related files below were intentionally changed in this pass.

## What Was Implemented So Far
### 1) Permission false-positive fix for current target file
Changed `/Users/adrian/Projects/ovim/ovim-core/src/editor/ai_chat_tools.rs`:
- Added `is_active_chat_target_path(&self, path: &Path) -> bool`.
- Updated `maybe_require_tool_policy_approval(...)` for `SensitivePrompt`:
  - Before: any mutation triggered approval.
  - Now: mutation triggers approval **only when target is not the active chat target file**.
  - Still triggers approval for project scans and sensitive paths.

### 2) Project detection/root selection improvement
Changed `/Users/adrian/Projects/ovim/ovim-core/src/editor/ai_chat_tools.rs`:
- Updated `ai_project_start_path()` to prefer:
  1. active chat target file,
  2. chat origin file,
  3. current file,
  4. cwd fallback.

This addresses likely project-boundary/root mis-detection when user navigates during a chat.

### 3) Better pending-approval summary text
Changed `/Users/adrian/Projects/ovim/ovim-core/src/editor/ai_chat.rs`:
- `ai_chat_pending_tool_approval_summary()` now says:
  - `Tool approval requested: <tool> (<path>)`
  instead of the old outside-project-only wording.

## Tests Added
In `/Users/adrian/Projects/ovim/ovim-core/src/editor/ai_chat_tools.rs` test module:
- `edit_range_on_active_target_does_not_require_approval`
- `edit_range_with_other_path_requires_approval_in_sensitive_mode`
- `ai_repo_root_prefers_active_target_file`

All use Tokio runtime wrappers (required by file IO/editor internals in these paths).

## Tests Run and Passing
Executed and passing:
- `cargo test -p ovim-core edit_range_on_active_target_does_not_require_approval -- --nocapture`
- `cargo test -p ovim-core edit_range_with_other_path_requires_approval_in_sensitive_mode -- --nocapture`
- `cargo test -p ovim-core ai_repo_root_prefers_active_target_file -- --nocapture`

Also ran `cargo fmt --all`.

## What Is Still Pending (Main Feature Work)
### A) Add bash tool with default whitelist
Likely implementation points:
- Tool registration + schema in `/Users/adrian/Projects/ovim/ovim-core/src/ai/tools/builtins.rs`
  - Add a new external tool (e.g. `bash` / `run_bash`).
- External dispatch path in `/Users/adrian/Projects/ovim/ovim-core/src/editor/ai_chat_tools.rs`
  - Currently `SideEffect::External` returns `external tools not yet supported`.
- Execution model recommendation:
  - Avoid shell interpolation (`sh -c`).
  - Parse command into argv safely.
  - Enforce allowlist on argv[0] (or first N tokens for patterns).
  - Return stdout/stderr with bounded output.
- Decide where default whitelist lives:
  - either hardcoded safe defaults,
  - or config-backed with fallback defaults.

### B) Dedicated permission request UI (overlay/popup)
Current behavior:
- Approval info is primarily in model selector strip in `/Users/adrian/Projects/ovim/ovim/src/ui/renderer/ai_chat.rs` (hard to notice).
- Key handling for approval is in `/Users/adrian/Projects/ovim/ovim-core/src/editor/input/ai_chat_mode.rs`.

Recommended placement (from architecture review):
- Add a centered modal/dialog in overlay layer:
  - `/Users/adrian/Projects/ovim/ovim/src/ui/renderer/overlays.rs`
  - wired from `/Users/adrian/Projects/ovim/ovim/src/ui/renderer/core.rs::render_overlays(...)`
- Reason:
  - Existing high-attention floating UI already lives in overlays (picker, LSP manager, hover, review card).
  - Gives a unified home for interruptive prompts.

Suggested behavior:
- Show dialog for both:
  - pending tool approval
  - pending no-repo folder approval
- Keep bottom/status hints as secondary context only.
- Ensure keyboard affordances are explicit in dialog (e.g. `Ctrl-Y`, `Ctrl-A`, `Ctrl-N`; maybe Enter/Esc aliases if desired).

### C) Commit cadence
User asked “commit regularly.” No commit done yet in this pass due concurrent uncommitted edits. Next agent should commit scoped, path-limited commits (only touched files).

## Proposed Next Commit Sequence
1. Commit current fixes/tests (only `ai_chat_tools.rs` + `ai_chat.rs`).
2. Commit bash tool introduction + external dispatch + tests.
3. Commit dedicated permission overlay UI + input tweaks + renderer tests/snapshots (if available).
4. Optional cleanup commit for copy/hints/docs.

## Commands Useful for Picking Up
- Inspect current AI-chat diffs:
  - `git diff -- /Users/adrian/Projects/ovim/ovim-core/src/editor/ai_chat_tools.rs /Users/adrian/Projects/ovim/ovim-core/src/editor/ai_chat.rs`
- Re-run targeted tests:
  - `cargo test -p ovim-core edit_range_on_active_target_does_not_require_approval -- --nocapture`
  - `cargo test -p ovim-core edit_range_with_other_path_requires_approval_in_sensitive_mode -- --nocapture`
  - `cargo test -p ovim-core ai_repo_root_prefers_active_target_file -- --nocapture`

---

## Update: Completed Implementation + Recent Commits

Since the initial handoff above, the requested upgrade has been implemented in scoped commits:

1. `bec0af8` Fix AI approval noise for active file and improve root detection
2. `d4ffb8e` Add whitelisted bash tool and approval key handling
3. `6999fe9` Render AI permission requests as centered overlay dialogs
4. `73b0a50` Harden AI bash tool binary resolution and tests
5. `1c722ae` Stabilize AI chat tool path tests in no-repo sessions
6. `084fa93` Prioritize blocking approval modals over overlay toasts
7. `fc75cb9` Harden AI repo root detection with git discovery fallback

### What changed in latest two commits
- `/Users/adrian/Projects/ovim/ovim-core/src/editor/ai_chat_tools.rs`
  - Path-scoped mutation tests now set `editor.ai_state.no_repo_session_allowed_root` explicitly to tempdir roots, avoiding false failures when cwd is outside a git repo.
- `/Users/adrian/Projects/ovim/ovim/src/ui/renderer/core.rs`
  - Added a dedicated blocking-modal tier for approval dialogs (`render_blocking_modals`).
  - Added helper `has_blocking_modal`.
  - Top-right toasts are now suppressed while a blocking approval modal is active.

### Validation
- `cargo test -p ovim-core` passed (all tests).
- `cargo test -p ovim --no-run` passed (warnings only).

### Additional root-detection hardening
- `/Users/adrian/Projects/ovim/ovim-core/src/editor/ai_chat_tools.rs`
  - `ai_repo_root()` now resolves via `git2::Repository::discover` first, then falls back to marker walk-up (`.git` exists) for test/partial repo layouts.
  - Added `ai_repo_root_detects_git_file_marker` regression test for `.git` file marker scenarios.

### Parallel-work safety
- Other agents are actively modifying unrelated files (LSP migration/tests/dashboard).
- Keep any further AI-upgrade edits scoped; do not revert unrelated dirty files.
