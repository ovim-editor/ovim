//! Loading files into the editor, rehighlighting and modelines.

use super::Editor;
use crate::buffer::Buffer;
use anyhow::Result;

impl Editor {
    /// Loads a file into the editor (async version)
    /// If the file is already open in a buffer, switches to that buffer
    /// Otherwise, adds it as a new buffer
    pub async fn load_file_async<P: AsRef<std::path::Path>>(&mut self, path: P) -> Result<()> {
        let path_str = path.as_ref().to_string_lossy().to_string();

        // Check if file is already open in a buffer
        // (compared by canonical identity, not by spelling: `:e rel/path`
        // must find the buffer that was opened as `/abs/rel/path`, otherwise
        // a duplicate buffer for one file is created and LSP edits land in the
        // wrong twin - OV-00450)
        if let Some(i) = self.find_buffer_by_path(&path_str) {
            // File already open - use the canonical switch path so every
            // file-scoped UI/LSP cache is reset consistently.
            self.switch_to_buffer(i);
            // Point the current tab at the existing buffer
            self.sync_current_tab_buffer();
            return Ok(());
        }

        // Store old file path before loading new file
        let old_file_path = self.buffer().file_path().map(|s| s.to_string());

        // Save current file to alternate file register
        if let Some(current_path) = old_file_path.as_ref() {
            self.registers.set_alternate_file(current_path.to_string());
        }

        // Load new buffer
        let new_buffer = Buffer::load_file_async(path).await?;
        let modeline = crate::modeline::Modeline::parse(&new_buffer.rope().to_string());

        // Load git branch name for the new file
        self.git_branch = new_buffer.file_path().and_then(crate::git::branch_name);

        self.add_buffer(new_buffer);
        if let Some(modeline) = modeline.as_ref() {
            self.apply_modeline(modeline);
        }

        // Update current file register
        self.registers.set_current_file(path_str);

        // Point the current tab at the newly loaded buffer (the tab title is
        // derived from the buffer, so no separate title update is needed)
        self.sync_current_tab_buffer();

        // Mark that we need to send didClose for the old file
        if let Some(old) = old_file_path {
            self.queue_lsp_did_close(old);
        }

        Ok(())
    }

    /// Loads a file into the editor (blocking wrapper around load_file_async)
    pub fn load_file<P: AsRef<std::path::Path>>(&mut self, path: P) -> Result<()> {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.load_file_async(path))
        })
    }

    /// Process pending syntax re-highlighting (CPU-intensive, runs in background)
    /// This now uses incremental parsing - the parse tree was already updated incrementally
    /// via InputEdit when the buffer was modified, so we just need to query it for highlights.
    pub async fn process_pending_rehighlight(&mut self) {
        // Check if buffer needs re-highlighting
        if !self.buffer().needs_rehighlight() {
            return;
        }

        // Rebuild highlight cache from the incrementally-updated parse tree
        // This is FAST because tree-sitter already updated the tree via InputEdit.
        // We're just querying the tree for highlights, not re-parsing!
        let _ = self.buffer_mut().rebuild_highlight_cache();

        // Fix Bug 2: Mark dirty after highlighting update so the UI re-renders
        // Without this, highlighting updates after debounce but screen doesn't refresh
        self.mark_dirty();
    }

    /// Immediate viewport-only syntax rehighlight.
    /// Queries tree-sitter for just the visible lines and updates the cache.
    /// This is called immediately after input so highlights are accurate without waiting for the debounce.
    pub fn process_viewport_rehighlight(&mut self) {
        if !self.buffer().needs_rehighlight() {
            return;
        }

        let start_line = self.scroll_offset();
        let end_line = start_line + self.viewport_height();

        self.buffer_mut()
            .rebuild_viewport_highlight_cache(start_line, end_line);

        self.mark_dirty();
    }

    /// Apply modeline options to editor settings
    pub(super) fn apply_modeline(&mut self, modeline: &crate::modeline::Modeline) {
        // Indentation options
        let indent = self.indent_options().with_modeline(modeline);
        self.set_local_indent_options(indent);

        // Display options
        if let Some(tw) = modeline.get_int("textwidth", "tw") {
            self.options.textwidth = Some(tw);
        }
        if let Some(nu) = modeline.get_bool("number", "nu") {
            self.options.number = nu;
        }
        if let Some(rnu) = modeline.get_bool("relativenumber", "rnu") {
            self.options.relative_number = rnu;
        }
        if let Some(cul) = modeline.get_bool("cursorline", "cul") {
            self.options.cursorline = cul;
        }

        // Search options
        if let Some(ic) = modeline.get_bool("ignorecase", "ic") {
            self.options.ignorecase = ic;
        }
        if let Some(scs) = modeline.get_bool("smartcase", "scs") {
            self.options.smartcase = scs;
        }

        // Other options
        if let Some(sm) = modeline.get_bool("showmatch", "sm") {
            self.options.showmatch = sm;
        }
    }
}
