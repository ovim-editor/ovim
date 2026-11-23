//! Sessions, workflows, AI status, the embedded browser.

use super::Ex;
use crate::command_result::{err, ok, CommandResult};
use crate::editor::Editor;

/// `:ai status` / `:ai env`: the active AI profile and the environment
/// variables it depends on, masked.
pub(super) fn ai(editor: &mut Editor, ex: &Ex) -> CommandResult {
    match ex.args {
        "status" | "env" => ai_status(editor),
        _ => err("Usage: :ai status|env"),
    }
}

/// The first and last four characters of a secret.
fn mask(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() > 8 {
        let head: String = chars[..4].iter().collect();
        let tail: String = chars[chars.len() - 4..].iter().collect();
        format!("{head}...{tail}")
    } else {
        "****".to_string()
    }
}

fn ai_status(editor: &mut Editor) -> CommandResult {
    let config = &editor.ai_state.config;
    let active = &editor.ai_state.active_profile;

    let mut lines = Vec::new();
    lines.push("**AI Configuration**".to_string());
    lines.push(format!("Active profile: {}", active));
    lines.push(format!("Default profile: {}", config.default_profile));
    lines.push(format!(
        "Profiles: {}",
        config
            .profiles
            .keys()
            .cloned()
            .collect::<Vec<_>>()
            .join(", ")
    ));
    let approval_mode = match config.tool_approval_mode {
        crate::ai::ToolApprovalMode::Auto => "auto",
        crate::ai::ToolApprovalMode::SensitivePrompt => "sensitive_prompt",
        crate::ai::ToolApprovalMode::AlwaysPrompt => "always_prompt",
    };
    lines.push(format!("Tool approval mode: {}", approval_mode));

    // Show context mappings
    if !config.contexts.is_empty() {
        let ctx_str: Vec<String> = config
            .contexts
            .iter()
            .map(|(k, v)| format!("{}→{}", k, v))
            .collect();
        lines.push(format!("Contexts: {}", ctx_str.join(", ")));
    }

    lines.push(String::new());

    // Show details for active profile
    if let Some(profile) = config.resolve_profile(active) {
        lines.push(format!("**Profile '{}' details:**", active));
        lines.push(format!("  Provider: {}", profile.provider));
        lines.push(format!("  Model: {}", profile.model));
        if let Some(ref url) = profile.base_url {
            lines.push(format!("  Base URL: {}", url));
        }
        lines.push(format!("  Edit format: {}", profile.edit_format));

        // Environment variable check
        let env_name = profile.api_key_env.as_deref().unwrap_or("(none)");
        lines.push(format!("  API key env var: {}", env_name));
        if let Some(ref name) = profile.api_key_env {
            match std::env::var(name) {
                Ok(val) => {
                    let masked = mask(&val);
                    lines.push(format!("  Env var status: SET ({})", masked));
                }
                Err(_) => {
                    lines.push("  Env var status: NOT SET".to_string());
                }
            }
        }
    } else {
        lines.push(format!("Active profile '{}' not found!", active));
    }

    // Show all AI-related env vars visible to this process
    lines.push(String::new());
    lines.push("**AI-related env vars visible to process:**".to_string());
    let mut found_any = false;
    for (key, val) in std::env::vars() {
        if key.contains("OPENAI")
            || key.contains("ANTHROPIC")
            || key.contains("OVIM")
            || key.contains("API_KEY")
        {
            let masked = mask(&val);
            lines.push(format!("  {} = {}", key, masked));
            found_any = true;
        }
    }
    if !found_any {
        lines.push("  (none found matching OPENAI/ANTHROPIC/OVIM/API_KEY)".to_string());
    }

    ok(lines.join("\n"))
}

/// `:workflow [list|reload|status|run {name} [k=v ...]]`.
pub(super) fn workflow(editor: &mut Editor, ex: &Ex) -> CommandResult {
    let subcmd = ex.args;

    match subcmd {
        "" | "list" => {
            if let Err(e) = editor.ensure_workflows_loaded() {
                return err(format!("Failed to load workflows: {}", e));
            }
            let names = editor.workflow_names_sorted();
            let message = if names.is_empty() {
                "No workflows found.".to_string()
            } else {
                format!("{} workflow(s):\n{}", names.len(), names.join("\n"))
            };
            ok(message)
        }
        "reload" => match editor.reload_workflows() {
            Ok(count) => ok(format!("Loaded {} workflow(s)", count)),
            Err(e) => err(format!("Failed to reload workflows: {}", e)),
        },
        "status" => ok(editor.workflow_status_report()),
        s if s.starts_with("run ") => {
            let mut parts = s["run ".len()..].split_whitespace();
            let Some(name) = parts.next() else {
                return err("Usage: :workflow run <name> [k=v ...]");
            };

            let mut inputs = std::collections::BTreeMap::new();
            for pair in parts {
                let Some((key, raw_value)) = pair.split_once('=') else {
                    return err(format!("Invalid input '{}': expected k=v", pair));
                };
                let value = serde_json::from_str::<serde_json::Value>(raw_value)
                    .unwrap_or_else(|_| serde_json::Value::String(raw_value.to_string()));
                inputs.insert(key.to_string(), value);
            }

            match editor.run_workflow(name, inputs) {
                Ok(run_id) => ok(format!("Workflow '{}' started (run #{})", name, run_id)),
                Err(e) => err(format!("Failed to run workflow '{}': {}", name, e)),
            }
        }
        _ => err(format!(
                "Unknown workflow subcommand '{}'. Usage: :workflow [list|reload|run <name> [k=v ...]|status]",
                subcmd
            )),
    }
}

