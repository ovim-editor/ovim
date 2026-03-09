use crate::editor::Editor;
use lsp_types::Position;
use std::time::{Duration, Instant};

const INLAY_HINT_REFRESH_DEBOUNCE: Duration = Duration::from_millis(250);

fn should_skip_inlay_hint_refresh(
    last_key: Option<&crate::editor::lsp_state::InlayHintRequestKey>,
    last_at: Option<Instant>,
    next_key: &crate::editor::lsp_state::InlayHintRequestKey,
    now: Instant,
) -> bool {
    last_key == Some(next_key)
        && last_at.is_some_and(|last| now.duration_since(last) < INLAY_HINT_REFRESH_DEBOUNCE)
}

fn current_inlay_hint_request_key(
    editor: &Editor,
) -> Option<crate::editor::lsp_state::InlayHintRequestKey> {
    let file_path = editor.buffer().file_path()?.to_string();
    crate::syntax::LanguageRegistry::get_lsp_language_id(&file_path)?;

    let start_line = editor.scroll_offset();
    let end_line = start_line + editor.viewport_height() + 10;

    Some(crate::editor::lsp_state::InlayHintRequestKey {
        file_path,
        start_line,
        end_line,
        lsp_version: editor.lsp_state.current_file_lsp_sent_version,
    })
}

#[cfg(test)]
fn viewport_matches_request_key(
    editor: &Editor,
    request_key: &crate::editor::lsp_state::InlayHintRequestKey,
) -> bool {
    editor
        .buffer()
        .file_path()
        .is_some_and(|path| path == request_key.file_path)
        && editor.scroll_offset() == request_key.start_line
        && editor.scroll_offset() + editor.viewport_height() + 10 == request_key.end_line
}

fn pending_request_matches_viewport(
    editor: &Editor,
    pending: &crate::editor::lsp_state::PendingInlayHintRequest,
) -> bool {
    pending.buffer_version == editor.buffer().version()
        && editor
            .buffer()
            .file_path()
            .is_some_and(|path| path == pending.request_key.file_path)
        && editor.scroll_offset() == pending.request_key.start_line
        && editor.scroll_offset() + editor.viewport_height() + 10 == pending.request_key.end_line
}

impl Editor {
    /// Returns true when the viewport/LSP-sync fingerprint differs from the last
    /// applied or pending inlay hint request.
    pub fn inlay_hints_refresh_needed_for_viewport(&self) -> bool {
        if self.lsp_state.lsp_manager.is_none() {
            return false;
        }

        let Some(next_key) = current_inlay_hint_request_key(self) else {
            return false;
        };

        if self
            .lsp_state
            .pending_inlay_hints
            .as_ref()
            .is_some_and(|pending| pending_request_matches_viewport(self, pending))
        {
            return false;
        }

        if self.lsp_state.applied_inlay_hint_request.as_ref() == Some(&next_key) {
            return false;
        }

        !should_skip_inlay_hint_refresh(
            self.lsp_state.last_inlay_hint_request.as_ref(),
            self.lsp_state.last_inlay_hint_request_at,
            &next_key,
            Instant::now(),
        )
    }

