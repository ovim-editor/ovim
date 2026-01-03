use crate::buffer::Buffer;
use crate::buffer::Cursor;

/// Represents a window - a viewport into a buffer
#[derive(Debug, Clone)]
pub struct Window {
    /// The buffer this window is viewing
    buffer_id: usize,
    /// Window-local cursor position (for when buffer is shared)
    cursor: Cursor,
    /// Scroll offset (top line visible in window)
    scroll_offset: usize,
    /// Window width (columns)
    width: u16,
    /// Window height (rows)
    height: u16,
}

impl Window {
    /// Creates a new window for a buffer
    pub fn new(buffer_id: usize, width: u16, height: u16) -> Self {
        Self {
            buffer_id,
            cursor: Cursor::new(0, 0),
            scroll_offset: 0,
            width,
            height,
        }
    }

    /// Gets the buffer ID
    pub fn buffer_id(&self) -> usize {
        self.buffer_id
    }

    /// Gets a reference to the cursor
    pub fn cursor(&self) -> &Cursor {
        &self.cursor
    }

    /// Gets a mutable reference to the cursor
    pub fn cursor_mut(&mut self) -> &mut Cursor {
        &mut self.cursor
    }

    /// Gets the scroll offset
    pub fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    /// Sets the scroll offset
    pub fn set_scroll_offset(&mut self, offset: usize) {
        self.scroll_offset = offset;
    }

    /// Gets the window width
    pub fn width(&self) -> u16 {
        self.width
    }

    /// Gets the window height
    pub fn height(&self) -> u16 {
        self.height
    }

    /// Sets the window dimensions
    pub fn set_dimensions(&mut self, width: u16, height: u16) {
        self.width = width;
        self.height = height;
    }

    /// Ensures the cursor is visible in the window, adjusting scroll if needed
    /// scrolloff: minimum number of lines to keep above/below cursor
    pub fn ensure_cursor_visible(&mut self, buffer: &Buffer, scrolloff: usize) {
        let cursor_line = self.cursor.line();
        let visible_lines = self.height as usize;

        // Ensure cursor line is within buffer bounds
        let max_line = buffer.line_count().saturating_sub(1);
        if cursor_line > max_line {
            self.cursor.set_position(max_line, 0);
        }

        // Adjust scroll to maintain scrolloff distance from top/bottom
        // Scroll up if cursor is too close to top
        if cursor_line < self.scroll_offset + scrolloff {
            self.scroll_offset = cursor_line.saturating_sub(scrolloff);
        }
        // Scroll down if cursor is too close to bottom
        else if cursor_line + scrolloff >= self.scroll_offset + visible_lines {
            self.scroll_offset = cursor_line + scrolloff + 1 - visible_lines.min(cursor_line + scrolloff + 1);
        }
    }

    /// Centers the cursor in the window
    /// Note: Centering doesn't need scrolloff adjustment since cursor is already far from edges
    pub fn center_cursor(&mut self) {
        let cursor_line = self.cursor.line();
        let visible_lines = self.height as usize;
        let center_offset = visible_lines / 2;
        self.scroll_offset = cursor_line.saturating_sub(center_offset);
    }

    /// Scrolls viewport down N lines
    /// Returns whether cursor needed to be adjusted to stay visible
    pub fn scroll_down(&mut self, lines: usize, buffer_line_count: usize) -> bool {
        let max_scroll = buffer_line_count.saturating_sub(self.height as usize);
        let new_scroll = (self.scroll_offset + lines).min(max_scroll);
        let cursor_line = self.cursor.line();

        // Check if cursor would be above viewport after scrolling
        let needs_adjustment = cursor_line < new_scroll;

        self.scroll_offset = new_scroll;

        // Move cursor down if it would be above viewport
        if needs_adjustment {
            let col = self.cursor.col();
            self.cursor.set_position(new_scroll, col);
        }

        needs_adjustment
    }

    /// Scrolls viewport up N lines
    /// Returns whether cursor needed to be adjusted to stay visible
    pub fn scroll_up(&mut self, lines: usize) -> bool {
        let new_scroll = self.scroll_offset.saturating_sub(lines);
        let cursor_line = self.cursor.line();
        let last_visible = new_scroll + (self.height as usize).saturating_sub(1);

        // Check if cursor would be below viewport after scrolling
        let needs_adjustment = cursor_line > last_visible;

        self.scroll_offset = new_scroll;

        // Move cursor up if it would be below viewport
        if needs_adjustment {
            let col = self.cursor.col();
            self.cursor.set_position(last_visible, col);
        }

        needs_adjustment
    }

