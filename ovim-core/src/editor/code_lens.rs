//! Code lens support (`textDocument/codeLens`).
//!
//! Lenses are requested for the current buffer once edits settle, again when
//! the server sends `workspace/codeLens/refresh`, and after a buffer/file
//! switch. They are shown as end-of-line virtual text (`▶ Run | ▶ Debug`) and
//! run with `<Space>cl` (run) / `<Space>cL` (debug) on the lens line.
//!
//! A `hyperion.run` lens does **not** execute on the server: it goes through
//! the same launch flow as `<Space>rr` (`hyperion.resolveLaunch` at the lens
//! position, build, real JVM), never Hyperion's bytecode VM. Other lens
//! commands are sent to the owning server with `workspace/executeCommand`.

use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use super::decoration::{Decoration, DecorationPlacement, DecorationSource, DecorationStyle};
use super::Editor;
use crate::launch::LaunchMode;

/// Edits must be quiet for this long before lenses are re-requested.
const DEBOUNCE: Duration = Duration::from_millis(350);

/// The command Hyperion attaches to its main-method lens.
pub const RUN_LENS_COMMAND: &str = "hyperion.run";

/// One resolved lens from a server.
#[derive(Debug, Clone, PartialEq)]
pub struct LensEntry {
    /// 0-based line of the lens range start.
    pub line: usize,
    /// UTF-16 character of the lens range start.
    pub character: u32,
    pub title: String,
    pub command: String,
    pub arguments: Vec<Value>,
    /// Server that produced the lens (commands run there).
    pub server_id: String,
}

type LensResponse = anyhow::Result<Vec<(String, lsp_types::CodeLens)>>;

struct Inflight {
    _task: JoinHandle<()>,
    rx: oneshot::Receiver<LensResponse>,
    file_path: String,
    version: usize,
}

#[derive(Default)]
pub(crate) struct CodeLensState {
    /// File the current lenses belong to.
    file_path: Option<String>,
    lenses: Vec<LensEntry>,
    version: Option<usize>,
    generation: u64,
    /// (file, buffer version) of the last request sent.
    requested: Option<(String, usize)>,
    /// (file, version, first seen) waiting out the debounce.
    settling: Option<(String, usize, Instant)>,
    inflight: Option<Inflight>,
    force: bool,
}

impl Editor {
    /// Lenses currently shown for the buffer.
    pub fn code_lenses(&self) -> &[LensEntry] {
        let state = &self.launch.code_lens;
        if state.file_path.as_deref() == self.buffer().file_path()
            && state.version == Some(self.buffer().version())
        {
            &state.lenses
        } else {
            &[]
        }
    }

    /// Lenses on a 0-based line.
    pub fn code_lenses_on_line(&self, line: usize) -> Vec<&LensEntry> {
        self.code_lenses()
            .iter()
            .filter(|l| l.line == line)
            .collect()
    }

    fn clear_code_lens_state(&mut self) {
        let had = !self.launch.code_lens.lenses.is_empty();
        let st = &mut self.launch.code_lens;
        st.generation = st.generation.wrapping_add(1);
        st.lenses.clear();
        st.version = None;
        st.requested = None;
        st.settling = None;
        st.inflight = None;
        if had {
            let rope = self.buffer().rope().clone();
            self.decorations
                .replace_source(DecorationSource::CodeLens, Vec::new(), &rope);
            self.mark_dirty();
        }
    }

