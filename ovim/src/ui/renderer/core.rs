use crate::editor::{Editor, SplitDirection, WindowNode};
use crate::syntax::Theme;
use anyhow::Result;
use crossterm::cursor::SetCursorStyle;
use crossterm::terminal::SetTitle;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame, Terminal as RatatuiTerminal,
};
use std::io;

use super::buffer::render_buffer;
use super::dashboard::render_dashboard;
use super::file_tree_widget::render_file_tree;
use super::helpers::char_col_to_display_col;
use super::layout::{BufferLayout, OverlayContext};
use super::overlays::{render_completion_menu, render_hover_window};
use super::picker_widget::render_picker;
use super::status_widgets::{
    render_command_line, render_diagnostic_badge, render_message_line, render_path_completion,
    render_progress_line, render_rename_input, render_search_line, render_status_line,
    render_tab_bar,
};

// ---------------------------------------------------------------------------
// Frame layout types
// ---------------------------------------------------------------------------

/// Areas computed from the frame layout (tab bar, file tree, buffer, status, command, progress).
struct FrameAreas {
    tab_area: Option<Rect>,
    file_tree_area: Option<Rect>,
    buffer_chunk: Rect,
    status_chunk: Rect,
    command_chunk: Rect,
    progress_chunk: Option<Rect>,
}

// ---------------------------------------------------------------------------
// Extracted render phases (free functions)
// ---------------------------------------------------------------------------

/// Phase 1: Fill the frame with blanks and initialize the window manager.
fn clear_frame(frame: &mut Frame, editor: &mut Editor) {
    let area = frame.area();
    editor.init_window_manager(area.width, area.height);

    let blank_line = " ".repeat(area.width as usize);
    let blank_lines: Vec<Line> = (0..area.height)
        .map(|_| Line::from(blank_line.clone()))
        .collect();
    let bg_paragraph = Paragraph::new(blank_lines).style(Style::default().bg(Color::Reset));
    frame.render_widget(bg_paragraph, area);
}

/// Phase 2: Compute the frame layout (tab bar, file tree, buffer, status splits).
///
/// Returns `None` if the editor is in dashboard mode (caller should render
/// the dashboard and return early).
fn compute_frame_layout(frame: &Frame, editor: &Editor) -> Option<FrameAreas> {
    if editor.should_show_dashboard() {
        return None;
    }

    let main_area = frame.area();

    // Tab bar (if multiple tabs) + rest
    let (tab_area, remaining_area) = if editor.tab_count() > 1 {
        let vertical_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(1)].as_ref())
            .split(main_area);
        (Some(vertical_chunks[0]), vertical_chunks[1])
    } else {
        (None, main_area)
    };

    // File tree (if visible) + rest
    let (file_tree_area, content_area) = if editor.file_tree().is_visible() {
        let horizontal_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(50), Constraint::Min(1)].as_ref())
            .split(remaining_area);
        (Some(horizontal_chunks[0]), horizontal_chunks[1])
    } else {
        (None, remaining_area)
    };

    // Buffer + optional progress line + status line + command line
    let has_progress = editor.lsp_progress_message().is_some();
    let chunks = if has_progress {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints(
                [
                    Constraint::Min(1),
                    Constraint::Length(1), // progress line
                    Constraint::Length(1), // status line
                    Constraint::Length(1), // command/message line
                ]
                .as_ref(),
            )
            .split(content_area)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints(
                [
                    Constraint::Min(1),
                    Constraint::Length(1), // status line
                    Constraint::Length(1), // command/message line
                ]
                .as_ref(),
            )
            .split(content_area)
    };

    let (status_chunk, command_chunk, progress_chunk) = if has_progress {
        (chunks[2], chunks[3], Some(chunks[1]))
    } else {
        (chunks[1], chunks[2], None)
    };

    Some(FrameAreas {
        tab_area,
        file_tree_area,
        buffer_chunk: chunks[0],
        status_chunk,
        command_chunk,
        progress_chunk,
    })
}

