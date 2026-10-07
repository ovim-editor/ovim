//! Server-initiated traffic: file-watch forwarding and `window/showMessage`
//! notices and requests.

use super::*;

impl Editor {
    /// Forwards on-disk changes made outside the editor to the servers that
    /// registered `workspace/didChangeWatchedFiles`, and keeps the watcher's
    /// roots in step with the registrations.
    pub async fn process_workspace_file_events(&mut self) {
        let Some(lsp) = self.lsp.state.lsp_manager.clone() else {
            return;
        };
        let wanted = lsp.watched_file_roots();
        self.lsp.state.workspace_watcher.sync_roots(&wanted);
        if let Some(error) = self.lsp.state.workspace_watcher.take_error() {
            self.set_lsp_status(format!("LSP: {error}"));
        }
        let matcher = lsp.clone();
        let Some(events) = self
            .lsp
            .state
            .workspace_watcher
            .poll(std::time::Instant::now(), &move |path| {
                matcher.watched_path_matches(path)
            })
        else {
            return;
        };
        let sent = lsp.send_watched_file_changes(&events).await;
        if sent > 0 {
            // The server may publish new diagnostics for open documents in
            // response; make sure we re-pull rather than trust stale ones.
            self.lsp.slots.diagnostics.invalidate();
            self.lsp.slots.inlay_hints.invalidate();
        }
    }

    /// Shows `window/showMessage` notices (toast + status line), offers
    /// `window/showMessageRequest` actions in a picker, and sends the answers.
    pub async fn process_server_messages(&mut self) {
        use super::toast::{ToastLevel, ToastRequest, ToastSource};
        use crate::lsp::{MessageSeverity, ServerMessage};

        let Some(lsp) = self.lsp.state.lsp_manager.clone() else {
            return;
        };
        for message in lsp.take_server_messages() {
            match message {
                ServerMessage::Notice {
                    server_id,
                    severity,
                    message,
                } => {
                    let level = match severity {
                        MessageSeverity::Error => ToastLevel::Error,
                        MessageSeverity::Warning => ToastLevel::Warning,
                        MessageSeverity::Info | MessageSeverity::Log => ToastLevel::Info,
                    };
                    let text = message.lines().next().unwrap_or_default().to_string();
                    self.set_status_message(format!("{server_id}: {text}"));
                    self.push_toast(
                        ToastRequest::new(ToastSource::Lsp, level, message)
                            .with_title(server_id.clone())
                            .with_dedupe_key(format!("lsp-message:{server_id}")),
                    );
                }
                ServerMessage::Request(request) => {
                    self.lsp.state.queued_message_requests.push_back(request);
                }
            }
        }

        // A request whose picker was dismissed (Esc) is answered "no action".
        if self.lsp.state.active_message_request.is_some()
            && !self
                .picker()
                .is_some_and(|picker| picker.is_message_action_picker())
        {
            self.answer_active_message_request(None);
        }
        // Offer the next request once the user is not in another picker.
        if self.lsp.state.active_message_request.is_none()
            && self.picker().is_none()
            && self.mode() == crate::mode::Mode::Normal
        {
            if let Some(request) = self.lsp.state.queued_message_requests.pop_front() {
                let base_dir =
                    std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
                let picker = crate::editor::picker::Picker::new_message_actions(
                    base_dir,
                    request.actions.clone(),
                );
                self.set_status_message(format!("{}: {}", request.server_id, request.message));
                self.set_picker(picker);
                self.set_mode(crate::mode::Mode::Picker);
                self.mark_picker_selection_changed();
                self.lsp.state.active_message_request = Some(request);
                self.mark_dirty();
            }
        }

        for (request, chosen) in std::mem::take(&mut self.lsp.state.message_replies) {
            lsp.reply_message_request(&request, chosen.as_deref()).await;
        }
    }

    /// Records the user's answer to the active `showMessageRequest`
    /// (`None` = dismissed); the reply is sent on the next tick.
    pub(in crate::editor) fn answer_active_message_request(&mut self, index: Option<usize>) {
        let Some(request) = self.lsp.state.active_message_request.take() else {
            return;
        };
        let chosen = index.and_then(|i| request.actions.get(i).cloned());
        self.lsp.state.message_replies.push((request, chosen));
    }
}
