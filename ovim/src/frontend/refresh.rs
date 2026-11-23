use crate::editor::Editor;

/// Post-input refresh: rehighlight the viewport if the buffer needs it, then
/// mark dirty.
pub fn refresh_after_input(editor: &mut Editor) {
    if editor.buffer().needs_rehighlight() {
        editor.process_viewport_rehighlight();
    }
    editor.mark_dirty();
}

/// Shared post-mutation refresh for API endpoints that bypass key dispatch.
pub fn refresh_after_api_mutation(editor: &mut Editor, force_full_lsp_sync: bool) {
    if force_full_lsp_sync {
        editor.mark_buffer_modified_force_send();
    }
    editor.request_diagnostics_refresh();
    refresh_after_input(editor);
}
