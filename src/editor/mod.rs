mod buffer_manager;
mod change;
mod change_tracking;
mod command_context;
mod command_history;
mod completion;
mod filetree;
mod fold;
pub(crate) mod fuzzy;
pub(crate) mod grep;
mod input;
mod input_context;
mod input_state;
pub mod lsp_manager_panel;
mod keymap;
mod lsp_state;
mod lsp_integration;
mod lua_integration;
mod macros;
mod mark_jump;
mod marks;
mod motions;
mod operators;
pub(crate) mod path_completion;
mod performance;
pub mod nucleo_matcher;
pub mod picker;
mod picker_manager;
pub(crate) mod picker_state;
mod quickfix;
mod register;
mod search;
mod search_context;
mod search_manager;
mod tabpage;
mod tab_manager;
mod textobjects;
mod theme;
mod ui_features;
mod undo;
mod visual_context;
mod viewport_state;
mod visual_mode;
mod window;
mod window_viewport;
mod wrap_map;

pub use change::{Change, ChangeBuilder, ChangeManager, InsertEntryMode, Position, Range, TextObjectType};
pub use command_context::CommandContext;
pub use completion::CompletionMenu;
pub use filetree::{FileTree, TreeNode};
pub use fold::{Fold, FoldManager};
pub use input::InputHandler;
pub use input::mouse::handle_mouse_event;
pub use input::shell_expansion;
pub use input_context::InputContext;
pub use input_state::{CharMotion, InputState, TextObjectPrefix};
pub use lsp_state::{HoverContentType, LspAction, LspResultType, LspState};
pub use macros::MacroManager;
pub use keymap::{KeyMapManager, KeyMapping, MapMode};
pub use marks::{GlobalMark, JumpList, Mark, MarkManager, TagEntry, TagStack};
pub use motions::Motions;
pub use operators::Operator;
pub use performance::{PerformanceMetrics, MAX_LATENCY_SAMPLES};
pub use picker::{Picker, PickerAction, PickerField, PickerMode, PickerResult};
pub use picker_state::PickerState;
pub use quickfix::{LocationList, QuickfixEntry, QuickfixEntryType, QuickfixList};
pub use path_completion::PathCompletionState;
pub use register::{RegisterManager, RegisterType};
pub use search::Search;
pub use search_context::{SearchContext, VisualSearchState};
pub use tabpage::{TabPage, TabPageManager};
pub use textobjects::{TextObjectRange, TextObjects};
pub use undo::UndoManager;
pub use visual_context::{VisualContext, VisualSelection};
pub use window::{SplitDirection, Window, WindowManager, WindowNode};
pub use lsp_manager_panel::LspManagerPanel;
pub use viewport_state::ViewportState;
pub use wrap_map::WrapMap;

/// Editor options and settings
#[derive(Debug, Clone)]
pub struct EditorOptions {
    /// Width of tab character (default: 4)
    pub tab_width: usize,
    /// Number of spaces to use for autoindent (default: 4)
    pub shift_width: usize,
    /// Use spaces instead of tabs (default: true)
    pub expand_tab: bool,
    /// Show line numbers (default: false)
    pub number: bool,
    /// Show relative line numbers (default: false)
    pub relative_number: bool,
    /// Number of lines to scroll for half-page movements (default: None = calculate from viewport)
    pub scroll: Option<usize>,
    /// Maximum width of text content (default: None = use full terminal width)
    /// When set, content is centered horizontally with margins on both sides
    pub textwidth: Option<usize>,
    /// Ignore case in search patterns (default: false)
    pub ignorecase: bool,
    /// Smart case: case-insensitive if pattern is all lowercase, case-sensitive otherwise (default: false)
    /// Only applies when ignorecase is also set
    pub smartcase: bool,
    /// Highlight the current line (default: false)
    pub cursorline: bool,
    /// Highlight matching brackets (default: true)
    pub showmatch: bool,
    /// Create swap files for crash recovery (default: true)
    pub swapfile: bool,
    /// Create backup files before saving (default: false)
    pub backup: bool,
    /// Minimum number of lines to keep above and below cursor (default: 10)
    pub scrolloff: usize,
    /// Wrap long lines (default: true)
    pub wrap: bool,
    /// Horizontal scroll step size (default: 0 = jump to center cursor)
    pub sidescroll: usize,
    /// Minimum columns to keep left and right of cursor (default: 5)
    pub sidescrolloff: usize,
    /// Clipboard mode: "unnamedplus" (default), "unnamed", or "" (vim-compatible)
    /// When set, yank/delete/paste use the system clipboard by default
    pub clipboard: String,
    /// Whether `-` key auto-reveals current file in the file tree (default: true)
    pub file_tree_reveal: bool,
}

impl Default for EditorOptions {
    fn default() -> Self {
        Self {
            tab_width: 4,
            shift_width: 4,
            expand_tab: true,
            number: true,
            relative_number: false,
            scroll: None,
            textwidth: Some(150),
            ignorecase: false,
            smartcase: false,
            cursorline: false,
            showmatch: true,
            swapfile: true,
            backup: false,
            scrolloff: 10,
            wrap: true,
            sidescroll: 0,
            sidescrolloff: 5,
            clipboard: "unnamedplus".to_string(),
            file_tree_reveal: true,
        }
    }
}

use crate::buffer::Buffer;
use crate::lua::LuaContext;
use crate::mode::Mode;
use crate::syntax::ColorSchemeRegistry;
use anyhow::Result;
use std::collections::HashMap;
use tokio::sync::mpsc;

/// Calculates the display width of a string, accounting for tabs and wide characters.
fn display_width(text: &str, tab_width: usize) -> usize {
    crate::display::display_width(text, tab_width)
}

/// Commands sent from background tasks to the LSP manager via channel
#[derive(Debug)]
pub enum LspCommand {
    /// Start a language server
    StartServer {
        language: String,
        command: String,
        args: Vec<String>,
        root_path: std::path::PathBuf,
        response_tx: tokio::sync::oneshot::Sender<Result<()>>,
    },
    /// Send didOpen notification
    DidOpen {
        uri: lsp_types::Uri,
        language_id: String,
        version: i32,
        text: String,
        response_tx: tokio::sync::oneshot::Sender<Result<()>>,
    },
    /// Start notification listener
    StartNotificationListener { language_id: String },
}

/// Cached preview highlights: line_idx -> Vec<(range, highlight_group)>
pub type PreviewHighlights =
    HashMap<usize, Vec<(std::ops::Range<usize>, crate::syntax::HighlightGroup)>>;

