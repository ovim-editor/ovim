use crate::markdown_conceal::{apply_conceal, extract_concealed_links, scan_markdown_conceal};
use crate::{MouseButton, MouseEvent, MouseEventKind};
use anyhow::Result;

use crate::display::{char_display_width, display_col_to_char_col};
use crate::editor::Editor;
use crate::mode::Mode;
use crate::unicode::{char_to_grapheme_col, grapheme_count, GraphemeCol};

/// Top-level mouse event dispatcher.
/// Returns `Ok(Some(url))` when a concealed markdown link was clicked and should be opened.
pub fn handle_mouse_event(editor: &mut Editor, event: MouseEvent) -> Result<Option<String>> {
    match event.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            return handle_left_click(editor, event.column, event.row);
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            handle_left_drag(editor, event.column, event.row)?;
        }
        MouseEventKind::Up(MouseButton::Left) => {
            handle_left_release(editor)?;
        }
        MouseEventKind::ScrollUp => {
            handle_scroll(editor, true, event.row)?;
        }
        MouseEventKind::ScrollDown => {
            handle_scroll(editor, false, event.row)?;
        }
        MouseEventKind::Down(MouseButton::Middle) => {
            handle_middle_click(editor, event.column, event.row)?;
        }
        _ => {} // Right click, mouse move, horizontal scroll: ignored
    }
    Ok(None)
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

    let display_col_in_row = rel_col - gutter_width;
    let line_text = |line: usize| {
        editor
            .buffer()
            .line(line)
            .map(|l| l.trim_end_matches('\n').to_string())
            .unwrap_or_default()
    };

    // Determine buffer line and full display column, accounting for wrap
    let (buffer_line, display_col) = if editor.options.wrap {
        if let Some(wrap_map) = editor.wrap_map() {
            let viewport_visual_row = wrap_map.logical_to_visual(editor.scroll_offset());
            let absolute_visual_row = rel_row + viewport_visual_row;
            let (logical_line, sub_line) = wrap_map.visual_to_logical(absolute_visual_row);
            let line = logical_line.min(editor.buffer().line_count().saturating_sub(1));
            let row_text = line_text(line);
            let (row_start, _) = wrap_map
                .sub_line_display_range(&row_text, sub_line)
                .unwrap_or((0, 0));
            // In wrap mode, display columns are contiguous across segment starts.
            let col = row_start + display_col_in_row;
            (line, col)
        } else {
            let line = (rel_row + editor.scroll_offset())
                .min(editor.buffer().line_count().saturating_sub(1));
            (line, display_col_in_row + editor.horizontal_offset())
        }
    } else {
        let line =
            (rel_row + editor.scroll_offset()).min(editor.buffer().line_count().saturating_sub(1));
        (line, display_col_in_row + editor.horizontal_offset())
    };

    // Convert display column to character column using tab/wide-char aware function
    let line_text = line_text(buffer_line);
    let tab_width = editor.options.tab_width;
    let char_col = display_col_to_char_col(&line_text, display_col, tab_width);
    let grapheme_col = char_to_grapheme_col(&line_text, char_col).0;

    // Clamp to line length (Normal mode: last char, Insert mode: past last char)
    let line_len = grapheme_count(&line_text);
    let max_col = if editor.mode() == Mode::Insert {
        line_len
    } else {
        line_len.saturating_sub(1)
    };
    let clamped_col = grapheme_col.min(max_col);

    Some((buffer_line, clamped_col))
}

/// Checks if a click at the given screen coordinates lands on a concealed markdown link.
/// Returns the URL if it does.
fn check_concealed_link_click(editor: &Editor, screen_col: u16, screen_row: u16) -> Option<String> {
    // Only active for markdown files with concealment on
    if !editor.options.markdown_conceal {
        return None;
    }
    let is_md = editor
        .buffer()
        .file_path()
        .map(|p| p.ends_with(".md"))
        .unwrap_or(false);
    if !is_md {
        return None;
    }

    let area = editor.render_cache.last_buffer_area?;
    let gutter_width = editor.render_cache.last_gutter_width;

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
        return None;
    }

    let display_col_in_row = rel_col - gutter_width;

    // Determine buffer line (same logic as screen_to_buffer)
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

    // Don't check cursor line (concealment is disabled there)
    if buffer_line == editor.buffer().cursor().line() {
        return None;
    }

    let display_col = if editor.options.wrap {
        // In wrap mode, display_col_in_row is the column within the sub-line
        // For simplicity, only handle the first sub-line (sub_line == 0)
        display_col_in_row
    } else {
        display_col_in_row + editor.horizontal_offset()
    };

    // Get the line text and scan for concealed links
    let line_text = editor
        .buffer()
        .line(buffer_line)
        .map(|l| l.trim_end_matches('\n').to_string())
        .unwrap_or_default();

    let spans = scan_markdown_conceal(&line_text);
    if spans.is_empty() {
        return None;
    }

    let transform = apply_conceal(&line_text, &spans);
    let links = extract_concealed_links(&spans, &transform);

    // The display_col is in display space (after tab expansion).
    // The link view_start/view_end are in concealed char space (no tab expansion).
    // For simplicity, treat them as equivalent (links in markdown rarely coexist with tabs).
    for link in &links {
        if display_col >= link.view_start && display_col < link.view_end {
            return Some(link.url.clone());
        }
    }

    None
}

