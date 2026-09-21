use super::ai_state::AiSelectionSnapshot;
use super::Editor;
use crate::edit::Edit;
use crate::mode::Mode;
use crate::unicode::GraphemeCol;
use anyhow::Result;

impl Editor {
    /// Enable user preference storage at application startup. Bare editors and
    /// tests stay in memory unless their caller explicitly supplies storage.
    pub fn load_ai_chat_preference(&mut self) {
        self.ai_state.chat_preference = crate::ai::chat_preference::ChatPreference::discover();
    }

    /// Returns configured AI profile names sorted for deterministic picker navigation.
    pub fn ai_profile_names_sorted(&self) -> Vec<String> {
        let mut names: Vec<String> = self.ai_state.config.profiles.keys().cloned().collect();
        names.sort();
        names
    }

    /// Choices are owned by core so GUI, terminal keys and mouse stay consistent.
    pub fn ai_chat_model_options(&self) -> Vec<crate::ai::AiChatModelOption> {
        let mut options = Vec::new();
        for name in self.ai_profile_names_sorted() {
            let profile = &self.ai_state.config.profiles[&name];
            let mut models = if profile.provider == crate::ai::AiProviderKind::ClaudeCode {
                crate::ai::claude_code::MODEL_PRESETS
                    .iter()
                    .map(|preset| preset.id)
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            if name == self.ai_chat_effective_profile() {
                let selected = self.ai_chat_selected_model();
                if !selected.is_empty() && !models.contains(&selected) {
                    models.push(selected);
                }
            }
            if !models.contains(&profile.model.as_str()) {
                models.push(&profile.model);
            }
            for model in models {
                options.push(crate::ai::AiChatModelOption {
                    id: name.clone(),
                    label: profile.display_name().into(),
                    provider: profile.provider.to_string(),
                    model: model.into(),
                });
            }
        }
        options
    }

    pub fn ai_chat_selected_model(&self) -> &str {
        let Some(profile) = self
            .ai_state
            .config
            .resolve_profile(&self.ai_chat_effective_profile())
        else {
            return "";
        };
        let model = match self.ai_state.chat.as_ref() {
            Some(chat) => chat.model_override.as_deref(),
            None => self
                .ai_chat_remembered_selection()
                .and_then(|selection| selection.model.as_deref()),
        };
        model
            .filter(|model| profile.validate_chat_model(model).is_ok())
            .unwrap_or(&profile.model)
    }

    pub(crate) fn ai_chat_remembered_selection(
        &self,
    ) -> Option<&crate::ai::chat_preference::ChatSelection> {
        if self.ai_state.chat_config_override {
            return None;
        }
        self.ai_state
            .chat_preference
            .selection
            .as_ref()
            .filter(|selection| selection.resolve(&self.ai_state.config).is_some())
    }

    /// Resolve the same model for requests and presentation without changing
    /// the profile used by queries, workflows or delegated agents.
    pub(crate) fn ai_chat_resolved_profile(&self) -> Option<crate::ai::AiProfileConfig> {
        let mut profile = self
            .ai_state
            .config
            .resolve_profile(&self.ai_chat_effective_profile())?
            .clone();
        profile.model = self.ai_chat_selected_model().into();
        Some(profile)
    }

    /// Interactive profile selection is chat-specific and remembered.
    pub fn ai_select_chat_profile(&mut self, profile_name: &str) -> bool {
        let Some(profile) = self.ai_state.config.resolve_profile(profile_name) else {
            self.set_status_message(format!("Unknown AI profile: {profile_name}"));
            return false;
        };
        let model = profile.model.clone();
        self.ai_select_chat_model(profile_name, &model)
    }

    pub fn ai_select_chat_model(&mut self, profile_name: &str, model: &str) -> bool {
        let Some(profile) = self.ai_state.config.resolve_profile(profile_name) else {
            self.set_status_message(format!("Unknown AI profile: {profile_name}"));
            return false;
        };
        if let Err(error) = profile.validate_chat_model(model) {
            self.set_status_message(error.to_string());
            return false;
        }
        let selection = crate::ai::chat_preference::ChatSelection {
            profile: profile_name.into(),
            provider: profile.provider,
            model: (profile.provider == crate::ai::AiProviderKind::ClaudeCode)
                .then(|| model.into()),
        };
        if !self.apply_ai_chat_selection(profile_name, selection.model.clone()) {
            return false;
        }
        if let Some(chat) = self.ai_state.chat.as_mut() {
            chat.follow_chat_default = chat.opts.name != "query";
        }
        // A live user choice wins until configuration is loaded again.
        self.ai_state.chat_config_override = false;
        match self.ai_state.chat_preference.remember(selection) {
            Ok(()) => self.set_status_message(format!("AI profile: {profile_name} · {model}")),
            Err(error) => {
                crate::log_warn!("ai", "Could not save chat preference: {error:#}");
                self.set_status_message(format!(
                    "AI profile: {profile_name} · {model} (could not remember selection: {error})"
                ));
            }
        }
        true
    }

    /// Apply a session choice without changing defaults or writing preferences.
    /// Programmatic chat options and interactive selectors share this boundary.
    pub(crate) fn apply_ai_chat_selection(
        &mut self,
        profile_name: &str,
        model: Option<String>,
    ) -> bool {
        let Some(profile) = self.ai_state.config.resolve_profile(profile_name) else {
            self.set_status_message(format!("Unknown AI profile: {profile_name}"));
            return false;
        };
        let owns_agent_loop = profile.provider.owns_agent_loop();
        if self.ai_chat_has_pending_work() {
            self.set_status_message("Wait for or stop the active turn before changing profiles");
            return false;
        }
        if let Some(chat) = self.ai_state.chat.as_mut() {
            chat.opts.profile = Some(profile_name.into());
            chat.model_override = model;
            chat.follow_chat_default = false;
            if owns_agent_loop && chat.reasoning_effort_override.as_deref() == Some("none") {
                chat.reasoning_effort_override = None;
            }
        }
        self.mark_dirty();
        true
    }

    pub fn ai_cycle_chat_model(&mut self, forward: bool) {
        let options = self.ai_chat_model_options();
        if options.is_empty() {
            return;
        }
        let profile = self.ai_chat_effective_profile();
        let model = self.ai_chat_selected_model();
        let current = options
            .iter()
            .position(|option| option.id == profile && option.model == model)
            .unwrap_or(0);
        let next = if forward {
            (current + 1) % options.len()
        } else {
            (current + options.len() - 1) % options.len()
        };
        self.ai_select_chat_model(&options[next].id, &options[next].model);
    }

    /// Select a specific AI profile. Reports and returns false when unknown.
    pub fn ai_set_profile(&mut self, profile_name: &str) -> bool {
        let Some(profile) = self.ai_state.config.resolve_profile(profile_name) else {
            self.set_status_message(format!("Unknown AI profile: {profile_name}"));
            return false;
        };
        let provider = profile.provider;
        let model = profile.model.clone();
        if !self.apply_ai_chat_selection(profile_name, None) {
            return false;
        }
        for context in ["chat", "query"] {
            self.ai_state
                .config
                .contexts
                .insert(context.into(), profile_name.into());
        }
        self.ai_state.config.default_profile = profile_name.into();
        self.ai_state.active_profile = profile_name.to_string();
        self.set_status_message(format!(
            "AI profile: {} ({}/{})",
            profile_name, provider, model
        ));
        true
    }

    pub fn ai_cycle_profile(&mut self, forward: bool) {
        let names = self.ai_profile_names_sorted();
        if names.is_empty() {
            self.set_status_message("No AI profiles configured".to_string());
            return;
        }
        let current_idx = names
            .iter()
            .position(|name| name == &self.ai_state.active_profile)
            .unwrap_or(0);
        let next_idx = if forward {
            (current_idx + 1) % names.len()
        } else if current_idx == 0 {
            names.len() - 1
        } else {
            current_idx - 1
        };
        self.ai_set_profile(&names[next_idx]);
    }

    /// Attach the current visual selection to the editable AI chat.
    pub fn start_ai_chat_from_visual(&mut self) -> Result<()> {
        self.start_ai_chat_from_visual_with_profile(None)
    }

    /// Compatibility entry point for `vim.ai.edit_selection({ profile = ... })`.
    pub fn start_ai_chat_from_visual_with_profile(
        &mut self,
        profile: Option<String>,
    ) -> Result<()> {
        if let Some(profile_name) = profile.as_deref() {
            if self.ai_state.config.resolve_profile(profile_name).is_none() {
                self.set_status_message(format!("Unknown AI profile: {profile_name}"));
                return Ok(());
            }
        }
        if !self.capture_ai_selection_from_visual()? {
            return Ok(());
        }

        self.open_ai_chat(crate::ai::chat_types::ChatOpts {
            name: "chat".into(),
            profile: profile.clone(),
            allow_edits: true,
            ..Default::default()
        })?;
        let selection = self
            .ai_state
            .active_selection
            .as_ref()
            .expect("selection was captured")
            .clone();
        let buffer = self
            .get_buffer_by_id(selection.buffer_id)
            .expect("selected buffer remains open");
        let source_context = buffer
            .display_name()
            .filter(|name| name.starts_with("Diff · "))
            .map(|_| {
                buffer
                    .rope()
                    .lines()
                    .take(2)
                    .map(|line| line.to_string())
                    .collect::<String>()
                    .trim()
                    .to_string()
            })
            .filter(|context| !context.is_empty());
        let attachment_path = source_context
            .as_deref()
            .and_then(|context| context.lines().find_map(|line| line.strip_prefix("file: ")))
            .map(str::to_string)
            .or_else(|| buffer.file_path().map(ToString::to_string));
        let attachment = super::ai_chat_state::CodeAttachment {
            buffer_id: selection.buffer_id,
            path: attachment_path,
            start_line: selection.start_line,
            start_column: selection.start_col,
            end_line: selection.end_line,
            end_column: selection.end_col.saturating_sub(1),
            linewise: selection.selection_mode == Mode::VisualLine,
            buffer_revision: buffer.version(),
            source_context,
            text: selection.selected_text,
        };
        let label = attachment.label();
        if let Some(chat) = self.ai_state.chat.as_mut() {
            chat.active_buffer_id = selection.buffer_id;
            chat.pending_code_attachment = Some(attachment);
        }
        self.set_status_message(format!("Attached {label} to AI chat"));
        Ok(())
    }

    fn capture_ai_selection_from_visual(&mut self) -> Result<bool> {
        if self.mode() == Mode::VisualBlock {
            self.set_status_message("AI chat does not support visual block selections".to_string());
            return Ok(false);
        }

        let Some(((start_line, start_col), (end_line, end_col))) = self.visual_selection() else {
            self.set_status_message("No visual selection to attach".to_string());
            return Ok(false);
        };

        let rope = self.buffer().rope();
        let rope_len = rope.len_chars();
        let (start_col, end_col, start_char, end_char) = match self.mode() {
            Mode::VisualLine => {
                let start = rope.line_to_char(start_line).min(rope_len);
                let end = if end_line + 1 < self.buffer().raw_line_count() {
                    rope.line_to_char(end_line + 1)
                } else {
                    rope_len
                };
                let end_col =
                    crate::unicode::grapheme_count(&crate::display::line_content(rope, end_line));
                (0, end_col, start, end.min(rope_len))
            }
            _ => {
                let start_line_text = crate::display::line_content(rope, start_line);
                let end_line_text = crate::display::line_content(rope, end_line);
                let end_col = end_col.saturating_add(1);
                let start = rope.line_to_char(start_line)
                    + crate::unicode::grapheme_to_char_col(
                        &start_line_text,
                        GraphemeCol(start_col),
                    )
                    .0;
                let end = rope.line_to_char(end_line)
                    + crate::unicode::grapheme_to_char_col(&end_line_text, GraphemeCol(end_col)).0;
                (start_col, end_col, start.min(rope_len), end.min(rope_len))
            }
        };

        if end_char <= start_char {
            self.set_status_message("Visual selection is empty".to_string());
            return Ok(false);
        }

        self.ai_state.active_selection = Some(AiSelectionSnapshot {
            buffer_id: self.buffer().id(),
            start_line,
            start_col,
            end_line,
            end_col,
            start_char,
            end_char,
            anchor_line: start_line,
            selected_text: rope.slice(start_char..end_char).to_string(),
            selection_mode: self.mode(),
        });
        Ok(true)
    }

    pub(crate) fn cursor_abs_char(&self) -> usize {
        let cursor = self.buffer().cursor();
        let rope = self.buffer().rope();
        if rope.len_lines() == 0 {
            return 0;
        }
        let line = cursor.line().min(rope.len_lines().saturating_sub(1));
        let line_start = rope.line_to_char(line);
        let line_end = if line + 1 < rope.len_lines() {
            rope.line_to_char(line + 1)
        } else {
            rope.len_chars()
        };
        let content_end = if line_end > line_start && rope.char(line_end - 1) == '\n' {
            line_end - 1
        } else {
            line_end
        };
        line_start + cursor.col().0.min(content_end.saturating_sub(line_start))
    }

    pub(crate) fn set_cursor_from_abs_char(&mut self, abs_char: usize) {
        let rope = self.buffer().rope();
        let clamped = abs_char.min(rope.len_chars());
        let line = rope.char_to_line(clamped);
        let char_col = clamped.saturating_sub(rope.line_to_char(line));
        let line_text = crate::display::line_content(rope, line);
        let col =
            crate::unicode::char_to_grapheme_col(&line_text, crate::unicode::CharCol(char_col));
        self.buffer_mut().cursor_mut().set_position(line, col);
        if !matches!(
            self.mode(),
            Mode::Insert | Mode::Replace | Mode::Command | Mode::Search | Mode::RenameInput
        ) {
            self.buffer_mut().validate_cursor_position();
        }
    }
}

pub(crate) fn remap_abs_char_through_edits(mut abs_char: usize, edits: &[Edit]) -> usize {
    for edit in edits {
        match edit {
            Edit::Insert { offset, text } => {
                if *offset <= abs_char {
                    abs_char = abs_char.saturating_add(text.chars().count());
                }
            }
            Edit::Delete { offset, text } => {
                let delete_end = offset.saturating_add(text.chars().count());
                if abs_char >= delete_end {
                    abs_char = abs_char.saturating_sub(text.chars().count());
                } else if abs_char > *offset {
                    abs_char = *offset;
                }
            }
        }
    }
    abs_char
}

#[cfg(test)]
mod model_selection_tests {
    use super::super::ai_external_agent::tests::{attach, editor};

