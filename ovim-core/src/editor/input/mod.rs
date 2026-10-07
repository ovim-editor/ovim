use crate::editor::{Editor, InputState, MapMode, Nesting};
use crate::mode::Mode;
use crate::{KeyCode, KeyEvent, Modifiers};
use anyhow::Result;

/// Command handling submodule
mod commands;

/// Shell command expansion (%, #, modifiers)
pub mod shell_expansion;

/// Number operations (Ctrl-A, Ctrl-X, g Ctrl-A, g Ctrl-X)
mod numbers;

/// Case operations (toggle, upper, lower)
mod case;

/// Helper functions for cursor movement and editing
pub(crate) mod helpers;

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

mod debug_keys;
mod debug_panel_mode;
/// LSP Manager mode handler
mod lsp_manager_mode;
mod run_console_mode;
mod search_replace_mode;

/// Rename input mode handler
mod rename_input_mode;

/// AI chat mode handler
mod ai_chat_mode;

/// Modal code-walkthrough input policy
mod code_explanation_mode;

/// Mouse event handler (click, drag, scroll)
pub mod mouse;

/// Insert mode handler
mod insert_mode;

/// Visual mode handler (Visual, VisualLine, VisualBlock)
mod visual_mode;

/// Shared text object key decoding for operators and visual mode.
mod text_objects;

/// Normal mode handler (decomposed into submodules)
mod normal;

/// Handles input events for the editor
pub struct InputHandler;

