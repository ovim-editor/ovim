use anyhow::Result;
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

use crate::display::display_col_to_char_col;
use crate::editor::Editor;
use crate::mode::Mode;

/// Top-level mouse event dispatcher.
pub fn handle_mouse_event(editor: &mut Editor, event: MouseEvent) -> Result<()> {
    match event.kind {
        MouseEventKind::Down(MouseButton::Left) => handle_left_click(editor, event.column, event.row),
        MouseEventKind::Drag(MouseButton::Left) => handle_left_drag(editor, event.column, event.row),
        MouseEventKind::Up(MouseButton::Left) => handle_left_release(editor),
        MouseEventKind::ScrollUp => handle_scroll(editor, true),
        MouseEventKind::ScrollDown => handle_scroll(editor, false),
        MouseEventKind::Down(MouseButton::Middle) => handle_middle_click(editor, event.column, event.row),
        _ => Ok(()), // Right click, other events: ignored
    }
}

/// Converts screen coordinates to buffer (line, col).
/// Returns `None` if the click is outside the buffer area.
fn screen_to_buffer(editor: &Editor, screen_col: u16, screen_row: u16) -> Option<(usize, usize)> {
    let area = editor.render_cache.last_buffer_area?;
    let gutter_width = editor.render_cache.last_gutter_width;

    // Hit-test: must be within buffer area
    if screen_col < area.x
        || screen_row < area.y
        || screen_col >= area.x + area.width
        || screen_row >= area.y + area.height
    {
        return None;
    }

    let rel_col = (screen_col - area.x) as usize;
    let rel_row = (screen_row - area.y) as usize;

    // Click in gutter region — handled separately
    if rel_col < gutter_width {
        return None;
    }

    let text_width = editor.render_cache.last_text_width;
    let display_col_in_row = rel_col - gutter_width;

    // Determine buffer line and full display column, accounting for wrap
    let (buffer_line, display_col) = if editor.options.wrap {
        if let Some(wrap_map) = editor.wrap_map() {
            let viewport_visual_row = wrap_map.logical_to_visual(editor.scroll_offset());
            let absolute_visual_row = rel_row + viewport_visual_row;
            let (logical_line, sub_line) = wrap_map.visual_to_logical(absolute_visual_row);
            let line = logical_line.min(editor.buffer().line_count().saturating_sub(1));
            // In wrap mode, display column = sub_line * wrap_width + col within row
            let col = sub_line * text_width + display_col_in_row;
            (line, col)
        } else {
            let line = (rel_row + editor.scroll_offset()).min(editor.buffer().line_count().saturating_sub(1));
            (line, display_col_in_row + editor.horizontal_offset())
        }
    } else {
        let line = (rel_row + editor.scroll_offset()).min(editor.buffer().line_count().saturating_sub(1));
        (line, display_col_in_row + editor.horizontal_offset())
    };

    // Convert display column to character column using tab/wide-char aware function
    let line_text = editor
        .buffer()
        .line(buffer_line)
        .map(|l| l.trim_end_matches('\n').to_string())
        .unwrap_or_default();
    let tab_width = editor.options.tab_width;
    let char_col = display_col_to_char_col(&line_text, display_col, tab_width);

    // Clamp to line length (Normal mode: last char, Insert mode: past last char)
    let line_len = line_text.chars().count();
    let max_col = if editor.mode() == Mode::Insert {
        line_len
    } else {
        line_len.saturating_sub(1)
    };
    let clamped_col = char_col.min(max_col);

    Some((buffer_line, clamped_col))
}