    /// Spawn a background inlay hint refresh for the current viewport.
    pub fn request_inlay_hints_refresh(&mut self) {
        let Some(lsp) = self.lsp_state.lsp_manager.clone() else {
            return;
        };

        let Some(request_key) = current_inlay_hint_request_key(self) else {
            return;
        };

        let now = Instant::now();
        if should_skip_inlay_hint_refresh(
            self.lsp_state.last_inlay_hint_request.as_ref(),
            self.lsp_state.last_inlay_hint_request_at,
            &request_key,
            now,
        ) {
            return;
        }

        if self
            .lsp_state
            .pending_inlay_hints
            .as_ref()
            .is_some_and(|pending| pending.request_key == request_key)
        {
            return;
        }

        if let Some(pending) = self.lsp_state.pending_inlay_hints.take() {
            pending.request.task.abort();
        }

        let Some(uri) = crate::lsp::uri_from_file_path(&request_key.file_path) else {
            return;
        };

        let Some(language_id) =
            crate::syntax::LanguageRegistry::get_lsp_language_id(&request_key.file_path)
        else {
            return;
        };
        let buffer_version = self.buffer().version();

        let state_key = request_key.file_path.clone();
        let initial_content = self.buffer().rope().to_string();
        let sync_plan = self.document_sync_request_plan(&state_key, &initial_content);

        self.lsp_state.last_inlay_hint_request = Some(request_key.clone());
        self.lsp_state.last_inlay_hint_request_at = Some(now);

        self.lsp_state.inlay_hint_request_seq =
            self.lsp_state.inlay_hint_request_seq.wrapping_add(1);
        let seq = self.lsp_state.inlay_hint_request_seq;

        let request_key_for_task = request_key.clone();
        let language_id = language_id.to_string();
        let (tx, rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let mut synced_content = None;
            let content = initial_content;
            let mut lsp_version = lsp.get_last_sent_version(&uri).await;

            match sync_plan.action {
                super::super::DocumentSyncRequestAction::Noop => {}
                super::super::DocumentSyncRequestAction::DidOpen => {
                    if lsp
                        .did_open_broadcast(uri.clone(), &language_id, 1, content.clone())
                        .await
                        .is_ok()
                    {
                        synced_content = Some(content.clone());
                        lsp_version = lsp.get_last_sent_version(&uri).await;
                    }
                }
                super::super::DocumentSyncRequestAction::FlushQueued => {
                    let _ = lsp
                        .flush_pending_changes_broadcast(&uri, &language_id)
                        .await;
                    synced_content = Some(content.clone());
                    lsp_version = lsp.get_last_sent_version(&uri).await;
                }
                super::super::DocumentSyncRequestAction::QueueChangeAndFlush => {
                    if lsp
                        .did_change_broadcast(
                            uri.clone(),
                            &language_id,
                            content.clone(),
                            sync_plan.old_content,
                        )
                        .await
                        .is_ok()
                    {
                        let _ = lsp
                            .flush_pending_changes_broadcast(&uri, &language_id)
                            .await;
                        synced_content = Some(content.clone());
                        lsp_version = lsp.get_last_sent_version(&uri).await;
                    }
                }
            }

            if lsp_version <= 0 {
                return Err(anyhow::anyhow!(
                    "LSP document not ready for inlay hints: {}",
                    request_key_for_task.file_path
                ));
            }

            let range = lsp_types::Range {
                start: Position {
                    line: request_key_for_task.start_line as u32,
                    character: 0,
                },
                end: Position {
                    line: request_key_for_task.end_line as u32,
                    character: 0,
                },
            };

            let synced_lsp_version = synced_content.as_ref().map(|_| lsp_version);
            let result = lsp
                .inlay_hints(&uri, range, &language_id)
                .await
                .map(|hints| crate::editor::lsp_state::InlayHintTaskResult {
                    request_key: crate::editor::lsp_state::InlayHintRequestKey {
                        lsp_version,
                        ..request_key_for_task.clone()
                    },
                    buffer_version,
                    synced_content,
                    synced_lsp_version,
                    hints,
                });

            let _ = tx.send(result);

            Ok(crate::editor::lsp_state::InlayHintTaskResult {
                request_key: request_key_for_task,
                buffer_version,
                synced_content: None,
                synced_lsp_version: None,
                hints: Vec::new(),
            })
        });

