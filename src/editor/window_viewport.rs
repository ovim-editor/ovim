//! Window splits and viewport scrolling

use super::{Editor, Motions, SplitDirection, WindowManager};

impl Editor {
    // === Window Management ===

    /// Gets a reference to the window manager
    pub fn window_manager(&self) -> Option<&WindowManager> {
        self.window_manager.as_ref()
    }

    /// Gets a mutable reference to the window manager
    pub fn window_manager_mut(&mut self) -> Option<&mut WindowManager> {
        self.window_manager.as_mut()
    }

    /// Initializes the window manager with the current viewport dimensions
    /// Call this once viewport size is known (typically from UI layer)
    pub fn init_window_manager(&mut self, width: u16, height: u16) {
        if self.window_manager.is_none() {
            self.window_manager = Some(WindowManager::new(0, width, height));
        }
    }

    /// Splits the current window horizontally (creates window above/below)
    pub fn split_window_horizontal(&mut self) {
        // Initialize window manager if needed (fallback dimensions)
        if self.window_manager.is_none() {
            self.init_window_manager(80, 24);
        }

        if let Some(wm) = &mut self.window_manager {
            wm.split_focused(SplitDirection::Horizontal, 0);
        }
    }

    /// Splits the current window vertically (creates window left/right)
    pub fn split_window_vertical(&mut self) {
        // Initialize window manager if needed (fallback dimensions)
        if self.window_manager.is_none() {
            self.init_window_manager(80, 24);
        }

        if let Some(wm) = &mut self.window_manager {
            wm.split_focused(SplitDirection::Vertical, 0);
        }
    }

    /// Moves focus to the next window
    pub fn focus_next_window(&mut self) {
        if let Some(wm) = &mut self.window_manager {
            wm.focus_next();
        }
    }

    /// Moves focus to the previous window
    pub fn focus_prev_window(&mut self) {
        if let Some(wm) = &mut self.window_manager {
            wm.focus_prev();
        }
    }

    /// Gets the current number of windows
    pub fn window_count(&self) -> usize {
        self.window_manager
            .as_ref()
            .map(|wm| wm.window_count())
            .unwrap_or(1)
    }

    // === Viewport Scrolling ===

    /// Scrolls viewport down N lines
    pub fn scroll_viewport_down(&mut self, lines: usize) {
        let buffer_line_count = self.buffer().line_count();
        if let Some(wm) = &mut self.window_manager {
            if let Some(window) = wm.focused_window_mut() {
                window.scroll_down(lines, buffer_line_count);
            }
        }
    }

    /// Scrolls viewport up N lines
    pub fn scroll_viewport_up(&mut self, lines: usize) {
        if let Some(wm) = &mut self.window_manager {
            if let Some(window) = wm.focused_window_mut() {
                window.scroll_up(lines);
            }
        }
    }

    /// Centers cursor in viewport
    pub fn center_cursor_in_viewport(&mut self) {
        if let Some(wm) = &mut self.window_manager {
            if let Some(window) = wm.focused_window_mut() {
                window.center_cursor();
            }
        }
    }

    /// Moves cursor line to top of viewport
    pub fn move_cursor_line_to_top(&mut self) {
        if let Some(wm) = &mut self.window_manager {
            if let Some(window) = wm.focused_window_mut() {
                window.move_cursor_to_top();
            }
        }
    }

    /// Moves cursor line to bottom of viewport
    pub fn move_cursor_line_to_bottom(&mut self) {
        if let Some(wm) = &mut self.window_manager {
            if let Some(window) = wm.focused_window_mut() {
                window.move_cursor_to_bottom();
            }
        }
    }

    /// Scrolls down half a page (both viewport and cursor)
    pub fn scroll_half_page_down(&mut self) {
        // Extract window info first to avoid borrowing conflicts
        let (viewport_start, viewport_height) = if let Some(wm) = &self.window_manager {
            if let Some(window) = wm.focused_window() {
                (window.scroll_offset(), window.height() as usize)
            } else {
                return;
            }
        } else {
            return;
        };

        // Now we can mutably borrow buffer
        let new_viewport =
            Motions::scroll_half_page_down(self.buffer_mut(), viewport_start, viewport_height);

        // Finally update window scroll offset
        if let Some(wm) = &mut self.window_manager {
            if let Some(window) = wm.focused_window_mut() {
                window.set_scroll_offset(new_viewport);
            }
        }
    }

    /// Scrolls up half a page (both viewport and cursor)
    pub fn scroll_half_page_up(&mut self) {
        // Extract window info first to avoid borrowing conflicts
        let (viewport_start, viewport_height) = if let Some(wm) = &self.window_manager {
            if let Some(window) = wm.focused_window() {
                (window.scroll_offset(), window.height() as usize)
            } else {
                return;
            }
        } else {
            return;
        };

        // Now we can mutably borrow buffer
        let new_viewport =
            Motions::scroll_half_page_up(self.buffer_mut(), viewport_start, viewport_height);

        // Finally update window scroll offset
        if let Some(wm) = &mut self.window_manager {
            if let Some(window) = wm.focused_window_mut() {
                window.set_scroll_offset(new_viewport);
            }
        }
    }

    /// Scrolls forward (down) one page (both viewport and cursor)
    pub fn scroll_page_down(&mut self) {
        // Extract window info first to avoid borrowing conflicts
        let (viewport_start, viewport_height) = if let Some(wm) = &self.window_manager {
            if let Some(window) = wm.focused_window() {
                (window.scroll_offset(), window.height() as usize)
            } else {
                return;
            }
        } else {
            return;
        };

        // Now we can mutably borrow buffer
        let new_viewport =
            Motions::scroll_page_down(self.buffer_mut(), viewport_start, viewport_height);

        // Finally update window scroll offset
        if let Some(wm) = &mut self.window_manager {
            if let Some(window) = wm.focused_window_mut() {
                window.set_scroll_offset(new_viewport);
            }
        }
    }

    /// Scrolls backward (up) one page (both viewport and cursor)
    pub fn scroll_page_up(&mut self) {
        // Extract window info first to avoid borrowing conflicts
        let (viewport_start, viewport_height) = if let Some(wm) = &self.window_manager {
            if let Some(window) = wm.focused_window() {
                (window.scroll_offset(), window.height() as usize)
            } else {
                return;
            }
        } else {
            return;
        };

        // Now we can mutably borrow buffer
        let new_viewport =
            Motions::scroll_page_up(self.buffer_mut(), viewport_start, viewport_height);

        // Finally update window scroll offset
        if let Some(wm) = &mut self.window_manager {
            if let Some(window) = wm.focused_window_mut() {
                window.set_scroll_offset(new_viewport);
            }
        }
    }
}