/// Phase 3: Render the buffer area (split or single window), returning
/// the viewport start line and the focused window's layout.
fn render_buffer_area(
    frame: &mut Frame,
    editor: &mut Editor,
    theme: &Theme,
    areas: &FrameAreas,
) -> (usize, BufferLayout) {
    let has_splits = editor
        .window_manager()
        .map(|wm| wm.root().count_windows() > 1)
        .unwrap_or(false);

    if has_splits {
        // For splits, use buffer_chunk width for wrap map (focused window area
        // isn't known until render_window_tree runs)
        if editor.options.wrap {
            let est_layout = BufferLayout::compute(editor, areas.buffer_chunk);
            editor.ensure_wrap_map(est_layout.text_width);
        }

        // Render split windows recursively
        if let Some(wm) = editor.window_manager() {
            let focused_index = wm.focused_window_index();
            let mut current_index = 0;
            if let Some((vs, ly)) = render_window_tree(
                frame,
                editor,
                theme,
                wm.root(),
                areas.buffer_chunk,
                focused_index,
                &mut current_index,
            ) {
                (vs, ly)
            } else {
                let fallback_layout = BufferLayout::compute(editor, areas.buffer_chunk);
                let viewport_start = render_buffer(frame, editor, theme, &fallback_layout);
                (viewport_start, fallback_layout)
            }
        } else {
            let fallback_layout = BufferLayout::compute(editor, areas.buffer_chunk);
            let viewport_start = render_buffer(frame, editor, theme, &fallback_layout);
            (viewport_start, fallback_layout)
        }
    } else {
        // Single window — apply textwidth centering if set
        let buffer_area = if let Some(textwidth) = editor.options.textwidth {
            let max_width = textwidth as u16;
            if areas.buffer_chunk.width > max_width {
                let margin = (areas.buffer_chunk.width - max_width) / 2;
                Rect {
                    x: areas.buffer_chunk.x + margin,
                    y: areas.buffer_chunk.y,
                    width: max_width,
                    height: areas.buffer_chunk.height,
                }
            } else {
                areas.buffer_chunk
            }
        } else {
            areas.buffer_chunk
        };

        let single_layout = BufferLayout::compute(editor, buffer_area);

        if editor.options.wrap {
            editor.ensure_wrap_map(single_layout.text_width);
        }

        let viewport_start = render_buffer(frame, editor, theme, &single_layout);
        (viewport_start, single_layout)
    }
}

/// Phase 4: Render the status area (progress line + status line + command/message line).
fn render_status_area(frame: &mut Frame, editor: &Editor, areas: &FrameAreas) {
    if let Some(progress_chunk) = areas.progress_chunk {
        if let Some(progress_msg) = editor.lsp_progress_message() {
            render_progress_line(frame, &progress_msg, progress_chunk);
        }
    }

    // Status line is always visible (mode, filename, position, diagnostics, LSP)
    render_status_line(frame, editor, areas.status_chunk);

    // Command/message line below the status line
    if editor.mode() == crate::mode::Mode::Command {
        render_command_line(frame, editor, areas.command_chunk);
    } else if editor.mode() == crate::mode::Mode::Search {
        render_search_line(frame, editor, areas.command_chunk);
    } else if editor.mode() == crate::mode::Mode::RenameInput {
        render_rename_input(frame, editor, areas.command_chunk);
    } else {
        render_message_line(frame, editor, areas.command_chunk);
    }
}

/// Phase 5: Render overlay widgets (picker, hover, completion, path completion).
fn render_overlays(
    frame: &mut Frame,
    editor: &mut Editor,
    theme: &Theme,
    ctx: &OverlayContext,
    command_chunk: Rect,
) {
    // Diagnostic badge (top-right of buffer area) — hidden during full-screen overlays
    let mode = editor.mode();
    if !matches!(
        mode,
        crate::mode::Mode::Picker
            | crate::mode::Mode::LspManager
            | crate::mode::Mode::HoverPreview
            | crate::mode::Mode::HoverNavigate
    ) {
        render_diagnostic_badge(frame, editor, ctx.layout.buffer_area);
    }

    // LSP Manager overlay
    if editor.mode() == crate::mode::Mode::LspManager {
        if let Some(panel) = editor.lsp_manager_panel() {
            super::lsp_manager::render_lsp_manager(frame, panel);
        }
    }

    // Picker overlay
    if editor.mode() == crate::mode::Mode::Picker {
        render_picker(frame, editor);
    }

    // Hover window
    if editor.mode().is_hover() {
        if let Some(hover_text) = editor.hover_info() {
            let is_preview = editor.mode() == crate::mode::Mode::HoverPreview;
            let hover_pos = editor.hover_position();
            let content_type = editor.hover_content_type();
            render_hover_window(
                frame,
                editor,
                hover_text,
                editor.hover_scroll(),
                ctx,
                hover_pos,
                is_preview,
                theme,
                content_type,
            );
        }
    }

    // Completion menu (LSP)
    if editor.completion_menu().is_visible() {
        render_completion_menu(frame, editor, ctx);
    }

    // Path completion popup (command mode)
    if editor.path_completion().is_visible() {
        render_path_completion(frame, editor, command_chunk);
    }
}