/// The main editor state
pub struct Editor {
    /// List of open buffers
    pub buffers: Vec<Buffer>,
    /// Index of the currently active buffer
    current_buffer_index: usize,
    /// Window manager for split windows
    window_manager: Option<WindowManager>,
    /// Current editing mode
    mode: Mode,
    /// Whether the editor should quit
    should_quit: bool,
    /// Input context (counts, operators, pending commands, registers, input state machine)
    input: InputContext,
    /// Register manager for yank/delete operations
    registers: RegisterManager,
    /// Visual mode context (selection start, block insert state, last selection)
    visual: VisualContext,
    /// Command-line mode context (buffer, history, navigation)
    command: CommandContext,
    /// Search-related state
    pub search: SearchContext,
    /// Mark manager for buffer marks
    marks: MarkManager,
    /// Key mapping manager
    keymaps: KeyMapManager,
    /// Jump list for Ctrl-O and Ctrl-I
    jump_list: JumpList,
    /// Tag stack for Ctrl-T (LSP goto definition/implementation/type navigation)
    tag_stack: TagStack,
    /// Macro manager for recording and playback
    macro_manager: MacroManager,
    /// Last find motion (for ; and , repeat)
    /// (char, FindType::Find/Till, FindDirection::Forward/Backward)
    last_find: Option<(char, FindType, FindDirection)>,
    /// Picker state (picker, preview cache, layout, file list cache, etc.)
    pub(crate) picker_state: PickerState,
    /// LSP-related state
    lsp_state: LspState,
    /// Channel sender for LSP commands from background tasks
    lsp_command_tx: Option<mpsc::UnboundedSender<LspCommand>>,
    /// Channel receiver for LSP commands from background tasks
    lsp_command_rx: Option<mpsc::UnboundedReceiver<LspCommand>>,
    /// Lua context for configuration and plugins (optional)
    lua_context: Option<LuaContext>,
    /// Bridge for Lua-Editor communication (optional)
    editor_bridge: Option<crate::lua::EditorBridge>,
    /// Last insert position (line, col) for gi command
    last_insert_position: Option<(usize, usize)>,
    /// Completion menu popup (LSP)
    completion_menu: CompletionMenu,
    /// Path completion state for command-line mode
    path_completion: PathCompletionState,
    /// Color scheme registry
    color_scheme_registry: ColorSchemeRegistry,
    /// Editor options and settings
    pub options: EditorOptions,
    /// Viewport and scroll state
    pub(crate) viewport: ViewportState,
    /// Current color scheme name
    current_color_scheme: String,
    /// File tree explorer
    file_tree: FileTree,
    /// Quickfix list (global error/location list)
    quickfix_list: QuickfixList,
    /// Location list (per-window error/location list)
    location_list: LocationList,
    /// Whether quickfix window is open
    quickfix_window_open: bool,
    /// Whether location list window is open
    location_window_open: bool,
    /// Substitute confirmation state: matches to confirm (line, start_col, end_col, replacement)
    substitute_matches: Vec<(usize, usize, usize, String)>,
    /// Current match index for substitute confirmation
    substitute_match_index: usize,
    /// Regex pattern for substitute confirmation (for highlighting)
    substitute_pattern: Option<regex::Regex>,
    /// Tab page manager
    tab_page_manager: TabPageManager,
    /// Performance metrics
    metrics: PerformanceMetrics,
    /// Dashboard menu selected index (0-5)
    dashboard_selected: usize,
    /// Pending semantic change operation (for ci", cw, etc.)
    /// When Some, insert mode exit will create a semantic change instead of composite
    pending_semantic_change: Option<PendingSemanticChange>,
    /// Replace mode tracking for dot-repeat
    replace_mode_state: Option<ReplaceModeState>,
    /// Mouse interaction state (dragging, drag origin)
    pub(crate) mouse_state: MouseState,
    /// Cached buffer area from last render (for screen-to-buffer coordinate conversion)
    pub(crate) last_buffer_area: Option<ratatui::layout::Rect>,
    /// Cached gutter width from last render
    pub(crate) last_gutter_width: usize,
    /// Dashboard cat animation state
    cat_animation: Option<crate::ui::CatAnimation>,
    /// LSP Manager panel state
    lsp_manager_panel: Option<LspManagerPanel>,
    /// Channel for receiving LSP install progress updates
    install_progress_rx: Option<tokio::sync::mpsc::UnboundedReceiver<lsp_manager_panel::InstallProgress>>,
    /// Channel sender for LSP install progress (cloned into background tasks)
    install_progress_tx: Option<tokio::sync::mpsc::UnboundedSender<lsp_manager_panel::InstallProgress>>,
    /// Pending install requests to be picked up by the event loop
    pending_installs: Vec<lsp_manager_panel::PendingInstallRequest>,
    /// Rename input buffer (for LSP rename mode)
    rename_buffer: String,
    /// Cursor position within the rename input buffer
    rename_cursor: usize,
    /// Whether the diagnostic badge overlay has been dismissed (double-Escape)
    diagnostic_badge_dismissed: bool,
    /// Last diagnostic count when badge state was set (for detecting changes)
    diagnostic_badge_last_count: (usize, usize),
    /// Last time Escape was pressed in normal mode (for double-Escape detection)
    last_escape_time: Option<std::time::Instant>,
}

/// Cached picker layout rects for mouse hit-testing
#[derive(Debug, Clone)]
pub struct PickerLayout {
    /// Search input area
    pub query_field: ratatui::layout::Rect,
    /// File filter area (LiveGrep only)
    pub filter_field: Option<ratatui::layout::Rect>,
    /// Results list area
    pub results_area: ratatui::layout::Rect,
    /// Scroll offset of results (for mapping row to result index)
    pub results_scroll_offset: usize,
}

/// Tracks mouse interaction state for click and drag
#[derive(Debug, Clone, Default)]
pub struct MouseState {
    /// Whether a drag is in progress
    pub is_dragging: bool,
    /// Buffer position where the drag started (line, col)
    pub drag_origin: Option<(usize, usize)>,
}

/// State for tracking Replace mode for dot-repeat
#[derive(Clone, Debug)]
pub struct ReplaceModeState {
    /// Cursor position when R was pressed
    pub start_position: (usize, usize),
    /// Characters typed during replace mode
    pub replacements: String,
    /// Original text that was overwritten
    pub old_text: String,
}

/// Tracks a pending semantic change operation
#[derive(Clone, Debug)]
pub struct PendingSemanticChange {
    /// The type of text object being changed
    pub object_type: Option<TextObjectType>,
    /// True if this is a word change (cw)
    pub is_word_change: bool,
    /// True if this is a search match change (cgn)
    pub is_search_match_change: bool,
    /// Search pattern for cgn repeat
    pub search_pattern: Option<String>,
    /// Search direction for cgn repeat
    pub search_forward: Option<bool>,
    /// The original text that was deleted
    pub old_text: String,
    /// The original range of the deletion
    pub old_range: Range,
    /// Cursor position before the change
    pub cursor_before: Position,
}