    /// Requests code lenses when the buffer changed and settled (or a server
    /// asked for a refresh). Called from the tick; never waits on the server.
    pub async fn request_code_lens_if_needed(&mut self) {
        let Some(manager) = self.lsp_manager() else {
            return;
        };
        let forced = manager.take_code_lens_refresh();
        let Some(path) = self.buffer().file_path().map(str::to_string) else {
            self.clear_code_lens_state();
            return;
        };
        if self.launch.code_lens.file_path.as_deref() != Some(path.as_str()) {
            // Switched files: the old file's lenses must not linger.
            self.clear_code_lens_state();
            self.launch.code_lens.file_path = Some(path.clone());
        }
        let Some(language_id) = self.language_id_for_path(&path) else {
            return;
        };
        let version = self.buffer().version();
        let st = &mut self.launch.code_lens;
        st.force |= forced;
        let key = (path.clone(), version);
        if !st.force && st.requested.as_ref() == Some(&key) {
            return;
        }
        if st
            .inflight
            .as_ref()
            .is_some_and(|f| f.file_path == path && f.version == version)
        {
            return;
        }
        if !st.force {
            match &st.settling {
                Some((f, v, since)) if *f == path && *v == version => {
                    if since.elapsed() < DEBOUNCE {
                        return;
                    }
                }
                _ => {
                    st.settling = Some((path.clone(), version, Instant::now()));
                    return;
                }
            }
        }
        let Some(uri) = crate::lsp::uri_from_file_path(&path) else {
            return;
        };
        // The server must see the text the lenses will be computed for.
        self.ensure_lsp_document_synced().await;
        let st = &mut self.launch.code_lens;
        st.force = false;
        st.requested = Some(key);
        let (tx, rx) = oneshot::channel();
        let file = std::path::PathBuf::from(&path);
        let task = tokio::spawn(async move {
            let result = manager.code_lenses(&uri, &file, &language_id).await;
            let _ = tx.send(result);
        });
        st.inflight = Some(Inflight {
            _task: task,
            rx,
            file_path: path,
            version,
        });
    }

    /// Applies a finished code lens response. Returns true when the display changed.
    pub fn poll_code_lens(&mut self) -> bool {
        let Some(mut inflight) = self.launch.code_lens.inflight.take() else {
            return false;
        };
        let response = match inflight.rx.try_recv() {
            Ok(response) => response,
            Err(oneshot::error::TryRecvError::Empty) => {
                self.launch.code_lens.inflight = Some(inflight);
                return false;
            }
            Err(oneshot::error::TryRecvError::Closed) => return false,
        };
        let current = self.buffer().file_path().map(str::to_string);
        if current.as_deref() != Some(inflight.file_path.as_str())
            || self.buffer().version() != inflight.version
        {
            // Computed for text that is gone; ask again for the current text.
            self.launch.code_lens.requested = None;
            return false;
        }
        let lenses = match response {
            Ok(lenses) => lenses,
            Err(e) => {
                crate::lsp_debug!("LSP-CODELENS", "codeLens request failed: {e}");
                return false;
            }
        };
        let entries: Vec<LensEntry> = lenses
            .into_iter()
            .filter_map(|(server_id, lens)| {
                let command = lens.command?;
                Some(LensEntry {
                    line: lens.range.start.line as usize,
                    character: lens.range.start.character,
                    title: command.title,
                    command: command.command,
                    arguments: command.arguments.unwrap_or_default(),
                    server_id,
                })
            })
            .collect();
        if entries == self.launch.code_lens.lenses
            && self.launch.code_lens.version == Some(inflight.version)
        {
            return false;
        }
        let rope = self.buffer().rope().clone();
        let decorations = lens_decorations(&entries, &rope, inflight.version as u64);
        self.decorations
            .replace_source(DecorationSource::CodeLens, decorations, &rope);
        self.launch.code_lens.generation = self.launch.code_lens.generation.wrapping_add(1);
        self.launch.code_lens.version = Some(inflight.version);
        self.launch.code_lens.lenses = entries;
        self.mark_dirty();
        true
    }

    /// `<Space>cl` (Run) / `<Space>cL` (Debug): execute the lens on the
    /// cursor line.
    pub fn run_code_lens_at_cursor(&mut self, mode: LaunchMode) {
        let line = self.buffer().cursor().line();
        let lenses: Vec<LensEntry> = self
            .code_lenses_on_line(line)
            .into_iter()
            .cloned()
            .collect();
        let Some(lens) = lenses
            .iter()
            .find(|l| l.command == RUN_LENS_COMMAND)
            .or_else(|| lenses.first())
            .cloned()
        else {
            self.set_status_message("No code lens on this line");
            return;
        };
        self.execute_code_lens(lens, mode);
    }

