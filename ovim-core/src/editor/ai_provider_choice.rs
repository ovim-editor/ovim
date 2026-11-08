//! First-run choice of how AI chat inference runs.
//!
//! Opening chat without a remembered selection or an authored chat profile
//! asks once instead of silently starting a provider that may need sign-in.
//! The choice is remembered through the ordinary chat preference.

use crate::ai::AiProviderKind;
use crate::{KeyCode, KeyEvent};

use super::ai_chat_state::ChatModelPickerSection;
use super::Editor;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderChoiceOption {
    /// Profile to select, or `None` for "pick another provider".
    pub profile: Option<String>,
    pub label: String,
    pub detail: String,
    pub recommended: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderChoiceDialog {
    pub options: Vec<ProviderChoiceOption>,
    pub selected: usize,
}

fn on_path(program: &str) -> bool {
    which::which(program).is_ok()
}

impl Editor {
    pub fn provider_choice_dialog(&self) -> Option<&ProviderChoiceDialog> {
        self.ai_state.provider_choice_dialog.as_ref()
    }

    pub fn has_provider_choice_dialog(&self) -> bool {
        self.ai_state.provider_choice_dialog.is_some()
    }

    fn first_profile_with(&self, provider: AiProviderKind, preferred: &str) -> Option<String> {
        let profiles = &self.ai_state.config.profiles;
        if profiles
            .get(preferred)
            .is_some_and(|profile| profile.provider == provider)
        {
            return Some(preferred.to_string());
        }
        self.ai_profile_names_sorted()
            .into_iter()
            .find(|name| profiles[name].provider == provider)
    }

    fn provider_choice_options(&self) -> Vec<ProviderChoiceOption> {
        let mut options = Vec::new();
        if let Some(profile) = self.first_profile_with(AiProviderKind::ClaudeCode, "claude_code") {
            let detail = if on_path("node") && on_path("claude") {
                "Claude Code with your Claude subscription. Uses your existing `claude` login."
            } else {
                "Claude Code with your Claude subscription. Needs Node.js and Claude Code; run `claude auth login` once."
            };
            options.push(ProviderChoiceOption {
                profile: Some(profile),
                label: "Claude".into(),
                detail: detail.into(),
                recommended: true,
            });
        }
        // Prefer the profile the built-in configuration routes chat to.
        let codex_default = self
            .ai_state
            .config
            .contexts
            .get("chat")
            .cloned()
            .unwrap_or_else(|| "codex_sol".into());
        if let Some(profile) = self.first_profile_with(AiProviderKind::Codex, &codex_default) {
            let signed_in = matches!(
                crate::ai::codex_auth::credential_readiness(),
                crate::ai::codex_auth::CredentialReadiness::Ready
            );
            options.push(ProviderChoiceOption {
                profile: Some(profile),
                label: "Codex".into(),
                detail: if signed_in {
                    "OpenAI models with your ChatGPT plan, in Ovim's own agent harness. Signed in."
                } else {
                    "OpenAI models with your ChatGPT plan, in Ovim's own agent harness. Sign in once with your ChatGPT account."
                }
                .into(),
                recommended: false,
            });
        }
        options.push(ProviderChoiceOption {
            profile: None,
            label: "Other provider".into(),
            detail: "Local Ollama, or API keys for OpenAI or Anthropic. Opens the model picker."
                .into(),
            recommended: false,
        });
        options
    }

    /// Ask how to run inference when nothing decides it yet: no remembered
    /// selection, no chat profile authored in user configuration, and no
    /// explicit profile for this conversation.
    pub(crate) fn maybe_prompt_provider_choice_on_chat_open(&mut self) {
        if self.ai_state.provider_choice_dialog.is_some()
            || self.ai_state.chat_config_override
            || self.ai_state.chat_preference.selection.is_some()
            || self.has_codex_auth_dialog()
        {
            return;
        }
        let follows_default = self
            .ai_state
            .chat
            .as_ref()
            .is_some_and(|chat| chat.follow_chat_default && chat.opts.name != "query");
        if !follows_default {
            return;
        }
        let options = self.provider_choice_options();
        // A custom configuration without both subscriptions has already made
        // its choice; asking would only add a step.
        if options
            .iter()
            .filter(|option| option.profile.is_some())
            .count()
            < 2
        {
            return;
        }
        self.ai_state.provider_choice_dialog = Some(ProviderChoiceDialog {
            options,
            selected: 0,
        });
        self.mark_dirty();
    }

    pub(crate) fn handle_provider_choice_key(&mut self, key: KeyEvent) {
        let Some(dialog) = self.ai_state.provider_choice_dialog.as_mut() else {
            return;
        };
        let count = dialog.options.len();
        match key.code {
            KeyCode::Esc => {
                self.ai_state.provider_choice_dialog = None;
                self.set_status_message(
                    "Using the default AI provider for now; you'll be asked again next time",
                );
                // The default may still need sign-in before it can answer.
                self.maybe_prompt_codex_auth_on_chat_open();
            }
            KeyCode::Up | KeyCode::Char('k') | KeyCode::BackTab => {
                dialog.selected = (dialog.selected + count - 1) % count;
            }
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
                dialog.selected = (dialog.selected + 1) % count;
            }
            KeyCode::Enter => {
                let selected = dialog.selected;
                self.confirm_provider_choice(selected);
            }
            KeyCode::Char(digit @ '1'..='9') => {
                let index = digit as usize - '1' as usize;
                if index < count {
                    self.confirm_provider_choice(index);
                }
            }
            _ => {}
        }
        self.mark_dirty();
    }

    fn confirm_provider_choice(&mut self, index: usize) {
        let Some(option) = self
            .ai_state
            .provider_choice_dialog
            .as_ref()
            .and_then(|dialog| dialog.options.get(index).cloned())
        else {
            return;
        };
        self.ai_state.provider_choice_dialog = None;
        match option.profile {
            Some(profile) => {
                if self.ai_select_chat_profile(&profile) {
                    self.maybe_prompt_codex_auth_on_chat_open();
                }
            }
            None => {
                self.open_ai_chat_model_picker(ChatModelPickerSection::Model);
                self.set_status_message("Choose a profile; your choice is remembered");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::chat_preference::ChatPreference;
    use crate::ai::AiProfileConfig;
    use crate::mode::Mode;
    use crate::Modifiers;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: Modifiers::NONE,
        }
    }

    fn profile(name: &str, provider: AiProviderKind) -> AiProfileConfig {
        let mut profile = crate::ai::AiConfig::default().profiles["local"].clone();
        profile.name = name.into();
        profile.provider = provider;
        profile.model = "default".into();
        profile
    }

    fn editor(dir: &tempfile::TempDir) -> Editor {
        let mut editor = Editor::new();
        editor.ai_state.chat_preference = ChatPreference::load(dir.path().join("pref.json"));
        let config = &mut editor.ai_state.config;
        for (name, provider) in [
            ("claude_code", AiProviderKind::ClaudeCode),
            ("codex_sol", AiProviderKind::Codex),
        ] {
            config.profiles.insert(name.into(), profile(name, provider));
        }
        config.contexts.insert("chat".into(), "claude_code".into());
        editor
    }

    fn open_chat(editor: &mut Editor) {
        editor
            .open_ai_chat(crate::ai::chat_types::ChatOpts {
                name: "chat".into(),
                ..Default::default()
            })
            .unwrap();
    }

    #[test]
    fn first_chat_open_asks_with_claude_recommended_first() {
        let dir = tempfile::tempdir().unwrap();
        let mut editor = editor(&dir);
        open_chat(&mut editor);
        let dialog = editor.provider_choice_dialog().expect("chooser");
        let labels: Vec<_> = dialog.options.iter().map(|o| o.label.as_str()).collect();
        assert_eq!(labels, ["Claude", "Codex", "Other provider"]);
        assert!(dialog.options[0].recommended);
        assert_eq!(dialog.selected, 0);
        // The chooser replaces the old unconditional Exa prompt.
        assert!(!editor.ai_chat_has_exa_setup_dialog());
    }

    #[test]
    fn choosing_remembers_the_profile_and_stops_asking() {
        let dir = tempfile::tempdir().unwrap();
        let mut editor = editor(&dir);
        open_chat(&mut editor);
        editor.handle_provider_choice_key(key(KeyCode::Enter));
        assert!(!editor.has_provider_choice_dialog());
        assert_eq!(editor.ai_chat_effective_profile(), "claude_code");
        assert_eq!(
            ChatPreference::load(dir.path().join("pref.json"))
                .selection
                .map(|selection| selection.profile),
            Some("claude_code".to_string())
        );
        editor.set_mode(Mode::Normal);
        open_chat(&mut editor);
        assert!(!editor.has_provider_choice_dialog());
    }

    #[test]
    fn digits_pick_directly_and_other_opens_the_model_picker() {
        let dir = tempfile::tempdir().unwrap();
        let mut editor = editor(&dir);
        open_chat(&mut editor);
        editor.handle_provider_choice_key(key(KeyCode::Char('3')));
        assert!(!editor.has_provider_choice_dialog());
        assert_eq!(
            editor.ai_chat_focus(),
            crate::ai::chat_types::ChatFocus::ModelSelector
        );
        // Nothing was chosen yet, so nothing is remembered.
        assert!(editor.ai_state.chat_preference.selection.is_none());
    }

    #[test]
    fn dismissing_keeps_the_default_and_asks_again_later() {
        let dir = tempfile::tempdir().unwrap();
        let mut editor = editor(&dir);
        open_chat(&mut editor);
        editor.handle_provider_choice_key(key(KeyCode::Down));
        assert_eq!(editor.provider_choice_dialog().unwrap().selected, 1);
        editor.handle_provider_choice_key(key(KeyCode::Esc));
        assert!(!editor.has_provider_choice_dialog());
        assert!(editor.ai_state.chat_preference.selection.is_none());
        editor.set_mode(Mode::Normal);
        open_chat(&mut editor);
        assert!(editor.has_provider_choice_dialog());
    }

    #[test]
    fn a_single_subscription_profile_needs_no_choice() {
        let dir = tempfile::tempdir().unwrap();
        let mut editor = editor(&dir);
        editor.ai_state.config.profiles.remove("claude_code");
        editor
            .ai_state
            .config
            .contexts
            .insert("chat".into(), "codex_sol".into());
        open_chat(&mut editor);
        assert!(!editor.has_provider_choice_dialog());
    }

    #[test]
    fn authored_chat_profile_is_respected_without_asking() {
        let dir = tempfile::tempdir().unwrap();
        let mut editor = editor(&dir);
        editor.ai_state.chat_config_override = true;
        open_chat(&mut editor);
        assert!(!editor.has_provider_choice_dialog());
    }
}