/// Cached preview data for the picker
#[derive(Clone)]
pub struct PreviewCache {
    /// File content
    pub content: String,
    /// Cached syntax-highlighted lines (line_idx -> highlights)
    /// Uses RefCell for interior mutability so we can cache highlights even with immutable reference
    pub highlighted_lines: std::cell::RefCell<PreviewHighlights>,
    /// Detected language (if any)
    pub language: Option<crate::syntax::Language>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FindType {
    Find, // f/F - cursor lands on character
    Till, // t/T - cursor lands before/after character
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FindDirection {
    Forward,
    Backward,
}

impl Editor {
    /// Creates a new editor with an empty buffer
    /// Starts in Dashboard mode when no file is opened
    pub fn new() -> Self {
        let buffer = Buffer::new();

        Self {
            buffers: vec![buffer],
            current_buffer_index: 0,
            window_manager: None, // Will be initialized when viewport size is known
            mode: Mode::Dashboard,
            should_quit: false,
            input: InputContext::new(),
            registers: RegisterManager::new(),
            visual: VisualContext::new(),
            command: CommandContext::new(),
            search: SearchContext::new(),
            marks: MarkManager::new(),
            keymaps: KeyMapManager::new(),
            jump_list: JumpList::new(),
            tag_stack: TagStack::new(),
            macro_manager: MacroManager::new(),
            last_find: None,
            picker_state: PickerState::new(),
            lsp_state: LspState::new(),
            lsp_command_tx: None,
            lsp_command_rx: None,
            lua_context: None,
            editor_bridge: None,
            last_insert_position: None,
            completion_menu: CompletionMenu::new(),
            path_completion: PathCompletionState::new(),
            color_scheme_registry: ColorSchemeRegistry::new(),
            current_color_scheme: "tokyonight".to_string(),
            options: EditorOptions::default(),
            viewport: ViewportState::default(),
            file_tree: FileTree::new(),
            quickfix_list: QuickfixList::new(),
            location_list: LocationList::new(),
            quickfix_window_open: false,
            location_window_open: false,
            substitute_matches: Vec::new(),
            substitute_match_index: 0,
            substitute_pattern: None,
            tab_page_manager: TabPageManager::new(),
            metrics: PerformanceMetrics::new(),
            dashboard_selected: 0,
            pending_semantic_change: None,
            replace_mode_state: None,
            mouse_state: MouseState::default(),
            last_buffer_area: None,
            last_gutter_width: 0,
            cat_animation: Some(crate::ui::CatAnimation::new()),
            lsp_manager_panel: None,
            install_progress_rx: None,
            install_progress_tx: None,
            pending_installs: Vec::new(),
            rename_buffer: String::new(),
            rename_cursor: 0,
            diagnostic_badge_dismissed: false,
            diagnostic_badge_last_count: (0, 0),
            last_escape_time: None,
        }
    }

    /// Creates an editor with initial content
    pub fn with_content(content: &str) -> Self {
        let buffer = Buffer::new_from_str(content);

        Self {
            buffers: vec![buffer],
            current_buffer_index: 0,
            window_manager: None, // Will be initialized when viewport size is known
            mode: Mode::default(),
            should_quit: false,
            input: InputContext::new(),
            registers: RegisterManager::new(),
            visual: VisualContext::new(),
            command: CommandContext::new(),
            search: SearchContext::new(),
            marks: MarkManager::new(),
            keymaps: KeyMapManager::new(),
            jump_list: JumpList::new(),
            tag_stack: TagStack::new(),
            macro_manager: MacroManager::new(),
            last_find: None,
            picker_state: PickerState::new(),
            lsp_state: LspState::new(),
            lsp_command_tx: None,
            lsp_command_rx: None,
            lua_context: None,
            editor_bridge: None,
            last_insert_position: None,
            completion_menu: CompletionMenu::new(),
            path_completion: PathCompletionState::new(),
            color_scheme_registry: ColorSchemeRegistry::new(),
            current_color_scheme: "tokyonight".to_string(),
            options: EditorOptions::default(),
            viewport: ViewportState::default(),
            file_tree: FileTree::new(),
            quickfix_list: QuickfixList::new(),
            location_list: LocationList::new(),
            quickfix_window_open: false,
            location_window_open: false,
            substitute_matches: Vec::new(),
            substitute_match_index: 0,
            substitute_pattern: None,
            tab_page_manager: TabPageManager::new(),
            metrics: PerformanceMetrics::new(),
            dashboard_selected: 0,
            pending_semantic_change: None,
            replace_mode_state: None,
            mouse_state: MouseState::default(),
            last_buffer_area: None,
            last_gutter_width: 0,
            cat_animation: Some(crate::ui::CatAnimation::new()),
            lsp_manager_panel: None,
            install_progress_rx: None,
            install_progress_tx: None,
            pending_installs: Vec::new(),
            rename_buffer: String::new(),
            rename_cursor: 0,
            diagnostic_badge_dismissed: false,
            diagnostic_badge_last_count: (0, 0),
            last_escape_time: None,
        }
    }

    /// Renders the editor to an in-memory buffer and returns ANSI output
    /// Used for headless mode to get pixel-perfect terminal representation
    pub fn render_to_ansi(&mut self, width: u16, height: u16) -> Result<String> {
        use crate::ui::buffer_to_ansi;
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        // Create a test backend with specified dimensions
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend)?;

        // Render using the normal UI rendering code
        terminal.draw(|f| {
            crate::ui::Renderer::render_to_frame(f, self);
        })?;

        // Convert buffer to ANSI string
        let buffer = terminal.backend().buffer();
        Ok(buffer_to_ansi(buffer))
    }

    // ==================== Rename Input ====================

    pub fn rename_buffer(&self) -> &str {
        &self.rename_buffer
    }

    pub fn rename_cursor(&self) -> usize {
        self.rename_cursor
    }

    pub fn set_rename_buffer(&mut self, s: String) {
        self.rename_buffer = s;
    }

    pub fn set_rename_cursor(&mut self, pos: usize) {
        self.rename_cursor = pos;
    }

    // ==================== Core Editor Methods ====================

    pub fn start_macro_recording(&mut self, register: char) -> bool {
        self.macro_manager.start_recording(register)
    }

    /// Stops macro recording
    pub fn stop_macro_recording(&mut self) {
        self.macro_manager.stop_recording();
    }

    /// Records a key event in the current macro
    pub fn record_macro_event(&mut self, event: crossterm::event::KeyEvent) {
        self.macro_manager.record_event(event);
    }

    /// Returns whether currently recording a macro
    pub fn is_recording_macro(&self) -> bool {
        self.macro_manager.is_recording()
    }

    /// Gets the register being recorded
    pub fn recording_register(&self) -> Option<char> {
        self.macro_manager.recording_register()
    }

    /// Gets a macro by register for playback
    pub fn get_macro(&self, register: char) -> Option<&Vec<crossterm::event::KeyEvent>> {
        self.macro_manager.get_macro(register)
    }

    /// Gets the current mode
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Sets the mode
    pub fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
        // Clear count and pending operator when changing modes
        self.input.count = None;
        self.input.pending_operator = None;
        self.input.pending_command = None;
        self.input.pending_register = None;

        // Clear visual selection when leaving visual modes
        if !matches!(mode, Mode::Visual | Mode::VisualLine | Mode::VisualBlock) {
            self.visual.visual_start = None;
        }
    }

