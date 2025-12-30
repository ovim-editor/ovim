mod buffer_manager;
mod change;
mod change_tracking;
mod command_history;
mod completion;
mod filetree;
mod fold;
mod input;
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

pub use change::{Change, ChangeBuilder, ChangeManager, Position, Range};
pub use completion::CompletionMenu;
pub use filetree::{FileTree, TreeNode};
pub use fold::{Fold, FoldManager};
pub use input::InputHandler;
pub use lsp_state::{LspAction, LspResultType, LspState};
pub use macros::MacroManager;
pub use keymap::{KeyMapManager, KeyMapping, MapMode};
pub use marks::{GlobalMark, JumpList, Mark, MarkManager};
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
use crate::lsp::LspManager;
#[cfg(feature = "lua")]
use crate::lua::LuaContext;
use crate::mode::Mode;
use crate::syntax::ColorSchemeRegistry;
use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;
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
    /// Mark manager for buffer marks
    marks: MarkManager,
    /// Key mapping manager
    keymaps: KeyMapManager,
    /// Jump list for Ctrl-O and Ctrl-I
    jump_list: JumpList,
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
}

/// Cached preview data for the picker
#[derive(Clone)]
pub struct PreviewCache {
    /// File content
    pub content: String,
    /// Cached syntax-highlighted lines (line_idx -> highlights)
    /// Uses RefCell for interior mutability so we can cache highlights even with immutable reference
    pub highlighted_lines: std::cell::RefCell<
        HashMap<usize, Vec<(std::ops::Range<usize>, crate::syntax::HighlightGroup)>>,
    >,
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
    pub fn new() -> Self {
        let buffer = Buffer::new();

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
            command_line: String::new(),
            command_history: Vec::new(),
            command_history_index: None,
            search_buffer: String::new(),
            search_forward: true,
            current_search: None,
            marks: MarkManager::new(),
            keymaps: KeyMapManager::new(),
            jump_list: JumpList::new(),
            macro_manager: MacroManager::new(),
            last_find: None,
            picker: None,
            leader_key: ' ',
            pending_leader: false,
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
        }
    }

    /// Creates an editor with initial content
    pub fn with_content(content: &str) -> Self {
        let buffer = Buffer::from_str(content);

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
            command_line: String::new(),
            command_history: Vec::new(),
            command_history_index: None,
            search_buffer: String::new(),
            search_forward: true,
            current_search: None,
            marks: MarkManager::new(),
            keymaps: KeyMapManager::new(),
            jump_list: JumpList::new(),
            macro_manager: MacroManager::new(),
            last_find: None,
            picker: None,
            leader_key: ' ',
            pending_leader: false,
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

}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}