        self.lsp_state.pending_inlay_hints =
            Some(crate::editor::lsp_state::PendingInlayHintRequest {
                seq,
                request_key,
                buffer_version,
                request: crate::editor::lsp_state::PendingLspRequest {
                    task,
                    receiver: rx,
                    started: Instant::now(),
                },
            });
    }

    /// Get inlay hints for a specific line (0-indexed).
    pub fn inlay_hints_for_line(&self, line: usize) -> Vec<&lsp_types::InlayHint> {
        self.lsp_state
            .inlay_hints
            .iter()
            .filter(|h| h.position.line as usize == line)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_duplicate_refresh_inside_debounce_window() {
        let key = crate::editor::lsp_state::InlayHintRequestKey {
            file_path: "src/Test.java".to_string(),
            start_line: 10,
            end_line: 40,
            lsp_version: 3,
        };
        let now = Instant::now();

        assert!(should_skip_inlay_hint_refresh(
            Some(&key),
            Some(now),
            &key,
            now + Duration::from_millis(100),
        ));
    }

    #[test]
    fn allows_refresh_after_debounce_window_or_key_change() {
        let key = crate::editor::lsp_state::InlayHintRequestKey {
            file_path: "src/Test.java".to_string(),
            start_line: 10,
            end_line: 40,
            lsp_version: 3,
        };
        let changed_key = crate::editor::lsp_state::InlayHintRequestKey {
            file_path: "src/Test.java".to_string(),
            start_line: 20,
            end_line: 50,
            lsp_version: 3,
        };
        let now = Instant::now();

        assert!(!should_skip_inlay_hint_refresh(
            Some(&key),
            Some(now),
            &key,
            now + Duration::from_millis(300),
        ));
        assert!(!should_skip_inlay_hint_refresh(
            Some(&key),
            Some(now),
            &changed_key,
            now + Duration::from_millis(100),
        ));
    }

    #[test]
    fn viewport_change_requires_refresh_after_initial_sync() {
        let mut editor = Editor::with_content("class Test {}\n");
        editor.enable_lsp();
        editor.set_file_path("/tmp/Test.java".to_string());
        editor.set_viewport_height(20);
        editor.lsp_state.current_file_lsp_sent_version = 1;

        assert!(editor.inlay_hints_refresh_needed_for_viewport());

        let key = current_inlay_hint_request_key(&editor).expect("request key");
        editor.lsp_state.applied_inlay_hint_request = Some(key);
        assert!(!editor.inlay_hints_refresh_needed_for_viewport());

        editor.viewport.scroll_offset = 5;
        assert!(editor.inlay_hints_refresh_needed_for_viewport());
    }

    #[test]
    fn initial_viewport_probe_can_request_background_sync() {
        let mut editor = Editor::with_content("class Test {}\n");
        editor.enable_lsp();
        editor.set_file_path("/tmp/Test.java".to_string());
        editor.set_viewport_height(20);

        assert!(editor.inlay_hints_refresh_needed_for_viewport());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn matching_pending_request_suppresses_duplicate_refresh() {
        let mut editor = Editor::with_content("class Test {}\n");
        editor.enable_lsp();
        editor.set_file_path("/tmp/Test.java".to_string());
        editor.set_viewport_height(20);

        let key = current_inlay_hint_request_key(&editor).expect("request key");
        let (_, receiver) = tokio::sync::oneshot::channel::<
            anyhow::Result<crate::editor::lsp_state::InlayHintTaskResult>,
        >();
        editor.lsp_state.pending_inlay_hints =
            Some(crate::editor::lsp_state::PendingInlayHintRequest {
                seq: 1,
                request_key: key,
                request: crate::editor::lsp_state::PendingLspRequest {
                    task: tokio::spawn(async {
                        Ok(crate::editor::lsp_state::InlayHintTaskResult {
                            request_key: crate::editor::lsp_state::InlayHintRequestKey {
                                file_path: String::new(),
                                start_line: 0,
                                end_line: 0,
                                lsp_version: 0,
                            },
                            buffer_version: 0,
                            synced_content: None,
                            synced_lsp_version: None,
                            hints: Vec::new(),
                        })
                    }),
                    receiver,
                    started: Instant::now(),
                },
                buffer_version: editor.buffer().version(),
            });

        assert!(!editor.inlay_hints_refresh_needed_for_viewport());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn pending_request_same_viewport_suppresses_refresh_after_sent_version_advances() {
        let mut editor = Editor::with_content("class Test {}\n");
        editor.enable_lsp();
        editor.set_file_path("/tmp/Test.java".to_string());
        editor.set_viewport_height(20);
        editor.lsp_state.current_file_lsp_sent_version = 1;

        let pending_key = crate::editor::lsp_state::InlayHintRequestKey {
            file_path: "/tmp/Test.java".to_string(),
            start_line: 0,
            end_line: 30,
            lsp_version: 0,
        };
        let (_, receiver) = tokio::sync::oneshot::channel::<
            anyhow::Result<crate::editor::lsp_state::InlayHintTaskResult>,
        >();
        editor.lsp_state.pending_inlay_hints =
            Some(crate::editor::lsp_state::PendingInlayHintRequest {
                seq: 1,
                request_key: pending_key,
                request: crate::editor::lsp_state::PendingLspRequest {
                    task: tokio::spawn(async {
                        Ok(crate::editor::lsp_state::InlayHintTaskResult {
                            request_key: crate::editor::lsp_state::InlayHintRequestKey {
                                file_path: String::new(),
                                start_line: 0,
                                end_line: 0,
                                lsp_version: 0,
                            },
                            buffer_version: 0,
                            synced_content: None,
                            synced_lsp_version: None,
                            hints: Vec::new(),
                        })
                    }),
                    receiver,
                    started: Instant::now(),
                },
                buffer_version: editor.buffer().version(),
            });

        assert!(!editor.inlay_hints_refresh_needed_for_viewport());
    }

    #[test]
    fn buffer_edits_do_not_require_refresh_until_sent_version_changes() {
        let mut editor = Editor::with_content("class Test {}\n");
        editor.enable_lsp();
        editor.set_file_path("/tmp/Test.java".to_string());
        editor.set_viewport_height(20);
        editor.lsp_state.current_file_lsp_sent_version = 3;

        let key = current_inlay_hint_request_key(&editor).expect("request key");
        editor.lsp_state.applied_inlay_hint_request = Some(key);

        editor.buffer_mut().insert_text_at(0, 0, "x");
        assert!(!editor.inlay_hints_refresh_needed_for_viewport());

        editor.lsp_state.current_file_lsp_sent_version = 4;
        assert!(editor.inlay_hints_refresh_needed_for_viewport());
    }

    #[test]
    fn viewport_match_checks_file_buffer_and_scroll() {
        let mut editor = Editor::with_content("class Test {}\n");
        editor.set_file_path("/tmp/Test.java".to_string());
        editor.set_viewport_height(20);

        let key = current_inlay_hint_request_key(&editor).expect("request key");
        assert!(viewport_matches_request_key(&editor, &key));

        editor.viewport.scroll_offset = 3;
        assert!(!viewport_matches_request_key(&editor, &key));
    }
}