    /// Gets the dashboard selected menu index
    pub fn dashboard_selected(&self) -> usize {
        self.dashboard_selected
    }

    /// Sets the dashboard selected menu index
    pub fn set_dashboard_selected(&mut self, index: usize) {
        self.dashboard_selected = index;
    }

    /// Returns true if the dashboard should be shown
    /// Dashboard is shown when: no file loaded AND buffer is empty/default
    pub fn should_show_dashboard(&self) -> bool {
        self.mode == Mode::Dashboard
    }

    /// Returns a mutable reference to the cat animation (if active).
    pub fn cat_animation_mut(&mut self) -> Option<&mut crate::ui::CatAnimation> {
        self.cat_animation.as_mut()
    }

    /// Startle the cat (e.g. on terminal resize while it's on the logo).
    pub fn startle_cat(&mut self) {
        if let Some(ref mut anim) = self.cat_animation {
            anim.startle();
        }
    }

    /// Tick the cat animation. Returns true if a frame advanced (needs redraw).
    pub fn tick_cat_animation(&mut self) -> bool {
        if let Some(ref mut anim) = self.cat_animation {
            if anim.is_active() {
                return anim.tick();
            }
            // Animation finished — drop it
            self.cat_animation = None;
        }
        false
    }

    /// Gets the pending command
    pub fn pending_command(&self) -> Option<char> {
        self.input.pending_command
    }

    /// Sets the pending command
    pub fn set_pending_command(&mut self, cmd: char) {
        self.input.pending_command = Some(cmd);
    }

    /// Gets the current input state (new state machine)
    pub fn input_state(&self) -> &InputState {
        &self.input.input_state
    }

    /// Sets the input state (new state machine)
    pub fn set_input_state(&mut self, state: InputState) {
        self.input.input_state = state;
    }

    /// Resets input state to Normal
    pub fn reset_input_state(&mut self) {
        self.input.input_state = InputState::Normal;
    }

    /// Sets the viewport height (called from UI layer)
    pub fn set_viewport_height(&mut self, height: usize) {
        self.viewport.viewport_height = height;
    }

    /// Caches the buffer layout from the last render (for mouse coordinate conversion)
    pub fn set_last_layout(&mut self, buffer_area: ratatui::layout::Rect, gutter_width: usize) {
        self.last_buffer_area = Some(buffer_area);
        self.last_gutter_width = gutter_width;
    }

    /// Gets the viewport height
    pub fn viewport_height(&self) -> usize {
        self.viewport.viewport_height
    }

    /// Gets the scroll offset (top visible line)
    pub fn scroll_offset(&self) -> usize {
        // If we have a window manager, use the focused window's scroll offset
        // This allows viewport commands (zz, zt, zb) to control scrolling
        if let Some(wm) = &self.window_manager {
            if let Some(window) = wm.focused_window() {
                return window.scroll_offset();
            }
        }
        // Fall back to editor-level scroll offset for headless/test mode
        self.viewport.scroll_offset
    }

    /// Gets a reference to the wrap map (if available)
    pub fn wrap_map(&self) -> Option<&WrapMap> {
        self.viewport.wrap_map.as_ref()
    }

    /// Ensures the wrap map is built and up-to-date for the current buffer.
    /// Called from the rendering layer before drawing wrapped lines.
    pub fn ensure_wrap_map(&mut self, text_width: usize) {
        if !self.options.wrap {
            self.viewport.wrap_map = None;
            return;
        }
        let width = text_width.max(1);
        let tab_width = self.options.tab_width;
        let line_count = self.buffer().line_count();
        let buf_version = self.buffer().version();

        // Check if existing map is up to date or can be incrementally updated
        let needs_action = if let Some(ref map) = self.viewport.wrap_map {
            if map.buffer_version() == buf_version && map.wrap_width() == width && map.line_count() == line_count {
                return; // Already up to date
            }
            if map.wrap_width() == width && map.line_count() == line_count {
                // Only buffer content changed — can use incremental update
                Some(true) // incremental
            } else {
                Some(false) // full rebuild
            }
        } else {
            None // no map yet
        };

        // Extract data needed for closures before mutably borrowing self.viewport.wrap_map
        let cursor_line = self.buffer().cursor().line();
        let rope = self.buffer().rope().clone();
        let make_line_len = |line_idx: usize| -> usize {
            if line_idx < rope.len_lines() {
                let line = rope.line(line_idx);
                let text = line.to_string();
                let text = text.trim_end_matches('\n');
                display_width(text, tab_width)
            } else {
                0
            }
        };

        match needs_action {
            Some(true) => {
                // Incremental: only invalidate cursor line
                let map = self.viewport.wrap_map.as_mut().unwrap();
                map.invalidate_line(cursor_line, make_line_len);
                map.set_buffer_version(buf_version);
            }
            Some(false) => {
                // Full rebuild (width or line count changed)
                let map = self.viewport.wrap_map.as_mut().unwrap();
                map.rebuild(line_count, width, tab_width, buf_version, make_line_len);
            }
            None => {
                // Build from scratch
                let map = WrapMap::new(line_count, width, tab_width, buf_version, make_line_len);
                self.viewport.wrap_map = Some(map);
            }
        }
    }

    /// Gets the horizontal scroll offset (leftmost visible column)
    pub fn horizontal_offset(&self) -> usize {
        if let Some(wm) = &self.window_manager {
            if let Some(window) = wm.focused_window() {
                return window.horizontal_offset();
            }
        }
        // Fall back to 0 for headless/test mode
        0
    }