    #[test]
    fn claude_model_choices_preserve_configured_profiles_and_custom_ids() {
        let mut editor = editor();
        let before = editor.ai_state.config.profiles.len();
        let models: Vec<_> = editor
            .ai_chat_model_options()
            .into_iter()
            .filter(|option| option.id == "claude_code")
            .map(|option| option.model)
            .collect();
        assert_eq!(
            models,
            [
                "default",
                "claude-sonnet-5",
                "claude-opus-5",
                "claude-fable-5-1",
                "claude-haiku-4-5-20251001"
            ]
        );
        editor.ai_state.chat.as_mut().unwrap().input = "keep my draft".into();
        assert!(editor.ai_select_chat_model("claude_code", "opus"));
        assert_eq!(editor.ai_chat_selected_model(), "opus");
        assert_eq!(
            editor.ai_state.config.profiles["claude_code"].model,
            "default"
        );
        assert_eq!(editor.ai_state.config.profiles.len(), before);
        assert_eq!(editor.ai_chat_input(), "keep my draft");
        assert!(editor.ai_select_chat_profile("local"));
        assert!(editor.ai_select_chat_profile("claude_code"));
        assert_eq!(editor.ai_chat_selected_model(), "default");
        assert_eq!(editor.ai_state.config.default_profile, "claude_code");
        assert!(editor
            .try_execute_ai_chat_slash_command("/model claude-custom-version[1m]")
            .unwrap());
        assert_eq!(editor.ai_chat_selected_model(), "claude-custom-version[1m]");
        assert_eq!(
            editor
                .ai_chat_model_options()
                .iter()
                .filter(|option| option.model == "claude-custom-version[1m]")
                .count(),
            1
        );
        assert!(editor
            .try_execute_ai_chat_slash_command("/model default")
            .unwrap());
        assert_eq!(editor.ai_chat_selected_model(), "default");
        assert!(editor
            .try_execute_ai_chat_slash_command("/model local")
            .unwrap());
        assert_eq!(editor.ai_chat_effective_profile(), "local");
    }

