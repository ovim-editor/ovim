//! Lifecycle of source-mapped pseudocode reading buffers. The editable source
//! remains a separate buffer, with its undo history, diagnostics and file path.
use super::{Editor, InputState};
use crate::buffer::{Buffer, BufferId};
use crate::pseudocode::{Language, Projection};
use crate::unicode::{grapheme_index_for_byte, GraphemeCol};
use crate::{KeyCode, KeyEvent, Modifiers};

pub struct PseudocodeView {
    source_id: BufferId,
    source_version: usize,
    view_version: usize,
    projection: Projection,
    markdown: Option<MarkdownDocument>,
}

/// Presentation payload cached with the reading view, independent of viewport
/// geometry. Each Markdown line points into the source-mapped reading buffer.
#[derive(Debug, Clone, PartialEq)]
pub struct MarkdownDocument {
    pub text: String,
    pub view_lines: Vec<usize>,
    pub highlights: crate::buffer::LineHighlights,
}

impl Editor {
    pub fn pseudocode_markdown(&self, buffer_id: BufferId) -> Option<&MarkdownDocument> {
        self.ui_panels.pseudocode.get(&buffer_id)?.markdown.as_ref()
    }

    pub fn is_pseudocode_buffer(&self) -> bool {
        self.ui_panels.pseudocode.contains_key(&self.buffer().id())
    }

    pub fn set_pseudocode(&mut self, enabled: bool) -> anyhow::Result<()> {
        if !enabled {
            return self.leave_pseudocode(false);
        }
        let current_id = self.buffer().id();
        let source_id = self
            .ui_panels
            .pseudocode
            .get(&current_id)
            .map(|view| view.source_id)
            .unwrap_or(current_id);
        let source_index = self
            .find_buffer_index_by_id(source_id)
            .ok_or_else(|| anyhow::anyhow!("The pseudocode source buffer has been closed"))?;
        let source = &self.buffers[source_index];
        let path = source.file_path().unwrap_or("");
        let language = match crate::syntax::LanguageRegistry::detect_from_path(path) {
            Some(crate::syntax::Language::Java) => Language::Java,
            Some(crate::syntax::Language::Markdown) => Language::Markdown,
            _ => anyhow::bail!("Pseudocode supports Java and Markdown files"),
        };
        let source_text = source.rope().to_string();
        let projection = Projection::build(&source_text, language)?;
        let mut colored_source = Buffer::new_from_str(&source_text);
        colored_source.set_language_catalog(self.language_catalog.clone());
        colored_source.enable_syntax_highlighting_for_path(path);
        let mut highlights =
            projection.map_highlights(|line| colored_source.highlights_for_line(line).into_owned());
        let markdown = if language == Language::Markdown {
            for (line, styles) in highlights
                .iter_mut()
                .zip(projection.markdown_styles(&source_text)?)
            {
                line.extend(styles);
            }
            let formatted = Projection::formatted_markdown(&source_text)?;
            let view_lines = (0..formatted.lines.len())
                .map(|line| projection.view_line_for_source(formatted.source_position(line, 0).0))
                .collect();
            let highlights = formatted
                .map_highlights(|line| colored_source.highlights_for_line(line).into_owned());
            Some(MarkdownDocument {
                highlights,
                text: formatted.text,
                view_lines,
            })
        } else {
            None
        };
        let source_version = source.version();
        let source_line = if let Some(view) = self.ui_panels.pseudocode.get(&current_id) {
            view.projection
                .source_position(self.buffer().cursor().line(), 0)
                .0
        } else {
            source.cursor().line()
        };
        let title = format!(
            "Pseudocode: {}",
            std::path::Path::new(path)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
        );
        let existing = self
            .ui_panels
            .pseudocode
            .iter()
            .find(|(_, view)| view.source_id == source_id)
            .map(|(id, _)| *id)
            .and_then(|id| self.find_buffer_index_by_id(id));
        let index = if let Some(index) = existing {
            self.buffers[index].replace_content(&projection.text);
            self.switch_to_buffer(index);
            index
        } else {
            let mut buffer = Buffer::new_from_str(&projection.text);
            buffer.set_read_only(true);
            buffer.set_display_name(title);
            // No file path: this is never a save target or an LSP document.
            self.add_buffer(buffer);
            self.current_buffer_index
        };
        let view_line = projection.view_line_for_source(source_line);
        self.buffers[index].set_forced_highlights(highlights);
        let view_id = self.buffers[index].id();
        self.ui_panels.pseudocode.insert(
            view_id,
            PseudocodeView {
                source_id,
                source_version,
                view_version: self.buffers[index].version(),
                projection,
                markdown,
            },
        );
        self.buffer_mut()
            .cursor_mut()
            .set_position(view_line, GraphemeCol::ZERO);
        self.buffer_mut().validate_cursor_position();
        self.lsp.state.needs_lsp_init = false;
        self.set_mode(crate::mode::Mode::Normal);
        self.sync_current_tab_buffer();
        self.center_cursor_in_viewport();
        self.set_status_message("Pseudocode · Enter: source · r: refresh · :set nopseudo: off");
        self.mark_dirty();
        Ok(())
    }

