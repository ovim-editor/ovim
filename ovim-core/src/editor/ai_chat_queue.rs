use anyhow::Result;

use super::ai_chat_state::{QueuedChatInput, QueuedChatInputKind};
use super::Editor;

impl Editor {
    /// Tab submits a normal follow-up for the next round while work is active.
    pub fn schedule_ai_chat_message(&mut self) -> Result<()> {
        if !self.ai_chat_round_active() {
            return self.submit_ai_chat_message();
        }
        self.queue_current_ai_chat_input(QueuedChatInputKind::FollowUp)
    }

    pub fn ai_chat_round_active(&self) -> bool {
        self.ai_state
            .chat
            .as_ref()
            .is_some_and(|chat| chat.runtime_turn.is_some())
    }

    pub fn ai_chat_queued_inputs(&self) -> impl Iterator<Item = &QueuedChatInput> {
        self.ai_state
            .chat
            .as_ref()
            .into_iter()
            .flat_map(|chat| chat.queued_inputs.iter())
    }

    pub(crate) fn queue_current_ai_chat_input(
        &mut self,
        requested_kind: QueuedChatInputKind,
    ) -> Result<()> {
        let input = self.ai_chat_input().trim().to_string();
        if input.is_empty() {
            return Ok(());
        }
        let kind = if input.starts_with('/') {
            QueuedChatInputKind::Command
        } else {
            requested_kind
        };

        if let Some(chat) = self.ai_state.chat.as_mut() {
            chat.input.clear();
            chat.input_cursor = 0;
            chat.queued_inputs.push_back(QueuedChatInput {
                kind,
                content: input.clone(),
            });
            if kind == QueuedChatInputKind::Steer {
                if let Some(tx) = chat
                    .pending_job
                    .as_ref()
                    .and_then(|job| job.steer_tx.as_ref())
                {
                    let _ = tx.send(input.clone());
                }
            }
        }

        let status = match kind {
            QueuedChatInputKind::Steer => "Steer queued for the next tool call",
            QueuedChatInputKind::FollowUp => "Message queued for the next round",
            QueuedChatInputKind::Command => "Slash command queued for the end of this round",
        };
        self.set_lsp_status(status.to_string());
        Ok(())
    }

    /// Apply steers at ovim's post-tool continuation boundary (OpenAI,
    /// Anthropic, Ollama). Codex app-server acknowledges these asynchronously.
    pub(crate) fn apply_local_ai_chat_steers(&mut self) -> Result<()> {
        let steers = self.take_queued_ai_chat_inputs(QueuedChatInputKind::Steer);
        for content in steers {
            self.record_accepted_ai_chat_steer(content)?;
        }
        Ok(())
    }

    pub(crate) fn accept_provider_ai_chat_steer(&mut self, content: String) -> Result<()> {
        self.remove_first_matching_queued_steer(&content);
        self.record_accepted_ai_chat_steer(content)
    }

    pub(crate) fn reject_provider_ai_chat_steer(&mut self, content: &str, error: &str) {
        if let Some(chat) = self.ai_state.chat.as_mut() {
            if let Some(item) = chat
                .queued_inputs
                .iter_mut()
                .find(|item| item.kind == QueuedChatInputKind::Steer && item.content == content)
            {
                item.kind = QueuedChatInputKind::FollowUp;
            }
        }
        self.set_lsp_status(format!(
            "Could not steer the active turn; queued for next round: {error}"
        ));
    }

    fn record_accepted_ai_chat_steer(&mut self, content: String) -> Result<()> {
        let turn = self
            .active_ai_runtime_turn()
            .ok_or_else(|| anyhow::anyhow!("AI turn ended before steer was recorded"))?;
        let event = self
            .ai_state
            .agent_runtime
            .append_user_steering(&turn, content.clone())?;
        let node = self
            .conversation_mut()
            .map(|conversation| conversation.append_user_message(content));
        if let Some(node) = node {
            self.record_ai_chat_node(node, event.event_id);
        }
        Ok(())
    }

