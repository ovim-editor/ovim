use anyhow::Result;

use crate::ai::chat_types::{ChatFocus, ConversationTree};

use super::Editor;

#[derive(Debug, PartialEq, Eq)]
enum AiChatSlashCommand {
    Clear,
    Model { profile: Option<String> },
}

impl AiChatSlashCommand {
    fn parse(input: &str) -> Option<std::result::Result<Self, String>> {
        let command = input.strip_prefix('/')?;
        let mut parts = command.split_whitespace();
        let name = parts.next().unwrap_or_default();
        let arguments = parts.collect::<Vec<_>>();

        let parsed = match name {
            "clear" if arguments.is_empty() => Ok(Self::Clear),
            "clear" => Err("Usage: /clear".to_string()),
            "model" if arguments.len() <= 1 => Ok(Self::Model {
                profile: arguments.first().map(|value| (*value).to_string()),
            }),
            "model" => Err("Usage: /model [profile]".to_string()),
            "" => Err("Enter a slash command after /".to_string()),
            unknown => Err(format!("Unknown AI chat command: /{unknown}")),
        };
        Some(parsed)
    }
}

impl Editor {
    /// Parse and execute editor-owned chat commands before provider submission.
    /// Returns `true` when the input was a slash command, including invalid ones.
    pub(super) fn try_execute_ai_chat_slash_command(&mut self, input: &str) -> Result<bool> {
        let Some(command) = AiChatSlashCommand::parse(input) else {
            return Ok(false);
        };
        match command {
            Ok(AiChatSlashCommand::Clear) => self.clear_ai_chat_conversation()?,
            Ok(AiChatSlashCommand::Model { profile: None }) => {
                self.clear_ai_chat_input();
                if let Some(chat) = self.ai_state.chat.as_mut() {
                    chat.focus = ChatFocus::ModelSelector;
                }
            }
            Ok(AiChatSlashCommand::Model {
                profile: Some(profile),
            }) => {
                if self.ai_set_profile(&profile) {
                    self.clear_ai_chat_input();
                } else {
                    self.set_lsp_status(format!("Unknown AI profile: {profile}"));
                }
            }
            Err(message) => self.set_lsp_status(message),
        }
        Ok(true)
    }

    fn clear_ai_chat_input(&mut self) {
        if let Some(chat) = self.ai_state.chat.as_mut() {
            chat.input.clear();
            chat.input_cursor = 0;
        }
    }

    fn clear_ai_chat_conversation(&mut self) -> Result<()> {
        let key = self.ai_chat_conversation_key();
        let locator = self
            .ai_state
            .durable_chat_bindings
            .get(&key)
            .map(|binding| binding.locator.clone())
            .unwrap_or_else(|| {
                crate::agent_runtime::ConversationLocator(format!(
                    "buffer:{}:conversation:{}",
                    key.0, key.1
                ))
            });
        self.ai_state.agent_runtime.record_context_reset(&locator)?;
        self.reset_durable_ai_chat_provider_session()?;

        self.ai_state
            .conversations
            .insert(key.clone(), ConversationTree::new());
        self.ai_state.conversation_runtime_nodes.remove(&key);

        if let Some(chat) = self.ai_state.chat.as_mut() {
            chat.input.clear();
            chat.input_cursor = 0;
            chat.focus = ChatFocus::TextInput;
            chat.context_generation = chat.context_generation.saturating_add(1);
            chat.viewport = Default::default();
            chat.history = Default::default();
            chat.expanded_thinking.clear();
            chat.streaming_content = None;
            chat.streaming_thinking = None;
            chat.streaming_tool_calls.clear();
            chat.tool_event_summaries.clear();
            chat.tool_call_count = 0;
            chat.agent_edits.clear();
        }
        self.render_cache.ai_chat_last_total_rows = 0;
        self.render_cache.ai_chat_last_visible_start_row = 0;
        self.render_cache.ai_chat_last_visible_end_row = 0;
        self.render_cache.ai_chat_last_message_row_spans.clear();
        self.set_lsp_status("AI chat cleared".to_string());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::chat_types::{ChatOpts, ChatRole};

    fn open_test_chat(editor: &mut Editor) {
        editor
            .open_ai_chat(ChatOpts {
                name: "chat".into(),
                allow_edits: true,
                ..Default::default()
            })
            .expect("open chat");
    }

    #[test]
    fn parses_supported_commands_and_rejects_bad_shapes() {
        assert_eq!(
            AiChatSlashCommand::parse("/clear"),
            Some(Ok(AiChatSlashCommand::Clear))
        );
        assert_eq!(
            AiChatSlashCommand::parse("/model fast"),
            Some(Ok(AiChatSlashCommand::Model {
                profile: Some("fast".into())
            }))
        );
        assert!(AiChatSlashCommand::parse("hello").is_none());
        assert!(matches!(
            AiChatSlashCommand::parse("/clear now"),
            Some(Err(_))
        ));
        assert!(matches!(
            AiChatSlashCommand::parse("/unknown"),
            Some(Err(_))
        ));
    }

    #[test]
    fn clear_resets_conversation_and_provider_context_generation() {
        let mut editor = Editor::default();
        open_test_chat(&mut editor);
        editor
            .conversation_mut()
            .expect("conversation")
            .append_user_message("old context".into());
        let chat = editor.ai_state.chat.as_mut().expect("chat");
        chat.input = "/clear".into();
        chat.input_cursor = chat.input.len();

        editor.submit_ai_chat_message().expect("clear command");

        assert!(editor.ai_chat_messages().is_empty());
        assert!(editor.ai_chat_input().is_empty());
        assert_eq!(editor.ai_chat_state().expect("chat").context_generation, 1);
    }

    #[test]
    fn unknown_slash_command_is_not_submitted_as_a_message() {
        let mut editor = Editor::default();
        open_test_chat(&mut editor);
        let chat = editor.ai_state.chat.as_mut().expect("chat");
        chat.input = "/cler".into();
        chat.input_cursor = chat.input.len();

        editor.submit_ai_chat_message().expect("unknown command");

        assert!(editor
            .ai_chat_messages()
            .iter()
            .all(|message| message.role != ChatRole::User));
        assert_eq!(editor.ai_chat_input(), "/cler");
    }
}
