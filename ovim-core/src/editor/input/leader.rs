//! Leader sequence handler for <Space>-prefixed commands.
//!
//! This module handles the Leader input state, processing commands like:
//! - `<Space>th` - Type hierarchy (LSP)
//! - `<Space>ca` - Code actions (LSP)
//! - `<Space>e` - Show diagnostic at cursor
//! - `<Space>o` - Document outline
//! - etc.

use crate::{KeyCode, KeyEvent};
use anyhow::Result;

use crate::editor::input_state::InputState;
use crate::editor::{Editor, Picker};
use crate::mode::Mode;

/// Handles input when in Leader state.
///
/// Called when the editor is in `InputState::Leader { keys }` state.
/// The `keys` vector contains any keys already pressed after the leader.
pub fn handle_leader_input(editor: &mut Editor, key: KeyEvent, keys: &[char]) -> Result<()> {
    // Handle Escape - cancel leader sequence
    if key.code == KeyCode::Esc {
        editor.reset_input_state();
        return Ok(());
    }

    let KeyCode::Char(c) = key.code else {
        // Non-character key - cancel
        editor.reset_input_state();
        return Ok(());
    };

    if keys.is_empty() {
        // First key after leader
        handle_first_leader_key(editor, c)
    } else {
        // Subsequent key in sequence
        handle_leader_sequence(editor, keys, c)
    }
}

/// Handles the first key after leader (<Space>).
fn handle_first_leader_key(editor: &mut Editor, key: char) -> Result<()> {
    match key {
        // Single-key commands
        'e' => {
            // <Space>e - Show diagnostic at cursor (like vim.diagnostic.open_float())
            editor.show_diagnostic_at_cursor();
            editor.reset_input_state();
        }
        'o' => {
            // <Space>o - Document outline (symbol tree)
            editor.open_outline_picker();
            editor.reset_input_state();
        }
        'S' => {
            // <Space>S - Workspace symbols
            editor.request_workspace_symbols();
            editor.reset_input_state();
        }
        'i' => {
            // <Space>i - Organize imports
            editor.request_organize_imports();
            editor.reset_input_state();
        }

        // AI chat commands
        ' ' => {
            // <Space><Space> - Open AI chat
            editor.open_ai_chat(crate::ai::chat_types::ChatOpts {
                name: "chat".into(),
                allow_edits: true,
                ..Default::default()
            })?;
            editor.reset_input_state();
        }
        '?' => {
            // <Space>? - Open AI query (read-only)
            editor.open_ai_chat(crate::ai::chat_types::ChatOpts {
                name: "query".into(),
                profile: editor.ai_chat_context_profile("query"),
                allow_edits: false,
                ..Default::default()
            })?;
            editor.reset_input_state();
        }

        // Multi-key sequences - accumulate the key
        'd' => {
            // <Space>d... - Debug prefix
            editor.set_input_state(InputState::Leader { keys: vec!['d'] });
        }
        'r' => {
            // <Space>r... - Run prefix (rr run, rd debug, rl rerun, rc configs, rs stop, rt/rf console)
            editor.set_input_state(InputState::Leader { keys: vec!['r'] });
        }
        'l' => {
            // <Space>l... - LSP manager prefix
            editor.set_input_state(InputState::Leader { keys: vec!['l'] });
        }
        't' => {
            // <Space>t... - Tests (tn/tf/ta/ts/tl/tv/tt/to) + type hierarchy (th)
            editor.set_input_state(InputState::Leader { keys: vec!['t'] });
        }
        'c' => {
            // <Space>c... - Code actions/call hierarchy prefix
            editor.set_input_state(InputState::Leader { keys: vec!['c'] });
        }
        's' => {
            // <Space>s... - Search prefix
            editor.set_input_state(InputState::Leader { keys: vec!['s'] });
        }
        'g' => {
            // <Space>g... - Git prefix (gd diff review, gf fetch base)
            editor.set_input_state(InputState::Leader { keys: vec!['g'] });
        }

        // Unknown key - cancel sequence
        _ => {
            editor.reset_input_state();
        }
    }

    Ok(())
}

