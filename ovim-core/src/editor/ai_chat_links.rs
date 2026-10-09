//! Resolve chat links without sending local files to an external application.

use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use url::Url;

use super::Editor;
use crate::mode::Mode;
use crate::unicode::GraphemeCol;

impl Editor {
    /// Return website/email destinations to the frontend's system opener.
    /// Existing local files open in the editor; failed links leave chat intact.
    pub(super) fn open_ai_chat_link(&mut self, destination: &str) -> Option<String> {
        match self.resolve_ai_chat_link(destination) {
            Ok(ChatLinkTarget::External(url)) => Some(url),
            Ok(ChatLinkTarget::File { path, line, column }) => {
                if let Err(error) = self.open_file(&path) {
                    self.set_status_message(format!("Could not open chat link: {error}"));
                    return None;
                }
                self.sync_current_tab_buffer();
                self.close_ai_chat();
                self.set_mode(Mode::Normal);
                if let Some(line) = line {
                    self.buffer_mut()
                        .cursor_mut()
                        .set_position(line - 1, GraphemeCol(column.unwrap_or(1) - 1));
                    self.buffer_mut().validate_cursor_position();
                }
                self.center_cursor_in_viewport();
                self.mark_dirty();
                None
            }
            Err(error) => {
                self.set_status_message(format!("Could not open chat link: {error}"));
                None
            }
        }
    }

    fn resolve_ai_chat_link(&self, destination: &str) -> Result<ChatLinkTarget> {
        if destination.is_empty() || destination.chars().any(char::is_control) {
            bail!("Invalid destination");
        }
        if let Ok(url) = Url::parse(destination) {
            if matches!(url.scheme(), "http" | "https" | "mailto") {
                return Ok(ChatLinkTarget::External(url.into()));
            }
        }

        let (path, fragment) = destination
            .split_once('#')
            .map_or((destination, None), |(path, fragment)| {
                (path, Some(fragment))
            });
        let (path, suffix_line, column) = split_file_location(path);
        let line = fragment
            .and_then(|fragment| fragment.strip_prefix('L'))
            .and_then(|fragment| fragment.split('-').next())
            .and_then(positive_number)
            .or(suffix_line);
        if path.is_empty() {
            bail!("This link has no file destination");
        }
        let path = match Url::parse(path) {
            Ok(url) if url.scheme() == "file" => url
                .to_file_path()
                .map_err(|()| anyhow::anyhow!("Invalid file URL"))?,
            Ok(url) => bail!("Unsupported link scheme: {}", url.scheme()),
            Err(_) => {
                let root = self.ai_chat_link_root();
                let base = Url::from_directory_path(&root)
                    .map_err(|()| anyhow::anyhow!("Invalid project directory"))?;
                // Decode URL escapes relative to the project directory.
                let relative = if Path::new(path).is_absolute() {
                    path.to_string()
                } else {
                    format!("./{path}")
                };
                base.join(&relative)?
                    .to_file_path()
                    .map_err(|()| anyhow::anyhow!("Invalid file path"))?
            }
        };
        if !path.is_file() {
            bail!("File does not exist: {}", path.display());
        }
        Ok(ChatLinkTarget::File { path, line, column })
    }

    fn ai_chat_link_root(&self) -> PathBuf {
        if let Some(root) = self.file_tree().root_path() {
            return root.to_path_buf();
        }
        self.ai_state
            .chat
            .as_ref()
            .and_then(|chat| self.get_buffer_by_id(chat.origin_buffer_id))
            .and_then(|buffer| buffer.file_path())
            .map(|path| crate::project_root::vcs_root_or_dir(Path::new(path)))
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
    }
}

enum ChatLinkTarget {
    External(String),
    File {
        path: PathBuf,
        line: Option<usize>,
        column: Option<usize>,
    },
}

fn positive_number(text: &str) -> Option<usize> {
    text.parse().ok().filter(|number| *number > 0)
}

fn split_file_location(path: &str) -> (&str, Option<usize>, Option<usize>) {
    let Some((prefix, last)) = path.rsplit_once(':') else {
        return (path, None, None);
    };
    let Some(last) = positive_number(last) else {
        return (path, None, None);
    };
    if let Some((path, line)) = prefix.rsplit_once(':') {
        if let Some(line) = positive_number(line) {
            return (path, Some(line), Some(last));
        }
    }
    (prefix, Some(last), None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::ChatOpts;

    fn editor_at(root: &Path) -> Editor {
        let mut editor = Editor::default();
        editor.set_workspace_root(root).unwrap();
        editor.open_ai_chat(ChatOpts::default()).unwrap();
        editor
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn project_links_open_existing_files_at_the_requested_location() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("my file.rs");
        std::fs::write(&file, "one\ntwo\nthree\n").unwrap();
        for destination in [
            "my%20file.rs:2:2".to_string(),
            format!("{}:2:2", file.display()),
            format!("{}#L2", Url::from_file_path(&file).unwrap()),
            "my%20file.rs#L2-L3".to_string(),
        ] {
            let mut editor = editor_at(root.path());
            assert!(editor.open_ai_chat_link(&destination).is_none());
            assert_eq!(editor.mode(), Mode::Normal, "{destination}");
            assert_eq!(
                editor.buffer().file_path(),
                file.canonicalize().unwrap().to_str()
            );
            assert_eq!(editor.buffer().cursor().line(), 1);
            assert_eq!(
                editor.tab_page_manager().current_tab().buffer_id(),
                Some(editor.buffer().id())
            );
            assert!(
                editor.ai_state.chat.is_some(),
                "Chat state must survive opening a file"
            );
        }
    }

    #[test]
    fn external_links_are_returned_to_the_frontend() {
        let root = tempfile::tempdir().unwrap();
        let mut editor = editor_at(root.path());
        for destination in [
            "https://example.com/docs#L2",
            "http://example.com/",
            "mailto:a@example.com",
        ] {
            assert_eq!(
                editor.open_ai_chat_link(destination).as_deref(),
                Some(destination)
            );
            assert_eq!(editor.mode(), Mode::AiChat);
        }
    }

    #[test]
    fn invalid_links_leave_chat_and_buffers_intact() {
        let root = tempfile::tempdir().unwrap();
        let mut editor = editor_at(root.path());
        let buffer = editor.buffer().id();
        for destination in [
            "",
            "#heading",
            "missing.rs:4",
            "javascript:alert(1)",
            "data:text/plain,test",
            "https://example.com/\n",
        ] {
            assert!(editor.open_ai_chat_link(destination).is_none());
            assert_eq!(editor.mode(), Mode::AiChat);
            assert_eq!(editor.buffer().id(), buffer);
        }
        assert!(!root.path().join("missing.rs").exists());
    }

    #[test]
    fn file_location_suffixes_are_one_based_and_preserve_other_colons() {
        assert_eq!(
            split_file_location("src/main.rs:42:3"),
            ("src/main.rs", Some(42), Some(3))
        );
        assert_eq!(
            split_file_location("file:with:colon.rs:42"),
            ("file:with:colon.rs", Some(42), None)
        );
        assert_eq!(split_file_location("file.rs:0"), ("file.rs:0", None, None));
    }
}
