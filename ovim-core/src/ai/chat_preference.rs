//! The last interactive chat selection, independent of authored configuration.
use super::{AiConfig, AiProfileConfig, AiProviderKind};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ChatSelection {
    pub profile: String,
    pub provider: AiProviderKind,
    /// Only providers with an interactive model picker need an override.
    pub model: Option<String>,
}

impl ChatSelection {
    pub fn resolve<'a>(&self, config: &'a AiConfig) -> Option<&'a AiProfileConfig> {
        let profile = config.resolve_profile(&self.profile)?;
        if profile.provider != self.provider {
            return None;
        }
        if let Some(model) = &self.model {
            if profile.validate_chat_model(model).is_err() {
                return None;
            }
        }
        Some(profile)
    }
}

#[derive(Serialize, Deserialize)]
struct Document {
    version: u32,
    selection: ChatSelection,
}

#[derive(Default)]
pub(crate) struct ChatPreference {
    path: Option<PathBuf>,
    pub selection: Option<ChatSelection>,
}

impl ChatPreference {
    pub fn discover() -> Self {
        let path = std::env::var_os("OVIM_CHAT_PREFERENCE_FILE")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .or_else(|| dirs::data_local_dir().map(|root| root.join("ovim/chat-preference.json")));
        let Some(path) = path else {
            return Self::default();
        };
        Self::load(path)
    }

    pub fn load(path: PathBuf) -> Self {
        let selection = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<Document>(&bytes) {
                Ok(document) if document.version == 1 => Some(document.selection),
                _ => {
                    crate::log_warn!("ai", "Ignoring invalid chat preference: {}", path.display());
                    None
                }
            },
            Err(error) => {
                if error.kind() != std::io::ErrorKind::NotFound {
                    crate::log_warn!(
                        "ai",
                        "Could not read chat preference {}: {error}",
                        path.display()
                    );
                }
                None
            }
        };
        Self {
            path: Some(path),
            selection,
        }
    }

    pub fn reload(&mut self) {
        if let Some(path) = self.path.clone() {
            *self = Self::load(path);
        }
    }

    /// Commit only complete selections. Unique temporary files also make
    /// concurrent editors safe: the last successful selection wins.
    pub fn remember(&mut self, selection: ChatSelection) -> Result<()> {
        self.selection = Some(selection.clone());
        let Some(path) = &self.path else {
            return Ok(());
        };
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| std::path::Path::new("."));
        std::fs::create_dir_all(parent).context("create chat preference directory")?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        serde_json::to_writer_pretty(
            &mut file,
            &Document {
                version: 1,
                selection,
            },
        )?;
        file.write_all(b"\n")?;
        file.as_file().sync_all()?;
        file.persist(path).context("save chat preference")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local_selection() -> ChatSelection {
        ChatSelection {
            profile: "local".into(),
            provider: AiProviderKind::Ollama,
            model: None,
        }
    }

    #[test]
    fn selection_round_trips_and_last_writer_wins() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/preference.json");
        let mut first = ChatPreference::load(path.clone());
        let mut second = ChatPreference::load(path.clone());
        assert!(first.selection.is_none());
        first.remember(local_selection()).unwrap();
        assert_eq!(
            ChatPreference::load(path.clone()).selection,
            Some(local_selection())
        );
        let next = ChatSelection {
            profile: "claude_code".into(),
            provider: AiProviderKind::ClaudeCode,
            model: Some("opus[1m]".into()),
        };
        second.remember(next.clone()).unwrap();
        assert_eq!(ChatPreference::load(path).selection, Some(next));
        assert_eq!(
            std::fs::read_dir(dir.path().join("nested"))
                .unwrap()
                .count(),
            1
        );
    }

    #[test]
    fn corrupt_and_future_documents_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preference.json");
        for contents in [
            "{",
            "null",
            r#"{"version":2,"selection":{"profile":"local","provider":"ollama","model":null}}"#,
        ] {
            std::fs::write(&path, contents).unwrap();
            assert!(ChatPreference::load(path.clone()).selection.is_none());
        }
    }

    #[test]
    fn stale_provider_and_invalid_models_do_not_resolve() {
        let config = AiConfig::default();
        let mut selection = local_selection();
        assert!(selection.resolve(&config).is_some());
        selection.profile = "removed".into();
        assert!(selection.resolve(&config).is_none());
        selection = local_selection();
        selection.provider = AiProviderKind::ClaudeCode;
        assert!(selection.resolve(&config).is_none());
        selection = local_selection();
        for model in [
            "",
            "bad model",
            "opus\n",
            "bad\0model",
            "different-model",
            &"x".repeat(513),
        ] {
            selection.model = Some(model.into());
            assert!(selection.resolve(&config).is_none());
        }
    }

    #[test]
    fn write_failure_keeps_the_live_choice() {
        let dir = tempfile::tempdir().unwrap();
        let mut preference = ChatPreference::load(dir.path().to_path_buf());
        assert!(preference.remember(local_selection()).is_err());
        assert_eq!(preference.selection, Some(local_selection()));
    }
}