const MAX_MAPPING_REMAP_DEPTH: usize = 32;

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
        editor.with_execution_scope(|editor| {
            Self::handle_key_event_internal(editor, key_event, true, true, 0)
        })
    }

    fn handle_key_event_internal(
        editor: &mut Editor,
        key_event: KeyEvent,
        allow_remap: bool,
        record_macro: bool,
        remap_depth: usize,
    ) -> Result<()> {
        editor.dismiss_blame_mouse_hover();
        // Record the event if we're recording a macro (but don't record the 'q'
        // that stops recording). A 'q' only terminates recording when it reaches
        // the terminator branch — i.e. Normal mode with no pending
        // operator/command/register/input state. A 'q' typed as an *argument*
        // (fq, rq, mq, `q, "q, @q, …) must still be recorded, otherwise replaying
        // the macro silently drops it.
        let is_recording_terminator = key_event.code == KeyCode::Char('q')
            && editor.mode() == Mode::Normal
            && editor.input_state().is_normal()
            && editor.pending_register().is_none();
        let should_record_macro =
            record_macro && editor.is_recording_macro() && !is_recording_terminator;

        if should_record_macro {
            editor.record_macro_event(key_event);
        }

        // Intercept input when LSP install consent dialog is showing
        if editor.has_pending_lsp_install() {
            match key_event.code {
                KeyCode::Enter => {
                    // Approve install (once)
                    editor.resolve_pending_lsp_install(crate::editor::LspInstallConsent::Yes);
                }
                KeyCode::Char('a') | KeyCode::Char('A') => {
                    // Always auto-install (sets autoinstall=auto)
                    editor.resolve_pending_lsp_install(crate::editor::LspInstallConsent::Always);
                }
                KeyCode::Esc => {
                    // Skip install
                    editor.resolve_pending_lsp_install(crate::editor::LspInstallConsent::No);
                }
                _ => {} // Ignore other keys while dialog is showing
            }
            return Ok(());
        }

        // Ovim-owned Codex sign-in is global: it can block both chat and
        // selection inference, regardless of the editor mode underneath.
        if editor.has_codex_auth_dialog() {
            editor.handle_codex_auth_key(key_event);
            return Ok(());
        }

        code_explanation_mode::restore_owning_mode(editor);

        // Global keybindings (work in any mode)
        // Cmd+1 - toggle file tree
        if key_event.code == KeyCode::Char('1') && key_event.modifiers.contains(Modifiers::SUPER) {
            editor.toggle_file_tree();
            return Ok(());
        }

        let fold_prev = {
            let cursor = editor.buffer().cursor();
            (cursor.line(), cursor.col(), editor.buffer().version())
        };
        let completing_insert_normal = editor.editing.insert_normal_pending;
        let repeat_checkpoint = Self::repeat_checkpoint(editor);
        let register_before = Self::register_before_command(editor);
        let mapping_handled = if allow_remap {
            Self::try_handle_mode_mapping(editor, key_event, remap_depth)?
        } else {
            false
        };

        let result = if mapping_handled {
            Ok(())
        } else {
            match editor.mode() {
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
                    if let Some(forwarded_key) =
                        hover_mode::handle_hover_preview_mode(editor, key_event)?
                    {
                        Self::handle_normal_mode(editor, forwarded_key)?;
                    }
                    Ok(())
                }
                Mode::HoverNavigate => hover_mode::handle_hover_navigate_mode(editor, key_event),
                Mode::FileTree => filetree_mode::handle_filetree_mode(editor, key_event),
                Mode::SubstituteConfirm => {
                    substitute_mode::handle_substitute_confirm_mode(editor, key_event)
                }
                Mode::Dashboard => dashboard_mode::handle_dashboard_mode(editor, key_event),
                Mode::LspManager => lsp_manager_mode::handle_lsp_manager_mode(editor, key_event),
                Mode::RenameInput => rename_input_mode::handle_rename_input_mode(editor, key_event),
                Mode::AiChat => ai_chat_mode::handle_ai_chat_mode(editor, key_event),
                Mode::RunConsole => run_console_mode::handle_run_console_mode(editor, key_event),
                Mode::DebugPanel => debug_panel_mode::handle_debug_panel_mode(editor, key_event),
                Mode::SearchReplace => {
                    search_replace_mode::handle_search_replace_mode(editor, key_event)
                }
            }
        };

        if let Some(checkpoint) = repeat_checkpoint.filter(|_| !mapping_handled) {
            Self::forget_unrecorded_change(editor, checkpoint);
        }
        if let Some(register) = register_before.filter(|_| !mapping_handled) {
            Self::drop_unused_register(editor, register);
        }

        // Ctrl-O insert-normal: after one normal command, return to insert mode.
        // Only return if we're still in Normal mode (the command didn't switch to
        // Insert, Visual, Command, etc. on its own) and no pending multi-key state.
        if completing_insert_normal
            && editor.editing.insert_normal_pending
            && editor.mode() == Mode::Normal
        {
            // Check if the command is fully resolved (no pending operator/command)
            if editor.input_state().is_normal()
                && editor.pending_register().is_none()
                && editor.count().is_none()
            {
                editor.editing.insert_normal_pending = false;
                Self::restore_insert_position_at_eol(editor);
                editor.start_change_building(editor.cursor_position());
                editor.set_mode(Mode::Insert);
            }
        } else if completing_insert_normal && editor.mode() == Mode::Insert {
            // Commands such as `c` and `i` opened their own insert session.
            editor.editing.insert_normal_pending = false;
        }

        // Update scroll offset to keep cursor visible with scrolloff margin
        // Skip if:
        // 1. Viewport commands (zz, zt, zb) explicitly set scroll position
        // 2. There's a pending viewport command (e.g., 'z' waiting for 't')
        //    This prevents scroll changes between multi-key sequences like 'zt'
        // When the mapping layer handled this key it already ran the scroll
        // update on the inner (replayed) dispatch, consuming the one-shot viewport policy.
        // Re-running it here would scroll a second time and, worse, undo a
        // deliberate sub-row scroll (e.g. Ctrl-E) whose cursor sits off-screen by
        // design. Only run the post-command scroll update at this (outer) level
        // when we handled the key directly.
        // Folds: keep ranges aligned with the text, keep the cursor out of
        // closed folds, refresh the header markers.
        editor.sync_folds_after_key(fold_prev.0, fold_prev.1, fold_prev.2);
        editor.follow_breakpoints_through_edits();
        editor.report_refused_edit();

        let is_viewport_pending = matches!(
            editor.input_state(),
            InputState::ZPrefix | InputState::QuitPrefix
        );
        let preserve_viewport = editor.viewport.take_preserve_after_input();
        if !preserve_viewport && !is_viewport_pending && !mapping_handled {
            editor.update_scroll_offset();
        }

        // Safety net: ensure cursor is within buffer bounds after every key event.
        // Individual motions/operators should maintain this invariant themselves, but
        // this catch-all prevents any cursor-out-of-bounds state from persisting.
        //
        // Skip in Insert/Replace modes: `validate_cursor_position` uses Normal mode
        // semantics (cursor must be ON a character), but Insert mode legitimately
        // allows cursor at `line_len` (the append position, e.g. after `A`).
        //
        // VisualBlock also legitimately allows the cursor beyond a short line's
        // end (the block column). Clamping the column there collapses the block
        // over short lines and corrupts a subsequent block delete/yank, so only
        // clamp the line for VisualBlock and leave the column (and its goal)
        // intact.
        match editor.mode() {
            Mode::Insert | Mode::Replace => {}
            Mode::Visual => {
                // Characterwise selections may end on the newline cell.
                editor.buffer_mut().validate_cursor_line();
                let cursor = editor.buffer().cursor();
                let max_col = editor
                    .buffer()
                    .line_text(cursor.line())
                    .map(|text| crate::unicode::grapheme_count(&text))
                    .unwrap_or(0);
                if cursor.col().0 > max_col {
                    editor
                        .buffer_mut()
                        .cursor_mut()
                        .set_col(crate::unicode::GraphemeCol(max_col));
                }
            }
            Mode::VisualBlock => editor.buffer_mut().validate_cursor_line(),
            _ => editor.buffer_mut().validate_cursor_position(),
        }

        // Ctrl-O at the end of a line: the clamped cursor keeps the column it left from
        // as its goal, so a following `j`/`k` can land past the end of a longer line.
        if let Some(goal) = editor.editing.insert_normal_eol_goal.take() {
            let cursor = editor.buffer_mut().cursor_mut();
            if cursor.desired_col() < goal {
                cursor.update_desired_col(crate::unicode::GraphemeCol(goal));
            }
        }

        result
    }

    /// Where dot-repeat bookkeeping stood before a Normal/Visual-mode key:
    /// the buffer and its [`ChangeManager::repeat_checkpoint`]. `None` for keys
    /// whose edits are not "a command" (Insert mode typing, `:` commands, leader
    /// sequences, macro playback — the commands they run are checked themselves).
    fn repeat_checkpoint(editor: &Editor) -> Option<(usize, (u64, u64))> {
        let tracked_mode = matches!(
            editor.mode(),
            Mode::Normal | Mode::Visual | Mode::VisualLine | Mode::VisualBlock
        );
        let tracked_state = !matches!(
            editor.input_state(),
            InputState::Leader { .. }
                | InputState::MacroPrefix {
                    is_recording: false
                }
        );
        (tracked_mode && tracked_state).then(|| {
            (
                editor.current_buffer_index(),
                editor.buffer().change_manager().repeat_checkpoint(),
            )
        })
    }

    /// Back to Insert mode after Ctrl-O: the temporary Normal mode cannot rest past
    /// the end of a line, so a cursor that was there (and stayed on that line) or that
    /// `$`/`j` left with a goal column beyond the text, sitting on the last character,
    /// returns to just past it (vim's `ins_at_eol`) instead of before that character.
    fn restore_insert_position_at_eol(editor: &mut Editor) {
        let eol_line = editor.editing.insert_normal_eol_line.take();
        let cursor = editor.buffer().cursor();
        let (line, col) = (cursor.line(), cursor.col().0);
        let wants_eol = eol_line == Some(line) || cursor.desired_col() > col;
        let len = editor.buffer().line_index(line).grapheme_count();
        if wants_eol && col + 1 == len {
            editor
                .buffer_mut()
                .cursor_mut()
                .set_col_preserve_desired(crate::unicode::GraphemeCol(len));
        }
    }

    /// The register typed with `"x` that a Normal-mode command is about to run with,
    /// if the key about to be handled is a command of its own (not the register name
    /// itself, not a pending prefix's argument).
    fn register_before_command(editor: &Editor) -> Option<char> {
        (editor.mode() == Mode::Normal && editor.input_state().is_normal())
            .then(|| editor.pending_register())
            .flatten()
    }

    /// `"x` belongs to the command that follows it. When that command completed
    /// without taking the register (Esc, a plain motion, a cancelled operator), it
    /// must not linger and apply to the next, unrelated command.
    fn drop_unused_register(editor: &mut Editor, register: char) {
        let finished = editor.mode() == Mode::Normal
            && editor.input_state().is_normal()
            && editor.count().is_none()
            && !editor.has_pending_mapping();
        if finished && editor.pending_register() == Some(register) {
            editor.clear_pending_register();
        }
    }

    /// A key that edited the buffer without defining what `.` repeats must not
    /// leave an older change behind to be replayed instead (`ddgUwj.` repeating
    /// the `dd`).
    fn forget_unrecorded_change(editor: &mut Editor, (buffer, checkpoint): (usize, (u64, u64))) {
        if editor.current_buffer_index() == buffer {
            editor
                .buffer_mut()
                .change_manager_mut()
                .forget_unrecorded_change(checkpoint);
        }
    }

    fn active_mapping_mode(editor: &Editor) -> Option<MapMode> {
        match editor.mode() {
            Mode::Normal => Some(MapMode::Normal),
            Mode::Dashboard => Some(MapMode::Normal),
            Mode::Insert => Some(MapMode::Insert),
            Mode::Visual | Mode::VisualLine | Mode::VisualBlock => Some(MapMode::Visual),
            Mode::Command => Some(MapMode::Command),
            _ => None,
        }
    }

    fn is_mapping_context(editor: &Editor) -> bool {
        editor.pending_mapping_sequence().is_empty()
            && editor.count().is_none()
            && editor.input_state().is_normal()
            && editor.pending_register().is_none()
    }

    fn try_handle_mode_mapping(
        editor: &mut Editor,
        key_event: KeyEvent,
        remap_depth: usize,
    ) -> Result<bool> {
        let Some(map_mode) = Self::active_mapping_mode(editor) else {
            editor.clear_pending_mapping();
            return Ok(false);
        };

        if !editor.has_pending_mapping() && !Self::is_mapping_context(editor) {
            return Ok(false);
        }

        let Some(encoded_key) = Self::encode_key_for_mapping_lookup(key_event) else {
            if editor.has_pending_mapping() {
                let mut replay = editor.take_pending_mapping_events();
                replay.push(key_event);
                for event in replay {
                    Self::handle_key_event_internal(editor, event, false, false, remap_depth)?;
                }
                return Ok(true);
            }
            return Ok(false);
        };

        editor.append_pending_mapping(&encoded_key, key_event);
        let sequence = editor.pending_mapping_sequence().to_string();

        if let Some(mapping) = editor.keymaps().get_mapping(map_mode, &sequence).cloned() {
            editor.clear_pending_mapping();
            if remap_depth >= MAX_MAPPING_REMAP_DEPTH {
                editor.set_status_message("Mapping recursion limit reached".to_string());
                return Ok(true);
            }

            Self::execute_mapping_rhs(editor, &mapping.rhs, !mapping.noremap, remap_depth + 1)?;
            return Ok(true);
        }

        if editor.keymaps().has_prefix(map_mode, &sequence) {
            return Ok(true);
        }

        let replay = editor.take_pending_mapping_events();
        for event in replay {
            Self::handle_key_event_internal(editor, event, false, false, remap_depth)?;
        }
        Ok(true)
    }

    fn execute_mapping_rhs(
        editor: &mut Editor,
        rhs: &str,
        allow_remap: bool,
        remap_depth: usize,
    ) -> Result<()> {
        let events = Self::decode_mapping_rhs(rhs);
        for event in events {
            Self::handle_key_event_internal(editor, event, allow_remap, false, remap_depth)?;
        }
        Ok(())
    }

    fn encode_key_for_mapping_lookup(key_event: KeyEvent) -> Option<String> {
        if key_event.modifiers.contains(Modifiers::SUPER)
            || key_event.modifiers.contains(Modifiers::ALT)
        {
            return None;
        }

        match key_event.code {
            KeyCode::Char(c) => {
                if key_event.modifiers.contains(Modifiers::CONTROL) {
                    if c.is_ascii_alphabetic() {
                        let ctrl = ((c.to_ascii_lowercase() as u8) - b'a' + 1) as char;
                        return Some(ctrl.to_string());
                    }
                    return None;
                }

                if key_event.modifiers == Modifiers::NONE || key_event.modifiers == Modifiers::SHIFT
                {
                    Some(c.to_string())
                } else {
                    None
                }
            }
            KeyCode::Enter if key_event.modifiers == Modifiers::NONE => Some("\n".to_string()),
            KeyCode::Esc if key_event.modifiers == Modifiers::NONE => Some("\x1b".to_string()),
            KeyCode::Tab if key_event.modifiers == Modifiers::NONE => Some("\t".to_string()),
            KeyCode::Backspace if key_event.modifiers == Modifiers::NONE => {
                Some("\x7f".to_string())
            }
            KeyCode::Up if key_event.modifiers == Modifiers::NONE => Some("\x1b[A".to_string()),
            KeyCode::Down if key_event.modifiers == Modifiers::NONE => Some("\x1b[B".to_string()),
            KeyCode::Right if key_event.modifiers == Modifiers::NONE => Some("\x1b[C".to_string()),
            KeyCode::Left if key_event.modifiers == Modifiers::NONE => Some("\x1b[D".to_string()),
            _ => None,
        }
    }

    fn decode_mapping_rhs(rhs: &str) -> Vec<KeyEvent> {
        let chars: Vec<char> = rhs.chars().collect();
        let mut result = Vec::new();
        let mut i = 0usize;

        while i < chars.len() {
            let ch = chars[i];
            match ch {
                '\n' => {
                    result.push(KeyEvent::new(KeyCode::Enter, Modifiers::NONE));
                    i += 1;
                }
                '\t' => {
                    result.push(KeyEvent::new(KeyCode::Tab, Modifiers::NONE));
                    i += 1;
                }
                '\x7f' => {
                    result.push(KeyEvent::new(KeyCode::Backspace, Modifiers::NONE));
                    i += 1;
                }
                '\x1b' => {
                    if i + 2 < chars.len() && chars[i + 1] == '[' {
                        let arrow_key = match chars[i + 2] {
                            'A' => Some(KeyCode::Up),
                            'B' => Some(KeyCode::Down),
                            'C' => Some(KeyCode::Right),
                            'D' => Some(KeyCode::Left),
                            _ => None,
                        };
                        if let Some(code) = arrow_key {
                            result.push(KeyEvent::new(code, Modifiers::NONE));
                            i += 3;
                            continue;
                        }
                    }
                    result.push(KeyEvent::new(KeyCode::Esc, Modifiers::NONE));
                    i += 1;
                }
                c if c.is_ascii() => {
                    let byte = c as u8;
                    if (1..=26).contains(&byte) {
                        let ctrl_char = (byte - 1 + b'a') as char;
                        result.push(KeyEvent::new(KeyCode::Char(ctrl_char), Modifiers::CONTROL));
                    } else {
                        result.push(KeyEvent::new(KeyCode::Char(c), Modifiers::NONE));
                    }
                    i += 1;
                }
                c => {
                    result.push(KeyEvent::new(KeyCode::Char(c), Modifiers::NONE));
                    i += 1;
                }
            }
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
            // Operators and prefixes are resolved by the normal/ dispatcher.
            _ => {}
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

    /// Wrapper to call commands module's execute_command_string
    pub fn execute_command_string(editor: &mut Editor, command: &str) -> Result<()> {
        commands::execute_command_string(editor, command)
    }

    /// Execute a command line for the headless API / GUI: the same
    /// dispatcher as the `:` prompt, returning the result instead of showing
    /// it. See [`crate::commands::execute_command_api`].
    pub fn execute_command_api(
        editor: &mut Editor,
        command: &str,
    ) -> crate::command_result::CommandResult {
        crate::commands::execute_command_api(editor, command)
    }

    /// Wrapper to call commands module's handle_command_mode
    pub fn handle_command_mode_wrapper(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
        editor.with_execution_scope(|editor| commands::handle_command_mode(editor, key_event))
    }

    /// Type `keys` as if in Normal mode, for `:normal`. Each character is
    /// one key (`\x1b` is <Esc>, `\r`/`\n` <Enter>, `\t` <Tab>). An
    /// unfinished command or mode is ended with <Esc>, as vim does.
    /// `remap` is false for `:normal!`.
    pub(crate) fn type_normal_keys(editor: &mut Editor, keys: &str, remap: bool) -> Result<()> {
        editor
            .nested(Nesting::Normal, |editor| {
                Self::type_normal_keys_nested(editor, keys, remap)
            })
            .unwrap_or_else(|| Err(anyhow::anyhow!("E192: Recursive use of :normal too deep")))
    }

    fn type_normal_keys_nested(editor: &mut Editor, keys: &str, remap: bool) -> Result<()> {
        if editor.mode() != Mode::Normal {
            Self::handle_key_event_internal(
                editor,
                KeyEvent::new(KeyCode::Esc, Modifiers::NONE),
                false,
                false,
                0,
            )?;
        }
        for ch in keys.chars() {
            let code = match ch {
                '\x1b' => KeyCode::Esc,
                '\r' | '\n' => KeyCode::Enter,
                '\t' => KeyCode::Tab,
                '\x08' | '\x7f' => KeyCode::Backspace,
                ch => KeyCode::Char(ch),
            };
            Self::handle_key_event_internal(
                editor,
                KeyEvent::new(code, Modifiers::NONE),
                remap,
                false,
                0,
            )?;
        }
        // Leave whatever the keys started: an operator waiting for a motion,
        // Insert mode, a half-typed command line.
        for _ in 0..3 {
            if editor.mode() == Mode::Normal && editor.input_state().is_normal() {
                break;
            }
            Self::handle_key_event_internal(
                editor,
                KeyEvent::new(KeyCode::Esc, Modifiers::NONE),
                false,
                false,
                0,
            )?;
        }
        editor.mark_dirty();
        Ok(())
    }
}
