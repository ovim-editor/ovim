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

use super::buffer::{render_buffer, WindowRenderContext};
use super::dashboard::render_dashboard;
use super::file_tree_widget::render_file_tree;
use super::helpers::grapheme_col_to_display_col;
use super::layout::{BufferLayout, OverlayContext};
use super::line_cache::LineRenderCache;
use super::overlays::{
    render_ai_chat_permission_dialog, render_ai_review_shortcuts, render_completion_menu,
    render_hover_window, render_lsp_install_dialog,
};
use super::picker_widget::{render_picker, Fill};
use super::status_widgets::{
    ai_prompt_panel_height, render_ai_prompt_line, render_command_line, render_margin_widgets,
    render_message_line, render_path_completion, render_progress_line, render_rename_input,
    render_search_line, render_status_line, render_tab_bar, render_top_right_toasts,
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
    chat_area: Option<Rect>,
    debug_side_area: Option<Rect>,
    debug_output_area: Option<Rect>,
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

    // Debug panels (if visible and session active)
    let debug_panels_visible =
        editor.debug_state().panels_visible && editor.debug_state().session_active;

    // Debug side panel (right) — split from content area
    let (content_area, debug_side_area) = if debug_panels_visible {
        let width = (content_area.width / 3).clamp(25, 50);
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(1), Constraint::Length(width)])
            .split(content_area);
        (chunks[0], Some(chunks[1]))
    } else {
        (content_area, None)
    };

    // Debug output panel (bottom) — split from content area
    let (content_area, debug_output_area) =
        if debug_panels_visible && !editor.debug_state().output_lines.is_empty() {
            let height = 6u16.min(content_area.height / 4);
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(1), Constraint::Length(height)])
                .split(content_area);
            (chunks[0], Some(chunks[1]))
        } else {
            (content_area, None)
        };

    // Buffer + optional progress line + status line + command/prompt area
    let has_progress = editor.lsp_progress_message().is_some();
    let is_ai_chat = editor.mode() == crate::mode::Mode::AiChat;
    let command_height = if editor.mode() == crate::mode::Mode::AiPrompt {
        let max_height = content_area
            .height
            .saturating_sub(if has_progress { 2 } else { 1 })
            .max(1);
        ai_prompt_panel_height(editor, content_area.width, max_height)
    } else {
        1
    };

    // In AiChat mode with review-focused mode off, split into buffer (left) + chat panel (right)
    let review_mode = editor.ai_chat_review_mode();
    let (effective_content, chat_area) = if is_ai_chat && !review_mode {
        let allow_edits = editor.ai_chat_allow_edits();
        let (buffer_rect, chat_rect) =
            super::ai_chat::compute_chat_split(content_area, allow_edits);
        (buffer_rect, Some(chat_rect))
    } else {
        (content_area, None)
    };

    let chunks = if has_progress {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints(
                [
                    Constraint::Min(1),
                    Constraint::Length(1),              // progress line
                    Constraint::Length(1),              // status line
                    Constraint::Length(command_height), // command/message line
                ]
                .as_ref(),
            )
            .split(effective_content)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints(
                [
                    Constraint::Min(1),
                    Constraint::Length(1),              // status line
                    Constraint::Length(command_height), // command/message line
                ]
                .as_ref(),
            )
            .split(effective_content)
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
        chat_area,
        debug_side_area,
        debug_output_area,
    })
}