    fn take_queued_ai_chat_inputs(&mut self, kind: QueuedChatInputKind) -> Vec<String> {
        let Some(chat) = self.ai_state.chat.as_mut() else {
            return Vec::new();
        };
        let mut selected = Vec::new();
        chat.queued_inputs.retain(|item| {
            if item.kind == kind {
                selected.push(item.content.clone());
                false
            } else {
                true
            }
        });
        selected
    }

    fn remove_first_matching_queued_steer(&mut self, content: &str) {
        let Some(chat) = self.ai_state.chat.as_mut() else {
            return;
        };
        if let Some(index) = chat
            .queued_inputs
            .iter()
            .position(|item| item.kind == QueuedChatInputKind::Steer && item.content == content)
        {
            chat.queued_inputs.remove(index);
        }
    }

    /// Execute queued commands in order, then start at most one queued user
    /// follow-up. Any unconsumed steers become ordinary follow-ups when a turn
    /// completes without reaching another tool boundary.
    pub(crate) fn start_next_queued_ai_chat_input(&mut self) -> Result<bool> {
        loop {
            let item = self
                .ai_state
                .chat
                .as_mut()
                .and_then(|chat| chat.queued_inputs.pop_front());
            let Some(mut item) = item else {
                return Ok(false);
            };
            if item.kind == QueuedChatInputKind::Steer {
                item.kind = QueuedChatInputKind::FollowUp;
            }
            if let Some(chat) = self.ai_state.chat.as_mut() {
                chat.input = item.content;
                chat.input_cursor = chat.input.len();
            }
            match item.kind {
                QueuedChatInputKind::Command => {
                    let command = self.ai_chat_input().to_string();
                    self.try_execute_ai_chat_slash_command(&command)?;
                    // Invalid commands deliberately retain input for correction;
                    // clear it before moving to the next scheduled item.
                    if let Some(chat) = self.ai_state.chat.as_mut() {
                        chat.input.clear();
                        chat.input_cursor = 0;
                    }
                }
                QueuedChatInputKind::FollowUp | QueuedChatInputKind::Steer => {
                    self.submit_ai_chat_message()?;
                    return Ok(true);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::chat_types::ChatOpts;

    fn editor_with_active_round() -> Editor {
        let mut editor = Editor::default();
        editor.open_ai_chat(ChatOpts::default()).unwrap();
        let turn = editor.begin_ai_runtime_turn("initial request").unwrap();
        editor.ai_state.chat.as_mut().unwrap().runtime_turn = Some(Box::new(turn));
        editor
    }

    #[test]
    fn enter_queues_steer_and_local_tool_boundary_accepts_it() {
        let mut editor = editor_with_active_round();
        let chat = editor.ai_state.chat.as_mut().unwrap();
        chat.input = "change direction".into();
        chat.input_cursor = chat.input.len();

        editor.submit_ai_chat_message().unwrap();
        assert_eq!(
            editor.ai_state.chat.as_ref().unwrap().queued_inputs.len(),
            1
        );
        assert_eq!(
            editor.ai_state.chat.as_ref().unwrap().queued_inputs[0].kind,
            QueuedChatInputKind::Steer
        );

        editor.apply_local_ai_chat_steers().unwrap();
        assert!(editor
            .ai_state
            .chat
            .as_ref()
            .unwrap()
            .queued_inputs
            .is_empty());
        assert_eq!(
            editor.ai_chat_messages().last().unwrap().content,
            "change direction"
        );
    }

    #[test]
    fn tab_queues_follow_up_and_slash_command_is_visually_typed_as_command() {
        let mut editor = editor_with_active_round();
        let chat = editor.ai_state.chat.as_mut().unwrap();
        chat.input = "after this".into();
        chat.input_cursor = chat.input.len();
        editor.schedule_ai_chat_message().unwrap();

        let chat = editor.ai_state.chat.as_mut().unwrap();
        chat.input = "/clear".into();
        chat.input_cursor = chat.input.len();
        editor.schedule_ai_chat_message().unwrap();

        let queued = &editor.ai_state.chat.as_ref().unwrap().queued_inputs;
        assert_eq!(queued[0].kind, QueuedChatInputKind::FollowUp);
        assert_eq!(queued[1].kind, QueuedChatInputKind::Command);
        assert_eq!(queued[1].content, "/clear");
    }
}
