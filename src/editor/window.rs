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
    /// Horizontal scroll offset (leftmost visible column)
    horizontal_offset: usize,
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
            horizontal_offset: 0,
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

    /// Gets the horizontal scroll offset
    pub fn horizontal_offset(&self) -> usize {
        self.horizontal_offset
    }

    /// Sets the horizontal scroll offset
    pub fn set_horizontal_offset(&mut self, offset: usize) {
        self.horizontal_offset = offset;
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

    /// Ensures the cursor column is visible horizontally, adjusting horizontal offset if needed
    /// Returns true if horizontal offset changed
    pub fn ensure_cursor_visible_horizontal(
        &mut self,
        cursor_col: usize,
        wrap: bool,
        sidescroll: usize,
        sidescrolloff: usize,
    ) -> bool {
        // If wrap enabled, no horizontal scrolling needed
        if wrap {
            if self.horizontal_offset != 0 {
                self.horizontal_offset = 0;
                return true;
            }
            return false;
        }

        let visible_width = self.width as usize;
        let old_offset = self.horizontal_offset;

        // Calculate bounds with sidescrolloff
        let left_bound = self.horizontal_offset + sidescrolloff;
        let right_bound = self.horizontal_offset + visible_width.saturating_sub(sidescrolloff + 1);

        // Cursor is too far left
        if cursor_col < left_bound {
            if sidescroll == 0 {
                // Jump to center cursor horizontally
                self.horizontal_offset = cursor_col.saturating_sub(visible_width / 2);
            } else {
                // Scroll left by sidescroll amount
                let scroll_amount = left_bound - cursor_col;
                let scroll_step = scroll_amount.div_ceil(sidescroll) * sidescroll;
                self.horizontal_offset = self.horizontal_offset.saturating_sub(scroll_step);
            }
        }
        // Cursor is too far right
        else if cursor_col > right_bound {
            if sidescroll == 0 {
                // Jump to center cursor horizontally
                self.horizontal_offset = cursor_col.saturating_sub(visible_width / 2);
            } else {
                // Scroll right by sidescroll amount
                let scroll_amount = cursor_col - right_bound;
                let scroll_step = scroll_amount.div_ceil(sidescroll) * sidescroll;
                self.horizontal_offset += scroll_step;
            }
        }

        old_offset != self.horizontal_offset
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
    /// Current layout width (updated via update_dimensions)
    layout_width: u16,
    /// Current layout height (updated via update_dimensions)
    layout_height: u16,
}

/// A rectangular area (position + size) for spatial window calculations
#[derive(Debug, Clone, Copy)]
struct Rect {
    x: u16,
    y: u16,
    width: u16,
    height: u16,
}

impl Rect {
    /// Creates a new rectangle
    fn new(x: u16, y: u16, width: u16, height: u16) -> Self {
        Self { x, y, width, height }
    }

    /// Returns the right edge (x + width)
    fn right(&self) -> u16 {
        self.x.saturating_add(self.width)
    }

    /// Returns the bottom edge (y + height)
    fn bottom(&self) -> u16 {
        self.y.saturating_add(self.height)
    }

    /// Returns the center x coordinate
    fn center_x(&self) -> u16 {
        self.x.saturating_add(self.width / 2)
    }

    /// Returns the center y coordinate
    fn center_y(&self) -> u16 {
        self.y.saturating_add(self.height / 2)
    }

    /// Calculates vertical overlap with another rect (used for left/right navigation)
    fn vertical_overlap(&self, other: &Rect) -> u16 {
        let top = self.y.max(other.y);
        let bottom = self.bottom().min(other.bottom());
        bottom.saturating_sub(top)
    }

    /// Calculates horizontal overlap with another rect (used for up/down navigation)
    fn horizontal_overlap(&self, other: &Rect) -> u16 {
        let left = self.x.max(other.x);
        let right = self.right().min(other.right());
        right.saturating_sub(left)
    }

    /// Calculates squared distance between centers (avoids floating point)
    fn distance_squared(&self, other: &Rect) -> u32 {
        let dx = (self.center_x() as i32 - other.center_x() as i32).abs() as u32;
        let dy = (self.center_y() as i32 - other.center_y() as i32).abs() as u32;
        dx * dx + dy * dy
    }
}

impl WindowManager {
    /// Creates a new window manager with a single window
    pub fn new(buffer_id: usize, width: u16, height: u16) -> Self {
        let window = Window::new(buffer_id, width, height);
        Self {
            root: WindowNode::new_leaf(window),
            focused_window: 0,
            layout_width: width,
            layout_height: height,
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
        self.layout_width = width;
        self.layout_height = height;
        self.root.update_dimensions(0, 0, width, height);
    }

    /// Gets the total number of windows
    pub fn window_count(&self) -> usize {
        self.root.count_windows()
    }

    /// Collects all window rectangles in depth-first order
    /// Returns a vector of (window_index, Rect) pairs
    fn collect_window_rects(&self, width: u16, height: u16) -> Vec<(usize, Rect)> {
        let mut rects = Vec::new();
        let mut current_index = 0;
        Self::collect_rects_recursive(&self.root, 0, 0, width, height, &mut rects, &mut current_index);
        rects
    }

    /// Recursively collects window rectangles
    fn collect_rects_recursive(
        node: &WindowNode,
        x: u16,
        y: u16,
        width: u16,
        height: u16,
        rects: &mut Vec<(usize, Rect)>,
        current_index: &mut usize,
    ) {
        match node {
            WindowNode::Leaf(_) => {
                rects.push((*current_index, Rect::new(x, y, width, height)));
                *current_index += 1;
            }
            WindowNode::Split {
                direction,
                ratio,
                first,
                second,
            } => {
                match direction {
                    SplitDirection::Horizontal => {
                        // Windows stacked vertically
                        let first_height = (height as f32 * *ratio) as u16;
                        let second_height = height.saturating_sub(first_height).saturating_sub(1); // -1 for separator
                        Self::collect_rects_recursive(first, x, y, width, first_height, rects, current_index);
                        Self::collect_rects_recursive(
                            second,
                            x,
                            y + first_height + 1,
                            width,
                            second_height,
                            rects,
                            current_index,
                        );
                    }
                    SplitDirection::Vertical => {
                        // Windows side by side
                        let first_width = (width as f32 * *ratio) as u16;
                        let second_width = width.saturating_sub(first_width).saturating_sub(1); // -1 for separator
                        Self::collect_rects_recursive(first, x, y, first_width, height, rects, current_index);
                        Self::collect_rects_recursive(
                            second,
                            x + first_width + 1,
                            y,
                            second_width,
                            height,
                            rects,
                            current_index,
                        );
                    }
                }
            }
        }
    }

    /// Finds the best window to focus in a given direction
    /// Returns true if focus changed
    fn focus_directional<F>(&mut self, width: u16, height: u16, is_candidate: F) -> bool
    where
        F: Fn(&Rect, &Rect) -> bool,
    {
        let total_windows = self.root.count_windows();
        if total_windows <= 1 {
            return false;
        }

        let rects = self.collect_window_rects(width, height);

        // Find current window rect
        let current_rect = rects
            .iter()
            .find(|(idx, _)| *idx == self.focused_window)
            .map(|(_, rect)| *rect);

        let Some(current_rect) = current_rect else {
            return false;
        };

        // Find best candidate window
        let best_candidate = rects
            .iter()
            .filter(|(idx, _)| *idx != self.focused_window)
            .filter(|(_, rect)| is_candidate(&current_rect, rect))
            .min_by_key(|(_, rect)| {
                // Primary: prefer windows with overlap (0 distance)
                // Secondary: choose nearest by distance
                let distance = current_rect.distance_squared(rect);
                distance
            });

        if let Some((new_index, _)) = best_candidate {
            self.focused_window = *new_index;
            true
        } else {
            false
        }
    }

    /// Moves focus to the window to the left
    /// Returns true if focus changed
    pub fn focus_left(&mut self) -> bool {
        let width = self.layout_width;
        let height = self.layout_height;
        self.focus_directional(width, height, |current, candidate| {
            // Candidate must be to the left (right edge <= current left edge)
            if candidate.right() > current.x {
                return false;
            }
            // Prefer windows with vertical overlap
            current.vertical_overlap(candidate) > 0 || true // Accept any window to the left
        })
    }

    /// Moves focus to the window to the right
    /// Returns true if focus changed
    pub fn focus_right(&mut self) -> bool {
        let width = self.layout_width;
        let height = self.layout_height;
        self.focus_directional(width, height, |current, candidate| {
            // Candidate must be to the right (left edge >= current right edge)
            if candidate.x < current.right() {
                return false;
            }
            // Prefer windows with vertical overlap
            current.vertical_overlap(candidate) > 0 || true // Accept any window to the right
        })
    }

    /// Moves focus to the window above
    /// Returns true if focus changed
    pub fn focus_up(&mut self) -> bool {
        let width = self.layout_width;
        let height = self.layout_height;
        self.focus_directional(width, height, |current, candidate| {
            // Candidate must be above (bottom edge <= current top edge)
            if candidate.bottom() > current.y {
                return false;
            }
            // Prefer windows with horizontal overlap
            current.horizontal_overlap(candidate) > 0 || true // Accept any window above
        })
    }

    /// Moves focus to the window below
    /// Returns true if focus changed
    pub fn focus_down(&mut self) -> bool {
        let width = self.layout_width;
        let height = self.layout_height;
        self.focus_directional(width, height, |current, candidate| {
            // Candidate must be below (top edge >= current bottom edge)
            if candidate.y < current.bottom() {
                return false;
            }
            // Prefer windows with horizontal overlap
            current.horizontal_overlap(candidate) > 0 || true // Accept any window below
        })
    }
}
