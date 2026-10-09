mod ai_auto_mode;
mod ai_background_tools;
mod ai_base_manifest;
mod ai_browser;
mod ai_chat;
mod ai_chat_code_attachment;
mod ai_chat_commands;
mod ai_chat_exa;
mod ai_chat_images;
pub mod ai_chat_input;
mod ai_chat_links;
mod ai_chat_mutations;
#[cfg(test)]
mod ai_chat_parked_tests;
mod ai_chat_presentation;
mod ai_chat_queue;
mod ai_chat_review;
mod ai_chat_scratch;
mod ai_chat_selection;
mod ai_chat_session;
pub(crate) mod ai_chat_state;
mod ai_chat_tools;
mod ai_chat_turn;
mod ai_chat_viewport;
mod ai_code_explanation;
mod ai_codex_auth;
mod ai_compaction;
mod ai_comprehension;
mod ai_custom_diff;
mod ai_durable_chat;
mod ai_editor_mcp;
mod ai_external_agent;
pub(crate) mod ai_integration;
mod ai_run_events;
mod ai_session_temp;
mod ai_shell_process;
mod ai_skills;
mod ai_state;
mod ai_subagents;
mod ai_tool_execution;
mod ai_tool_path;
mod ai_tool_streaming;
mod ai_workflow;
mod blame_commands;
mod buffer_manager;
mod build_state;
mod change_building;
mod change_tracking;
mod clipboard;
mod code_explanation;
mod code_lens;
mod command_context;
mod command_history;
mod completion;
mod completion_accept;
pub mod completion_match;
mod debug_integration;
mod debug_results;
pub mod decoration;
mod diff_review;
mod editing_state;
mod execution;
#[cfg(test)]
mod execution_tests;
mod file_loading;
mod file_rename;
mod filetree;
mod folding;
pub mod fuzzy;
pub mod git_tools;
pub mod grep;
#[cfg(test)]
mod incremental_wrap_tests;
mod input;
mod input_accessors;
mod input_context;
mod input_state;
mod keymap;
mod launch_flow;
mod lsp_columns;
mod lsp_integration;
pub mod lsp_manager_panel;
pub(crate) mod lsp_slot;
mod lsp_state;
mod lsp_subsystem;
mod lsp_ui;
mod lua_integration;
mod macros;
mod mark_jump;
mod marks;
pub(crate) mod motions;
mod navigation_state;
pub mod nucleo_matcher;
mod operators;
pub mod outline;
pub mod path_completion;
#[cfg(test)]
mod per_window_wrap_tests;
mod performance;
pub mod picker;
mod picker_manager;
pub mod picker_state;
pub mod problems;
pub mod project_nav;
mod pseudocode;
pub mod search_replace;
#[cfg(test)]
mod size_tests;
mod snippet_session;
#[cfg(test)]
mod wrap_decoration_tests;
pub use pseudocode::MarkdownDocument;
mod quickfix;
mod register;
mod register_ops;
mod render_cache;
mod search_context;
mod search_manager;
mod services;
mod single_line_input;
mod status;
mod tab_manager;
mod tabpage;
mod test_panel;
pub(crate) mod test_runner;
mod theme;
mod theme_state;
mod toast;
mod ui_features;
mod ui_panels;
mod undo;
mod viewport_scroll;
mod viewport_state;
mod visual_context;
mod visual_mode;
mod window;
mod window_viewport;
mod workspace_watch;
mod wrap_map;
mod yank_flash;

// Re-export sibling modules for backward compatibility
pub use crate::fold;
pub use crate::search;
pub use crate::textobjects;