/// Sets the hardware cursor position based on the current mode.
fn set_cursor_position(
    frame: &mut Frame,
    editor: &mut Editor,
    ctx: &OverlayContext,
    command_chunk: Rect,
) {
    let layout = ctx.layout;
    let viewport_start = ctx.viewport_start;
    let cursor_pos = editor.buffer().cursor();
    let cursor_line = cursor_pos.line();
    let cursor_col = cursor_pos.col();

    if editor.mode() == crate::mode::Mode::LspManager {
        if let Some(panel) = editor.lsp_manager_panel() {
            if panel.filter_focused {
                let mgr_area = super::lsp_manager::get_lsp_manager_area(frame.area());
                let inner_x = mgr_area.x + 1;
                let inner_y = mgr_area.y + 1;
                let cursor_x = inner_x + 2 + panel.filter_query.len() as u16;
                frame.set_cursor_position((cursor_x, inner_y));
            }
        }
        return;
    }

    if editor.mode() == crate::mode::Mode::Picker {
        if let Some(picker) = editor.picker() {
            let picker_area = super::picker_widget::get_picker_area(frame.area());
            // Inner area is picker_area inset by 1 on each side (border)
            let inner_x = picker_area.x + 1;
            let inner_width = picker_area.width.saturating_sub(2) as usize;
            let cursor_y = picker_area.y + 1;

            let cursor_x = if picker.has_file_filter() {
                use crate::editor::PickerField;
                let search_width = (inner_width * 70 / 100).max(10);
                match picker.active_field() {
                    PickerField::Query => {
                        // icon(1) + space(1) + cursor_pos
                        let pos = picker.query_cursor();
                        (inner_x + 2 + pos as u16).min(inner_x + search_width as u16 - 1)
                    }
                    PickerField::FileFilter => {
                        // search_width + sep(1) + icon(1) + space(1) + cursor_pos
                        let pos = picker.file_filter_cursor();
                        let filter_start = inner_x + search_width as u16 + 1; // after separator
                        (filter_start + 2 + pos as u16).min(inner_x + inner_width as u16 - 1)
                    }
                }
            } else {
                let cursor_pos = picker.query_cursor();
                (inner_x + 2 + cursor_pos as u16).min(inner_x + inner_width as u16 - 1)
            };

            frame.set_cursor_position((cursor_x, cursor_y));
        }
    } else if editor.mode() == crate::mode::Mode::Command {
        let cmd_cursor_x =
            (editor.command_line().len() + 1).min(command_chunk.width.saturating_sub(1) as usize);
        frame.set_cursor_position((command_chunk.x + cmd_cursor_x as u16, command_chunk.y));
    } else if editor.mode() == crate::mode::Mode::Search {
        let search_cursor_x = (editor.search.search_buffer.len() + 1)
            .min(command_chunk.width.saturating_sub(1) as usize);
        frame.set_cursor_position((command_chunk.x + search_cursor_x as u16, command_chunk.y));
    } else if editor.mode() == crate::mode::Mode::RenameInput {
        // "rename: " is 8 chars
        let rename_cursor_x =
            (editor.rename_cursor() + 8).min(command_chunk.width.saturating_sub(1) as usize);
        frame.set_cursor_position((command_chunk.x + rename_cursor_x as u16, command_chunk.y));
    } else {
        let rope = editor.buffer().rope();
        let line_count = editor.buffer().line_count();
        let line_text = if cursor_line < line_count {
            rope.line(cursor_line).to_string()
        } else {
            String::new()
        };
        let line_text = line_text.trim_end_matches('\n');

        let tab_width = editor.options.tab_width;
        let display_col = char_col_to_display_col(line_text, cursor_col, tab_width);

        let buffer_area = layout.buffer_area;
        let gutter_width = layout.gutter_width;
        let text_width = layout.text_width;

        let (cursor_y, cursor_x) = if editor.options.wrap && text_width > 0 {
            if let Some(wrap_map) = editor.wrap_map() {
                let (abs_visual_row, _) = wrap_map.cursor_to_visual(cursor_line, display_col);
                let viewport_visual_row = wrap_map.logical_to_visual(viewport_start);
                let screen_row = abs_visual_row.saturating_sub(viewport_visual_row);
                let screen_col = display_col % text_width;
                (
                    screen_row.min(buffer_area.height.saturating_sub(1) as usize),
                    screen_col.min(text_width.saturating_sub(1)),
                )
            } else {
                let screen_line = cursor_line.saturating_sub(viewport_start);
                let h_offset = editor.horizontal_offset();
                let adjusted_col = display_col.saturating_sub(h_offset);
                (
                    screen_line.min(buffer_area.height.saturating_sub(1) as usize),
                    adjusted_col.min(text_width.saturating_sub(1)),
                )
            }
        } else {
            let screen_line = cursor_line.saturating_sub(viewport_start);
            let h_offset = editor.horizontal_offset();
            let adjusted_col = display_col.saturating_sub(h_offset);
            (
                screen_line.min(buffer_area.height.saturating_sub(1) as usize),
                adjusted_col.min(text_width.saturating_sub(1)),
            )
        };

        frame.set_cursor_position((
            buffer_area.x + gutter_width as u16 + cursor_x as u16,
            buffer_area.y + cursor_y as u16,
        ));
    }
}

