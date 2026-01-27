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
#[derive(Debug, Clone, Copy)]
pub struct BufferLayout {
    /// The full area allocated to this buffer (after textwidth narrowing).
    pub buffer_area: Rect,
    /// Total gutter width in columns (sign + line number + spacing).
    pub gutter_width: usize,
    /// Width available for text content (buffer_area.width - gutter_width).
    pub text_width: usize,
    /// Width of the line-number column alone (0 when numbers are off).
    pub line_num_width: usize,
}

impl BufferLayout {
    /// Computes the layout dimensions from the editor state and allocated area.
    pub fn compute(editor: &Editor, area: Rect) -> Self {
        let show_numbers = editor.options.number || editor.options.relative_number;
        let line_count = editor.buffer().line_count();
        let line_num_width = if show_numbers {
            line_count.to_string().len().max(3)
        } else {
            0
        };
        // Sign column is always present (git signs, diagnostics).
        let gutter_width = SIGN_WIDTH + line_num_width + GUTTER_SPACING;
        let text_width = (area.width as usize).saturating_sub(gutter_width);

        Self {
            buffer_area: area,
            gutter_width,
            text_width,
            line_num_width,
        }
    }
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
