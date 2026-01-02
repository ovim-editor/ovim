mod buffer_manager;
mod change;
mod change_tracking;
mod command_history;
mod completion;
mod filetree;
mod fold;
mod input;
mod input_state;
mod keymap;
mod lsp_state;
mod lsp_integration;
mod lua_integration;
mod macros;
mod mark_jump;
mod marks;
mod motions;
mod operators;
mod performance;
mod picker;
mod picker_manager;
mod quickfix;
mod register;
mod search;
mod search_manager;
mod tabpage;
mod tab_manager;
mod textobjects;
mod theme;
mod ui_features;
mod undo;
mod visual_mode;
mod window;
mod window_viewport;

pub use change::{Change, ChangeBuilder, ChangeManager, Position, Range, TextObjectType};
pub use completion::CompletionMenu;
pub use filetree::{FileTree, TreeNode};
pub use fold::{Fold, FoldManager};
pub use input::InputHandler;
pub use input_state::{CharMotion, InputState, TextObjectPrefix};
pub use lsp_state::{LspAction, LspResultType, LspState};
pub use macros::MacroManager;
pub use keymap::{KeyMapManager, KeyMapping, MapMode};
pub use marks::{GlobalMark, JumpList, Mark, MarkManager, TagEntry, TagStack};
pub use motions::Motions;
pub use operators::{Operator, Operators};
pub use performance::MAX_LATENCY_SAMPLES;
pub use picker::{Picker, PickerMode, PickerResult};
pub use quickfix::{LocationList, QuickfixEntry, QuickfixEntryType, QuickfixList};
pub use register::{RegisterManager, RegisterType};
pub use search::Search;
pub use tabpage::{TabPage, TabPageManager};
pub use textobjects::{TextObjectRange, TextObjects};
pub use undo::UndoManager;
pub use window::{SplitDirection, Window, WindowManager, WindowNode};

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
}

impl Default for EditorOptions {
    fn default() -> Self {
        Self {
            tab_width: 4,
            shift_width: 4,
            expand_tab: true,
            number: false,
            relative_number: false,
            scroll: None,
            textwidth: None,
            ignorecase: false,
            smartcase: false,
            cursorline: false,
            showmatch: true,
            swapfile: true,
            backup: false,
            scrolloff: 10,
        }
    }
}

use crate::buffer::Buffer;
#[cfg(feature = "lua")]
use crate::lua::LuaContext;
use crate::mode::Mode;
use crate::syntax::ColorSchemeRegistry;
use anyhow::Result;
use std::collections::HashMap;
use tokio::sync::mpsc;

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