    /// Updates scroll offset to keep cursor visible
    ///
    /// Uses scrolloff for comfortable cursor positioning during normal movements.
    /// Viewport commands (zt, zz, zb) can override this by setting skip_scroll_update.
    pub fn update_scroll_offset(&mut self) {
        // Skip if viewport command just ran - it has full control over positioning
        if self.viewport.skip_scroll_update {
            return;
        }

        let cursor_line = self.buffer().cursor().line();
        let visible_lines = self.viewport.viewport_height;
        let current_offset = self.scroll_offset();
        let scrolloff = self.options.scrolloff;

        // Calculate new scroll offset
        let new_offset;

        // Only use wrap-aware scrolling if the wrap map covers the current buffer.
        // After edits (e.g. `o` inserting a line) the map is stale until the next
        // render pass rebuilds it.  Using stale data causes cursor_to_visual to
        // return 0 for the new line, jumping the viewport to the top.
        let wrap_map_usable = self.options.wrap
            && self
                .viewport.wrap_map
                .as_ref()
                .is_some_and(|m| m.line_count() >= self.buffer().rope().len_lines());

        if wrap_map_usable {
            if let Some(ref wrap_map) = self.viewport.wrap_map {
                // Wrap-aware scrolling: work in visual rows
                let cursor_col = self.buffer().cursor().col();
                // Get the display column for proper sub-line calculation
                let line_text = if cursor_line < self.buffer().line_count() {
                    let text = self.buffer().rope().line(cursor_line).to_string();
                    text.trim_end_matches('\n').to_string()
                } else {
                    String::new()
                };
                let disp_col = crate::display::char_col_to_display_col(&line_text, cursor_col, self.options.tab_width);
                let (cursor_visual_row, _) = wrap_map.cursor_to_visual(cursor_line, disp_col);
                let viewport_visual_start = wrap_map.logical_to_visual(current_offset);

                if cursor_visual_row < viewport_visual_start + scrolloff {
                    // Cursor above viewport top margin — scroll up
                    // Find the logical line whose visual start puts cursor at scrolloff from top
                    let target_visual = cursor_visual_row.saturating_sub(scrolloff);
                    let (new_line, _) = wrap_map.visual_to_logical(target_visual);
                    new_offset = new_line;
                } else if cursor_visual_row + scrolloff >= viewport_visual_start + visible_lines {
                    // Cursor below viewport bottom margin — scroll down
                    let target_visual = if visible_lines > scrolloff + 1 {
                        cursor_visual_row.saturating_sub(visible_lines - scrolloff - 1)
                    } else {
                        cursor_visual_row.saturating_sub(visible_lines / 2)
                    };
                    let (new_line, _) = wrap_map.visual_to_logical(target_visual);
                    new_offset = new_line;
                } else {
                    new_offset = current_offset;
                }
            } else {
                // Wrap enabled but no wrap map yet — use logical line fallback
                new_offset = Self::compute_logical_scroll_offset(
                    cursor_line, current_offset, visible_lines, scrolloff,
                );
            }
        } else {
            new_offset = Self::compute_logical_scroll_offset(
                cursor_line, current_offset, visible_lines, scrolloff,
            );
        };

        // Clamp scroll offset to buffer bounds.
        // Use rope's raw line count (includes trailing empty line after final \n)
        // because the cursor can be on that line even though line_count() excludes it.
        let raw_last_line = self.buffer().rope().len_lines().saturating_sub(1);
        let new_offset = new_offset.min(raw_last_line);

        // Update both editor-level and window-level scroll offsets
        self.viewport.scroll_offset = new_offset;

        // Extract cursor column and options before mutably borrowing window_manager
        let cursor_col = self.buffer().cursor().col();
        let wrap = self.options.wrap;
        let sidescroll = self.options.sidescroll;
        let sidescrolloff = self.options.sidescrolloff;

        if let Some(wm) = &mut self.window_manager {
            if let Some(window) = wm.focused_window_mut() {
                window.set_scroll_offset(new_offset);

                // Update horizontal scroll offset to keep cursor visible horizontally
                if window.ensure_cursor_visible_horizontal(
                    cursor_col,
                    wrap,
                    sidescroll,
                    sidescrolloff,
                ) {
                    // Horizontal offset changed, mark for re-render
                    self.mark_dirty();
                }
            }
        }
    }

    /// Computes scroll offset using logical line counting (non-wrap path)
    fn compute_logical_scroll_offset(
        cursor_line: usize,
        current_offset: usize,
        visible_lines: usize,
        scrolloff: usize,
    ) -> usize {
        if cursor_line < current_offset + scrolloff {
            cursor_line.saturating_sub(scrolloff)
        } else if cursor_line + scrolloff >= current_offset + visible_lines {
            if visible_lines > scrolloff + 1 {
                cursor_line.saturating_sub(visible_lines - scrolloff - 1)
            } else {
                cursor_line.saturating_sub(visible_lines / 2)
            }
        } else {
            current_offset
        }
    }

    /// Calculates half-page scroll amount
    /// Uses options.scroll if set, otherwise viewport_height / 2
    pub fn half_page_scroll(&self) -> usize {
        self.options
            .scroll
            .unwrap_or(self.viewport.viewport_height / 2)
    }

    /// Clears the pending command
    pub fn clear_pending_command(&mut self) {
        self.input.pending_command = None;
    }

    /// Returns whether the editor should quit
    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    /// Sets the quit flag
    pub fn quit(&mut self) {
        self.should_quit = true;
    }

    /// Gets the current count
    pub fn count(&self) -> Option<usize> {
        self.input.count
    }

    /// Sets the count
    pub fn set_count(&mut self, count: usize) {
        self.input.count = Some(count);
    }

    /// Appends a digit to the count
    pub fn append_count(&mut self, digit: usize) {
        let current = self.input.count.unwrap_or(0);
        self.input.count = Some(current * 10 + digit);
    }

    /// Clears the count
    pub fn clear_count(&mut self) {
        self.input.count = None;
    }

    /// Gets the effective count (count or 1)
    pub fn effective_count(&self) -> usize {
        self.input.count.unwrap_or(1)
    }

    /// Gets the pending operator
    pub fn pending_operator(&self) -> Option<Operator> {
        self.input.pending_operator
    }

    /// Sets the pending operator
    pub fn set_pending_operator(&mut self, op: Operator) {
        self.input.pending_operator = Some(op);
    }

    /// Clears the pending operator
    pub fn clear_pending_operator(&mut self) {
        self.input.pending_operator = None;
    }

    /// Gets a reference to the registers
    pub fn registers(&self) -> &RegisterManager {
        &self.registers
    }

    /// Gets a mutable reference to the registers
    pub fn registers_mut(&mut self) -> &mut RegisterManager {
        &mut self.registers
    }

    /// Gets a reference to the keymaps
    pub fn keymaps(&self) -> &KeyMapManager {
        &self.keymaps
    }

    /// Gets a mutable reference to the keymaps
    pub fn keymaps_mut(&mut self) -> &mut KeyMapManager {
        &mut self.keymaps
    }

    /// Gets the pending register for next operation
    pub fn pending_register(&self) -> Option<char> {
        self.input.pending_register
    }

    /// Sets the pending register for next operation
    pub fn set_pending_register(&mut self, reg: char) {
        self.input.pending_register = Some(reg);
    }

    /// Clears the pending register
    pub fn clear_pending_register(&mut self) {
        self.input.pending_register = None;
    }