// ---------------------------------------------------------------------------
// Split window rendering (unchanged)
// ---------------------------------------------------------------------------

/// Recursively renders windows in a split layout
/// Returns (viewport_start, layout) for the focused window (for cursor positioning)
fn render_window_tree(
    frame: &mut Frame,
    editor: &Editor,
    theme: &Theme,
    node: &WindowNode,
    area: Rect,
    focused_index: usize,
    current_index: &mut usize,
) -> Option<(usize, BufferLayout)> {
    match node {
        WindowNode::Leaf(_window) => {
            let is_focused = *current_index == focused_index;
            *current_index += 1;

            let layout = BufferLayout::compute(editor, area);
            let viewport_start = render_buffer(frame, editor, theme, &layout);

            if is_focused {
                Some((viewport_start, layout))
            } else {
                None
            }
        }
        WindowNode::Split {
            direction,
            ratio,
            first,
            second,
        } => {
            let (first_area, sep_area, second_area) = match direction {
                SplitDirection::Horizontal => {
                    let first_height = (area.height as f32 * *ratio) as u16;
                    let sep_height = 1u16;
                    let second_height = area.height.saturating_sub(first_height + sep_height);

                    let first_rect = Rect {
                        x: area.x,
                        y: area.y,
                        width: area.width,
                        height: first_height,
                    };
                    let sep_rect = Rect {
                        x: area.x,
                        y: area.y + first_height,
                        width: area.width,
                        height: sep_height,
                    };
                    let second_rect = Rect {
                        x: area.x,
                        y: area.y + first_height + sep_height,
                        width: area.width,
                        height: second_height,
                    };
                    (first_rect, sep_rect, second_rect)
                }
                SplitDirection::Vertical => {
                    let first_width = (area.width as f32 * *ratio) as u16;
                    let sep_width = 1u16;
                    let second_width = area.width.saturating_sub(first_width + sep_width);

                    let first_rect = Rect {
                        x: area.x,
                        y: area.y,
                        width: first_width,
                        height: area.height,
                    };
                    let sep_rect = Rect {
                        x: area.x + first_width,
                        y: area.y,
                        width: sep_width,
                        height: area.height,
                    };
                    let second_rect = Rect {
                        x: area.x + first_width + sep_width,
                        y: area.y,
                        width: second_width,
                        height: area.height,
                    };
                    (first_rect, sep_rect, second_rect)
                }
            };

            render_separator(frame, sep_area, *direction);

            let first_result = render_window_tree(
                frame,
                editor,
                theme,
                first,
                first_area,
                focused_index,
                current_index,
            );
            let second_result = render_window_tree(
                frame,
                editor,
                theme,
                second,
                second_area,
                focused_index,
                current_index,
            );

            first_result.or(second_result)
        }
    }
}

