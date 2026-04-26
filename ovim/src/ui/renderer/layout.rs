use crate::editor::Editor;
use ratatui::layout::Rect;

/// Width of the sign column (git signs, diagnostics). Always present.
pub const SIGN_WIDTH: usize = 2;
/// Spacing between gutter and text content.
pub const GUTTER_SPACING: usize = 1;

/// Single source of truth for buffer layout dimensions within a render frame.
///
/// Computed once per frame from editor options and the allocated area,
/// then passed by reference to all rendering functions that need gutter
/// or text-area measurements.
///
/// The split between `buffer_area` and `render_area` matters in centered
/// (textwidth) mode: document content lives in `buffer_area` (the centered
/// code-box), but EOL decorations (diagnostics) render in the right margin
/// up to `render_area`'s right edge. In all other modes the two are equal.
#[derive(Debug, Clone, Copy)]
pub struct BufferLayout {
    /// The area where document content lives — the centered code-box in
    /// textwidth mode, or the full allocation otherwise. `text_width` and
    /// `gutter_width` are derived from this rect, and the cursor is
    /// positioned in its coordinate space.
    pub buffer_area: Rect,
    /// The area lines are actually drawn into. Equal to `buffer_area` in
    /// the common case; wider than `buffer_area` in centered mode so EOL
    /// decorations can extend into the right margin.
    pub render_area: Rect,
    /// Total gutter width in columns (sign + line number + spacing).
    pub gutter_width: usize,
    /// Width available for document text inside the code-box
    /// (buffer_area.width - gutter_width).
    pub text_width: usize,
    /// Width of the line-number column alone (0 when numbers are off).
    pub line_num_width: usize,
    /// Width of the blame column (0 when blame is off).
    pub blame_width: usize,
}

impl BufferLayout {
    /// Computes the layout from the editor state and allocated area.
    /// `render_area` defaults to `area` (no diagnostic margin).
    pub fn compute(editor: &Editor, area: Rect) -> Self {
        Self::compute_with_render_area(editor, area, area)
    }

    /// Computes the layout for centered mode where lines render into a
    /// rect wider than the centered code-box. `area` is the centered band;
    /// `render_area` is the rect that includes the right diagnostic margin.
    pub fn compute_with_render_area(editor: &Editor, area: Rect, render_area: Rect) -> Self {
        let show_numbers = editor.options.number || editor.options.relative_number;
        let line_count = editor.buffer().line_count();
        let line_num_width = if show_numbers {
            line_count.to_string().len().max(3)
        } else {
            0
        };

        // Blame column width: bracket(1) + space(1) + hash(5) + space(1) + author(truncated) + space(1)
        let blame_width = if editor.options.blame {
            if let Some(blame) = editor.buffer().git_blame() {
                // 1 bracket + 1 space + 5 hash + 1 space + author + 1 trailing space
                let author_len = blame.max_author_len().min(15);
                1 + 1 + 5 + 1 + author_len.max(3) + 1
            } else {
                0
            }
        } else {
            0
        };

        // Sign column is always present (git signs, diagnostics).
        let gutter_width = blame_width + SIGN_WIDTH + line_num_width + GUTTER_SPACING;
        let text_width = (area.width as usize).saturating_sub(gutter_width);

        Self {
            buffer_area: area,
            render_area,
            gutter_width,
            text_width,
            line_num_width,
            blame_width,
        }
    }

    /// Total render width inside the gutter — the column count available
    /// to text + EOL diagnostic, measured from the right edge of the
    /// gutter to the right edge of `render_area`. Equals `text_width`
    /// when `render_area == buffer_area`; in centered mode it equals
    /// `text_width + diag_margin_width`.
    pub fn render_width(&self) -> usize {
        let buffer_left = self.buffer_area.x as usize + self.gutter_width;
        let render_right = self.render_area.x as usize + self.render_area.width as usize;
        render_right.saturating_sub(buffer_left)
    }
}

/// Context passed to overlay widgets (hover, completion) and cursor
/// positioning that need to locate themselves relative to the buffer viewport.
#[derive(Debug, Clone, Copy)]
pub struct OverlayContext<'a> {
    pub layout: &'a BufferLayout,
    pub viewport_start: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_rect(width: u16, height: u16) -> Rect {
        Rect {
            x: 0,
            y: 0,
            width,
            height,
        }
    }

    /// Helper: create a minimal editor for layout tests.
    fn test_editor() -> Editor {
        Editor::default()
    }

    #[test]
    fn test_numbers_on_default() {
        let mut editor = test_editor();
        editor.options.number = true;
        editor.options.relative_number = false;
        let layout = BufferLayout::compute(&editor, make_rect(80, 24));
        // line_count=1 → "1".len()=1, max(3)=3, gutter=2+3+1=6
        assert_eq!(layout.line_num_width, 3);
        assert_eq!(layout.gutter_width, 6);
        assert_eq!(layout.text_width, 74);
    }

    #[test]
    fn test_numbers_off() {
        let mut editor = test_editor();
        editor.options.number = false;
        editor.options.relative_number = false;
        let layout = BufferLayout::compute(&editor, make_rect(80, 24));
        // line_num_width=0, gutter = SIGN_WIDTH(2) + 0 + GUTTER_SPACING(1) = 3
        assert_eq!(layout.line_num_width, 0);
        assert_eq!(layout.gutter_width, 3);
        assert_eq!(layout.text_width, 77);
    }

    #[test]
    fn test_tiny_area() {
        let mut editor = test_editor();
        editor.options.number = true;
        let layout = BufferLayout::compute(&editor, make_rect(5, 1));
        // gutter_width=6, text_width = 5 - 6 = 0 (saturating)
        assert_eq!(layout.gutter_width, 6);
        assert_eq!(layout.text_width, 0);
        assert_eq!(layout.buffer_area.width, 5);
    }

    #[test]
    fn test_relative_number() {
        let mut editor = test_editor();
        editor.options.number = false;
        editor.options.relative_number = true;
        let layout = BufferLayout::compute(&editor, make_rect(80, 24));
        assert_eq!(layout.line_num_width, 3);
        assert_eq!(layout.gutter_width, 6);
    }
}