/// `:session [list]` shows sessions; `:session stop` removes this
/// frontend's registration; `:session start NAME` registers one when an
/// automation API is running (a plain TUI has none, so automation sessions
/// are explicit at startup).
pub(super) fn session(editor: &mut Editor, ex: &Ex) -> CommandResult {
    use crate::session::SessionInfo;

    let subcmd = ex.args;

    match subcmd {
        "" | "list" => {
            // Show active sessions
            match SessionInfo::list_all() {
                Ok(sessions) if sessions.is_empty() => {
                    let msg = if let Some(name) = editor.active_session() {
                        format!("Active session: {}", name)
                    } else {
                        "No registered sessions. Start an explicit automation session with: ovim <file> --headless --session NAME".to_string()
                    };
                    ok(msg)
                }
                Ok(sessions) => {
                    let mut msg = format!("{} active session(s):", sessions.len());
                    for s in &sessions {
                        let marker = if editor.active_session() == Some(&s.session_name) {
                            " (this)"
                        } else {
                            ""
                        };
                        msg.push_str(&format!(
                            "\n  {} (PID {}, port {}){}",
                            s.session_name, s.pid, s.port, marker
                        ));
                    }
                    ok(msg)
                }
                Err(e) => err(format!("Failed to list sessions: {}", e)),
            }
        }
        s if s.starts_with("start ") => {
            let name = s["start ".len()..].trim();
            if name.is_empty() {
                return err("Usage: :session start NAME");
            }

            // Validate with the same rule reads enforce (charset + 64-char cap),
            // so a registered session can always be read back and targeted.
            if let Err(e) = SessionInfo::validate_session_name(name) {
                return err(e.to_string());
            }

            // Check if already registered
            if let Some(existing) = editor.active_session() {
                return err(format!(
                    "Already registered as session '{}'. Use :session stop first.",
                    existing
                ));
            }

            // Need API port to register
            let port = match editor.api_port() {
                Some(p) => p,
                None => {
                    return err(
                        "Interactive sessions do not expose the automation API. Start one explicitly with: ovim <file> --headless --session NAME",
                    );
                }
            };

            let file = editor.buffer().file_path().map(|s| s.to_string());

            let session_info = SessionInfo::new(port, file, name.to_string());
            match session_info.write() {
                Ok(()) => {
                    editor.set_active_session(name.to_string());
                    ok(format!("Session '{}' registered", name))
                }
                Err(e) => err(format!("Failed to register session: {}", e)),
            }
        }
        "stop" => {
            match editor.take_active_session() {
                Some(name) => {
                    // Delete the session file
                    let port = editor.api_port().unwrap_or(0);
                    let session_info = SessionInfo::new(port, None, name.clone());
                    let _ = session_info.delete();
                    ok(format!("Session '{}' unregistered", name))
                }
                None => err("No active session to stop"),
            }
        }
        _ => err(format!(
            "Unknown session subcommand: '{}'. Usage: :session [start NAME|stop|list]",
            subcmd
        )),
    }
}

/// `:clearaedits`: drop the markers on lines the agent edited.
pub(super) fn clear_agent_edits(editor: &mut Editor, _ex: &Ex) -> CommandResult {
    if let Some(chat) = editor.ai_state.chat.as_mut() {
        chat.agent_edits.clear();
    }
    ok("Agent edit markers cleared.")
}

/// `:browser`: open a tab in the frontend's embedded browser.
/// `:openwin [dir]`: queue a project window for a window-capable frontend.
/// The path stays verbatim; expansion and validation belong to the frontend
/// that actually opens the window.
pub(super) fn openwin(editor: &mut Editor, ex: &Ex) -> CommandResult {
    let path = ex.args.trim();
    editor.request_open_window((!path.is_empty()).then(|| path.to_string()));
    crate::command_result::ok_silent()
}

pub(super) fn browser(editor: &mut Editor, _ex: &Ex) -> CommandResult {
    match editor.request_browser_start() {
        Ok(()) => ok("Opening browser"),
        Err(error) => err(format!("Could not open embedded browser: {error}")),
    }
}