/// Renders a separator line between split windows
fn render_separator(frame: &mut Frame, area: Rect, direction: SplitDirection) {
    let sep_char = match direction {
        SplitDirection::Horizontal => '─',
        SplitDirection::Vertical => '│',
    };

    let sep_style = Style::default()
        .fg(Color::DarkGray)
        .add_modifier(Modifier::DIM);

    match direction {
        SplitDirection::Horizontal => {
            let line_text = sep_char.to_string().repeat(area.width as usize);
            let line = Line::from(Span::styled(line_text, sep_style));
            let paragraph = Paragraph::new(vec![line]);
            frame.render_widget(paragraph, area);
        }
        SplitDirection::Vertical => {
            let lines: Vec<Line> = (0..area.height)
                .map(|_| Line::from(Span::styled(sep_char.to_string(), sep_style)))
                .collect();
            let paragraph = Paragraph::new(lines);
            frame.render_widget(paragraph, area);
        }
    }
}

// ---------------------------------------------------------------------------
// Renderer struct
// ---------------------------------------------------------------------------

/// Handles rendering the editor state to the terminal
pub struct Renderer {
    terminal: RatatuiTerminal<CrosstermBackend<io::Stdout>>,
    /// Reserved for future theme application
    #[allow(dead_code)]
    theme: Theme,
}

impl Default for Renderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderer {
    /// Creates a new renderer
    pub fn new() -> Self {
        let backend = CrosstermBackend::new(io::stdout());
        let terminal = RatatuiTerminal::new(backend).expect("Failed to create terminal");
        Self {
            terminal,
            theme: Theme::default(),
        }
    }

    /// Renders editor to a frame (used by both TUI and headless rendering)
    pub fn render_to_frame(frame: &mut Frame, editor: &mut Editor) {
        clear_frame(frame, editor);

        let areas = match compute_frame_layout(frame, editor) {
            Some(areas) => areas,
            None => {
                let area = frame.area();
                render_dashboard(frame, editor, area);
                return;
            }
        };

        let scheme = editor
            .get_color_scheme()
            .cloned()
            .unwrap_or_else(crate::syntax::ColorScheme::tokyonight);
        let theme = Theme::from_scheme(scheme);

        // Render chrome
        if let Some(tab_area) = areas.tab_area {
            render_tab_bar(frame, editor, tab_area);
        }
        if let Some(tree_area) = areas.file_tree_area {
            render_file_tree(frame, editor, tree_area);
        }

        // Render buffer content
        let (viewport_start, layout) = render_buffer_area(frame, editor, &theme, &areas);

        // Update viewport dimensions and cache layout for mouse coordinate conversion
        editor.set_viewport_height(layout.buffer_area.height as usize);
        editor.set_last_layout(layout.buffer_area, layout.gutter_width, layout.text_width);
        if let Some(wm) = editor.window_manager_mut() {
            wm.update_dimensions(layout.buffer_area.width, layout.buffer_area.height);
        }

        // Render status + overlays + cursor
        render_status_area(frame, editor, &areas);
        let ctx = OverlayContext {
            layout: &layout,
            viewport_start,
        };
        render_overlays(frame, editor, &theme, &ctx, areas.command_chunk);
        set_cursor_position(frame, editor, &ctx, areas.command_chunk);
    }

    /// Renders the editor state to the terminal
    pub fn render(&mut self, editor: &mut Editor) -> Result<()> {
        let cursor_style = match editor.mode() {
            crate::mode::Mode::Insert => SetCursorStyle::BlinkingBar,
            crate::mode::Mode::Picker => SetCursorStyle::BlinkingBar,
            crate::mode::Mode::Command => SetCursorStyle::BlinkingBar,
            crate::mode::Mode::Search => SetCursorStyle::BlinkingBar,
            crate::mode::Mode::RenameInput => SetCursorStyle::BlinkingBar,
            crate::mode::Mode::HoverPreview | crate::mode::Mode::HoverNavigate => {
                SetCursorStyle::SteadyBlock
            }
            _ => SetCursorStyle::SteadyBlock,
        };
        let title = editor
            .buffer()
            .file_path()
            .map(|p| {
                std::path::Path::new(p)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(p)
            })
            .unwrap_or("ovim");
        crossterm::execute!(io::stdout(), cursor_style, SetTitle(title))?;

        self.terminal.autoresize()?;

        self.terminal.draw(|frame| {
            Self::render_to_frame(frame, editor);
        })?;

        use std::io::Write;
        io::stdout().flush()?;

        Ok(())
    }

    /// Clears the terminal
    pub fn clear(&mut self) -> Result<()> {
        self.terminal.clear()?;
        Ok(())
    }
}