/// Handles subsequent keys in a leader sequence.
fn handle_leader_sequence(editor: &mut Editor, keys: &[char], next_key: char) -> Result<()> {
    match (keys, next_key) {
        // <Space>d... sequences (debug)
        (&['d'], 'b') => {
            // <Space>db - Toggle breakpoint at cursor line
            editor.toggle_breakpoint();
            editor.reset_input_state();
        }
        (&['d'], 'c') => {
            // <Space>dc - Continue (if a session is active) or debug whatever
            // is at the cursor (same path as F5).
            if editor.is_debug_active() {
                editor
                    .dap_manager_mut()
                    .queue(crate::dap::PendingDebugAction::Continue);
            } else {
                editor.launch_at_cursor(crate::launch::LaunchMode::Debug);
            }
            editor.reset_input_state();
        }
        (&['d'], 'C') => {
            // <Space>dC - Pick a run configuration to debug
            editor.launch_pick_config(crate::launch::LaunchMode::Debug);
            editor.reset_input_state();
        }
        (&['d'], 'r') => {
            // <Space>dr - Restart: rerun the last run/debug
            editor.launch_last();
            editor.reset_input_state();
        }
        (&['d'], 'n') => {
            // <Space>dn - Step over (next)
            if editor.is_debug_active() {
                editor
                    .dap_manager_mut()
                    .queue(crate::dap::PendingDebugAction::StepOver);
            }
            editor.reset_input_state();
        }
        (&['d'], 'i') => {
            // <Space>di - Step into
            if editor.is_debug_active() {
                editor
                    .dap_manager_mut()
                    .queue(crate::dap::PendingDebugAction::StepIn);
            }
            editor.reset_input_state();
        }
        (&['d'], 'o') => {
            // <Space>do - Step out
            if editor.is_debug_active() {
                editor
                    .dap_manager_mut()
                    .queue(crate::dap::PendingDebugAction::StepOut);
            }
            editor.reset_input_state();
        }
        (&['d'], 's') => {
            // <Space>ds - Stop the run / debug session
            editor.launch_stop();
            editor.reset_input_state();
        }
        (&['d'], 'v') => {
            // <Space>dv - Toggle debug panels visibility
            editor.toggle_debug_panels();
            editor.reset_input_state();
        }
        (&['d'], 'f') | (&['d'], 'B') => {
            // <Space>df / <Space>dB - Focus the debug panel (stack, variables,
            // watches, breakpoints)
            editor.focus_debug_panel();
            editor.reset_input_state();
        }
        (&['d'], 'w') => {
            // <Space>dw - Add a watch expression (word under the cursor, or type one)
            match editor.debug_expression_at_cursor() {
                Some(expression) => editor.add_watch(expression),
                None => {
                    editor.set_mode(crate::mode::Mode::Command);
                    editor.set_command_line("DebugWatch ");
                }
            }
            editor.reset_input_state();
        }
        (&['d'], 'E') => {
            // <Space>dE - Toggle "break on exceptions"
            if let Err(message) = editor.toggle_exception_filter("") {
                editor.set_status_message(message);
            }
            editor.reset_input_state();
        }
        (&['d'], 'k') => {
            // <Space>dk - Select frame up (caller)
            if editor.is_debug_active() {
                editor.select_frame_up();
            }
            editor.reset_input_state();
        }
        (&['d'], 'j') => {
            // <Space>dj - Select frame down (callee)
            if editor.is_debug_active() {
                editor.select_frame_down();
            }
            editor.reset_input_state();
        }

        // <Space>r... sequences (run)
        (&['r'], 'r') => {
            // <Space>rr - Run whatever is at the cursor (no debugger)
            editor.launch_at_cursor(crate::launch::LaunchMode::Run);
            editor.reset_input_state();
        }
        (&['r'], 'd') => {
            // <Space>rd - Debug whatever is at the cursor
            editor.launch_at_cursor(crate::launch::LaunchMode::Debug);
            editor.reset_input_state();
        }
        (&['r'], 'l') => {
            // <Space>rl - Rerun the last run/debug
            editor.launch_last();
            editor.reset_input_state();
        }
        (&['r'], 'c') => {
            // <Space>rc - Pick a run configuration to run
            editor.launch_pick_config(crate::launch::LaunchMode::Run);
            editor.reset_input_state();
        }
        (&['r'], 'C') => {
            // <Space>rC - Pick a run configuration to debug
            editor.launch_pick_config(crate::launch::LaunchMode::Debug);
            editor.reset_input_state();
        }
        (&['r'], 's') => {
            // <Space>rs - Stop
            editor.launch_stop();
            editor.reset_input_state();
        }
        (&['r'], 't') => {
            // <Space>rt - Toggle the run console
            editor.toggle_run_console();
            editor.reset_input_state();
        }
        (&['r'], 'f') => {
            // <Space>rf - Focus the run console (scroll, jump to source)
            editor.focus_run_console();
            editor.reset_input_state();
        }
        (&['r'], 'x') => {
            // <Space>rx - Clear finished runs from the console
            editor.clear_run_console();
            editor.reset_input_state();
        }

        // <Space>l... sequences
        (&['l'], 'm') => {
            // <Space>lm - LSP Manager panel
            editor.open_lsp_manager();
            editor.reset_input_state();
        }

        // <Space>t... sequences
        (&['t'], 'h') => {
            // <Space>th - Type hierarchy
            editor.request_type_hierarchy();
            editor.reset_input_state();
        }
        (&['t'], 'f') => {
            // <Space>tf - Test file (run tests for current file)
            editor.run_test_file();
            editor.reset_input_state();
        }
        (&['t'], 'n') => {
            // <Space>tn - Test nearest (run test function at/above cursor)
            editor.run_test_nearest();
            editor.reset_input_state();
        }
        (&['t'], 'a') | (&['t'], 's') => {
            // <Space>ta / <Space>ts - Test suite (run full test suite)
            editor.run_test_all();
            editor.reset_input_state();
        }
        (&['t'], 'd') => {
            // <Space>td - Debug the nearest test (Java / Kotlin)
            editor.debug_test_nearest();
            editor.reset_input_state();
        }
        (&['t'], 'D') => {
            // <Space>tD - Debug the file's tests (Java / Kotlin)
            editor.debug_test_file();
            editor.reset_input_state();
        }
        (&['t'], 'l') => {
            // <Space>tl - Test last (re-run last test command)
            editor.run_test_last();
            editor.reset_input_state();
        }
        (&['t'], 'v') => {
            // <Space>tv - Test visit (jump to last-tested file/position)
            editor.test_visit();
            editor.reset_input_state();
        }
        (&['t'], 't') => {
            // <Space>tt - Toggle the test panel
            editor.toggle_test_panel();
            editor.reset_input_state();
        }
        (&['t'], 'o') => {
            // <Space>to - Open raw test/make output in a scratch buffer
            editor.open_test_output_buffer();
            editor.reset_input_state();
        }

        // <Space>c... sequences
        (&['c'], 'a') => {
            // <Space>ca - Code actions
            editor.request_code_actions();
            editor.reset_input_state();
        }
        (&['c'], 'i') => {
            // <Space>ci - Incoming calls (call hierarchy)
            editor.request_call_hierarchy_incoming();
            editor.reset_input_state();
        }
        (&['c'], 'o') => {
            // <Space>co - Outgoing calls (call hierarchy)
            editor.request_call_hierarchy_outgoing();
            editor.reset_input_state();
        }
        (&['c'], 'l') => {
            // <Space>cl - Run the code lens on this line
            editor.run_code_lens_at_cursor(crate::launch::LaunchMode::Run);
            editor.reset_input_state();
        }
        (&['c'], 'L') => {
            // <Space>cL - Debug the code lens on this line
            editor.run_code_lens_at_cursor(crate::launch::LaunchMode::Debug);
            editor.reset_input_state();
        }

        // <Space>s... sequences
        (&['s'], 'f') => {
            // <Space>sf - Find files
            let (base_dir, preferred_dir) = editor.picker_dirs();
            let picker = Picker::new_file_finder(base_dir, preferred_dir);
            editor.set_picker(picker);
            editor.set_mode(Mode::Picker);
            editor.mark_picker_selection_changed();
            editor.reset_input_state();
        }
        (&['s'], 'g') => {
            // <Space>sg - Live grep
            let (base_dir, preferred_dir) = editor.picker_dirs();
            let picker = Picker::new_live_grep(base_dir, preferred_dir);
            editor.set_picker(picker);
            editor.set_mode(Mode::Picker);
            editor.reset_input_state();
        }

        (&['s'], 'd') => {
            // <Space>sd - Problems (all published diagnostics, by file)
            editor.open_problems_picker(crate::editor::problems::ProblemFilter::All);
            editor.reset_input_state();
        }
        (&['s'], 'h') => {
            // <Space>sh - Recent files (this project, across sessions)
            editor.open_recent_files_picker();
            editor.reset_input_state();
        }
        (&['s'], 'b') => {
            // <Space>sb - Open buffers
            editor.open_buffer_picker();
            editor.reset_input_state();
        }
        (&['s'], 'S') => {
            // <Space>sS - Workspace symbols (live query)
            editor.open_workspace_symbol_picker();
            editor.reset_input_state();
        }
        (&['s'], 'r') => {
            // <Space>sr - Replace in files (prefilled with the word under the cursor)
            let word = editor.buffer().word_under_cursor().map(|(word, _, _)| word);
            editor.open_search_replace(word);
            editor.reset_input_state();
        }

        // <Space>g... sequences (git)
        (&['g'], 'd') => {
            // <Space>gd - Toggle the branch diff review
            editor.toggle_diff_review();
            editor.reset_input_state();
        }
        (&['g'], 'f') => {
            // <Space>gf - Fetch the review base branch
            editor.fetch_review_base();
            editor.reset_input_state();
        }
        (&['g'], 'g') => {
            // <Space>gg - Git status (changed files)
            editor.open_git_status_picker();
            editor.reset_input_state();
        }
        (&['g'], 's') => {
            // <Space>gs - Stage the hunk under the cursor
            editor.git_stage_hunk();
            editor.reset_input_state();
        }
        (&['g'], 'u') => {
            // <Space>gu - Unstage the hunk under the cursor
            editor.git_unstage_hunk();
            editor.reset_input_state();
        }
        (&['g'], 'S') => {
            // <Space>gS - Stage the file
            editor.git_stage_file();
            editor.reset_input_state();
        }
        (&['g'], 'U') => {
            // <Space>gU - Unstage the file
            editor.git_unstage_file();
            editor.reset_input_state();
        }
        (&['g'], 'c') => {
            // <Space>gc - Commit (message buffer)
            editor.open_commit_message(false);
            editor.reset_input_state();
        }
        (&['g'], 'C') => {
            // <Space>gC - Amend the last commit
            editor.open_commit_message(true);
            editor.reset_input_state();
        }
        (&['g'], 'l') => {
            // <Space>gl - History of the current file
            editor.open_file_history_picker();
            editor.reset_input_state();
        }
        (&['g'], 'L') => {
            // <Space>gL - History of the current line
            editor.open_line_history_picker();
            editor.reset_input_state();
        }
        (&['g'], 'm') => {
            // <Space>gm... - merge conflicts (n/p navigate, o ours, t theirs, b both, x neither)
            editor.set_input_state(InputState::Leader {
                keys: vec!['g', 'm'],
            });
        }
        (&['g', 'm'], 'n') => {
            editor.goto_conflict(true);
            editor.reset_input_state();
        }
        (&['g', 'm'], 'p') => {
            editor.goto_conflict(false);
            editor.reset_input_state();
        }
        (&['g', 'm'], 'o') => {
            editor.resolve_conflict(crate::git::conflict::Resolution::Ours);
            editor.reset_input_state();
        }
        (&['g', 'm'], 't') => {
            editor.resolve_conflict(crate::git::conflict::Resolution::Theirs);
            editor.reset_input_state();
        }
        (&['g', 'm'], 'b') => {
            editor.resolve_conflict(crate::git::conflict::Resolution::Both);
            editor.reset_input_state();
        }
        (&['g', 'm'], 'x') => {
            editor.resolve_conflict(crate::git::conflict::Resolution::Neither);
            editor.reset_input_state();
        }

        // Unknown sequence - cancel
        _ => {
            editor.reset_input_state();
        }
    }

    Ok(())
}
