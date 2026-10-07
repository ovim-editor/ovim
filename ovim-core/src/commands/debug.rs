//! Debugger commands: `:debug {subcommand}`, `:eval` and the `:Debug*`
//! panel, watch and breakpoint commands.

use super::Ex;
use crate::command_result::{err, ok, ok_silent, CommandResult};
use crate::dap::PendingDebugAction;
use crate::editor::{BreakpointExtra, Editor};

const DEBUG_USAGE: &str = "Usage: :debug [start [cmd]|run|last|config|stop|continue|next|stepin|stepout|breakpoint|panels|console]";

fn step(editor: &mut Editor, action: PendingDebugAction, message: &'static str) -> CommandResult {
    editor.dap_manager_mut().queue(action);
    ok(message)
}

/// `:debug {subcommand}`: launch, step and inspect. The launch
/// subcommands are the `:Run*` / `:Debug*` table commands under another
/// name.
pub(super) fn debug(editor: &mut Editor, ex: &Ex) -> CommandResult {
    let same_as = match ex.args {
        "run" => "Run",
        // Same path as F5: resolve what is at the cursor (or fall back to
        // configurations), build, launch, attach.
        "start" => "Debug",
        "last" | "restart" => "RunLast",
        "config" | "pick" => "DebugConfig",
        "console" => "RunConsole",
        _ => "",
    };
    if !same_as.is_empty() {
        return super::run_line(editor, same_as);
    }
    match ex.args {
        "breakpoint" | "bp" => {
            editor.toggle_breakpoint();
            ok("Breakpoint toggled")
        }
        "panels" => {
            editor.toggle_debug_panels();
            ok_silent()
        }
        "continue" | "c" => step(editor, PendingDebugAction::Continue, "Continue"),
        "next" | "n" | "step" => step(editor, PendingDebugAction::StepOver, "Step over"),
        "stepin" | "si" => step(editor, PendingDebugAction::StepIn, "Step in"),
        "stepout" | "so" => step(editor, PendingDebugAction::StepOut, "Step out"),
        "stop" if editor.launch_stop() => ok("Stopping"),
        "stop" => ok_silent(),
        "" => ok(DEBUG_USAGE),
        other => match other.strip_prefix("start ") {
            // :debug start <adapter> [args...] — the F5 flow, custom adapter.
            Some(rest) => {
                let mut parts = rest.split_whitespace();
                let command = parts.next().unwrap_or_default().to_string();
                let args: Vec<String> = parts.map(String::from).collect();
                editor
                    .launch_at_cursor_with(crate::launch::LaunchMode::Debug, Some((command, args)));
                ok_silent()
            }
            None => err(format!(
                "Unknown debug subcommand: '{other}'. {}",
                DEBUG_USAGE.replace("Usage: :debug [start [cmd]|", "Usage: :debug [start|")
            )),
        },
    }
}

/// `:eval {expression}`: evaluate in the stopped frame.
pub(super) fn eval(editor: &mut Editor, ex: &Ex) -> CommandResult {
    if ex.args.is_empty() {
        return err("Usage: :eval <expression>");
    }
    if !editor.is_debug_stopped() {
        return err("Not stopped at a breakpoint");
    }
    editor
        .dap_manager_mut()
        .queue(PendingDebugAction::Evaluate {
            expression: ex.args.to_string(),
        });
    ok("Evaluating...")
}

pub(super) fn focus_panel(editor: &mut Editor, _ex: &Ex) -> CommandResult {
    editor.focus_debug_panel();
    ok_silent()
}

/// `:DebugWatch {expression}`.
pub(super) fn watch(editor: &mut Editor, ex: &Ex) -> CommandResult {
    if ex.args.is_empty() {
        return err("Usage: :DebugWatch <expression>");
    }
    editor.add_watch(ex.args.to_string());
    editor.dap_manager_mut().state.panels_visible = true;
    ok(format!("Watching {}", ex.args))
}