pub use crate::change::{
    ApplyPos, Change, ChangeBuilder, ChangeManager, CursorPos, InsertEntryMode, Range,
    TextObjectType,
};
pub use ai_chat_code_attachment::split_code_attachment_message;
pub use ai_chat_commands::AiChatSlashCompletion;
pub use ai_chat_session::AI_CHAT_REASONING_EFFORTS;
pub use ai_chat_state::{
    AiChatActivity, ChatModelPickerSection, CodeAttachment, ComprehensionPolicy, QueuedChatInput,
    QueuedChatInputKind,
};
pub use ai_shell_process::{ShellInspectorView, ShellProcessPhase};
pub use ai_state::{CodexAuthDialogPhase, CodexAuthDialogSummary};
pub use ai_subagents::PreparedHeadlessAgentControl;
pub use build_state::{PendingShellCommand, PendingTerminalSession};
pub use code_explanation::{
    CodeExplanationCardLayout, CodeExplanationDiscussionView, CodeExplanationPageView,
    CodeExplanationView, ConceptExplanationCardLayout,
};
pub use code_lens::LensEntry;
pub use command_context::CommandContext;
pub use completion::{
    completion_documentation_markdown, completion_item_is_deprecated, completion_kind_style,
    completion_row_text, CompletionAnchor, CompletionKindClass, CompletionKindStyle,
    CompletionMenu, CompletionRowText,
};
pub use completion_accept::CompletionAcceptMode;
pub use debug_integration::{BreakpointExtra, BreakpointMarker};
pub use diff_review::{
    DiffLayout, DiffOverlayViewState, DiffReviewState, PendingGitFetch, DIFF_REVIEW_TITLE_PREFIX,
};
pub use editing_state::{EditingState, PendingChangeRepeat};
pub(crate) use execution::Nesting;
pub use filetree::{FileTree, FileTreeAction, FileTreeClipboardKind, TreeNode};
pub use fold::{Fold, FoldManager};
pub use input::mouse::handle_mouse_event;
pub use input::shell_expansion;
pub use input::InputHandler;
pub use input_context::InputContext;
pub use input_state::{CharMotion, InputState, TextObjectPrefix};
pub use keymap::{KeyMapManager, KeyMapping, MapMode};
pub use launch_flow::{LaunchRequest, LaunchSource};
pub use lsp_manager_panel::LspManagerPanel;
pub use lsp_state::{
    CompletionIntent, HoverContentType, LspIntents, LspResultType, LspState, ProjectedDiagnostics,
    SignatureHelpState,
};
pub use lsp_ui::LspUi;
pub use macros::MacroManager;
pub use marks::{GlobalMark, JumpEntry, JumpList, MarkManager, TagEntry, TagStack};
pub use motions::Motions;
pub use navigation_state::NavigationState;
pub use operators::Operator;
pub use path_completion::PathCompletionState;
pub use performance::{PerformanceMetrics, MAX_LATENCY_SAMPLES};
pub use picker::{
    GitPick, Picker, PickerAction, PickerField, PickerMode, PickerResult, PickerRole,
};
pub use picker_state::PickerState;
pub use quickfix::{LocationList, QuickfixEntry, QuickfixEntryType, QuickfixList};
pub use register::{RegisterManager, RegisterType};
pub use render_cache::{ChatLink, RenderCache};
pub use search::Search;
pub use search_context::{SearchContext, VisualSearchState};
pub use services::EditorServices;
pub use single_line_input::SingleLineInput;
pub use snippet_session::SnippetSession;
pub use tabpage::{TabPage, TabPageId, TabPageManager};
pub use test_panel::{
    format_duration, TestFailure, TestPanelState, TestRun, TestRunStatus, TestSourceLocation,
};
pub use textobjects::{TextObjectRange, TextObjects};
pub use theme_state::ThemeState;
pub use toast::{Toast, ToastCenter, ToastLevel, ToastRequest, ToastSource};
pub use ui_panels::UiPanels;
pub use undo::UndoManager;
pub use viewport_state::ViewportState;
pub use visual_context::{BlockInsert, VisualContext, VisualSelection};
pub use window::{SplitDirection, Window, WindowManager, WindowNode, WindowView, WindowViewNode};
pub use wrap_map::WrapMap;

/// Margin background color for textwidth shading
#[derive(Debug, Clone, PartialEq)]
pub enum MarginColor {
    /// No margin shading (default — preserves terminal transparency)
    None,
    /// Solid RGB color
    Solid(u8, u8, u8),
}