    /// Loads a file into the editor (async version)
    /// If the file is already open in a buffer, switches to that buffer
    /// Otherwise, adds it as a new buffer
    pub async fn load_file_async<P: AsRef<std::path::Path>>(&mut self, path: P) -> Result<()> {
        let path_str = path.as_ref().to_string_lossy().to_string();

        // Check if file is already open in a buffer
        for (i, buf) in self.buffers.iter().enumerate() {
            if buf.file_path() == Some(&path_str) {
                // File already open - just switch to it
                // Save current file to alternate file register
                if let Some(current_path) = self.buffer().file_path() {
                    self.registers.set_alternate_file(current_path.to_string());
                }
                self.current_buffer_index = i;
                // Update current file register
                self.registers.set_current_file(path_str);
                // Sync tab's buffer index to match the existing buffer
                self.sync_current_tab_buffer_index();
                // Still need to initialize LSP for this file if it hasn't been yet
                self.lsp_state.needs_lsp_init = true;
                return Ok(());
            }
        }

        // Store old file path before loading new file
        let old_file_path = self.buffer().file_path().map(|s| s.to_string());

        // Save current file to alternate file register
        if let Some(current_path) = old_file_path.as_ref() {
            self.registers.set_alternate_file(current_path.to_string());
        }

        // Load new buffer
        let new_buffer = Buffer::load_file_async(path).await?;

        // Parse and apply modeline options from the loaded file
        let content = new_buffer.rope().to_string();
        if let Some(modeline) = crate::modeline::Modeline::parse(&content) {
            self.apply_modeline(&modeline);
        }

        self.add_buffer(new_buffer);

        // Update current file register
        self.registers.set_current_file(path_str);

        // Update tab title to match the loaded file
        self.update_current_tab_title();

        // Sync tab's buffer index to match the newly loaded buffer
        self.sync_current_tab_buffer_index();

        // Mark that we need to send didClose for the old file
        if old_file_path.is_some() {
            self.lsp_state.pending_did_close_file = old_file_path;
        }

        Ok(())
    }