/// `:DebugUnwatch [number|expression]` (default: the last watch).
pub(super) fn unwatch(editor: &mut Editor, ex: &Ex) -> CommandResult {
    let what = ex.args;
    let watches = &editor.debug_state().watches;
    let index = if what.is_empty() {
        watches.len().checked_sub(1)
    } else if let Ok(n) = what.parse::<usize>() {
        n.checked_sub(1).filter(|i| *i < watches.len())
    } else {
        watches.iter().position(|w| w.expression == what)
    };
    match index {
        Some(index) => {
            editor.remove_watch(index);
            ok("Watch removed")
        }
        None => err("No such watch (use :DebugUnwatch <number|expression>)"),
    }
}

/// `:DebugBreakpoints [list|on|off|clear]`.
pub(super) fn breakpoints(editor: &mut Editor, ex: &Ex) -> CommandResult {
    match ex.args {
        "" | "list" => {
            editor.focus_debug_panel();
            ok_silent()
        }
        "on" | "enable" => {
            editor.set_all_breakpoints_enabled(true);
            ok("All breakpoints enabled")
        }
        "off" | "disable" => {
            editor.set_all_breakpoints_enabled(false);
            ok("All breakpoints disabled")
        }
        "clear" => {
            editor.clear_all_breakpoints();
            ok("All breakpoints removed")
        }
        _ => err("Usage: :DebugBreakpoints [list|on|off|clear]"),
    }
}

/// `:DebugException [filter]`: toggle an exception breakpoint filter.
pub(super) fn exception(editor: &mut Editor, ex: &Ex) -> CommandResult {
    match editor.toggle_exception_filter(ex.args) {
        Ok(()) => ok_silent(),
        Err(message) => err(message),
    }
}

/// `:DebugExpand {name}`: toggle a variable in the debug panel.
pub(super) fn expand(editor: &mut Editor, ex: &Ex) -> CommandResult {
    let name = ex.args;
    if name.is_empty() {
        return err("Usage: :DebugExpand <variable_name>");
    }
    if !editor.is_debug_stopped() {
        return err("Not stopped at a breakpoint");
    }
    let Some(var_ref) = editor
        .debug_state()
        .variables
        .values()
        .flatten()
        .find(|var| var.name == name && var.variables_reference > 0)
        .map(|var| var.variables_reference)
    else {
        return err(format!("Variable '{}' not found or not expandable", name));
    };
    let state = &mut editor.dap_manager_mut().state;
    if !state.expanded_refs.remove(&var_ref) {
        state.expanded_refs.insert(var_ref);
        editor
            .dap_manager_mut()
            .queue(PendingDebugAction::FetchVariables { var_ref });
    }
    editor.mark_dirty();
    ok(format!("Toggled expansion of '{}'", name))
}

/// `:DebugLogpoint {message}`: log instead of stopping at the cursor line.
pub(super) fn logpoint(editor: &mut Editor, ex: &Ex) -> CommandResult {
    ok(editor.set_cursor_breakpoint_extra(BreakpointExtra::Logpoint, ex.args))
}

/// `:DebugHitCount {n|>n|%n}`: stop only when the hit count matches.
pub(super) fn hit_count(editor: &mut Editor, ex: &Ex) -> CommandResult {
    ok(editor.set_cursor_breakpoint_extra(BreakpointExtra::HitCount, ex.args))
}

/// `:DebugCondition [expr]`: a conditional breakpoint at the cursor; no
/// expression removes the condition.
pub(super) fn condition(editor: &mut Editor, ex: &Ex) -> CommandResult {
    if ex.args.is_empty() {
        if let Some(file_path) = editor.buffer().file_path().map(|s| s.to_string()) {
            let line = editor.buffer().cursor().line() as u64 + 1;
            let path = std::path::PathBuf::from(&file_path);
            editor
                .dap_manager_mut()
                .state
                .set_breakpoint_condition(&path, line, None);
        }
    } else {
        editor.toggle_conditional_breakpoint(ex.args.to_string());
    }
    // A live session hears about it right away.
    editor.dap_manager_mut().request_breakpoint_sync();
    ok("Conditional breakpoint set")
}