/// Controls LSP auto-install behavior
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AutoInstallMode {
    /// Show a consent dialog before installing (default)
    #[default]
    Prompt,
    /// Install automatically without asking
    Auto,
    /// Never auto-install, only show install hints
    Off,
}

/// Editor options and settings
#[derive(Debug, Clone)]
pub struct EditorOptions {
    /// Width of tab character (default: 4)
    pub tab_width: usize,
    /// Number of spaces to use for autoindent (default: 4)
    pub shift_width: usize,
    /// Use spaces instead of tabs (default: true)
    pub expand_tab: bool,
    /// Insert-mode soft tab stop. -1 follows shiftwidth, 0 follows tabstop.
    pub soft_tab_stop: isize,
    /// Preserve the exact existing whitespace prefix when autoindenting.
    pub copy_indent: bool,
    /// Show line numbers (default: false)
    pub number: bool,
    /// Show relative line numbers (default: false)
    pub relative_number: bool,
    /// Number of lines to scroll for half-page movements (default: None = calculate from viewport)
    pub scroll: Option<usize>,
    /// Maximum width of text content (default: None = use full terminal width)
    /// When set, content is centered horizontally with margins on both sides
    pub textwidth: Option<usize>,
    /// Highlight a vertical column at the specified column (default: None)
    /// Useful for keeping lines under a certain width
    pub colorcolumn: Option<usize>,
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
    /// Show git blame gutter (default: false)
    pub blame: bool,
    /// Default branch for diff review; None selects the repository default.
    pub pullbase: Option<String>,
    /// Canonical directory overrides; the closest ancestor of the repo root wins.
    pub pullbase_paths: std::collections::BTreeMap<std::path::PathBuf, String>,
    /// Conceal markdown constructs (links, images) when rendering (default: true)
    pub markdown_conceal: bool,
    /// Background color for textwidth margins
    pub margin_color: MarginColor,
    /// Extra columns of normal background between text edge and shaded margin area (default: 0)
    pub margin_padding: usize,
    /// Program to run for :make (default: "cargo build")
    pub makeprg: String,
    /// LSP auto-install behavior: Prompt (default), Auto, or Off
    pub lsp_auto_install: AutoInstallMode,
    /// Request and render LSP inlay hints (default: false).
    /// Off by default while OV-00257 (wrap math) and OV-00258 (stale
    /// placement) are open. Toggle on with `:set inlay_hints` to opt in.
    pub inlay_hints: bool,
    /// Open the completion menu automatically while typing in insert mode
    /// (identifier characters and server trigger characters). Ctrl-Space
    /// always works. Default: true.
    pub autocomplete: bool,
    /// Identifier characters typed before the menu opens by itself (default: 2).
    pub autocomplete_min_chars: usize,
    /// Milliseconds the typist must pause before an identifier-triggered
    /// request is sent (default: 40). Trigger characters ignore the delay.
    pub autocomplete_delay_ms: u64,
    /// Vim's `foldcolumn`: width of the fold marker column in the gutter
    /// (0 hides it). With `foldcolumn_auto` it is the maximum: the column is
    /// as wide as the deepest fold nesting and absent when there are no folds
    /// (nvim's `auto[:N]`). Default `auto:1`.
    pub foldcolumn: usize,
    pub foldcolumn_auto: bool,
}

impl Default for EditorOptions {
    fn default() -> Self {
        Self {
            tab_width: 4,
            shift_width: 4,
            expand_tab: true,
            soft_tab_stop: -1,
            copy_indent: false,
            number: true,
            relative_number: false,
            scroll: None,
            textwidth: Some(150),
            colorcolumn: None,
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
            blame: false,
            pullbase: None,
            pullbase_paths: Default::default(),
            markdown_conceal: true,
            margin_color: MarginColor::None,
            margin_padding: 0,
            makeprg: "cargo build".to_string(),
            lsp_auto_install: AutoInstallMode::default(),
            inlay_hints: false,
            autocomplete: true,
            autocomplete_min_chars: 2,
            autocomplete_delay_ms: 40,
            foldcolumn: 1,
            foldcolumn_auto: true,
        }
    }
}