    /// Activate exactly the lens displayed by a frontend. Reject stale frames
    /// after edits or file switches rather than running a different command.
    pub fn activate_code_lens(
        &mut self,
        file_path: &str,
        version: usize,
        index: usize,
        generation: u64,
        mode: LaunchMode,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.buffer().file_path() == Some(file_path)
                && self.buffer().version() == version
                && self.launch.code_lens.generation == generation,
            "Code lens changed; wait for it to refresh"
        );
        let lens = self
            .code_lenses()
            .get(index)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Code lens is no longer available"))?;
        anyhow::ensure!(
            mode == LaunchMode::Run || lens.command == RUN_LENS_COMMAND,
            "This code lens has no Debug action"
        );
        self.execute_code_lens(lens, mode);
        Ok(())
    }

    pub fn code_lens_generation(&self) -> u64 {
        self.launch.code_lens.generation
    }

    /// Individual actions, including the real-JVM Debug action for Hyperion.
    pub fn code_lens_actions(&self, line: usize) -> Vec<CodeLensAction> {
        lens_actions(self.code_lenses(), line)
    }

    fn execute_code_lens(&mut self, lens: LensEntry, mode: LaunchMode) {
        if lens.command == RUN_LENS_COMMAND {
            // Not Hyperion's bytecode VM: resolve the target at the lens and
            // run it on a real JVM through the launch flow.
            self.launch_at_position(mode, lens.line, lens.character);
            return;
        }
        let (command, arguments, server_id, title) = (
            lens.command.clone(),
            lens.arguments.clone(),
            lens.server_id.clone(),
            lens.title.clone(),
        );
        let Some(manager) = self.lsp_manager() else {
            return;
        };
        if tokio::runtime::Handle::try_current().is_err() {
            return;
        }
        self.set_status_message(format!("Running lens: {title}"));
        tokio::spawn(async move {
            let _ = manager
                .execute_command_on_server_id(command, Some(arguments), &server_id)
                .await;
        });
    }
}

/// A single clickable label within a code-lens decoration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeLensAction {
    pub index: usize,
    pub title: String,
    pub mode: LaunchMode,
}

fn lens_actions(entries: &[LensEntry], line: usize) -> Vec<CodeLensAction> {
    let mut actions = Vec::new();
    for (index, lens) in entries
        .iter()
        .enumerate()
        .filter(|(_, lens)| lens.line == line)
    {
        actions.push(CodeLensAction {
            index,
            title: lens.title.replace(['\t', '\n', '\r'], " "),
            mode: LaunchMode::Run,
        });
        if lens.command == RUN_LENS_COMMAND {
            actions.push(CodeLensAction {
                index,
                title: "▶ Debug".into(),
                mode: LaunchMode::Debug,
            });
        }
    }
    actions
}

/// End-of-line decorations, one per line with lenses (`▶ Run | ▶ Debug`).
fn lens_decorations(entries: &[LensEntry], rope: &ropey::Rope, version: u64) -> Vec<Decoration> {
    use std::collections::BTreeMap;
    let mut by_line: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    for line in entries
        .iter()
        .map(|lens| lens.line)
        .collect::<std::collections::BTreeSet<_>>()
    {
        by_line.insert(
            line,
            lens_actions(entries, line)
                .into_iter()
                .map(|action| action.title)
                .collect(),
        );
    }
    by_line
        .into_iter()
        .filter(|(line, _)| *line < rope.len_lines())
        .map(|(line, parts)| {
            let text = format!("  {}", parts.join(" │ "));
            Decoration {
                placement: DecorationPlacement::EndOfLine {
                    char_offset: rope.line_to_char(line),
                },
                source: DecorationSource::CodeLens,
                display_width: crate::display::display_width(&text, 1),
                text,
                style: DecorationStyle::new(crate::color::Color::Gray).with_italic(),
                priority: 10,
                source_version: version,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(line: usize, title: &str, command: &str) -> LensEntry {
        LensEntry {
            line,
            character: 4,
            title: title.into(),
            command: command.into(),
            arguments: vec![],
            server_id: "java".into(),
        }
    }

    #[test]
    fn a_run_lens_also_offers_debug_and_lenses_share_their_line() {
        let rope = ropey::Rope::from_str("class A {\n  void main() {}\n}\n");
        let decorations = lens_decorations(
            &[
                entry(1, "▶ Run", RUN_LENS_COMMAND),
                entry(1, "2 implementations", "hyperion.showImplementations"),
                entry(9, "beyond the buffer", "x"),
            ],
            &rope,
            3,
        );
        assert_eq!(
            decorations.len(),
            1,
            "one decoration per line, out-of-range dropped"
        );
        assert_eq!(decorations[0].text, "  ▶ Run │ ▶ Debug │ 2 implementations");
        assert_eq!(decorations[0].source_version, 3);
        assert_eq!(
            decorations[0].placement,
            DecorationPlacement::EndOfLine { char_offset: 10 }
        );
    }
}