/// Phase 3: Render the buffer area (split or single window), returning
/// the viewport start line and the focused window's layout.
fn render_buffer_area(
    frame: &mut Frame,
    editor: &mut Editor,
    theme: &Theme,
    areas: &FrameAreas,
    line_cache: &mut LineRenderCache,
) -> (usize, BufferLayout) {
    let has_splits = editor
        .window_manager()
        .map(|wm| wm.root().count_windows() > 1)
        .unwrap_or(false);

    if has_splits {
        // For splits, apply textwidth narrowing before computing wrap map width,
        // matching the single-window path. Without this, the wrap map gets the
        // full buffer_chunk width and wraps at the wrong column. (OV-00019)
        if editor.options.wrap {
            let est_area = if let Some(tw) = editor.options.textwidth {
                let max_w = tw as u16;
                if areas.buffer_chunk.width > max_w {
                    Rect {
                        width: max_w,
                        ..areas.buffer_chunk
                    }
                } else {
                    areas.buffer_chunk
                }
            } else {
                areas.buffer_chunk
            };
            let est_layout = BufferLayout::compute(editor, est_area);
            editor.ensure_wrap_map(est_layout.text_width);
        }

        // Render split windows recursively
        if let Some(wm) = editor.window_manager() {
            let focused_index = wm.focused_window_index();
            let root = wm.root().clone();
            let mut current_index = 0;
            let mut tree_ctx = RenderTreeContext {
                frame,
                editor,
                theme,
                focused_index,
                current_index: &mut current_index,
                line_cache,
            };
            if let Some((vs, ly)) = render_window_tree(
                &mut tree_ctx,
                &root,
                areas.buffer_chunk,
            ) {
                (vs, ly)
            } else {
                let fallback_layout = BufferLayout::compute(editor, areas.buffer_chunk);
                let viewport_start = render_buffer(
                    frame,
                    editor,
                    theme,
                    &fallback_layout,
                    line_cache,
                    true,
                    None,
                );
                (viewport_start, fallback_layout)
            }
        } else {
            let fallback_layout = BufferLayout::compute(editor, areas.buffer_chunk);
            let viewport_start = render_buffer(
                frame,
                editor,
                theme,
                &fallback_layout,
                line_cache,
                true,
                None,
            );
            (viewport_start, fallback_layout)
        }
    } else {
        // Single window — apply textwidth centering if set
        let buffer_area = if let Some(textwidth) = editor.options.textwidth {
            let max_width = textwidth as u16;
            if areas.buffer_chunk.width > max_width {
                let margin = (areas.buffer_chunk.width - max_width) / 2;

                // Render margin shading if configured
                if let crate::editor::MarginColor::Solid(r, g, b) = editor.options.margin_color {
                    let padding = editor.options.margin_padding as u16;
                    let shaded_margin = margin.saturating_sub(padding);
                    if shaded_margin > 0 {
                        let color = Color::Rgb(r, g, b);
                        let chunk = areas.buffer_chunk;
                        // Left margin shading
                        let left = Rect {
                            x: chunk.x,
                            y: chunk.y,
                            width: shaded_margin,
                            height: chunk.height,
                        };
                        frame.render_widget(Fill::bg(color), left);
                        // Right margin shading
                        let right_x = chunk.x + margin + max_width + padding;
                        let right_width = (chunk.x + chunk.width).saturating_sub(right_x);
                        if right_width > 0 {
                            let right = Rect {
                                x: right_x,
                                y: chunk.y,
                                width: right_width,
                                height: chunk.height,
                            };
                            frame.render_widget(Fill::bg(color), right);
                        }
                    }
                }

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

        let full_area = areas.buffer_chunk;
        let centered = buffer_area.width < full_area.width;
        let render_inline_vtext = !centered;

        let viewport_start = render_buffer(
            frame,
            editor,
            theme,
            &single_layout,
            line_cache,
            render_inline_vtext,
            None,
        );
        if centered {
            crate::ui::renderer::buffer::render_diagnostic_virtual_text_overlay(
                frame,
                editor,
                &single_layout,
                full_area,
            );
            render_margin_widgets(frame, editor, theme, full_area, buffer_area);
        }
        (viewport_start, single_layout)
    }
}

/// Phase 4: Render the status area (progress line + status line + command/message line).
fn render_status_area(frame: &mut Frame, editor: &mut Editor, theme: &Theme, areas: &FrameAreas) {
    if let Some(progress_chunk) = areas.progress_chunk {
        if let Some(progress_msg) = editor.lsp_progress_message() {
            render_progress_line(frame, &progress_msg, progress_chunk);
        }
    }

    // Status line is always visible (mode, filename, position, diagnostics, LSP)
    render_status_line(frame, editor, theme, areas.status_chunk);

    // Command/message line below the status line
    editor.render_cache.ai_prompt_input_area = None;
    editor.render_cache.ai_prompt_input_rows.clear();
    editor.render_cache.ai_prompt_model_hitboxes.clear();
    editor.render_cache.ai_prompt_model_trigger_hitbox = None;
    if editor.mode() == crate::mode::Mode::Command {
        render_command_line(frame, editor, areas.command_chunk);
    } else if editor.mode() == crate::mode::Mode::Search {
        render_search_line(frame, editor, areas.command_chunk);
    } else if editor.mode() == crate::mode::Mode::RenameInput {
        render_rename_input(frame, editor, areas.command_chunk);
    } else if editor.mode() == crate::mode::Mode::AiPrompt {
        let layout = render_ai_prompt_line(frame, editor, areas.command_chunk);
        editor.render_cache.ai_prompt_input_area = layout.input_area;
        editor.render_cache.ai_prompt_input_rows = layout.input_rows;
        editor.render_cache.ai_prompt_model_hitboxes = layout.model_hitboxes;
        editor.render_cache.ai_prompt_model_trigger_hitbox = layout.model_trigger_hitbox;
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
    if editor.mode() == crate::mode::Mode::AiChat && editor.ai_chat_review_mode() {
        render_ai_review_shortcuts(frame, theme, ctx.layout.buffer_area);
    }

    // Top-right toast overlays (diagnostics + transient notifications) — hidden during full-screen overlays
    let mode = editor.mode();
    let blocking_modal_active = has_blocking_modal(editor);
    let hide_toasts = matches!(
        mode,
        crate::mode::Mode::Picker
            | crate::mode::Mode::LspManager
            | crate::mode::Mode::HoverPreview
            | crate::mode::Mode::HoverNavigate
    ) || (mode == crate::mode::Mode::AiChat && editor.ai_chat_review_mode())
        || blocking_modal_active;
    if !hide_toasts {
        render_top_right_toasts(frame, editor, theme, ctx.layout.buffer_area);
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

fn has_blocking_modal(editor: &Editor) -> bool {
    editor.has_pending_lsp_install()
        || (editor.mode() == crate::mode::Mode::AiChat
            && (editor.ai_chat_has_pending_tool_approval()
                || editor.ai_chat_has_pending_no_repo_folder_approval()))
}

/// Render centered, blocking overlays after all other popup classes.
///
/// This tier is reserved for workflows that block agent/user progress until
/// explicitly resolved. Keep these dialogs highly visible and singular.
fn render_blocking_modals(frame: &mut Frame, editor: &Editor, theme: &Theme) {
    if editor.has_pending_lsp_install() {
        render_lsp_install_dialog(frame, editor, theme);
    } else if has_blocking_modal(editor) {
        render_ai_chat_permission_dialog(frame, editor, theme);
    }
}

/// Sets the hardware cursor position based on the current mode.
fn set_cursor_position(
    frame: &mut Frame,
    editor: &mut Editor,
    ctx: &OverlayContext,
    command_chunk: Rect,
    chat_area: Option<Rect>,
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
            (editor.command_cursor() + 1).min(command_chunk.width.saturating_sub(1) as usize);
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
    } else if editor.mode() == crate::mode::Mode::AiChat && chat_area.is_some() {
        if let Some(chat_rect) = chat_area {
            if let Some((cx, cy)) = super::ai_chat::chat_cursor_info(editor, chat_rect) {
                frame.set_cursor_position((cx, cy));
            }
        }
    } else if editor.mode() == crate::mode::Mode::AiPrompt {
        if !editor.render_cache.ai_prompt_input_rows.is_empty() {
            let cursor_byte = editor
                .ai_prompt_cursor()
                .min(editor.ai_prompt_input().len());
            let mut row = *editor.render_cache.ai_prompt_input_rows.last().unwrap();
            for candidate in &editor.render_cache.ai_prompt_input_rows {
                if cursor_byte < candidate.2 {
                    row = *candidate;
                    break;
                }
            }
            let row_start = row.1.min(editor.ai_prompt_input().len());
            let row_end = row.2.min(editor.ai_prompt_input().len()).max(row_start);
            let row_cursor = cursor_byte.clamp(row_start, row_end);
            let cursor_display = editor.ai_prompt_input()[row_start..row_cursor]
                .chars()
                .map(crate::display::char_display_width)
                .sum::<usize>();
            let clamped_x = row
                .0
                .x
                .saturating_add(cursor_display.min(row.0.width.saturating_sub(1) as usize) as u16);
            frame.set_cursor_position((clamped_x, row.0.y));
        } else if let Some(input_area) = editor.render_cache.ai_prompt_input_area {
            frame.set_cursor_position((input_area.x, input_area.y));
        } else {
            frame.set_cursor_position((command_chunk.x, command_chunk.y));
        }
    } else {
        let rope = editor.buffer().rope();
        let line_count = editor.buffer().line_count();
        let line_text = if cursor_line < line_count {
            rope.line(cursor_line).to_string()
        } else {
            String::new()
        };
        let line_text = line_text.trim_end_matches('\n').to_string();

        let tab_width = editor.options.tab_width;
        let display_col = grapheme_col_to_display_col(&line_text, cursor_col, tab_width);

        // Shift cursor right to account for inline decorations (inlay hints)
        // that appear before the cursor position.
        let inline_offset = editor.decorations.inline_width_before(cursor_line, cursor_col);
        let display_col = display_col + inline_offset;

        let buffer_area = layout.buffer_area;
        let gutter_width = layout.gutter_width;
        let text_width = layout.text_width;

        let (cursor_y, cursor_x) = if editor.options.wrap && text_width > 0 {
            if let Some(wrap_map) = editor.wrap_map() {
                let (abs_visual_row, visual_col) =
                    wrap_map.cursor_to_visual(cursor_line, display_col, &line_text);
                let viewport_visual_row = wrap_map.logical_to_visual(viewport_start);
                let screen_row = abs_visual_row.saturating_sub(viewport_visual_row);
                let screen_col = visual_col;
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

/// Invariant context for recursive window tree rendering.
struct RenderTreeContext<'a, 'b> {
    frame: &'a mut Frame<'b>,
    editor: &'a Editor,
    theme: &'a Theme,
    focused_index: usize,
    current_index: &'a mut usize,
    line_cache: &'a mut LineRenderCache,
}

/// Recursively renders windows in a split layout
/// Returns (viewport_start, layout) for the focused window (for cursor positioning)
fn render_window_tree(
    ctx: &mut RenderTreeContext,
    node: &WindowNode,
    area: Rect,
) -> Option<(usize, BufferLayout)> {
    match node {
        WindowNode::Leaf(window) => {
            let is_focused = *ctx.current_index == ctx.focused_index;
            *ctx.current_index += 1;

            let layout = BufferLayout::compute(ctx.editor, area);

            // For non-focused windows, use the window's own cursor and scroll state
            let window_context = if !is_focused {
                Some(WindowRenderContext {
                    cursor: Some(*window.cursor()),
                    scroll_offset: Some(window.scroll_offset()),
                    horizontal_offset: Some(window.horizontal_offset()),
                })
            } else {
                None
            };

            let viewport_start = render_buffer(
                ctx.frame,
                ctx.editor,
                ctx.theme,
                &layout,
                ctx.line_cache,
                true,
                window_context.as_ref(),
            );

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

            render_separator(ctx.frame, sep_area, *direction);

            let first_result = render_window_tree(ctx, first, first_area);
            let second_result = render_window_tree(ctx, second, second_area);

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
    /// Per-line render cache to avoid recomputing unchanged lines
    line_cache: LineRenderCache,
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
            line_cache: LineRenderCache::new(),
        }
    }

    /// Renders editor to a frame (used by both TUI and headless rendering)
    pub fn render_to_frame(
        frame: &mut Frame,
        editor: &mut Editor,
        line_cache: &mut LineRenderCache,
    ) {
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
            render_tab_bar(frame, editor, &theme, tab_area);
        }
        if let Some(tree_area) = areas.file_tree_area {
            render_file_tree(frame, editor, tree_area);
        }

        // Render buffer content
        let (viewport_start, layout) =
            render_buffer_area(frame, editor, &theme, &areas, line_cache);

        // Update viewport dimensions and cache layout for mouse coordinate conversion
        editor.set_viewport_height(layout.buffer_area.height as usize);
        editor.set_last_layout(
            crate::key_convert::convert_ratatui_rect(layout.buffer_area),
            layout.gutter_width,
            layout.text_width,
            layout.blame_width,
        );
        if let Some(wm) = editor.window_manager_mut() {
            wm.update_dimensions(layout.buffer_area.width, layout.buffer_area.height);
        }

        // Render chat panel (if in AiChat mode)
        if let Some(chat_area) = areas.chat_area {
            super::ai_chat::render_chat_panel(frame, editor, chat_area, &theme);
            editor.render_cache.last_chat_area =
                Some(crate::key_convert::convert_ratatui_rect(chat_area));
        } else {
            editor.render_cache.last_chat_area = None;
        }

        // Render debug panels (if visible)
        if let Some(debug_side) = areas.debug_side_area {
            super::debug_panels::render_debug_side_panel(frame, editor, debug_side);
        }
        if let Some(debug_output) = areas.debug_output_area {
            super::debug_panels::render_debug_output(frame, editor, debug_output);
        }

        // Render status + overlays + cursor
        render_status_area(frame, editor, &theme, &areas);
        let ctx = OverlayContext {
            layout: &layout,
            viewport_start,
        };
        render_overlays(frame, editor, &theme, &ctx, areas.command_chunk);
        render_blocking_modals(frame, editor, &theme);
        set_cursor_position(frame, editor, &ctx, areas.command_chunk, areas.chat_area);
    }

    /// Renders the editor state to the terminal
    pub fn render(&mut self, editor: &mut Editor) -> Result<()> {
        let cursor_style = match editor.mode() {
            crate::mode::Mode::Insert => SetCursorStyle::BlinkingBar,
            crate::mode::Mode::Picker => SetCursorStyle::BlinkingBar,
            crate::mode::Mode::Command => SetCursorStyle::BlinkingBar,
            crate::mode::Mode::Search => SetCursorStyle::BlinkingBar,
            crate::mode::Mode::RenameInput => SetCursorStyle::BlinkingBar,
            crate::mode::Mode::AiPrompt => SetCursorStyle::BlinkingBar,
            crate::mode::Mode::AiChat => SetCursorStyle::BlinkingBar,
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

        // Take the line cache out to avoid borrow conflict with terminal.draw()
        let mut line_cache = std::mem::take(&mut self.line_cache);
        self.terminal.draw(|frame| {
            Self::render_to_frame(frame, editor, &mut line_cache);
        })?;
        self.line_cache = line_cache;

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