impl EditorOptions {
    /// Snapshot the indentation-related options for one editing operation.
    pub fn indent_options(&self) -> crate::indentation::IndentOptions {
        crate::indentation::IndentOptions {
            tab_width: self.tab_width,
            shift_width: self.shift_width,
            soft_tab_stop: self.soft_tab_stop,
            expand_tab: self.expand_tab,
            copy_indent: self.copy_indent,
        }
        .normalized()
    }

    /// Replace the editor-level defaults used by newly created buffers.
    pub fn set_indent_options(&mut self, options: crate::indentation::IndentOptions) {
        let options = options.normalized();
        self.tab_width = options.tab_width;
        self.shift_width = options.shift_width;
        self.soft_tab_stop = options.soft_tab_stop;
        self.expand_tab = options.expand_tab;
        self.copy_indent = options.copy_indent;
    }
}

use crate::buffer::Buffer;
#[cfg(feature = "lua")]
use crate::lua::LuaContext;
use crate::mode::Mode;
use crate::unicode::GraphemeCol;
use anyhow::Result;
use std::collections::HashMap;

/// Cached preview highlights: line_idx -> Vec<(range, highlight_group)>
pub type PreviewHighlights =
    HashMap<usize, Vec<(std::ops::Range<usize>, crate::syntax::HighlightGroup)>>;

/// The main editor state
pub struct Editor {
    language_catalog: std::sync::Arc<crate::language_catalog::LanguageCatalog>,
    /// List of open buffers
    pub(crate) buffers: Vec<Buffer>,
    /// Index of the currently active buffer
    current_buffer_index: usize,
    /// Window manager for split windows
    window_manager: Option<WindowManager>,
    /// Current editing mode
    mode: Mode,
    /// Whether the editor should quit
    should_quit: bool,
    /// Exit code to use when quitting (0 = success, non-zero = error, used by :cq))
    exit_code: i32,
    /// Input context (counts, operators, pending commands, registers, input state machine)
    input: InputContext,
    /// Register manager for yank/delete operations
    registers: RegisterManager,
    /// Visual mode context (selection start, block insert state, last selection)
    pub(crate) visual: VisualContext,
    /// Command-line mode context (buffer, history, navigation)
    command: CommandContext,
    /// Search-related state
    pub search: SearchContext,
    /// Navigation state (marks, jump list, tag stack, find repeat)
    pub nav: NavigationState,
    /// Key mapping manager
    keymaps: KeyMapManager,
    /// Macro manager for recording and playback
    macro_manager: MacroManager,
    /// Picker state (picker, preview cache, layout, file list cache, etc.)
    pub picker_state: PickerState,
    /// LSP subsystem (state, commands, UI, install)
    pub(crate) lsp: lsp_subsystem::LspSubsystem,
    /// Lua context for configuration and plugins (optional)
    #[cfg(feature = "lua")]
    lua_context: Option<LuaContext>,
    /// Bridge for Lua-Editor communication (optional)
    #[cfg(feature = "lua")]
    editor_bridge: Option<crate::lua::EditorBridge>,
    /// Editing operation state (insert, replace, substitute, rename)
    pub editing: EditingState,
    /// Completion menu popup (LSP)
    completion_menu: Box<CompletionMenu>,
    /// Theme and color scheme state
    theme: ThemeState,
    /// Editor options and settings
    pub options: EditorOptions,
    /// Viewport and scroll state
    pub viewport: ViewportState,
    /// Tab page manager
    tab_page_manager: TabPageManager,
    /// Performance metrics
    metrics: PerformanceMetrics,
    /// Cached rendering state (mouse, layout geometry)
    pub render_cache: RenderCache,
    /// Transient yank flash highlight
    yank_flash: Option<yank_flash::YankFlash>,
    /// UI panels (file tree, quickfix, path completion, dashboard, diagnostic badge)
    pub ui_panels: UiPanels,
    /// DAP (Debug Adapter Protocol) manager for debug sessions
    dap_manager: Box<crate::dap::DapManager>,
    /// AI chat, selection, and in-buffer agent state
    pub ai_state: Box<ai_state::AiState>,
    /// Optional capabilities supplied by the active frontend.
    services: EditorServices,
    /// Deferred browser session request consumed by the async intent dispatcher.
    browser_start_pending: bool,
    /// API server port for an explicit headless automation session.
    api_port: Option<u16>,
    /// Active session name when a frontend registers one explicitly.
    active_session: Option<String>,
    /// Git branch name for the current file (if in a git repo)
    git_branch: Option<String>,
    /// Build/test subsystem state
    pub(crate) build: build_state::BuildState,
    /// Run/debug launch flow and the run console
    pub(crate) launch: Box<launch_flow::LaunchState>,
    /// Unified virtual text decorations (inlay hints, diagnostics, etc.)
    pub decorations: decoration::DecorationMap,
    /// Channel for receiving background git refresh results (status + blame)
    git_refresh_generation: u64,
    /// Background refreshes spawned but not yet drained by `poll_git_refresh`
    git_refresh_in_flight: usize,
    git_refresh_rx: tokio::sync::mpsc::Receiver<GitRefreshResult>,
    /// Sender half — cloned into spawn_blocking tasks
    pub(crate) git_refresh_tx: tokio::sync::mpsc::Sender<GitRefreshResult>,
}