/// Returns the buffer line if the click lands in the blame column area.
fn is_blame_click(editor: &Editor, screen_col: u16, screen_row: u16) -> Option<usize> {
    let area = editor.render_cache.last_buffer_area?;
    let blame_width = editor.render_cache.last_blame_width;

    if blame_width == 0 {
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

    if rel_col < blame_width {
        let buffer_line = if editor.options.wrap {
            if let Some(wrap_map) = editor.wrap_map() {
                let viewport_visual_row = wrap_map.logical_to_visual(editor.scroll_offset());
                let absolute_visual_row = rel_row + viewport_visual_row;
                let (logical_line, _sub_line) = wrap_map.visual_to_logical(absolute_visual_row);
                logical_line.min(editor.buffer().line_count().saturating_sub(1))
            } else {
                (rel_row + editor.scroll_offset())
                    .min(editor.buffer().line_count().saturating_sub(1))
            }
        } else {
            (rel_row + editor.scroll_offset()).min(editor.buffer().line_count().saturating_sub(1))
        };
        Some(buffer_line)
    } else {
        None
    }
}

/// Returns the buffer line if the click lands in the sign column (first 2 chars of gutter).
fn is_sign_column_click(editor: &Editor, screen_col: u16, screen_row: u16) -> Option<usize> {
    let area = editor.render_cache.last_buffer_area?;
    let gutter_width = editor.render_cache.last_gutter_width;
    let blame_width = editor.render_cache.last_blame_width;

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

    // Sign column starts after blame, spans 2 columns (SIGN_WIDTH)
    let sign_start = blame_width;
    let sign_end = blame_width + 2; // SIGN_WIDTH

    if rel_col < sign_start || rel_col >= sign_end {
        return None;
    }

    let rel_row = (screen_row - area.y) as usize;
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
                (rel_row + editor.scroll_offset())
                    .min(editor.buffer().line_count().saturating_sub(1))
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

fn handle_left_click(editor: &mut Editor, col: u16, row: u16) -> Result<Option<String>> {
    let mode = editor.mode();

    // Handle AI prompt panel clicks (model picker + prompt cursor placement).
    if mode == Mode::AiPrompt && handle_ai_prompt_click(editor, col, row)? {
        return Ok(None);
    }

    // Handle picker mode clicks
    if mode == Mode::Picker {
        handle_picker_click(editor, col, row)?;
        return Ok(None);
    }

    // Dismiss transient overlays on click
    if matches!(
        mode,
        Mode::Command | Mode::Search | Mode::HoverPreview | Mode::HoverNavigate
    ) {
        editor.set_mode(Mode::Normal);
        // Fall through to also move cursor
    } else if should_ignore_click(mode) {
        return Ok(None);
    }

    // Exit visual mode if active
    if matches!(mode, Mode::Visual | Mode::VisualLine | Mode::VisualBlock) {
        editor.set_mode(Mode::Normal);
    }

    // Check concealed markdown link click → open URL
    if let Some(url) = check_concealed_link_click(editor, col, row) {
        return Ok(Some(url));
    }

    // Check blame column click → show blame popup
    if let Some(line) = is_blame_click(editor, col, row) {
        editor.buffer_mut().cursor_mut().set_position(line, GraphemeCol::ZERO);
        editor.show_blame_info();
        return Ok(None);
    }

    // Check sign column click → toggle breakpoint
    if let Some(line) = is_sign_column_click(editor, col, row) {
        editor.buffer_mut().cursor_mut().set_position(line, GraphemeCol::ZERO);
        editor.toggle_breakpoint();
        return Ok(None);
    }

    // Check gutter click → select line (Visual Line mode)
    if let Some(line) = is_gutter_click(editor, col, row) {
        editor.buffer_mut().cursor_mut().set_position(line, GraphemeCol::ZERO);
        editor.set_visual_start(line, 0);
        editor.set_mode(Mode::VisualLine);
        editor.render_cache.mouse_state.is_dragging = true;
        editor.render_cache.mouse_state.drag_origin = Some((line, 0));
        return Ok(None);
    }

    // Buffer click → move cursor
    if let Some((line, char_col)) = screen_to_buffer(editor, col, row) {
        editor
            .buffer_mut()
            .cursor_mut()
            .set_position(line, GraphemeCol(char_col));
        editor.render_cache.mouse_state.is_dragging = true;
        editor.render_cache.mouse_state.drag_origin = Some((line, char_col));
    }

    Ok(None)
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

        editor
            .buffer_mut()
            .cursor_mut()
            .set_position(line, GraphemeCol(char_col));
    }

    Ok(())
}

fn handle_left_release(editor: &mut Editor) -> Result<()> {
    editor.render_cache.mouse_state.is_dragging = false;
    Ok(())
}

fn handle_scroll(editor: &mut Editor, up: bool, row: u16) -> Result<()> {
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

    // In AiChat mode, scroll the chat message history if the mouse is over the chat area
    if editor.mode() == Mode::AiChat {
        if let Some(chat_rect) = editor.render_cache.last_chat_area {
            if row >= chat_rect.y && row < chat_rect.y + chat_rect.height {
                if up {
                    editor.ai_chat_scroll_viewport_up(SCROLL_LINES);
                } else {
                    editor.ai_chat_scroll_viewport_down(SCROLL_LINES);
                }
                return Ok(());
            }
        }
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
        editor
            .buffer_mut()
            .cursor_mut()
            .set_position(line, GraphemeCol(char_col));
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

/// Handles a left click inside the AI prompt panel.
/// Returns true when the click was consumed.
fn handle_ai_prompt_click(editor: &mut Editor, col: u16, row: u16) -> Result<bool> {
    if let Some(trigger) = editor.render_cache.ai_prompt_model_trigger_hitbox {
        if trigger.contains(col, row) {
            editor.ai_toggle_model_picker();
            return Ok(true);
        }
    }

    for (rect, profile_name) in editor.render_cache.ai_prompt_model_hitboxes.clone() {
        if rect.contains(col, row) {
            let _ = editor.ai_set_profile(&profile_name);
            editor.ai_close_model_picker();
            return Ok(true);
        }
    }

    if editor.ai_state.prompt.model_picker_open {
        editor.ai_close_model_picker();
    }

    let prompt = editor.ai_prompt_input().to_string();
    let prompt_len = prompt.len();
    for (input_rect, start_byte, end_byte) in &editor.render_cache.ai_prompt_input_rows {
        if input_rect.contains(col, row) {
            let rel_display_col = (col.saturating_sub(input_rect.x)) as usize;
            let row_start = (*start_byte).min(prompt_len);
            let row_end = (*end_byte).min(prompt_len).max(row_start);
            let new_cursor = prompt_cursor_from_display_col_in_range(
                &prompt,
                row_start,
                row_end,
                rel_display_col,
            );
            editor.ai_state.prompt.cursor = new_cursor;
            return Ok(true);
        }
    }

    if let Some(input_rect) = editor.render_cache.ai_prompt_input_area {
        if input_rect.contains(col, row) {
            editor.ai_state.prompt.cursor = prompt_len;
            return Ok(true);
        }
    }

    Ok(false)
}

fn prompt_cursor_from_display_col_in_range(
    text: &str,
    start_byte: usize,
    end_byte: usize,
    display_col: usize,
) -> usize {
    let mut width = 0;
    for (rel_idx, ch) in text[start_byte..end_byte].char_indices() {
        let byte_idx = start_byte + rel_idx;
        let ch_width = char_display_width(ch);
        if width + ch_width > display_col {
            return byte_idx;
        }
        width += ch_width;
    }
    end_byte
}

/// Returns true if the screen point (col, row) is inside the rect.
fn rect_contains(rect: &crate::Rect, point: (u16, u16)) -> bool {
    rect.contains(point.0, point.1)
}