    /// Moves cursor line to top of viewport
    /// Respects scrolloff by positioning cursor scrolloff lines from the actual top
    pub fn move_cursor_to_top(&mut self, scrolloff: usize) {
        let cursor_line = self.cursor.line();
        // Position cursor scrolloff lines from top to respect scrolloff setting
        self.scroll_offset = cursor_line.saturating_sub(scrolloff);
    }

    /// Moves cursor line to bottom of viewport
    /// Respects scrolloff by positioning cursor scrolloff lines from the actual bottom
    pub fn move_cursor_to_bottom(&mut self, scrolloff: usize) {
        let cursor_line = self.cursor.line();
        let visible_lines = self.height as usize;
        // Position cursor scrolloff lines from bottom
        // Formula: cursor_line - (viewport_height - 1 - scrolloff)
        let bottom_position = visible_lines.saturating_sub(1).saturating_sub(scrolloff);
        self.scroll_offset = cursor_line.saturating_sub(bottom_position);
    }
}

/// Represents a split layout
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitDirection {
    Horizontal, // Windows stacked vertically (horizontal line separator)
    Vertical,   // Windows side by side (vertical line separator)
}

/// Represents a node in the window tree
#[derive(Debug, Clone)]
pub enum WindowNode {
    /// A leaf node containing a window
    Leaf(Window),
    /// A split node containing two children
    Split {
        direction: SplitDirection,
        ratio: f32, // 0.0 to 1.0, size of first child
        first: Box<WindowNode>,
        second: Box<WindowNode>,
    },
}

impl WindowNode {
    /// Creates a new leaf node
    pub fn new_leaf(window: Window) -> Self {
        WindowNode::Leaf(window)
    }

    /// Creates a new split node
    pub fn new_split(direction: SplitDirection, first: WindowNode, second: WindowNode) -> Self {
        WindowNode::Split {
            direction,
            ratio: 0.5, // Equal split by default
            first: Box::new(first),
            second: Box::new(second),
        }
    }

    /// Gets a reference to the window if this is a leaf node
    pub fn as_window(&self) -> Option<&Window> {
        match self {
            WindowNode::Leaf(window) => Some(window),
            _ => None,
        }
    }

    /// Gets a mutable reference to the window if this is a leaf node
    pub fn as_window_mut(&mut self) -> Option<&mut Window> {
        match self {
            WindowNode::Leaf(window) => Some(window),
            _ => None,
        }
    }

    /// Updates dimensions for all windows in the tree
    pub fn update_dimensions(&mut self, x: u16, y: u16, width: u16, height: u16) {
        // Note: x and y are currently only used in recursive calls to properly position
        // child windows in splits. The leaf case ignores them as Window only stores
        // width/height. This is intentional - the parameters maintain correct offsets
        // through the tree traversal even though they're unused at the leaves.
        match self {
            WindowNode::Leaf(window) => {
                let _ = (x, y); // Explicitly ignore - position tracked by parent
                window.set_dimensions(width, height);
            }
            WindowNode::Split {
                direction,
                ratio,
                first,
                second,
            } => {
                match direction {
                    SplitDirection::Horizontal => {
                        // Split horizontally (one window above another)
                        let first_height = (height as f32 * *ratio) as u16;
                        let second_height = height.saturating_sub(first_height).saturating_sub(1); // -1 for border
                        first.update_dimensions(x, y, width, first_height);
                        second.update_dimensions(x, y + first_height + 1, width, second_height);
                    }
                    SplitDirection::Vertical => {
                        // Split vertically (one window beside another)
                        let first_width = (width as f32 * *ratio) as u16;
                        let second_width = width.saturating_sub(first_width).saturating_sub(1); // -1 for border
                        first.update_dimensions(x, y, first_width, height);
                        second.update_dimensions(x + first_width + 1, y, second_width, height);
                    }
                }
            }
        }
    }

    /// Counts the number of leaf windows in the tree
    pub fn count_windows(&self) -> usize {
        match self {
            WindowNode::Leaf(_) => 1,
            WindowNode::Split { first, second, .. } => {
                first.count_windows() + second.count_windows()
            }
        }
    }
}

/// Manages windows and window layout
pub struct WindowManager {
    /// Root of the window tree
    root: WindowNode,
    /// Index of the currently focused window (0-based)
    focused_window: usize,
}

impl WindowManager {
    /// Creates a new window manager with a single window
    pub fn new(buffer_id: usize, width: u16, height: u16) -> Self {
        let window = Window::new(buffer_id, width, height);
        Self {
            root: WindowNode::new_leaf(window),
            focused_window: 0,
        }
    }

    /// Gets the root window node
    pub fn root(&self) -> &WindowNode {
        &self.root
    }