/// Result of a background git status/blame refresh.
pub struct GitRefreshResult {
    pub generation: u64,
    pub path: String,
    pub status: crate::git::GitStatus,
    pub blame: Option<crate::git::GitBlame>,
}

/// Pending LSP server installation awaiting user consent
#[derive(Debug, Clone)]
pub struct PendingLspInstall {
    /// Human-readable language name (e.g. "Python")
    pub language_name: String,
    /// LSP server command (e.g. "pyright-langserver")
    pub server_command: String,
    /// How it will be installed (e.g. "npm install -g pyright")
    pub method_description: String,
    /// File path that triggered the install
    pub file_path: String,
    /// Set when the install is of a companion server (e.g. Tailwind CSS)
    /// rather than the file's language server.
    pub companion_id: Option<String>,
}

/// User's response to the LSP install consent dialog
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LspInstallConsent {
    /// Install this one time
    Yes,
    /// Set autoinstall=auto for all future installs
    Always,
    /// Skip this install
    No,
}

/// Cached picker layout rects for mouse hit-testing
#[derive(Debug, Clone)]
pub struct PickerLayout {
    /// Search input area
    pub query_field: crate::Rect,
    /// File filter area (LiveGrep only)
    pub filter_field: Option<crate::Rect>,
    /// Results list area
    pub results_area: crate::Rect,
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
    /// Cursor position when R was pressed (grapheme-space).
    pub start_position: CursorPos,
    /// Characters typed during replace mode
    pub replacements: String,
    /// Original grapheme overwritten by each replacement. `None` means the
    /// corresponding character was appended past the end of the line.
    pub old_text: Vec<Option<String>>,
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
    /// Effective indentation policy for the active buffer.
    pub fn indent_options(&self) -> crate::indentation::IndentOptions {
        self.buffer()
            .indent_options()
            .unwrap_or_else(|| self.options.indent_options())
    }

    /// Set indentation for the active buffer and update the defaults inherited
    /// by buffers opened later. Existing buffers keep their own policy.
    pub fn set_indent_options(&mut self, options: crate::indentation::IndentOptions) {
        let options = options.normalized();
        self.options.set_indent_options(options);
        self.buffer_mut().set_indent_options(options);
    }

    /// Set indentation only for the active buffer. File policy sources such
    /// as EditorConfig and modelines use this to avoid cross-file leakage.
    pub fn set_local_indent_options(&mut self, options: crate::indentation::IndentOptions) {
        self.buffer_mut().set_indent_options(options);
    }