/// Returns the buffer line if the click lands in the gutter area.
fn is_gutter_click(editor: &Editor, screen_col: u16, screen_row: u16) -> Option<usize> {
    let area = editor.render_cache.last_buffer_area?;
    let gutter_width = editor.render_cache.last_gutter_width;

    if gutter_width == 0 {
        return None;
    }

    if screen_col < area.x
        || screen_row < area.y
        || screen_col >= area.x + area.width
        || screen_row >= area.y + area.height
    {
        return None;
    }

    let rel_col = (screen_col - area.x) as usize;
    let rel_row = (screen_row - area.y) as usize;

    if rel_col < gutter_width {
        let buffer_line = if editor.options.wrap {
            if let Some(wrap_map) = editor.wrap_map() {
                let viewport_visual_row = wrap_map.logical_to_visual(editor.scroll_offset());
                let absolute_visual_row = rel_row + viewport_visual_row;
                let (logical_line, _sub_line) = wrap_map.visual_to_logical(absolute_visual_row);
                logical_line.min(editor.buffer().line_count().saturating_sub(1))
            } else {
                (rel_row + editor.scroll_offset()).min(editor.buffer().line_count().saturating_sub(1))
            }
        } else {
            (rel_row + editor.scroll_offset()).min(editor.buffer().line_count().saturating_sub(1))
        };
        Some(buffer_line)
    } else {
        None
    }
}

/// Whether the current mode is one we should ignore mouse clicks in
/// (except scroll, which is handled separately).
fn should_ignore_click(mode: Mode) -> bool {
    matches!(
        mode,
        Mode::Command
            | Mode::Search
            | Mode::HoverPreview
            | Mode::HoverNavigate
            | Mode::Dashboard
            | Mode::FileTree
            | Mode::SubstituteConfirm
    )
}

fn handle_left_click(editor: &mut Editor, col: u16, row: u16) -> Result<()> {
    let mode = editor.mode();

    // Handle picker mode clicks
    if mode == Mode::Picker {
        return handle_picker_click(editor, col, row);
    }

    // Dismiss transient overlays on click
    if matches!(mode, Mode::Command | Mode::Search | Mode::HoverPreview | Mode::HoverNavigate) {
        editor.set_mode(Mode::Normal);
        // Fall through to also move cursor
    } else if should_ignore_click(mode) {
        return Ok(());
    }

    // Exit visual mode if active
    if matches!(mode, Mode::Visual | Mode::VisualLine | Mode::VisualBlock) {
        editor.set_mode(Mode::Normal);
    }

    // Check gutter click → select line (Visual Line mode)
    if let Some(line) = is_gutter_click(editor, col, row) {
        editor.buffer_mut().cursor_mut().set_position(line, 0);
        editor.set_visual_start(line, 0);
        editor.set_mode(Mode::VisualLine);
        editor.render_cache.mouse_state.is_dragging = true;
        editor.render_cache.mouse_state.drag_origin = Some((line, 0));
        return Ok(());
    }

    // Buffer click → move cursor
    if let Some((line, char_col)) = screen_to_buffer(editor, col, row) {
        editor.buffer_mut().cursor_mut().set_position(line, char_col);
        editor.render_cache.mouse_state.is_dragging = true;
        editor.render_cache.mouse_state.drag_origin = Some((line, char_col));
    }

    Ok(())
}

fn handle_left_drag(editor: &mut Editor, col: u16, row: u16) -> Result<()> {
    if !editor.render_cache.mouse_state.is_dragging {
        return Ok(());
    }

    let mode = editor.mode();
    if should_ignore_click(mode) && mode != Mode::Picker {
        return Ok(());
    }

    if let Some((line, char_col)) = screen_to_buffer(editor, col, row) {
        // Enter visual mode on first drag movement (if not already visual)
        if !matches!(mode, Mode::Visual | Mode::VisualLine | Mode::VisualBlock) {
            if let Some((origin_line, origin_col)) = editor.render_cache.mouse_state.drag_origin {
                editor.set_visual_start(origin_line, origin_col);
            }
            // If we started from a gutter click, stay in VisualLine
            if editor.mode() != Mode::VisualLine {
                editor.set_mode(Mode::Visual);
            }
        }

        editor.buffer_mut().cursor_mut().set_position(line, char_col);
    }

    Ok(())
}

