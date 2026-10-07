use super::layout::{FrameLayout, GutterWidths};
use crate::editor::Editor;
use ovim_core::Rect;

/// Recomputes viewport geometry from raw grid cells: `width`/`height` are the
/// full window in character cells (not the content area — [`FrameLayout`]
/// subtracts the chrome: tab bar, file tree and other side panels, the run
/// console, LSP progress line, and the status+command lines). Keeps viewport
/// height, window manager dimensions, the wrap map, and the scroll offset in
/// sync with the new size.
///
/// Call this whenever the grid geometry changes (terminal resize, window
/// resize, split/pane changes) — see the frontend contract in
/// [`crate::frontend`].
pub fn handle_viewport_resize(editor: &mut Editor, width: u16, height: u16) {
    // Ask the layout the renderer will use where the editor's windows end up,
    // so the wrap map is built for the width the next frame draws at.
    let (mut content_width, content_height) = match FrameLayout::compute(
        editor,
        Rect {
            x: 0,
            y: 0,
            width,
            height,
        },
    ) {
        Some(layout) => (layout.buffer.width, layout.buffer.height),
        // The dashboard has no windows; keep the status and command lines.
        None => (width, height.saturating_sub(2)),
    };

    // Apply textwidth centering: narrowing changes wrap width, but not viewport height.
    if let Some(textwidth) = editor.options.textwidth {
        let max_width = textwidth as u16;
        if content_width > max_width {
            content_width = max_width;
        }
    }

    // Update cached viewport dimensions for scroll calculations.
    editor.set_viewport_height(content_height as usize);

    // Update window sizes so horizontal scrolling calculations use the latest width.
    if let Some(wm) = editor.window_manager_mut() {
        wm.update_dimensions(content_width, content_height);
    } else {
        editor.init_window_manager(content_width, content_height);
    }

    // Keep the wrap map in sync with the new width so vertical scrolling stays accurate
    // in wrap mode.
    if editor.options.wrap {
        // Split panes have distinct widths and heights. Build their final
        // geometry before consuming a split's pending cursor-row anchor.
        let panes: Vec<_> = editor
            .window_manager()
            .map(|manager| {
                (0..manager.window_count())
                    .filter_map(|index| {
                        manager
                            .get_window(index)
                            .map(|window| (index, window.width(), window.height() as usize))
                    })
                    .collect()
            })
            .unwrap_or_default();
        for (index, pane_width, pane_height) in panes {
            let text_width = compute_text_width(editor, pane_width);
            editor.ensure_wrap_map_for_window(index, text_width);
            editor.repair_pending_split_wrap_viewport(index, pane_height);
        }
    }

    // Re-run scroll update so the cursor remains visible in the resized viewport.
    editor.update_scroll_offset();
}

/// Computes the wrap width (content width minus gutter) for a given content
/// area width. The gutter comes from [`GutterWidths`], the same source
/// `BufferLayout::compute` uses, so the two cannot drift apart.
pub fn compute_text_width(editor: &Editor, content_width: u16) -> usize {
    let gutter_width = GutterWidths::of(editor).total;

    // Apply textwidth narrowing (OV-00019: must match renderer's BufferLayout
    // which narrows buffer_area to textwidth before computing text_width).
    let effective_width = if let Some(textwidth) = editor.options.textwidth {
        let max = textwidth as u16;
        if content_width > max {
            max
        } else {
            content_width
        }
    } else {
        content_width
    };

    (effective_width as usize)
        .saturating_sub(gutter_width + usize::from(editor.is_diff_buffer() && effective_width > 1))
}

#[cfg(test)]
mod tests {
    use super::{compute_text_width, handle_viewport_resize};
    use crate::editor::Editor;

    #[test]
    fn resize_updates_viewport_and_wrap_map_and_keeps_cursor_visible() {
        // 200 logical lines, wide enough to exercise gutter sizing.
        let content: String = (1..=200)
            .map(|i| format!("line {i}: {}\n", "x".repeat(120)))
            .collect();

        let mut editor = Editor::with_content(&content);
        editor.options.number = true;
        editor.options.wrap = true;
        editor.options.scrolloff = 0;

        // Initial size.
        handle_viewport_resize(&mut editor, 80, 20);
        assert_eq!(editor.viewport_height(), 18);

        // Move cursor to EOF and ensure scroll offset is set.
        let last_line = editor.buffer().line_count().saturating_sub(1);
        editor
            .buffer_mut()
            .cursor_mut()
            .set_position(last_line, crate::unicode::GraphemeCol::ZERO);
        editor.update_scroll_offset();

        // Shrink the pane; cursor should remain visible in the new viewport.
        handle_viewport_resize(&mut editor, 80, 10);
        assert_eq!(editor.viewport_height(), 8);

        let cursor_line = editor.buffer().cursor().line();
        let scroll_offset = editor.scroll_offset();
        let visible = editor.viewport_height().max(1);
        assert!(
            cursor_line >= scroll_offset && cursor_line < scroll_offset + visible,
            "cursor should remain visible after resize: cursor_line={cursor_line} scroll_offset={scroll_offset} viewport={visible}"
        );

        // Wrap map should match the new text width (buffer width minus gutter).
        let wrap_width = editor.wrap_map().map(|m| m.wrap_width()).unwrap_or(0);
        assert_eq!(wrap_width, compute_text_width(&editor, 80).max(1));
    }
}