    /// Creates a new editor with an empty buffer
    /// Starts in Dashboard mode when no file is opened
    pub fn new() -> Self {
        let language_catalog = crate::language_catalog::LanguageCatalog::built_in();
        let mut buffer = Buffer::new();
        buffer.set_language_catalog(language_catalog.clone());
        let (git_tx, git_rx) = tokio::sync::mpsc::channel(4);
        Self {
            language_catalog,
            buffers: vec![buffer],
            current_buffer_index: 0,
            window_manager: None, // Will be initialized when viewport size is known
            mode: Mode::Dashboard,
            should_quit: false,
            exit_code: 0,
            input: InputContext::new(),
            registers: RegisterManager::new(),
            visual: VisualContext::new(),
            command: CommandContext::new(),
            search: SearchContext::new(),
            nav: NavigationState::default(),
            keymaps: KeyMapManager::new(),
            macro_manager: MacroManager::new(),
            picker_state: PickerState::new(),
            lsp: lsp_subsystem::LspSubsystem::default(),
            #[cfg(feature = "lua")]
            lua_context: None,
            #[cfg(feature = "lua")]
            editor_bridge: None,
            editing: EditingState::default(),
            completion_menu: Box::default(),
            theme: ThemeState::default(),
            options: EditorOptions::default(),
            viewport: ViewportState::default(),
            tab_page_manager: TabPageManager::new(),
            metrics: PerformanceMetrics::new(),
            render_cache: RenderCache::default(),
            yank_flash: None,
            ui_panels: UiPanels::default(),
            dap_manager: Box::new(crate::dap::DapManager::new()),
            ai_state: Box::new(ai_state::AiState::default()),
            services: EditorServices::default(),
            browser_start_pending: false,
            api_port: None,
            active_session: None,
            git_branch: None,
            build: build_state::BuildState::default(),
            launch: Box::default(),
            decorations: decoration::DecorationMap::new(),
            git_refresh_generation: 0,
            git_refresh_in_flight: 0,
            git_refresh_rx: git_rx,
            git_refresh_tx: git_tx,
        }
    }

    /// Creates an editor with initial content
    pub fn with_content(content: &str) -> Self {
        let language_catalog = crate::language_catalog::LanguageCatalog::built_in();
        let mut buffer = Buffer::new_from_str(content);
        buffer.set_language_catalog(language_catalog.clone());
        let (git_tx, git_rx) = tokio::sync::mpsc::channel(4);
        Self {
            language_catalog,
            buffers: vec![buffer],
            current_buffer_index: 0,
            window_manager: None, // Will be initialized when viewport size is known
            mode: Mode::default(),
            should_quit: false,
            exit_code: 0,
            input: InputContext::new(),
            registers: RegisterManager::new(),
            visual: VisualContext::new(),
            command: CommandContext::new(),
            search: SearchContext::new(),
            nav: NavigationState::default(),
            keymaps: KeyMapManager::new(),
            macro_manager: MacroManager::new(),
            picker_state: PickerState::new(),
            lsp: lsp_subsystem::LspSubsystem::default(),
            #[cfg(feature = "lua")]
            lua_context: None,
            #[cfg(feature = "lua")]
            editor_bridge: None,
            editing: EditingState::default(),
            completion_menu: Box::default(),
            theme: ThemeState::default(),
            options: EditorOptions::default(),
            viewport: ViewportState::default(),
            tab_page_manager: TabPageManager::new(),
            metrics: PerformanceMetrics::new(),
            render_cache: RenderCache::default(),
            yank_flash: None,
            ui_panels: UiPanels::default(),
            dap_manager: Box::new(crate::dap::DapManager::new()),
            ai_state: Box::new(ai_state::AiState::default()),
            services: EditorServices::default(),
            browser_start_pending: false,
            api_port: None,
            active_session: None,
            git_branch: None,
            build: build_state::BuildState::default(),
            launch: Box::default(),
            decorations: decoration::DecorationMap::new(),
            git_refresh_generation: 0,
            git_refresh_in_flight: 0,
            git_refresh_rx: git_rx,
            git_refresh_tx: git_tx,
        }
    }

    /// Install optional services supplied by the active frontend.
    pub fn with_services(mut self, services: EditorServices) -> Self {
        self.services = services;
        self
    }

    pub fn services(&self) -> &EditorServices {
        &self.services
    }

    // ==================== Rename Input ====================