    fn leave_pseudocode(&mut self, jump: bool) -> anyhow::Result<()> {
        let Some(view) = self.ui_panels.pseudocode.get(&self.buffer().id()) else {
            return Ok(());
        };
        let Some(source_index) = self.find_buffer_index_by_id(view.source_id) else {
            if jump {
                anyhow::bail!("Source buffer closed; use :set nopseudo to close this view");
            }
            // Keep the buffer ID stable for splits, but discard the orphaned
            // derived content and turn it into a normal empty buffer.
            let id = self.buffer().id();
            self.ui_panels.pseudocode.remove(&id);
            self.buffer_mut().replace_content("");
            self.buffer_mut().set_read_only(false);
            self.buffer_mut().set_display_name("[No Name]");
            self.set_mode(crate::mode::Mode::Normal);
            self.set_status_message("Pseudocode off; source buffer was closed");
            self.mark_dirty();
            return Ok(());
        };
        if !jump {
            self.switch_to_buffer(source_index);
            self.sync_current_tab_buffer();
            self.set_mode(crate::mode::Mode::Normal);
            self.center_cursor_in_viewport();
            self.set_status_message("Pseudocode off");
            self.mark_dirty();
            return Ok(());
        }
        // Don't jump with stale coordinates after a background edit or API mutation.
        if self.buffers[source_index].version() != view.source_version
            || self.buffer().version() != view.view_version
        {
            self.set_pseudocode(true)?;
            self.set_status_message(
                "Source changed; pseudocode refreshed. Press Enter to open source.",
            );
            return Ok(());
        }
        let line = self.buffer().cursor().line();
        let text = self
            .buffer()
            .line_text(line)
            .map(|s| s.to_string())
            .unwrap_or_default();
        let byte = crate::unicode::byte_offset_for_grapheme(&text, self.buffer().cursor().col().0)
            .unwrap_or(text.len());
        let (source_line, source_byte) = view.projection.source_position(line, byte);
        let source_text = self.buffers[source_index]
            .line_text(source_line)
            .map(|s| s.to_string())
            .unwrap_or_default();
        let column = grapheme_index_for_byte(&source_text, source_byte);
        self.switch_to_buffer(source_index);
        self.buffer_mut()
            .cursor_mut()
            .set_position(source_line, GraphemeCol(column));
        self.buffer_mut().validate_cursor_position();
        self.sync_current_tab_buffer();
        self.set_mode(crate::mode::Mode::Normal);
        self.center_cursor_in_viewport();
        self.set_status_message("Source · :set pseudo to return to pseudocode");
        self.mark_dirty();
        Ok(())
    }

    /// Reading buffers allow navigation/search, with an explicit jump back to
    /// editable source. Handled before normal-mode operators can mutate text.
    pub(crate) fn handle_pseudocode_key(&mut self, key: KeyEvent) -> bool {
        if !self.is_pseudocode_buffer() {
            return false;
        }
        if self.mode() != crate::mode::Mode::Normal {
            return false;
        }
        if key
            .modifiers
            .intersects(Modifiers::CONTROL | Modifiers::SUPER | Modifiers::ALT)
        {
            if key.modifiers == Modifiers::CONTROL
                && matches!(key.code, KeyCode::Char('d' | 'u' | 'f' | 'b' | 'e' | 'y'))
            {
                return false;
            }
            self.cancel_pseudocode_command();
            return true;
        }
        if matches!(self.input_state(), InputState::GPrefix { .. })
            && !matches!(
                key.code,
                KeyCode::Char('g' | 'e' | 'E' | 'j' | 'k' | '0' | '^' | '$') | KeyCode::Esc
            )
        {
            self.cancel_pseudocode_command();
            return true;
        }
        if key.code == KeyCode::Enter {
            if let Err(error) = self.leave_pseudocode(true) {
                self.set_status_message(error.to_string());
            }
            return true;
        }
        let action = match key.code {
            KeyCode::Char('q') => Some(false),
            KeyCode::Char('r') => Some(true),
            _ => None,
        };
        if let Some(enabled) = action {
            if let Err(error) = self.set_pseudocode(enabled) {
                self.set_status_message(error.to_string());
            }
            return true;
        }
        // Deliberately no operators, macros, insert/visual modes, or LSP jumps:
        // those use source coordinates and belong in the original buffer.
        if matches!(
            key.code,
            KeyCode::Char(
                'h' | 'j'
                    | 'k'
                    | 'l'
                    | 'w'
                    | 'W'
                    | 'b'
                    | 'B'
                    | 'e'
                    | 'E'
                    | 'g'
                    | 'G'
                    | 'z'
                    | 't'
                    | 'f'
                    | 'F'
                    | 'T'
                    | ';'
                    | ','
                    | 'n'
                    | 'N'
                    | '*'
                    | '#'
                    | '/'
                    | '?'
                    | ':'
                    | '0'..='9' | '$' | '^' | '%' | '{' | '}' | 'H' | 'M' | 'L'
            ) | KeyCode::Up
                | KeyCode::Down
                | KeyCode::Left
                | KeyCode::Right
                | KeyCode::Home
                | KeyCode::End
                | KeyCode::PageUp
                | KeyCode::PageDown
                | KeyCode::Esc
        ) {
            return false;
        }
        self.cancel_pseudocode_command();
        true
    }

    fn cancel_pseudocode_command(&mut self) {
        self.reset_input_state();
        self.clear_count();
        self.set_status_message("Pseudocode is a reading view; press Enter to edit source");
    }
}