    /// Gets a mutable reference to the root window node
    pub fn root_mut(&mut self) -> &mut WindowNode {
        &mut self.root
    }

    /// Gets the index of the focused window
    pub fn focused_window_index(&self) -> usize {
        self.focused_window
    }

    /// Gets the focused window
    pub fn focused_window(&self) -> Option<&Window> {
        self.get_window(self.focused_window)
    }

    /// Gets the focused window mutably
    pub fn focused_window_mut(&mut self) -> Option<&mut Window> {
        self.get_window_mut(self.focused_window)
    }

    /// Gets a window by index (depth-first traversal)
    pub fn get_window(&self, index: usize) -> Option<&Window> {
        self.get_window_recursive(&self.root, index).map(|(w, _)| w)
    }

    /// Gets a window by index mutably
    pub fn get_window_mut(&mut self, index: usize) -> Option<&mut Window> {
        Self::get_window_recursive_mut_static(&mut self.root, index).map(|(w, _)| w)
    }

    /// Helper for recursive window lookup
    fn get_window_recursive<'a>(
        &self,
        node: &'a WindowNode,
        target_index: usize,
    ) -> Option<(&'a Window, usize)> {
        match node {
            WindowNode::Leaf(window) => {
                if target_index == 0 {
                    Some((window, 1))
                } else {
                    None
                }
            }
            WindowNode::Split { first, second, .. } => {
                if let Some((window, count)) = self.get_window_recursive(first, target_index) {
                    Some((window, count))
                } else if let Some((window, count)) =
                    self.get_window_recursive(second, target_index - first.count_windows())
                {
                    Some((window, count))
                } else {
                    None
                }
            }
        }
    }

    /// Static helper to avoid borrowing issues
    fn get_window_recursive_mut_static(
        node: &mut WindowNode,
        target_index: usize,
    ) -> Option<(&mut Window, usize)> {
        match node {
            WindowNode::Leaf(window) => {
                if target_index == 0 {
                    Some((window, 1))
                } else {
                    None
                }
            }
            WindowNode::Split { first, second, .. } => {
                let first_count = first.count_windows();
                if target_index < first_count {
                    Self::get_window_recursive_mut_static(first, target_index)
                } else {
                    Self::get_window_recursive_mut_static(second, target_index - first_count)
                }
            }
        }
    }

    /// Splits the focused window
    pub fn split_focused(&mut self, direction: SplitDirection, buffer_id: usize) {
        let focused_idx = self.focused_window;
        Self::split_window_by_index_static(&mut self.root, focused_idx, direction, buffer_id, 0);
    }

    /// Helper for recursive window splitting
    fn split_window_by_index_static(
        node: &mut WindowNode,
        target_index: usize,
        direction: SplitDirection,
        buffer_id: usize,
        current_index: usize,
    ) -> (bool, usize) {
        match node {
            WindowNode::Leaf(window) => {
                if current_index == target_index {
                    // Found the window to split - create new window with same dimensions
                    let new_window = Window::new(buffer_id, window.width(), window.height());
                    let old_window = std::mem::replace(window, Window::new(0, 0, 0));

                    // Replace this node with a split
                    *node = WindowNode::new_split(
                        direction,
                        WindowNode::new_leaf(old_window),
                        WindowNode::new_leaf(new_window),
                    );

                    (true, current_index + 1)
                } else {
                    (false, current_index + 1)
                }
            }
            WindowNode::Split { first, second, .. } => {
                let (found, next_index) = Self::split_window_by_index_static(
                    first,
                    target_index,
                    direction,
                    buffer_id,
                    current_index,
                );
                if found {
                    (true, next_index)
                } else {
                    Self::split_window_by_index_static(
                        second,
                        target_index,
                        direction,
                        buffer_id,
                        next_index,
                    )
                }
            }
        }
    }

    /// Moves focus to the next window
    pub fn focus_next(&mut self) {
        let total_windows = self.root.count_windows();
        if total_windows > 1 {
            self.focused_window = (self.focused_window + 1) % total_windows;
        }
    }

    /// Moves focus to the previous window
    pub fn focus_prev(&mut self) {
        let total_windows = self.root.count_windows();
        if total_windows > 1 {
            self.focused_window = if self.focused_window == 0 {
                total_windows - 1
            } else {
                self.focused_window - 1
            };
        }
    }

    /// Updates dimensions for all windows
    pub fn update_dimensions(&mut self, width: u16, height: u16) {
        self.root.update_dimensions(0, 0, width, height);
    }

    /// Gets the total number of windows
    pub fn window_count(&self) -> usize {
        self.root.count_windows()
    }
}