    pub fn rename_buffer(&self) -> &str {
        self.editing.rename_input.text()
    }

    pub fn rename_cursor(&self) -> usize {
        self.editing.rename_input.cursor()
    }

    pub fn set_rename_buffer(&mut self, s: String) {
        self.editing.rename_input = SingleLineInput::new(s);
    }

    pub(crate) fn rename_input_mut(&mut self) -> &mut SingleLineInput {
        &mut self.editing.rename_input
    }

    // ==================== Core Editor Methods ====================

    /// Gets the current mode
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Sets the mode
    pub fn set_mode(&mut self, mode: Mode) {
        // A buffer that is not modifiable (library source, virtual document)
        // cannot be typed into: `i`, `a`, `o`, `c...` stay in Normal mode.
        let mode = if matches!(mode, Mode::Insert | Mode::Replace) && !self.buffer().is_modifiable()
        {
            self.report_unmodifiable();
            // Insert-entering commands opened a change session first.
            self.finalize_change_building();
            self.editing.pending_change_repeat = None;
            Mode::Normal
        } else {
            mode
        };
        // A sticky `$` (curswant at end of line) makes a new block extend to
        // the end of every line, exactly as `$` typed inside the block does.
        if mode == Mode::VisualBlock && self.mode != Mode::VisualBlock {
            self.visual.visual_block_dollar = self.buffer().cursor().desired_col() == usize::MAX;
        }
        if self.mode.is_visual() && !mode.is_visual() {
            self.store_visual_marks();
        }
        self.mode = mode;
        if mode != Mode::Insert {
            self.editing.pending_literal = None;
            self.editing.insert_count = None;
        }
        // A mode change ends any half-typed command (count, operator,
        // prefix, character argument, register, mapping keys).
        self.input.count = None;
        self.input.input_state = InputState::Normal;
        self.input.pending_register = None;
        self.input.pending_mapping_sequence.clear();
        self.input.pending_mapping_events.clear();

        // Clear visual selection when leaving visual modes
        if !matches!(mode, Mode::Visual | Mode::VisualLine | Mode::VisualBlock) {
            self.visual.visual_start = None;
        }
    }

    /// Vim's E21, shown when an edit is refused by a `nomodifiable` buffer.
    pub(crate) fn report_unmodifiable(&mut self) {
        self.set_status_message("E21: Cannot make changes, 'modifiable' is off");
    }

    /// Reports (once) an edit that the buffer refused during the last key.
    pub(crate) fn report_refused_edit(&mut self) {
        if self.buffer_mut().take_refused_edit() {
            self.report_unmodifiable();
        }
    }

    /// Gets the dashboard selected menu index
    pub fn dashboard_selected(&self) -> usize {
        self.ui_panels.dashboard_selected
    }

    /// Sets the dashboard selected menu index
    pub fn set_dashboard_selected(&mut self, index: usize) {
        self.ui_panels.dashboard_selected = index;
    }

    /// Returns true if the dashboard should be shown
    /// Dashboard is shown when: no file loaded AND buffer is empty/default
    pub fn should_show_dashboard(&self) -> bool {
        self.mode == Mode::Dashboard
    }

    /// Returns a mutable reference to the cat animation (if active).
    pub fn cat_animation_mut(
        &mut self,
    ) -> Option<&mut Box<dyn crate::dashboard::DashboardAnimation>> {
        self.ui_panels.cat_animation.as_mut()
    }

    /// Startle the cat (e.g. on terminal resize while it's on the logo).
    pub fn startle_cat(&mut self) {
        if let Some(ref mut anim) = self.ui_panels.cat_animation {
            anim.startle();
        }
    }

    /// Set a yank flash for a linewise region (e.g. `yy`, `yj`, `yk`).
    pub fn set_yank_flash_lines(&mut self, start_line: usize, end_line: usize) {
        self.yank_flash = Some(yank_flash::YankFlash::lines(start_line, end_line));
    }

