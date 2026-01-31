use crate::editor::{Editor, InputState};
use crate::mode::Mode;
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};

/// Command handling submodule
mod commands;

/// Shell command expansion (%, #, modifiers)
pub mod shell_expansion;

/// Number operations (Ctrl-A, Ctrl-X, g Ctrl-A, g Ctrl-X)
mod numbers;

/// Case operations (toggle, upper, lower)
mod case;

/// Helper functions for cursor movement and editing
mod helpers;

/// Character motion handler (f, t, F, T, r, m, ', `) - new state machine
mod char_motion;

/// Leader sequence handler (<Space>...) - new state machine
mod leader;

/// Search mode handler (/, ?)
mod search_mode;

/// Replace mode handler (R)
mod replace_mode;

/// Picker mode handler (file finder, grep, code actions)
mod picker_mode;

/// Hover mode handlers (preview and navigate)
mod hover_mode;

/// File tree mode handler
mod filetree_mode;

/// Substitute confirm mode handler
mod substitute_mode;

/// Dashboard mode handler
mod dashboard_mode;

/// LSP Manager mode handler
mod lsp_manager_mode;

/// Rename input mode handler
mod rename_input_mode;

/// Mouse event handler (click, drag, scroll)
pub mod mouse;

/// Insert mode handler
mod insert_mode;

/// Visual mode handler (Visual, VisualLine, VisualBlock)
mod visual_mode;

/// Normal mode handler (decomposed into submodules)
mod normal;

/// Handles input events for the editor
pub struct InputHandler;

impl InputHandler {
    /// Processes a keyboard event and marks the editor dirty.
    /// Use this for single-event callers that want automatic dirty marking.
    pub fn handle_key_event(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
        Self::handle_key_event_no_dirty(editor, key_event)?;
        editor.mark_dirty();
        Ok(())
    }

    /// Processes a keyboard event without marking the editor dirty.
    /// Use this for batch processing where dirty should be marked once at the end.
    pub fn handle_key_event_no_dirty(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
        // Record the event if we're recording a macro
        // (but don't record the 'q' that stops recording)
        let should_record_macro = editor.is_recording_macro()
            && !(key_event.code == KeyCode::Char('q') && editor.mode() == Mode::Normal);

        if should_record_macro {
            editor.record_macro_event(key_event);
        }

        // Global keybindings (work in any mode)
        // Cmd+1 - toggle file tree
        if key_event.code == KeyCode::Char('1')
            && key_event.modifiers.contains(KeyModifiers::SUPER)
        {
            editor.toggle_file_tree();
            return Ok(());
        }

        let result = match editor.mode() {
            Mode::Normal => Self::handle_normal_mode(editor, key_event),
            Mode::Insert => insert_mode::handle_insert_mode(editor, key_event),
            Mode::Visual | Mode::VisualLine | Mode::VisualBlock => {
                visual_mode::handle_visual_mode(editor, key_event)
            }
            Mode::Command => commands::handle_command_mode(editor, key_event),
            Mode::Search => search_mode::handle_search_mode(editor, key_event),
            Mode::Replace => replace_mode::handle_replace_mode(editor, key_event),
            Mode::Picker => picker_mode::handle_picker_mode(editor, key_event),
            Mode::HoverPreview => {
                // HoverPreview may forward keys to normal mode
                if let Some(forwarded_key) = hover_mode::handle_hover_preview_mode(editor, key_event)? {
                    Self::handle_normal_mode(editor, forwarded_key)?;
                }
                Ok(())
            }
            Mode::HoverNavigate => hover_mode::handle_hover_navigate_mode(editor, key_event),
            Mode::FileTree => filetree_mode::handle_filetree_mode(editor, key_event),
            Mode::SubstituteConfirm => substitute_mode::handle_substitute_confirm_mode(editor, key_event),
            Mode::Dashboard => dashboard_mode::handle_dashboard_mode(editor, key_event),
            Mode::LspManager => lsp_manager_mode::handle_lsp_manager_mode(editor, key_event),
            Mode::RenameInput => rename_input_mode::handle_rename_input_mode(editor, key_event),
        };

        // Update scroll offset to keep cursor visible with scrolloff margin
        // Skip if:
        // 1. Viewport commands (zz, zt, zb) explicitly set scroll position
        // 2. There's a pending viewport command (e.g., 'z' waiting for 't')
        //    This prevents scroll changes between multi-key sequences like 'zt'
        let is_viewport_pending = matches!(editor.pending_command(), Some('z') | Some('Z'));
        if !editor.skip_scroll_update && !is_viewport_pending {
            editor.update_scroll_offset();
        } else {
            // Reset flag for next key event
            editor.skip_scroll_update = false;
        }

        result
    }