    /// Loads a file into the editor (blocking wrapper around load_file_async)
    pub fn load_file<P: AsRef<std::path::Path>>(&mut self, path: P) -> Result<()> {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.load_file_async(path))
        })
    }

    /// Process pending syntax re-highlighting (CPU-intensive, runs in background)
    /// This now uses incremental parsing - the parse tree was already updated incrementally
    /// via InputEdit when the buffer was modified, so we just need to query it for highlights.
    pub async fn process_pending_rehighlight(&mut self) {
        // Check if buffer needs re-highlighting
        if !self.buffer().needs_rehighlight() {
            return;
        }

        // Rebuild highlight cache from the incrementally-updated parse tree
        // This is FAST because tree-sitter already updated the tree via InputEdit.
        // We're just querying the tree for highlights, not re-parsing!
        let _ = self.buffer_mut().rebuild_highlight_cache();

        // Fix Bug 2: Mark dirty after highlighting update so the UI re-renders
        // Without this, highlighting updates after debounce but screen doesn't refresh
        self.mark_dirty();
    }

    /// Immediate viewport-only syntax rehighlight.
    /// Queries tree-sitter for just the visible lines and updates the cache.
    /// This is called immediately after input so highlights are accurate without waiting for the debounce.
    pub fn process_viewport_rehighlight(&mut self) {
        if !self.buffer().needs_rehighlight() {
            return;
        }

        let start_line = self.scroll_offset();
        let end_line = start_line + self.viewport_height();

        self.buffer_mut()
            .rebuild_viewport_highlight_cache(start_line, end_line);

        self.mark_dirty();
    }

    /// Apply modeline options to editor settings
    fn apply_modeline(&mut self, modeline: &crate::modeline::Modeline) {
        // Indentation options
        if let Some(ts) = modeline.get_int("tabstop", "ts") {
            self.options.tab_width = ts;
        }
        if let Some(sw) = modeline.get_int("shiftwidth", "sw") {
            self.options.shift_width = sw;
        }
        if let Some(et) = modeline.get_bool("expandtab", "et") {
            self.options.expand_tab = et;
        }

        // Display options
        if let Some(tw) = modeline.get_int("textwidth", "tw") {
            self.options.textwidth = Some(tw);
        }
        if let Some(nu) = modeline.get_bool("number", "nu") {
            self.options.number = nu;
        }
        if let Some(rnu) = modeline.get_bool("relativenumber", "rnu") {
            self.options.relative_number = rnu;
        }
        if let Some(cul) = modeline.get_bool("cursorline", "cul") {
            self.options.cursorline = cul;
        }

        // Search options
        if let Some(ic) = modeline.get_bool("ignorecase", "ic") {
            self.options.ignorecase = ic;
        }
        if let Some(scs) = modeline.get_bool("smartcase", "scs") {
            self.options.smartcase = scs;
        }

        // Other options
        if let Some(sm) = modeline.get_bool("showmatch", "sm") {
            self.options.showmatch = sm;
        }
    }

    /// Yanks text to the appropriate register (pending_register or default)
    pub fn yank_to_register(&mut self, text: String) {
        self.yank_to_register_with_type(text, RegisterType::Character);
    }

    /// Yanks text to the appropriate register with explicit type
    pub fn yank_to_register_with_type(&mut self, text: String, reg_type: RegisterType) {
        if let Some(reg) = self.input.pending_register {
            self.input.pending_register = None;
            match reg {
                '_' => return, // black hole register: discard
                '+' | '*' => {
                    self.registers.set_clipboard(text.clone());
                    self.registers.set_with_type(Some(reg), text, reg_type);
                    return;
                }
                _ => {
                    self.registers.set_with_type(Some(reg), text.clone(), reg_type);
                }
            }
        } else {
            self.registers.yank_with_type(text.clone(), reg_type);
        }
        // Sync to system clipboard when clipboard option is set and no explicit register was used
        if !self.options.clipboard.is_empty() {
            self.registers.set_clipboard(text);
        }
    }

    /// Deletes text and stores in the appropriate register (pending_register or default)
    pub fn delete_to_register(&mut self, text: String) {
        self.delete_to_register_with_type(text, RegisterType::Character);
    }

    /// Deletes text and stores in the appropriate register with explicit type
    pub fn delete_to_register_with_type(&mut self, text: String, reg_type: RegisterType) {
        if let Some(reg) = self.input.pending_register {
            self.input.pending_register = None;
            match reg {
                '_' => return, // black hole register: discard
                '+' | '*' => {
                    self.registers.set_clipboard(text.clone());
                    self.registers.set_with_type(Some(reg), text, reg_type);
                    return;
                }
                _ => {
                    self.registers.set_with_type(Some(reg), text.clone(), reg_type);
                }
            }
        } else {
            self.registers.delete_with_type(text.clone(), reg_type);
        }
        // Sync to system clipboard when clipboard option is set and no explicit register was used
        if !self.options.clipboard.is_empty() {
            self.registers.set_clipboard(text);
        }
    }

    /// Gets text from the appropriate register (pending_register or default)
    pub fn get_from_register(&mut self) -> String {
        let text = if let Some(reg) = self.input.pending_register {
            match reg {
                '_' => String::new(), // black hole register: always empty
                '+' | '*' => self.registers.get_clipboard(),
                _ => self.registers.get(Some(reg)),
            }
        } else if !self.options.clipboard.is_empty() {
            // When clipboard option is set, read from system clipboard
            self.registers.get_clipboard()
        } else {
            self.registers.get_default().to_string()
        };
        self.input.pending_register = None;
        text
    }

    /// Gets text and type from the appropriate register (pending_register or default)
    pub fn get_from_register_with_type(&mut self) -> (String, RegisterType) {
        let (text, reg_type) = if let Some(reg) = self.input.pending_register {
            match reg {
                '_' => (String::new(), RegisterType::Character), // black hole: always empty
                '+' | '*' => {
                    let clipboard_text = self.registers.get_clipboard();
                    (clipboard_text, RegisterType::Character)
                }
                _ => self.registers.get_with_type(Some(reg)),
            }
        } else if !self.options.clipboard.is_empty() {
            // When clipboard option is set, read from system clipboard
            // Use Character type since system clipboard doesn't carry type info
            let clipboard_text = self.registers.get_clipboard();
            // Check if the unnamed register has the same text - if so, use its type
            let (default_text, default_type) = self.registers.get_default_with_type();
            if default_text == clipboard_text {
                (clipboard_text, default_type)
            } else {
                // Clipboard has different content (from external paste), treat as character
                (clipboard_text, RegisterType::Character)
            }
        } else {
            let (t, rt) = self.registers.get_default_with_type();
            (t.to_string(), rt)
        };
        self.input.pending_register = None;
        (text, reg_type)
    }

    /// Handles a bracketed paste event (cmd-v / ctrl-shift-v in terminal)
    pub fn handle_paste_event(&mut self, text: &str) -> Result<()> {
        if text.is_empty() {
            return Ok(());
        }

        match self.mode() {
            Mode::Insert => {
                // Insert pasted text at cursor position as a single change
                let cursor = self.buffer().cursor();
                let cursor_before = (cursor.line(), cursor.col());
                let position = (cursor.line(), cursor.col());
                let change = Change::insert(position, text.to_string(), cursor_before);
                change.apply(self.buffer_mut());
                self.add_change(change);
            }
            Mode::Normal => {
                // Set unnamed register and paste after cursor
                self.registers.set(None, text.to_string());
                let cursor = self.buffer().cursor();
                let cursor_before = (cursor.line(), cursor.col());
                let position = (cursor.line(), cursor.col() + 1);
                let change = Change::insert(position, text.to_string(), cursor_before);
                change.apply(self.buffer_mut());
                self.add_change(change);
            }
            Mode::Command => {
                // Insert text into command buffer
                self.command.command_line.push_str(text);
            }
            Mode::Search => {
                // Insert text into search buffer
                self.search.search_buffer.push_str(text);
            }
            Mode::Picker => {
                if let Some(picker) = self.picker_mut() {
                    picker.insert_text(text);
                }
                self.mark_picker_query_changed();
            }
            _ => {
                // Visual modes: treat like normal mode paste
                self.registers.set(None, text.to_string());
            }
        }
        Ok(())
    }

    /// Starts building a composite change (e.g., when entering insert mode)
    pub fn start_change_building(&mut self, cursor_before: Position) {
        self.buffer_mut()
            .change_manager_mut()
            .start_building(cursor_before);
    }

    /// Sets how insert mode was entered on the current change builder (for dot repeat).
    pub fn set_change_entry_mode(&mut self, mode: InsertEntryMode) {
        self.buffer_mut()
            .change_manager_mut()
            .set_entry_mode(mode);
    }

    /// Adds a change to the change manager
    pub fn add_change(&mut self, change: Change) {
        self.buffer_mut().change_manager_mut().add_change(change);
    }

    /// Finalizes the current composite change
    pub fn finalize_change_building(&mut self) {
        let cursor_pos = (self.buffer().cursor().line(), self.buffer().cursor().col());
        self.buffer_mut()
            .change_manager_mut()
            .finalize_building_at(cursor_pos);
    }

    /// Sets a pending semantic change operation
    pub fn set_pending_semantic_change(&mut self, pending: PendingSemanticChange) {
        self.pending_semantic_change = Some(pending);
    }

    /// Takes and clears the pending semantic change operation
    pub fn take_pending_semantic_change(&mut self) -> Option<PendingSemanticChange> {
        self.pending_semantic_change.take()
    }

    /// Gets the leader key (default: space)
    pub fn leader_key(&self) -> char {
        self.input.leader_key
    }

    /// Sets the leader key
    pub fn set_leader_key(&mut self, key: char) {
        self.input.leader_key = key;
    }

    /// Gets cached diagnostic count (sync, suitable for UI rendering)
    pub fn cached_diagnostic_count(&self) -> (usize, usize, usize, usize) {
        self.lsp_state.diagnostic_count
    }

    /// Whether the diagnostic badge has been dismissed by double-Escape
    pub fn diagnostic_badge_dismissed(&self) -> bool {
        self.diagnostic_badge_dismissed
    }

    /// Dismiss the diagnostic badge (called on double-Escape)
    pub fn dismiss_diagnostic_badge(&mut self) {
        self.diagnostic_badge_dismissed = true;
    }

    /// Called when diagnostic counts change to potentially un-dismiss the badge
    pub fn on_diagnostic_counts_changed(&mut self, errors: usize, warnings: usize) {
        let new_count = (errors, warnings);
        if new_count != self.diagnostic_badge_last_count {
            self.diagnostic_badge_last_count = new_count;
            self.diagnostic_badge_dismissed = false;
        }
    }

    /// Get last escape time for double-Escape detection
    pub fn last_escape_time(&self) -> Option<std::time::Instant> {
        self.last_escape_time
    }

    /// Set last escape time
    pub fn set_last_escape_time(&mut self, time: std::time::Instant) {
        self.last_escape_time = Some(time);
    }

    /// Clear last escape time
    pub fn clear_last_escape_time(&mut self) {
        self.last_escape_time = None;
    }

    /// Gets a reference to the last change
    pub fn last_change(&self) -> Option<&Change> {
        self.buffer().change_manager().last_change()
    }

    /// Jump to next diagnostic (]d)
    pub fn goto_next_diagnostic(&mut self) {
        let current_line = self.buffer().cursor().line();
        let diagnostics = &self.lsp_state.current_file_diagnostics;

        // Find first diagnostic after current position
        let next = diagnostics
            .iter()
            .map(|d| d.range.start.line as usize)
            .filter(|&line| line > current_line)
            .min();

        if let Some(line) = next {
            self.buffer_mut().cursor_mut().set_position(line, 0);
        } else {
            // Wrap to first diagnostic
            if let Some(first) = diagnostics.first() {
                let line = first.range.start.line as usize;
                self.buffer_mut().cursor_mut().set_position(line, 0);
            }
        }
    }

    /// Jump to previous diagnostic ([d)
    pub fn goto_prev_diagnostic(&mut self) {
        let current_line = self.buffer().cursor().line();
        let diagnostics = &self.lsp_state.current_file_diagnostics;

        // Find last diagnostic before current position
        let prev = diagnostics
            .iter()
            .map(|d| d.range.start.line as usize)
            .filter(|&line| line < current_line)
            .max();

        if let Some(line) = prev {
            self.buffer_mut().cursor_mut().set_position(line, 0);
        } else {
            // Wrap to last diagnostic
            if let Some(last) = diagnostics.last() {
                let line = last.range.start.line as usize;
                self.buffer_mut().cursor_mut().set_position(line, 0);
            }
        }
    }

}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod size_tests {
    use super::*;
    use std::mem::size_of;
    use std::sync::{Arc, Mutex};

    /// This test measures the size of the Editor struct and its major components.
    /// This is critical for understanding stack overhead and determining if we need
    /// to box large fields or wrap the entire struct in Arc<Mutex<>>.
    ///
    /// Educational context:
    /// - Stack allocation is fast but limited (~2MB on most systems)
    /// - CPU cache lines are typically 64 bytes
    /// - Structs > 512 bytes should be considered for heap allocation
    /// - Structs > 2KB definitely should be boxed or arc'd
    ///
    /// In Rust, when you pass a value by ownership (not reference), it gets moved,
    /// which involves copying all the bytes. For large structs, this becomes expensive.
    #[test]
    fn measure_editor_size() {
        println!("\n=== Editor Struct Size Analysis ===\n");

        let editor_size = size_of::<Editor>();
        println!("Total Editor size: {} bytes ({:.2} KB)", editor_size, editor_size as f64 / 1024.0);

        // Measure major field types
        println!("\nMajor field sizes:");
        println!("  Vec<Buffer>:                {} bytes", size_of::<Vec<Buffer>>());
        println!("  Option<WindowManager>:      {} bytes", size_of::<Option<WindowManager>>());
        println!("  RegisterManager:            {} bytes", size_of::<RegisterManager>());
        println!("  MarkManager:                {} bytes", size_of::<MarkManager>());
        println!("  KeyMapManager:              {} bytes", size_of::<KeyMapManager>());
        println!("  JumpList:                   {} bytes", size_of::<JumpList>());
        println!("  TagStack:                   {} bytes", size_of::<TagStack>());
        println!("  MacroManager:               {} bytes", size_of::<MacroManager>());
        println!("  Option<Picker>:             {} bytes", size_of::<Option<Picker>>());
        println!("  InputState:                 {} bytes", size_of::<InputState>());
        println!("  LspState:                   {} bytes", size_of::<LspState>());
        println!("  CompletionMenu:             {} bytes", size_of::<CompletionMenu>());
        println!("  HashMap<String, PreviewCache>: {} bytes", size_of::<HashMap<String, PreviewCache>>());
        println!("  ColorSchemeRegistry:        {} bytes", size_of::<crate::syntax::ColorSchemeRegistry>());
        println!("  EditorOptions:              {} bytes", size_of::<EditorOptions>());
        println!("  FileTree:                   {} bytes", size_of::<FileTree>());
        println!("  QuickfixList:               {} bytes", size_of::<QuickfixList>());
        println!("  LocationList:               {} bytes", size_of::<LocationList>());
        println!("  TabPageManager:             {} bytes", size_of::<TabPageManager>());

        // Measure small scalar/enum fields for comparison
        println!("\nSmall field sizes (for reference):");
        println!("  Mode:                       {} bytes", size_of::<Mode>());
        println!("  bool:                       {} bytes", size_of::<bool>());
        println!("  usize:                      {} bytes", size_of::<usize>());
        println!("  Option<usize>:              {} bytes", size_of::<Option<usize>>());
        println!("  Option<char>:               {} bytes", size_of::<Option<char>>());
        println!("  String:                     {} bytes", size_of::<String>());
        println!("  Option<(usize, usize)>:     {} bytes", size_of::<Option<(usize, usize)>>());

        // Measure wrapping options
        println!("\nWrapping overhead:");
        println!("  Arc<Mutex<Editor>>:         {} bytes (pointer-sized)", size_of::<Arc<Mutex<Editor>>>());
        println!("  Box<Editor>:                {} bytes (pointer-sized)", size_of::<Box<Editor>>());

        // Analysis and recommendations
        println!("\n=== Analysis ===");

        const CACHE_LINE: usize = 64;
        const RECOMMENDED_MAX: usize = 512;
        const MUST_OPTIMIZE: usize = 2048;

        if editor_size <= CACHE_LINE {
            println!("Status: EXCELLENT - fits in a single cache line");
        } else if editor_size <= RECOMMENDED_MAX {
            println!("Status: GOOD - small enough for stack, no action needed");
        } else if editor_size <= MUST_OPTIMIZE {
            println!("Status: CONSIDER OPTIMIZATION - boxing large fields would help");
            println!("Recommendation: Box fields > 64 bytes to reduce struct size");
        } else {
            println!("Status: MUST OPTIMIZE - too large for efficient stack allocation");
            println!("Recommendation: Box all fields > 64 bytes OR wrap entire Editor in Arc<Mutex<>>");
        }

        println!("\n=== Stack Usage Patterns ===");
        println!("Current usage:");
        println!("  - run_headless_loop: takes &mut Editor (zero copy)");
        println!("  - UI rendering: borrows &Editor (zero copy)");
        println!("  - API handlers: borrow &mut Editor via channels (zero copy)");
        println!("\nVerdict: Editor is NEVER passed by value, only by reference.");
        println!("Stack overhead = {} bytes once per thread (in main function).", editor_size);

        // Educational note about the measurement
        println!("\n=== Educational Context ===");
        println!("Why size matters:");
        println!("1. Stack allocation: Creating Editor on stack uses {} bytes", editor_size);
        println!("2. Move semantics: Moving Editor copies {} bytes", editor_size);
        println!("3. Async futures: Each .await point may store Editor state");
        println!("4. Cache locality: Struct doesn't fit in L1 cache ({} bytes)", CACHE_LINE);
        println!("\nHowever, since Editor is always passed by &mut reference,");
        println!("the only overhead is the initial allocation in main().");
        println!("This is a one-time cost, not a per-call cost.");
    }

    /// Size regression test - fails if Editor grows beyond threshold
    /// This prevents accidental struct bloat during development
    #[test]
    fn editor_size_regression() {
        const MAX_ACCEPTABLE_SIZE: usize = 10_000; // 10KB - conservative threshold

        let actual = size_of::<Editor>();

        // This test allows for some growth but prevents runaway bloat
        assert!(
            actual <= MAX_ACCEPTABLE_SIZE,
            "Editor struct is {} bytes, exceeds maximum of {} bytes. \n\
             Consider:\n\
             1. Boxing large fields (HashMap, Vec, etc.)\n\
             2. Using Arc<Mutex<>> for shared state\n\
             3. Moving large data to heap with Box\n\
             \n\
             Run 'cargo test measure_editor_size -- --nocapture' to see size breakdown.",
            actual, MAX_ACCEPTABLE_SIZE
        );
    }

    /// Test that Arc<Mutex<Editor>> is pointer-sized (8 or 16 bytes)
    /// This verifies that wrapping in Arc doesn't add significant overhead
    #[test]
    fn arc_mutex_editor_is_pointer_sized() {
        let arc_size = size_of::<Arc<Mutex<Editor>>>();

        // Arc is a fat pointer (ptr + ref_count), should be 16 bytes on 64-bit
        assert!(
            arc_size <= 16,
            "Arc<Mutex<Editor>> should be pointer-sized (8-16 bytes), got {} bytes",
            arc_size
        );
    }
}