    /// Set a yank flash for a character-wise region (e.g. `yw`, `y$`).
    pub fn set_yank_flash_range(
        &mut self,
        start_line: usize,
        start_col: GraphemeCol,
        end_line: usize,
        end_col: GraphemeCol,
    ) {
        self.yank_flash = Some(yank_flash::YankFlash::range(
            start_line,
            start_col.0,
            end_line,
            end_col.0,
        ));
    }

    /// Get a reference to the current yank flash (if any).
    pub fn yank_flash(&self) -> Option<&yank_flash::YankFlash> {
        self.yank_flash.as_ref()
    }

    /// Get the last make/test output (if any).
    pub fn last_make_output(&self) -> Option<&str> {
        self.build.last_make_output.as_deref()
    }

    /// Take a pending shell command (if any) for the event loop to execute.
    pub fn take_pending_shell_command(&mut self) -> Option<build_state::PendingShellCommand> {
        self.build.pending_shell_command.take()
    }

    /// Take an interactive terminal session for the active frontend to run.
    pub fn take_pending_terminal_session(&mut self) -> Option<build_state::PendingTerminalSession> {
        self.build.pending_terminal_session.take()
    }

    /// Get the API server port.
    pub fn api_port(&self) -> Option<u16> {
        self.api_port
    }

    /// Set the API server port.
    pub fn set_api_port(&mut self, port: u16) {
        self.api_port = Some(port);
    }

    /// Get the active session name.
    pub fn active_session(&self) -> Option<&str> {
        self.active_session.as_deref()
    }

    /// Set the active session name.
    pub fn set_active_session(&mut self, name: String) {
        self.active_session = Some(name);
    }

    /// Take the active session name, leaving None.
    pub fn take_active_session(&mut self) -> Option<String> {
        self.active_session.take()
    }

    /// Set a pending LSP install awaiting user consent.
    pub fn set_pending_lsp_install(&mut self, install: PendingLspInstall) {
        self.lsp.pending_install = Some(install);
    }

    /// Check if there's an approved LSP install ready for the event loop.
    pub fn has_approved_lsp_install(&self) -> bool {
        self.lsp.approved_install.is_some()
    }

    /// Tick the yank flash. Returns true if it just expired (needs redraw to clear).
    pub fn tick_yank_flash(&mut self) -> bool {
        if let Some(ref flash) = self.yank_flash {
            if flash.is_expired() {
                self.yank_flash = None;
                return true;
            }
        }
        false
    }

    /// Tick transient toasts. Returns true if any expired toast was removed.
    pub fn tick_toasts(&mut self) -> bool {
        self.ui_panels.toast_center.prune_expired()
    }

    /// Tick the cat animation. Returns true if a frame advanced (needs redraw).
    pub fn tick_cat_animation(&mut self) -> bool {
        if let Some(ref mut anim) = self.ui_panels.cat_animation {
            if anim.is_active() {
                return anim.tick();
            }
            // Animation finished — drop it
            self.ui_panels.cat_animation = None;
        }
        false
    }

    /// Advance the AI chat working spinner. Returns true when its visible
    /// frame changed and the TUI needs another render without user input.
    pub fn tick_ai_chat_working_animation(&mut self) -> bool {
        if self.mode() != Mode::AiChat || !self.ai_chat_waiting() {
            return false;
        }
        let tick = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            / 80;
        if self.render_cache.ai_chat_working_animation_tick == tick {
            return false;
        }
        self.render_cache.ai_chat_working_animation_tick = tick;
        true
    }

    pub fn ai_chat_working_animation_frame(&self) -> usize {
        (self.render_cache.ai_chat_working_animation_tick % 8) as usize
    }

    /// Returns whether the editor should quit
    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    /// Sets the quit flag
    pub fn quit(&mut self) {
        self.should_quit = true;
    }

    /// Quit with a specific exit code (used by :cq)
    pub fn quit_with_code(&mut self, code: i32) {
        self.should_quit = true;
        self.exit_code = code;
    }

    /// Returns the exit code (0 = success, non-zero = error)
    pub fn exit_code(&self) -> i32 {
        self.exit_code
    }

    /// Gets the git branch name for the current file
    pub fn git_branch(&self) -> Option<&str> {
        self.git_branch.as_deref()
    }
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}