    /// Handles input in Normal mode
    fn handle_normal_mode(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
        // =====================================================================
        // STATE MACHINE DISPATCH
        // =====================================================================
        // Check the InputState first. This handles states that were
        // previously causing collisions (e.g., <Space>t vs t motion).
        match editor.input_state().clone() {
            InputState::AwaitingChar { motion, operator } => {
                // Handle f/t/F/T/r/m/'/` second character
                return char_motion::handle_char_motion(editor, key_event, motion, operator);
            }
            InputState::Leader { ref keys } => {
                // Handle leader sequences (<Space>...)
                let keys_clone = keys.clone();
                return leader::handle_leader_input(editor, key_event, &keys_clone);
            }
            InputState::Normal => {
                // Fall through to normal mode dispatcher
            }
            _ => {
                // For unhandled states, reset and fall through
                editor.reset_input_state();
            }
        }

        // =====================================================================
        // LEGACY LEADER HANDLING (pending_leader flag)
        // =====================================================================

        // Handle pending leader key sequences (e.g., <Space>sf, <Space>sg, <Space>ca)
        if editor.pending_leader() {
            editor.set_pending_leader(false);

            match key_event.code {
                KeyCode::Char('s') => {
                    // Expect 'f' or 'g' next
                    editor.set_pending_command('s');
                    return Ok(());
                }
                KeyCode::Char('c') => {
                    // Expect 'a' or 'i'/'o' next for code actions or call hierarchy
                    editor.set_pending_command('c');
                    return Ok(());
                }
                KeyCode::Char('o') => {
                    // <Space>o - Document outline (symbols)
                    editor.request_document_symbols();
                    return Ok(());
                }
                KeyCode::Char('S') => {
                    // <Space>S - Workspace symbols
                    editor.request_workspace_symbols();
                    return Ok(());
                }
                KeyCode::Char('t') => {
                    // Expect 'h' next for type hierarchy
                    editor.set_pending_command('t');
                    return Ok(());
                }
                KeyCode::Char('i') => {
                    // <Space>i - Organize imports
                    editor.request_organize_imports();
                    return Ok(());
                }
                _ => {
                    // Cancel leader sequence
                    return Ok(());
                }
            }
        }

        // Handle second key in leader sequences
        if let Some('s') = editor.pending_command() {
            editor.clear_pending_command();

            match key_event.code {
                KeyCode::Char('f') => {
                    // <Space>sf - Find files
                    let base_dir =
                        std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
                    let picker = crate::editor::Picker::new_file_finder(base_dir);
                    editor.set_picker(picker);
                    editor.set_mode(Mode::Picker);
                    editor.mark_picker_selection_changed();
                    return Ok(());
                }
                KeyCode::Char('g') => {
                    // <Space>sg - Live grep
                    let base_dir =
                        std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
                    let picker = crate::editor::Picker::new_live_grep(base_dir);
                    editor.set_picker(picker);
                    editor.set_mode(Mode::Picker);
                    // Note: live grep starts empty, so no preview to preload
                    return Ok(());
                }
                _ => {
                    // Invalid sequence
                    return Ok(());
                }
            }
        }

        // Handle 'c' prefix for leader sequences (code actions, call hierarchy)
        if let Some('c') = editor.pending_command() {
            editor.clear_pending_command();

            match key_event.code {
                KeyCode::Char('a') => {
                    // <Space>ca - Code actions
                    editor.request_code_actions();
                    return Ok(());
                }
                KeyCode::Char('i') => {
                    // <Space>ci - Call hierarchy incoming
                    editor.request_call_hierarchy_incoming();
                    return Ok(());
                }
                KeyCode::Char('o') => {
                    // <Space>co - Call hierarchy outgoing
                    editor.request_call_hierarchy_outgoing();
                    return Ok(());
                }
                _ => {
                    // Invalid sequence
                    return Ok(());
                }
            }
        }

        // Handle 't' prefix for leader sequences (type hierarchy)
        if let Some('t') = editor.pending_command() {
            editor.clear_pending_command();

            match key_event.code {
                KeyCode::Char('h') => {
                    // <Space>th - Type hierarchy
                    editor.request_type_hierarchy();
                    return Ok(());
                }
                _ => {
                    // Invalid sequence
                    return Ok(());
                }
            }
        }

        // =====================================================================
        // DELEGATE TO NORMAL MODE DISPATCHER
        // =====================================================================
        // All other normal mode handling is in the normal/ submodule
        normal::handle_normal_mode(editor, key_event)
    }

    // Removed ~3,100 lines of legacy normal mode handlers.
    // Now handled by normal/ submodule with focused handlers:
    // - normal/operators.rs       - Operator+motion combos (dd, dw, yy, cc, etc.)
    // - normal/text_objects.rs    - Text objects (diw, ci", dap, etc.)
    // - normal/pending_commands.rs - Multi-key sequences (g*, z*, m*, etc.)
    // - normal/mode_transitions.rs - Mode switches (i, a, v, :, etc.)
    // - normal/editing_commands.rs - Direct edits (x, D, p, J, u, etc.)
    // - normal/motions_input.rs   - Motions (h, j, k, l, w, b, G, etc.)

    /// Polls for the next event
    pub fn poll_event() -> Result<Option<Event>> {
        // Use a very short timeout to keep the event loop responsive
        // This allows status updates and rendering to happen frequently
        if event::poll(std::time::Duration::from_millis(16))? {
            // ~60 FPS
            Ok(Some(event::read()?))
        } else {
            Ok(None)
        }
    }

    /// Polls and returns all available events.
    /// First poll with 16ms timeout, then drain remaining with 0ms (non-blocking).
    /// This batches multiple rapid events (e.g., paste) into a single render cycle.
    pub fn poll_all_events() -> Result<Vec<Event>> {
        let mut events = Vec::new();

        // First poll with timeout (matches poll_event behavior)
        if event::poll(std::time::Duration::from_millis(16))? {
            events.push(event::read()?);

            // Drain remaining events with 0ms timeout (non-blocking)
            while event::poll(std::time::Duration::from_millis(0))? {
                events.push(event::read()?);
            }
        }

        Ok(events)
    }

    /// Wrapper to call commands module's execute_command_string
    pub fn execute_command_string(editor: &mut Editor, command: &str) -> Result<()> {
        commands::execute_command_string(editor, command)
    }

    /// Wrapper to call commands module's handle_command_mode
    pub fn handle_command_mode_wrapper(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
        commands::handle_command_mode(editor, key_event)
    }

    /// Wrapper to call commands module's parse_range
    pub fn parse_range_wrapper(editor: &Editor, range_str: &str) -> Option<(usize, usize)> {
        commands::parse_range(editor, range_str)
    }
}