    #[tokio::test]
    async fn invalid_or_busy_model_selection_is_atomic() {
        let mut editor = editor();
        for model in ["", " ", "opus\n", "bad\0model", &"a".repeat(513)] {
            assert!(!editor.ai_select_chat_model("claude_code", model));
        }
        assert!(!editor.ai_select_chat_model("unknown", "opus"));
        assert!(!editor.ai_select_chat_model("local", "opus"));
        assert_eq!(editor.ai_chat_selected_model(), "default");
        let _sender = attach(&mut editor);
        assert!(!editor.ai_select_chat_model("claude_code", "opus"));
        editor.ai_cycle_chat_model(true);
        assert_eq!(editor.ai_chat_selected_model(), "default");
        assert_eq!(editor.ai_chat_effective_profile(), "claude_code");
        assert_eq!(editor.ai_state.config.default_profile, "claude_code");
    }

    #[test]
    fn model_effort_defaults_respect_overrides_and_haiku_capabilities() {
        let mut editor = editor();
        for (model, expected) in [
            ("claude-fable-5-1", "medium"),
            ("fable[1m]", "medium"),
            ("claude-opus-5", "high"),
            ("claude-sonnet-5", "high"),
            ("claude-haiku-4-5-20251001", "default"),
            ("default", "default"),
            ("custom-deployment", "default"),
        ] {
            assert!(editor.ai_select_chat_model("claude_code", model));
            assert_eq!(editor.ai_chat_reasoning_effort(), expected, "{model}");
            assert_eq!(
                editor.ai_chat_default_reasoning_effort(),
                expected,
                "{model}"
            );
            assert_eq!(editor.ai_chat_reasoning_effort_selection(), "default");
            let profile = editor.ai_chat_resolved_profile().unwrap();
            assert_eq!(
                profile.resolve_reasoning_effort(None).unwrap_or("default"),
                expected
            );
        }
        assert!(editor.ai_select_chat_model("claude_code", "claude-fable-5-1"));
        editor
            .ai_state
            .config
            .profiles
            .get_mut("claude_code")
            .unwrap()
            .reasoning_effort = Some("low".into());
        assert_eq!(editor.ai_chat_reasoning_effort(), "low");
        assert!(editor.set_ai_chat_reasoning_effort("max"));
        assert_eq!(editor.ai_chat_reasoning_effort(), "max");
        assert_eq!(editor.ai_chat_default_reasoning_effort(), "low");
        assert!(editor.ai_select_chat_model("claude_code", "claude-haiku-4-5-20251001"));
        assert_eq!(editor.ai_chat_reasoning_effort(), "default");
        assert_eq!(editor.ai_chat_reasoning_effort_selection(), "default");
        assert_eq!(editor.ai_chat_reasoning_efforts(), ["default"]);
        assert!(!editor.set_ai_chat_reasoning_effort("high"));
        assert_eq!(
            editor
                .ai_chat_resolved_profile()
                .unwrap()
                .resolve_reasoning_effort(Some("max")),
            None
        );
        assert!(editor.ai_select_chat_model("claude_code", "claude-fable-5-1"));
        assert_eq!(editor.ai_chat_reasoning_effort(), "max");
        assert!(editor.set_ai_chat_reasoning_effort("default"));
        assert_eq!(editor.ai_chat_reasoning_effort(), "low");
        assert!(editor.ai_set_profile("local"));
        assert!(editor.set_ai_chat_reasoning_effort("none"));
        assert!(editor.ai_select_chat_model("claude_code", "claude-fable-5-1"));
        assert_eq!(editor.ai_chat_reasoning_effort_selection(), "default");
        assert_eq!(editor.ai_chat_reasoning_effort(), "low");
    }

