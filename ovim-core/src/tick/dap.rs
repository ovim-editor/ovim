//! Debug-adapter work driven by the tick: what the session task reported, and
//! the queued [`PendingDebugAction`](crate::dap::PendingDebugAction)s (start,
//! step, evaluate, ...) handed to it. Neither waits for the adapter.

use crate::editor::Editor;

/// Take in the session task's reports: adapter events and answers.
pub(super) fn process_dap_events(editor: &mut Editor) {
    // Edits that did not come through a key (API, LSP, Lua) move breakpoints too.
    editor.follow_breakpoints_through_edits();
    let count = editor.process_dap_events();
    if count > 0 {
        crate::log_debug!("tick", "Processed {} DAP reports", count);
    }
}

/// Hand the queued debug actions (start, stop, step, evaluate, etc.) to the
/// session task.
pub(super) fn process_pending_debug_action(editor: &mut Editor) {
    editor.run_pending_debug_actions();
}