fn handle_left_release(editor: &mut Editor) -> Result<()> {
    editor.render_cache.mouse_state.is_dragging = false;
    Ok(())
}

fn handle_scroll(editor: &mut Editor, up: bool) -> Result<()> {
    const SCROLL_LINES: usize = 3;

    // In picker mode, scroll the picker results
    if editor.mode() == Mode::Picker {
        if let Some(picker) = editor.picker_mut() {
            for _ in 0..SCROLL_LINES {
                if up {
                    picker.move_up();
                } else {
                    picker.move_down();
                }
            }
            editor.mark_picker_selection_changed();
        }
        return Ok(());
    }

    // In wrap mode, scroll by visual rows instead of logical lines.
    // We compute the target logical line from visual row arithmetic and
    // set the scroll offset directly via scroll_viewport_up/down with
    // the appropriate logical line delta.
    if editor.options.wrap {
        if let Some(wrap_map) = editor.wrap_map() {
            let scroll_offset = editor.scroll_offset();
            let current_visual = wrap_map.logical_to_visual(scroll_offset);
            let target_visual = if up {
                current_visual.saturating_sub(SCROLL_LINES)
            } else {
                (current_visual + SCROLL_LINES).min(wrap_map.total_visual_lines().saturating_sub(1))
            };
            let (target_line, _) = wrap_map.visual_to_logical(target_visual);
            if target_line < scroll_offset {
                editor.scroll_viewport_up(scroll_offset - target_line);
            } else if target_line > scroll_offset {
                editor.scroll_viewport_down(target_line - scroll_offset);
            }
            return Ok(());
        }
    }

    if up {
        editor.scroll_viewport_up(SCROLL_LINES);
    } else {
        editor.scroll_viewport_down(SCROLL_LINES);
    }

    Ok(())
}

fn handle_middle_click(editor: &mut Editor, col: u16, row: u16) -> Result<()> {
    let mode = editor.mode();
    if should_ignore_click(mode) {
        return Ok(());
    }

    // Move cursor to click position first
    if let Some((line, char_col)) = screen_to_buffer(editor, col, row) {
        editor.buffer_mut().cursor_mut().set_position(line, char_col);
    }

    // Paste from system clipboard register (+)
    let text = editor.registers().get(Some('+'));
    if !text.is_empty() {
        editor.handle_paste_event(&text)?;
    }

    Ok(())
}

/// Handles a left click inside the picker overlay.
fn handle_picker_click(editor: &mut Editor, col: u16, row: u16) -> Result<()> {
    use crate::editor::PickerField;

    let layout = match &editor.picker_state.last_layout {
        Some(l) => l.clone(),
        None => return Ok(()),
    };

    let point = (col, row);

    // Hit-test: query field
    if rect_contains(&layout.query_field, point) {
        if let Some(picker) = editor.picker_mut() {
            picker.set_active_field(PickerField::Query);
        }
        return Ok(());
    }

    // Hit-test: filter field (LiveGrep only)
    if let Some(ref filter_rect) = layout.filter_field {
        if rect_contains(filter_rect, point) {
            if let Some(picker) = editor.picker_mut() {
                picker.set_active_field(PickerField::FileFilter);
            }
            return Ok(());
        }
    }

    // Hit-test: results area
    if rect_contains(&layout.results_area, point) {
        let row_in_results = (row - layout.results_area.y) as usize;
        let clicked_index = layout.results_scroll_offset + row_in_results;

        if let Some(picker) = editor.picker_mut() {
            picker.set_selected_index(clicked_index);
        }
        editor.mark_picker_selection_changed();
        return Ok(());
    }

    Ok(())
}

/// Returns true if the screen point (col, row) is inside the rect.
fn rect_contains(rect: &ratatui::layout::Rect, point: (u16, u16)) -> bool {
    let (col, row) = point;
    col >= rect.x
        && col < rect.x + rect.width
        && row >= rect.y
        && row < rect.y + rect.height
}