    #[test]
    fn remembered_selection_survives_a_new_editor_without_changing_query_defaults() {
        use crate::ai::chat_preference::ChatPreference;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preference.json");
        let mut first = editor();
        first.ai_state.chat_preference = ChatPreference::load(path.clone());
        assert!(first.ai_select_chat_model("claude_code", "opus[1m]"));
        assert!(first.ai_select_chat_profile("local"));
        assert_eq!(first.ai_state.config.default_profile, "claude_code");
        assert_eq!(
            first.ai_chat_context_profile("query").as_deref(),
            Some("claude_code")
        );
        assert!(first.ai_select_chat_model("claude_code", "opus[1m]"));
        drop(first);

        let mut reopened = editor();
        reopened.ai_state.chat = None;
        reopened.ai_state.chat_preference = ChatPreference::load(path.clone());
        assert_eq!(reopened.ai_chat_selected_model(), "opus[1m]");
        reopened
            .open_ai_chat(crate::ai::ChatOpts::default())
            .unwrap();
        assert_eq!(reopened.ai_chat_selected_model(), "opus[1m]");
        assert_eq!(
            reopened.ai_chat_resolved_profile().unwrap().model,
            "opus[1m]"
        );
        reopened
            .open_ai_chat(crate::ai::ChatOpts {
                profile: Some("local".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(reopened.ai_chat_effective_profile(), "local");
        // A one-off explicit profile does not replace the remembered default.
        assert_eq!(
            ChatPreference::load(path.clone())
                .selection
                .unwrap()
                .model
                .as_deref(),
            Some("opus[1m]")
        );
        reopened.ai_state.chat = None;
        reopened
            .open_ai_chat(crate::ai::ChatOpts::default())
            .unwrap();
        reopened.close_ai_chat();
        reopened
            .open_ai_chat(crate::ai::ChatOpts::default())
            .unwrap();
        assert_eq!(reopened.ai_chat_selected_model(), "opus[1m]");

        reopened
            .open_ai_chat(crate::ai::ChatOpts {
                name: "query".into(),
                allow_edits: false,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(reopened.ai_chat_selected_model(), "default");
        reopened
            .open_ai_chat(crate::ai::ChatOpts {
                name: "explicit".into(),
                profile: Some("claude_code".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(reopened.ai_chat_selected_model(), "default");
        assert_eq!(
            ChatPreference::load(path)
                .selection
                .unwrap()
                .model
                .as_deref(),
            Some("opus[1m]")
        );
    }

    #[test]
    fn explicit_matching_profile_stops_following_the_chat_default() {
        let mut editor = editor();
        assert!(editor.ai_select_chat_profile("local"));
        assert!(editor.ai_state.chat.as_ref().unwrap().follow_chat_default);
        editor
            .open_ai_chat(crate::ai::ChatOpts {
                profile: Some("local".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(editor.ai_chat_effective_profile(), "local");
        assert!(!editor.ai_state.chat.as_ref().unwrap().follow_chat_default);
    }

    #[tokio::test]
    async fn rejected_selection_does_not_write_and_save_failure_is_visible() {
        use crate::ai::chat_preference::ChatPreference;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preference.json");
        let mut editor = editor();
        editor.ai_state.chat_preference = ChatPreference::load(path.clone());
        assert!(editor.ai_select_chat_model("claude_code", "opus"));
        let saved = std::fs::read(&path).unwrap();
        assert!(!editor.ai_select_chat_model("claude_code", "bad model"));
        let _sender = attach(&mut editor);
        assert!(!editor.ai_select_chat_profile("local"));
        assert_eq!(std::fs::read(&path).unwrap(), saved);
        editor.cancel_ai_chat_generation();
        editor.ai_state.chat_preference = ChatPreference::load(dir.path().to_path_buf());
        assert!(editor.ai_select_chat_profile("local"));
        assert_eq!(editor.ai_chat_effective_profile(), "local");
        assert!(editor
            .status_message()
            .contains("could not remember selection"));
    }

    #[test]
    fn terminal_keyboard_selects_models_within_the_same_profile() {
        let mut editor = editor();
        editor.open_ai_chat_model_picker(super::super::ChatModelPickerSection::Model);
        for expected in [
            "claude-sonnet-5",
            "claude-opus-5",
            "claude-fable-5-1",
            "claude-haiku-4-5-20251001",
        ] {
            super::super::input::InputHandler::handle_key_event(
                &mut editor,
                crate::KeyEvent::new(crate::KeyCode::Down, crate::Modifiers::NONE),
            )
            .unwrap();
            assert_eq!(editor.ai_chat_effective_profile(), "claude_code");
            assert_eq!(editor.ai_chat_selected_model(), expected);
        }
    }
}