/// Visual selection: (start_position, end_position, mode)
pub type VisualSelection = ((usize, usize), (usize, usize), Mode);

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
    /// Count prefix for commands (e.g., 5j means move down 5 lines)
    count: Option<usize>,
    /// Pending operator (e.g., d for delete, waiting for motion)
    pending_operator: Option<Operator>,
    /// Pending command character (e.g., 'g' waiting for second character)
    pending_command: Option<char>,
    /// Pending register selection (e.g., 'a' from "a for next operation)
    pending_register: Option<char>,
    /// Register manager for yank/delete operations
    registers: RegisterManager,
    /// Visual mode selection start (line, col)
    visual_start: Option<(usize, usize)>,
    /// Visual block insert/append state: (start_line, end_line, col, is_append, move_to_end)
    /// move_to_end: true for I/A (cursor at end_line), false for c (cursor at start_line)
    visual_block_insert_state: Option<(usize, usize, usize, bool, bool)>,
    /// Last visual selection (start, end, mode) for gv command
    last_visual_selection: Option<VisualSelection>,
    /// Command line buffer (for : commands)
    command_line: String,
    /// Command history for command line mode
    command_history: Vec<String>,
    /// Current position in command history (for up/down navigation)
    command_history_index: Option<usize>,
    /// Search buffer (for / and ? commands)
    search_buffer: String,
    /// Search direction: true for forward (/), false for backward (?)
    search_forward: bool,
    /// Current search state
    current_search: Option<Search>,
    /// Search start position (line, col) - saved when entering search mode, restored on ESC
    search_start_pos: Option<(usize, usize)>,
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
    /// Picker for fuzzy finding files/grep
    picker: Option<Picker>,
    /// Leader key (default: space)
    leader_key: char,
    /// Waiting for leader sequence (e.g., after pressing space)
    pending_leader: bool,
    /// Input state machine for Normal mode (new architecture)
    /// This will eventually replace pending_command, pending_leader, etc.
    input_state: InputState,
    /// LSP-related state
    lsp_state: LspState,
    /// Channel sender for LSP commands from background tasks
    lsp_command_tx: Option<mpsc::UnboundedSender<LspCommand>>,
    /// Channel receiver for LSP commands from background tasks
    lsp_command_rx: Option<mpsc::UnboundedReceiver<LspCommand>>,
    /// Lua context for configuration and plugins (optional)
    #[cfg(feature = "lua")]
    lua_context: Option<LuaContext>,
    /// Bridge for Lua-Editor communication (optional)
    #[cfg(feature = "lua")]
    editor_bridge: Option<crate::lua::EditorBridge>,
    /// Last insert position (line, col) for gi command
    last_insert_position: Option<(usize, usize)>,
    /// Completion menu popup
    completion_menu: CompletionMenu,
    /// Preview cache for picker (file_path -> (content, syntax highlights))
    preview_cache: HashMap<String, PreviewCache>,
    /// Color scheme registry
    color_scheme_registry: ColorSchemeRegistry,
    /// Editor options and settings
    pub options: EditorOptions,
    /// Viewport height (rows) - updated from UI layer
    viewport_height: usize,
    /// Scroll offset (top visible line) - maintained with scrolloff
    scroll_offset: usize,
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
    /// Last time picker query changed (for debouncing preview loading)
    last_picker_query_change: Option<std::time::Instant>,
    /// Last time picker selection moved (for debouncing preview loading)
    last_picker_selection_change: Option<std::time::Instant>,
    /// Currently loading preview path (to avoid duplicate requests)
    loading_preview: Option<String>,
    /// Last successfully shown preview path (to show while new one loads)
    pub last_shown_preview: Option<String>,
    /// Performance metrics: render count
    render_count: u64,
    /// Performance metrics: last render duration in microseconds
    last_render_duration_micros: Option<u64>,
    /// Performance metrics: last syntax highlighting duration in microseconds
    last_syntax_duration_micros: Option<u64>,
    /// Render dirty flag - set when UI needs redraw
    render_dirty: bool,
    /// Input latency samples in microseconds (circular buffer, max 1000 samples)
    input_latency_samples: Vec<u64>,
    /// Last LSP serialize (rope->string) duration in microseconds
    last_lsp_serialize_micros: Option<u64>,
    /// Last git status refresh duration in microseconds
    last_git_status_micros: Option<u64>,
    /// Last fold calculation duration in microseconds
    last_fold_calc_micros: Option<u64>,
    /// Last diagnostic query duration in microseconds
    last_diagnostic_query_micros: Option<u64>,
    /// Dashboard menu selected index (0-5)
    dashboard_selected: usize,
    /// Pending semantic change operation (for ci", cw, etc.)
    /// When Some, insert mode exit will create a semantic change instead of composite
    pending_semantic_change: Option<PendingSemanticChange>,
    /// Replace mode tracking for dot-repeat
    replace_mode_state: Option<ReplaceModeState>,
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
            count: None,
            pending_operator: None,
            pending_command: None,
            pending_register: None,
            registers: RegisterManager::new(),
            visual_start: None,
            visual_block_insert_state: None,
            last_visual_selection: None,
            command_line: String::new(),
            command_history: Vec::new(),
            command_history_index: None,
            search_buffer: String::new(),
            search_forward: true,
            current_search: None,
            search_start_pos: None,
            marks: MarkManager::new(),
            keymaps: KeyMapManager::new(),
            jump_list: JumpList::new(),
            tag_stack: TagStack::new(),
            macro_manager: MacroManager::new(),
            last_find: None,
            picker: None,
            leader_key: ' ',
            pending_leader: false,
            input_state: InputState::default(),
            lsp_state: LspState::new(),
            lsp_command_tx: None,
            lsp_command_rx: None,
            #[cfg(feature = "lua")]
            lua_context: None,
            #[cfg(feature = "lua")]
            editor_bridge: None,
            last_insert_position: None,
            completion_menu: CompletionMenu::new(),
            preview_cache: HashMap::new(),
            color_scheme_registry: ColorSchemeRegistry::new(),
            current_color_scheme: "tokyonight".to_string(),
            options: EditorOptions::default(),
            viewport_height: 24,
            scroll_offset: 0,
            file_tree: FileTree::new(),
            quickfix_list: QuickfixList::new(),
            location_list: LocationList::new(),
            quickfix_window_open: false,
            location_window_open: false,
            substitute_matches: Vec::new(),
            substitute_match_index: 0,
            substitute_pattern: None,
            tab_page_manager: TabPageManager::new(),
            last_picker_query_change: None,
            last_picker_selection_change: None,
            loading_preview: None,
            last_shown_preview: None,
            render_count: 0,
            last_render_duration_micros: None,
            last_syntax_duration_micros: None,
            render_dirty: true, // Start dirty to force initial render
            input_latency_samples: Vec::new(),
            last_lsp_serialize_micros: None,
            last_git_status_micros: None,
            last_fold_calc_micros: None,
            last_diagnostic_query_micros: None,
            dashboard_selected: 0,
            pending_semantic_change: None,
            replace_mode_state: None,
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
            count: None,
            pending_operator: None,
            pending_command: None,
            pending_register: None,
            registers: RegisterManager::new(),
            visual_start: None,
            visual_block_insert_state: None,
            last_visual_selection: None,
            command_line: String::new(),
            command_history: Vec::new(),
            command_history_index: None,
            search_buffer: String::new(),
            search_forward: true,
            current_search: None,
            search_start_pos: None,
            marks: MarkManager::new(),
            keymaps: KeyMapManager::new(),
            jump_list: JumpList::new(),
            tag_stack: TagStack::new(),
            macro_manager: MacroManager::new(),
            last_find: None,
            picker: None,
            leader_key: ' ',
            pending_leader: false,
            input_state: InputState::default(),
            lsp_state: LspState::new(),
            lsp_command_tx: None,
            lsp_command_rx: None,
            #[cfg(feature = "lua")]
            lua_context: None,
            #[cfg(feature = "lua")]
            editor_bridge: None,
            last_insert_position: None,
            completion_menu: CompletionMenu::new(),
            preview_cache: HashMap::new(),
            color_scheme_registry: ColorSchemeRegistry::new(),
            current_color_scheme: "tokyonight".to_string(),
            options: EditorOptions::default(),
            viewport_height: 24,
            scroll_offset: 0,
            file_tree: FileTree::new(),
            quickfix_list: QuickfixList::new(),
            location_list: LocationList::new(),
            quickfix_window_open: false,
            location_window_open: false,
            substitute_matches: Vec::new(),
            substitute_match_index: 0,
            substitute_pattern: None,
            tab_page_manager: TabPageManager::new(),
            last_picker_query_change: None,
            last_picker_selection_change: None,
            loading_preview: None,
            last_shown_preview: None,
            render_count: 0,
            last_render_duration_micros: None,
            last_syntax_duration_micros: None,
            render_dirty: true, // Start dirty to force initial render
            input_latency_samples: Vec::new(),
            last_lsp_serialize_micros: None,
            last_git_status_micros: None,
            last_fold_calc_micros: None,
            last_diagnostic_query_micros: None,
            dashboard_selected: 0,
            pending_semantic_change: None,
            replace_mode_state: None,
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
        self.count = None;
        self.pending_operator = None;
        self.pending_command = None;
        self.pending_register = None;

        // Clear visual selection when leaving visual modes
        if !matches!(mode, Mode::Visual | Mode::VisualLine | Mode::VisualBlock) {
            self.visual_start = None;
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

    /// Gets the pending command
    pub fn pending_command(&self) -> Option<char> {
        self.pending_command
    }

    /// Sets the pending command
    pub fn set_pending_command(&mut self, cmd: char) {
        self.pending_command = Some(cmd);
    }

    /// Gets the current input state (new state machine)
    pub fn input_state(&self) -> &InputState {
        &self.input_state
    }

    /// Sets the input state (new state machine)
    pub fn set_input_state(&mut self, state: InputState) {
        self.input_state = state;
    }

    /// Resets input state to Normal
    pub fn reset_input_state(&mut self) {
        self.input_state = InputState::Normal;
    }

    /// Sets the viewport height (called from UI layer)
    pub fn set_viewport_height(&mut self, height: usize) {
        self.viewport_height = height;
    }

    /// Gets the viewport height
    pub fn viewport_height(&self) -> usize {
        self.viewport_height
    }

    /// Gets the scroll offset (top visible line)
    pub fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    /// Updates scroll offset to keep cursor visible with scrolloff margin
    pub fn update_scroll_offset(&mut self) {
        let cursor_line = self.buffer().cursor().line();
        let scrolloff = self.options.scrolloff;
        let visible_lines = self.viewport_height;

        // Scroll up if cursor is too close to top
        if cursor_line < self.scroll_offset + scrolloff {
            self.scroll_offset = cursor_line.saturating_sub(scrolloff);
        }
        // Scroll down if cursor is too close to bottom
        else if cursor_line + scrolloff >= self.scroll_offset + visible_lines {
            self.scroll_offset = cursor_line + scrolloff + 1
                - visible_lines.min(cursor_line + scrolloff + 1);
        }

        // Ensure scroll_offset doesn't go beyond buffer
        let max_line = self.buffer().line_count().saturating_sub(1);
        if cursor_line > max_line {
            self.scroll_offset = 0;
        }
    }

    /// Calculates half-page scroll amount
    /// Uses options.scroll if set, otherwise viewport_height / 2
    pub fn half_page_scroll(&self) -> usize {
        self.options
            .scroll
            .unwrap_or(self.viewport_height / 2)
    }

    /// Clears the pending command
    pub fn clear_pending_command(&mut self) {
        self.pending_command = None;
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
        self.count
    }

    /// Sets the count
    pub fn set_count(&mut self, count: usize) {
        self.count = Some(count);
    }

    /// Appends a digit to the count
    pub fn append_count(&mut self, digit: usize) {
        let current = self.count.unwrap_or(0);
        self.count = Some(current * 10 + digit);
    }

    /// Clears the count
    pub fn clear_count(&mut self) {
        self.count = None;
    }

    /// Gets the effective count (count or 1)
    pub fn effective_count(&self) -> usize {
        self.count.unwrap_or(1)
    }

    /// Gets the pending operator
    pub fn pending_operator(&self) -> Option<Operator> {
        self.pending_operator
    }

    /// Sets the pending operator
    pub fn set_pending_operator(&mut self, op: Operator) {
        self.pending_operator = Some(op);
    }

    /// Clears the pending operator
    pub fn clear_pending_operator(&mut self) {
        self.pending_operator = None;
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
        self.pending_register
    }

    /// Sets the pending register for next operation
    pub fn set_pending_register(&mut self, reg: char) {
        self.pending_register = Some(reg);
    }

    /// Clears the pending register
    pub fn clear_pending_register(&mut self) {
        self.pending_register = None;
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
    pub async fn process_pending_rehighlight(&mut self) {
        // Check if buffer needs re-highlighting
        let Some((content, version, language)) = self.buffer().get_rehighlight_data() else {
            return;
        };

        // Spawn blocking task for CPU-intensive parsing
        let highlights = tokio::task::spawn_blocking(move || {
            // Create a new highlighter for this language
            let mut highlighter = match crate::syntax::SyntaxHighlighter::new(language) {
                Ok(h) => h,
                Err(_) => return None,
            };

            // Parse the content
            highlighter.parse(&content);

            // Build highlights for all lines using efficient single-pass method
            let all_highlights = highlighter.highlights_for_all_lines(&content);

            Some(all_highlights)
        })
        .await;

        // Apply highlights if successful and version still matches
        if let Ok(Some(highlights)) = highlights {
            self.buffer_mut().apply_highlights(highlights, version);
        }
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
        if let Some(reg) = self.pending_register {
            self.registers.set_with_type(Some(reg), text, reg_type);
            self.pending_register = None;
        } else {
            self.registers.yank_with_type(text, reg_type);
        }
    }

    /// Deletes text and stores in the appropriate register (pending_register or default)
    pub fn delete_to_register(&mut self, text: String) {
        self.delete_to_register_with_type(text, RegisterType::Character);
    }

    /// Deletes text and stores in the appropriate register with explicit type
    pub fn delete_to_register_with_type(&mut self, text: String, reg_type: RegisterType) {
        if let Some(reg) = self.pending_register {
            self.registers.set_with_type(Some(reg), text, reg_type);
            self.pending_register = None;
        } else {
            self.registers.delete_with_type(text, reg_type);
        }
    }

    /// Gets text from the appropriate register (pending_register or default)
    pub fn get_from_register(&mut self) -> String {
        let text = if let Some(reg) = self.pending_register {
            self.registers.get(Some(reg))
        } else {
            self.registers.get_default().to_string()
        };
        self.pending_register = None;
        text
    }

    /// Gets text and type from the appropriate register (pending_register or default)
    pub fn get_from_register_with_type(&mut self) -> (String, RegisterType) {
        let (text, reg_type) = if let Some(reg) = self.pending_register {
            self.registers.get_with_type(Some(reg))
        } else {
            let (t, rt) = self.registers.get_default_with_type();
            (t.to_string(), rt)
        };
        self.pending_register = None;
        (text, reg_type)
    }

    /// Starts building a composite change (e.g., when entering insert mode)
    pub fn start_change_building(&mut self, cursor_before: Position) {
        self.buffer_mut()
            .change_manager_mut()
            .start_building(cursor_before);
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

    /// Gets the leader key
    pub fn leader_key(&self) -> char {
        self.leader_key
    }

    /// Sets pending leader state
    pub fn set_pending_leader(&mut self, pending: bool) {
        self.pending_leader = pending;
    }

    /// Gets pending leader state
    pub fn pending_leader(&self) -> bool {
        self.pending_leader
    }

    /// Gets cached diagnostic count (sync, suitable for UI rendering)
    pub fn cached_diagnostic_count(&self) -> (usize, usize, usize, usize) {
        self.lsp_state.diagnostic_count
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
