//! Frozen Git reviews used by the agent's diff tools and chat replay.

use super::Editor;
use crate::ai::chat_types::ToolCallInfo;
use crate::ai::tools::ToolResult;
use crate::native_diff::{self, ChangeRef, DiffPairing, ReviewSnapshot};
use crate::run_log::{ArtifactStore, BlobId};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const DEFAULT_PAGE_SIZE: usize = 100;
const MAX_PAGE_BYTES: usize = 32 * 1024;
const MAX_FILE_SUMMARY_BYTES: usize = 8 * 1024;
const MAX_CONTEXT_BYTES: usize = 4 * 1024;
static NEXT_REFERENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Deserialize)]
struct ReadDiffArgs {
    snapshot_id: Option<String>,
    limit: Option<usize>,
    cursor: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct PageCursor {
    snapshot_id: String,
    line_offset: usize,
    byte_offset: usize,
    file_offset: usize,
}

impl PageCursor {
    fn decode(value: &str, snapshot_id: &str) -> Result<Self, String> {
        let bytes = URL_SAFE_NO_PAD
            .decode(value)
            .map_err(|_| "Invalid diff cursor".to_string())?;
        let cursor: Self =
            serde_json::from_slice(&bytes).map_err(|_| "Invalid diff cursor".to_string())?;
        if cursor.snapshot_id != snapshot_id {
            return Err("Diff cursor belongs to another snapshot".into());
        }
        Ok(cursor)
    }

    fn encode(&self) -> String {
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(self).expect("cursor serializes"))
    }
}

#[derive(Deserialize)]
struct ShowCustomDiffArgs {
    snapshot_id: String,
    title: String,
    pairings: Vec<DiffPairing>,
}

use crate::native_diff::store::SavedReview as SavedCustomDiff;

/// Content-addressed payloads live in the run's artifact store. Small named
/// references connect snapshot IDs and chat tool calls to those payloads.
struct DiffArtifacts {
    store: ArtifactStore,
    references: PathBuf,
}

impl DiffArtifacts {
    fn open(editor: &Editor) -> Result<Self, String> {
        let services = editor
            .ai_state
            .durable_runs
            .as_ref()
            .ok_or("Replay artifact storage is unavailable")?;
        let key = editor.ai_chat_conversation_key();
        let binding = editor
            .ai_state
            .durable_chat_bindings
            .get(&key)
            .ok_or("This chat has no durable run")?;
        let layout = services.store.layout();
        layout
            .ensure_run_directory(&binding.binding.run_id)
            .map_err(|error| format!("Could not prepare review storage: {error}"))?;
        let store = ArtifactStore::open(layout.artifact_directory(&binding.binding.run_id))
            .map_err(|error| format!("Could not open review storage: {error}"))?;
        let references = store.root().join("custom_diff");
        fs::create_dir_all(&references)
            .map_err(|error| format!("Could not prepare review references: {error}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&references, fs::Permissions::from_mode(0o700))
                .map_err(|error| format!("Could not secure review references: {error}"))?;
        }
        Ok(Self { store, references })
    }

    fn save<T: Serialize>(&self, namespace: &str, key: &str, value: &T) -> Result<(), String> {
        let bytes = serde_json::to_vec(value)
            .map_err(|error| format!("Could not serialize review: {error}"))?;
        let stored = self
            .store
            .put_bytes(&bytes)
            .map_err(|error| format!("Could not retain review: {error}"))?;
        write_reference(&self.reference_path(namespace, key), stored.blob_id)
    }

    fn load<T: DeserializeOwned>(&self, namespace: &str, key: &str) -> Result<T, String> {
        let reference = fs::read_to_string(self.reference_path(namespace, key))
            .map_err(|error| format!("Review artifact is unavailable: {error}"))?;
        let blob_id = reference
            .trim()
            .parse::<BlobId>()
            .map_err(|error| format!("Review reference is invalid: {error}"))?;
        let bytes = self
            .store
            .read(blob_id)
            .map_err(|error| format!("Review content is unavailable: {error}"))?;
        serde_json::from_slice(&bytes)
            .map_err(|error| format!("Review content is invalid: {error}"))
    }

    fn reference_path(&self, namespace: &str, key: &str) -> PathBuf {
        let digest = Sha256::digest(key.as_bytes());
        self.references
            .join(format!("{namespace}-{}", hex(&digest)))
    }
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(DIGITS[(byte >> 4) as usize] as char);
        result.push(DIGITS[(byte & 15) as usize] as char);
    }
    result
}

fn context_preview(line: &str, remaining: &mut usize) -> Option<String> {
    if *remaining < 4 {
        return None;
    }
    let limit = (*remaining).min(256);
    let end = (0..=limit.min(line.len()))
        .rev()
        .find(|index| line.is_char_boundary(*index))?;
    let mut preview = line[..end].to_owned();
    if end < line.len() {
        preview.push('…');
    }
    *remaining = remaining.saturating_sub(preview.len());
    Some(preview)
}

fn write_reference(path: &Path, blob_id: BlobId) -> Result<(), String> {
    let sequence = NEXT_REFERENCE.fetch_add(1, Ordering::Relaxed);
    let temporary = path.with_extension(format!("tmp-{}-{sequence}", std::process::id()));
    let result = (|| -> std::io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(blob_id.to_string().as_bytes())?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(|error| format!("Could not index review artifact: {error}"))
}

/// Native results have a target prefix; external MCP results contain text
/// blocks. Both carry this marker only after a review has been saved and opened.
pub(crate) fn custom_diff_replay_id(content: &str) -> Option<String> {
    fn marker(value: &Value) -> Option<String> {
        if let Some(id) = value.get("ovim_custom_diff_id").and_then(Value::as_str) {
            return Some(id.to_owned());
        }
        value.as_array()?.iter().find_map(|block| {
            block
                .get("text")
                .and_then(Value::as_str)
                .and_then(|text| serde_json::from_str::<Value>(text).ok())
                .as_ref()
                .and_then(marker)
        })
    }
    let body = content
        .strip_prefix("Target: ")
        .and_then(|rest| rest.split_once('\n').map(|(_, body)| body))
        .unwrap_or(content);
    serde_json::from_str::<Value>(body)
        .ok()
        .as_ref()
        .and_then(marker)
}

impl Editor {
    pub(super) fn load_walkthrough_diff_section(
        &self,
        snapshot_id: &str,
        old: Option<ChangeRef>,
        new: Option<ChangeRef>,
        fits: impl Fn(&str) -> bool,
    ) -> Result<native_diff::walkthrough::WalkthroughDiff, String> {
        let artifacts = DiffArtifacts::open(self)?;
        let snapshot: ReviewSnapshot = artifacts.load("snapshot", snapshot_id)?;
        if snapshot.id != snapshot_id {
            return Err("Stored review snapshot identity does not match the request".into());
        }
        let pairing = DiffPairing {
            label: None,
            message: None,
            old,
            new,
            related_to: None,
        };
        let review = snapshot
            .reassign(&[pairing])
            .map_err(|error| format!("Invalid diff section: {error:#}"))?;
        let section = &review.sections[0];
        Ok(native_diff::walkthrough::walkthrough_diff(
            &snapshot, section, fits,
        ))
    }

    /// The MCP request ID differs from the provider's chat tool ID. Alias the
    /// retained payload once the provider reports its completed tool call.
    pub(crate) fn bind_editor_custom_diff_result(
        &mut self,
        tool_id: &str,
        content: &str,
    ) -> Result<(), String> {
        let request_id =
            custom_diff_replay_id(content).ok_or("Custom diff result has no replay marker")?;
        if request_id == tool_id {
            return Ok(());
        }
        let artifacts = DiffArtifacts::open(self)?;
        let saved: SavedCustomDiff = artifacts.load("review", &request_id)?;
        artifacts.save("review", tool_id, &saved)
    }

    pub(super) fn execute_read_diff_tool(&mut self, args: &Value) -> ToolResult {
        match self.read_diff(args) {
            Ok(value) => ToolResult::Success(value.to_string()),
            Err(error) => ToolResult::Error(error),
        }
    }

    fn read_diff(&mut self, args: &Value) -> Result<Value, String> {
        self.ai_state
            .tool_registry
            .get("read_diff")
            .and_then(|tool| tool.custom_input_schema.as_ref())
            .expect("read_diff has a registered schema")
            .validate_instance(args)
            .map_err(|error| format!("Invalid read_diff input: {error}"))?;
        let args: ReadDiffArgs = serde_json::from_value(args.clone())
            .map_err(|error| format!("Invalid read_diff input: {error}"))?;
        if args.cursor.is_some() && args.snapshot_id.is_none() {
            return Err("A diff cursor requires snapshot_id".into());
        }
        let artifacts = DiffArtifacts::open(self)?;
        let snapshot = if let Some(id) = args.snapshot_id {
            let snapshot: ReviewSnapshot = artifacts.load("snapshot", &id)?;
            if snapshot.id != id {
                return Err("Stored review snapshot identity does not match the request".into());
            }
            snapshot
        } else {
            let path = self
                .ai_state
                .chat
                .as_ref()
                .and_then(|chat| chat.external_agent.as_ref())
                .map(|agent| agent.root.clone())
                .or_else(|| self.ai_effective_project_root())
                .ok_or_else(|| self.no_project_root_error())?;
            let base = self
                .resolve_review_base_for_path(&path)
                .map_err(|error| format!("Could not resolve diff base: {error:#}"))?;
            let snapshot = native_diff::review_snapshot(&path, &base)
                .map_err(|error| format!("Could not read Git diff: {error:#}"))?;
            if snapshot.patch.truncated {
                return Err("The Git diff exceeds Ovim's complete patch limit; a custom review would omit changes. Reduce the working diff or adjust Ovim's pullbase, then call read_diff again.".into());
            }
            artifacts.save("snapshot", &snapshot.id, &snapshot)?;
            snapshot
        };
        let cursor = match args.cursor {
            Some(value) => PageCursor::decode(&value, &snapshot.id)?,
            None => PageCursor {
                snapshot_id: snapshot.id.clone(),
                line_offset: 0,
                byte_offset: 0,
                file_offset: 0,
            },
        };
        let offset = cursor.line_offset;
        let byte_offset = cursor.byte_offset;
        let file_offset = cursor.file_offset;
        let limit = args.limit.unwrap_or(DEFAULT_PAGE_SIZE).clamp(1, 200);
        let patch_lines = snapshot.patch.text.lines().collect::<Vec<_>>();
        let total_changed_lines: usize = snapshot.blocks.iter().map(|block| block.line_count).sum();
        let mut changed_line_offset = 0;
        let mut shown_lines = 0;
        let mut shown_bytes = 0;
        let mut context_bytes = MAX_CONTEXT_BYTES;
        let mut next_byte_offset = None;
        let mut blocks = Vec::new();
        for block in &snapshot.blocks {
            let block_end = changed_line_offset + block.line_count;
            if block_end <= offset {
                changed_line_offset = block_end;
                continue;
            }
            if shown_lines == limit || shown_bytes == MAX_PAGE_BYTES {
                break;
            }
            let block_offset = offset.saturating_sub(changed_line_offset);
            let mut content = Vec::new();
            let mut partial_line = false;
            for index in block_offset..block.line_count {
                if shown_lines == limit || shown_bytes == MAX_PAGE_BYTES {
                    break;
                }
                let line = patch_lines
                    .get(block.patch_line_start + index)
                    .and_then(|line| line.get(1..))
                    .unwrap_or_default();
                let line_byte_offset = if shown_lines == 0 { byte_offset } else { 0 };
                if line_byte_offset > line.len() || !line.is_char_boundary(line_byte_offset) {
                    return Err("byte_offset is outside the requested changed line".into());
                }
                let remaining = MAX_PAGE_BYTES - shown_bytes;
                if line.len() - line_byte_offset > remaining {
                    if shown_lines > 0 {
                        break;
                    }
                    let end = (line_byte_offset..=line_byte_offset + remaining)
                        .rev()
                        .find(|end| line.is_char_boundary(*end))
                        .unwrap_or(line_byte_offset);
                    if end == line_byte_offset {
                        return Err("Changed line cannot be split at a UTF-8 boundary".into());
                    }
                    content.push(&line[line_byte_offset..end]);
                    shown_bytes += end - line_byte_offset;
                    next_byte_offset = Some(end);
                    partial_line = true;
                    break;
                }
                content.push(&line[line_byte_offset..]);
                shown_bytes += line.len() - line_byte_offset;
                shown_lines += 1;
            }
            if !content.is_empty() {
                let file = &snapshot.patch.files[block.file];
                let path = if block.kind == native_diff::PatchLineKind::Removed {
                    file.old_path.as_deref().unwrap_or(&file.path)
                } else {
                    &file.path
                };
                let context_before = (1..=3)
                    .take_while(|distance| block.patch_line_start >= *distance)
                    .map(|distance| {
                        let index = block.patch_line_start - distance;
                        let line = snapshot.patch.lines.get(index)?;
                        (line.kind == native_diff::PatchLineKind::Context
                            && line.file == Some(block.file))
                        .then(|| patch_lines[index].get(1..).unwrap_or_default())
                    })
                    .take_while(Option::is_some)
                    .flatten()
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .filter_map(|line| {
                        (block_offset == 0)
                            .then(|| context_preview(line, &mut context_bytes))
                            .flatten()
                    })
                    .collect::<Vec<_>>();
                let context_after = (0..3)
                    .map(|distance| {
                        let index = block.patch_line_start + block.line_count + distance;
                        let line = snapshot.patch.lines.get(index)?;
                        (line.kind == native_diff::PatchLineKind::Context
                            && line.file == Some(block.file))
                        .then(|| patch_lines[index].get(1..).unwrap_or_default())
                    })
                    .take_while(Option::is_some)
                    .flatten()
                    .filter_map(|line| {
                        (block_offset + content.len() == block.line_count && !partial_line)
                            .then(|| context_preview(line, &mut context_bytes))
                            .flatten()
                    })
                    .collect::<Vec<_>>();
                blocks.push(json!({
                    "block_id": block.id,
                    "kind": block.kind,
                    "path": path,
                    "start_line": block.start_line,
                    "line_count": block.line_count,
                    "block_offset": block_offset,
                    "shown_line_count": content.len() - usize::from(partial_line),
                    "content": content.join("\n"),
                    "context_before": (block_offset == 0).then_some(context_before),
                    "context_after": (block_offset + content.len() == block.line_count && !partial_line).then_some(context_after),
                    "partial_line": partial_line,
                    "fragment_byte_offset": partial_line.then_some(byte_offset),
                }));
            }
            let block_complete = block_offset + content.len() == block.line_count && !partial_line;
            changed_line_offset = block_end;
            if !block_complete {
                break;
            }
        }
        let mut files = Vec::new();
        let mut file_bytes = 0;
        for file in snapshot.patch.files.iter().skip(file_offset) {
            let size = serde_json::to_vec(file).map_or(0, |bytes| bytes.len());
            if file_bytes + size > MAX_FILE_SUMMARY_BYTES && !files.is_empty() {
                break;
            }
            files.push(file);
            file_bytes += size;
        }
        let next_line_offset = offset + shown_lines;
        let next_file_offset = file_offset + files.len();
        let next_cursor = (next_line_offset < total_changed_lines
            || next_file_offset < snapshot.patch.files.len())
        .then(|| {
            PageCursor {
                snapshot_id: snapshot.id.clone(),
                line_offset: next_line_offset,
                byte_offset: next_byte_offset.unwrap_or(0),
                file_offset: next_file_offset,
            }
            .encode()
        });
        Ok(json!({
            "snapshot_id": snapshot.id,
            "root": snapshot.patch.root,
            "base": snapshot.patch.base,
            "head": snapshot.patch.head,
            "merge_base": snapshot.patch.merge_base,
            "comparison_base_oid": snapshot.patch.comparison_base_oid,
            "file_count": snapshot.patch.files.len(),
            "files": files,
            "additions": snapshot.patch.additions(),
            "deletions": snapshot.patch.deletions(),
            "block_count": snapshot.blocks.len(),
            "total_changed_lines": total_changed_lines,
            "next_cursor": next_cursor,
            "blocks": blocks,
        }))
    }

    pub(super) fn execute_show_custom_diff_tool(&mut self, call: &ToolCallInfo) -> ToolResult {
        match self.show_custom_diff(call) {
            Ok(value) => ToolResult::Success(value.to_string()),
            Err(error) => ToolResult::Error(error),
        }
    }

    fn show_custom_diff(&mut self, call: &ToolCallInfo) -> Result<Value, String> {
        self.ai_state
            .tool_registry
            .get("show_custom_diff")
            .and_then(|tool| tool.custom_input_schema.as_ref())
            .expect("show_custom_diff has a registered schema")
            .validate_instance(&call.arguments)
            .map_err(|error| format!("Invalid show_custom_diff input: {error}"))?;
        let args: ShowCustomDiffArgs = serde_json::from_value(call.arguments.clone())
            .map_err(|error| format!("Invalid show_custom_diff input: {error}"))?;
        if args.title.trim().is_empty() || args.title.contains('\n') || args.title.contains('\r') {
            return Err("title must be a nonblank single line".into());
        }
        if args
            .pairings
            .iter()
            .filter_map(|pairing| pairing.label.as_deref())
            .any(|label| label.contains('\n') || label.contains('\r'))
        {
            return Err("pairing labels must be single lines".into());
        }
        if call.id.is_empty() {
            return Err("The tool call has no replay identity".into());
        }
        let artifacts = DiffArtifacts::open(self)?;
        let snapshot: ReviewSnapshot = artifacts.load("snapshot", &args.snapshot_id)?;
        if snapshot.id != args.snapshot_id {
            return Err("Stored review snapshot identity does not match the request".into());
        }
        let custom = snapshot
            .reassign(&args.pairings)
            .map_err(|error| format!("Could not reassign diff: {error:#}"))?;
        let saved = SavedCustomDiff {
            title: args.title,
            snapshot,
            pairings: args.pairings,
        };
        artifacts.save("review", &call.id, &saved)?;
        self.open_custom_diff_review(&saved.title, custom)
            .map_err(|error| format!("Could not open custom diff: {error:#}"))?;
        Ok(json!({
            "title": saved.title,
            "snapshot_id": args.snapshot_id,
            "sections": saved.pairings.len(),
            "pairings": saved.pairings.iter().filter(|section| section.old.is_some() && section.new.is_some()).count(),
            "ovim_custom_diff_id": call.id,
            "opened": true,
        }))
    }

    /// Open exactly the saved review associated with a chat tool call.
    pub fn replay_custom_diff(&mut self, tool_call_id: &str) -> bool {
        if self.ai_chat_waiting() {
            self.set_status_message("Finish the active agent work before replaying a diff");
            return false;
        }
        let result = (|| -> Result<(), String> {
            let call = self
                .ai_chat_tool_event_call(tool_call_id)
                .filter(|call| call.name == "show_custom_diff")
                .ok_or("That custom diff is no longer available to replay")?;
            let artifacts = DiffArtifacts::open(self)?;
            let saved: SavedCustomDiff = artifacts.load("review", &call.id)?;
            let (title, custom) = saved
                .into_custom()
                .map_err(|error| format!("Stored review is invalid: {error:#}"))?;
            self.open_custom_diff_review(&title, custom)
                .map_err(|error| format!("Could not open saved diff: {error:#}"))
        })();
        match result {
            Ok(()) => true,
            Err(error) => {
                self.set_status_message(error);
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::chat_types::{ChatOpts, ToolCallInfo};
    use crate::editor::ai_state::AiState;
    use crate::editor::code_explanation::CodeExplanationPageView;
    use crate::run_log::RunStorageLayout;
    use git2::{Repository, Signature};

    #[test]
    fn native_diff_tools_reject_unknown_arguments() {
        let mut editor = Editor::default();
        let result = editor.execute_read_diff_tool(&json!({"unexpected": true}));
        assert!(matches!(result, ToolResult::Error(message) if message.contains("unexpected")));
        let call = ToolCallInfo {
            id: "invalid-custom".into(),
            name: "show_custom_diff".into(),
            arguments: json!({"snapshot_id":"x", "title":"Review", "pairings":[], "extra":true}),
        };
        let result = editor.execute_show_custom_diff_tool(&call);
        assert!(matches!(result, ToolResult::Error(message) if message.contains("extra")));
    }

    #[test]
    fn custom_review_rejects_multiline_display_labels_before_storage() {
        let mut editor = Editor::default();
        for (title, label, expected) in [
            ("  ", "Move", "title"),
            ("First\nsecond", "Move", "title"),
            ("Review", "Move\rnext", "labels"),
        ] {
            let call = ToolCallInfo {
                id: "invalid-label".into(),
                name: "show_custom_diff".into(),
                arguments: json!({
                    "snapshot_id": "diff_example",
                    "title": title,
                    "pairings": [{
                        "label": label,
                        "old": {"block_id":"removed_0"},
                        "new": {"block_id":"added_0"}
                    }]
                }),
            };
            let result = editor.execute_show_custom_diff_tool(&call);
            assert!(matches!(result, ToolResult::Error(message) if message.contains(expected)));
        }
    }

    fn commit(repo: &Repository) {
        let mut index = repo.index().unwrap();
        index
            .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let signature = Signature::now("Ovim Test", "ovim@example.test").unwrap();
        repo.commit(Some("HEAD"), &signature, &signature, "initial", &tree, &[])
            .unwrap();
    }

    fn editor(anchor: &Path, layout: &RunStorageLayout) -> Editor {
        let mut editor = Editor::default();
        editor.open_file(anchor).unwrap();
        editor.ai_state = Box::new(AiState::with_run_storage_layout(layout.clone()).unwrap());
        editor.set_ai_conversation_resume_enabled(true);
        editor.open_ai_chat(ChatOpts::default()).unwrap();
        editor
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn walkthrough_diff_uses_only_selected_frozen_slice_and_replays_it() {
        let repository = tempfile::tempdir().unwrap();
        let storage = tempfile::tempdir().unwrap();
        let repo = Repository::init(repository.path()).unwrap();
        let anchor = repository.path().join("anchor.rs");
        fs::write(&anchor, "old one\nold two\n").unwrap();
        commit(&repo);
        fs::write(&anchor, "new one\nnew two\n").unwrap();
        let layout = RunStorageLayout::new(storage.path().join("runs"));
        let mut editor = editor(&anchor, &layout);
        let ToolResult::Success(result) = editor.execute_read_diff_tool(&json!({})) else {
            panic!("could not capture diff")
        };
        let page: Value = serde_json::from_str(&result).unwrap();
        let blocks = page["blocks"].as_array().unwrap();
        let removed = blocks
            .iter()
            .find(|block| block["kind"] == "removed")
            .unwrap();
        let added = blocks
            .iter()
            .find(|block| block["kind"] == "added")
            .unwrap();
        let snapshot_id = page["snapshot_id"].as_str().unwrap();
        assert!(editor
            .load_walkthrough_diff_section(
                snapshot_id,
                Some(ChangeRef {
                    block_id: removed["block_id"].as_str().unwrap().into(),
                    offset: Some(2),
                    count: Some(1),
                }),
                None,
                |_| true,
            )
            .is_err());
        assert!(editor
            .load_walkthrough_diff_section(
                snapshot_id,
                Some(ChangeRef {
                    block_id: added["block_id"].as_str().unwrap().into(),
                    offset: None,
                    count: None,
                }),
                None,
                |_| true,
            )
            .is_err());
        let original_buffer_id = editor.buffer().id();
        for (index, step) in [
            json!({"type":"diff", "snapshot_id": snapshot_id, "title":"Missing sides", "comment":"Explain."}),
            json!({"type":"diff", "snapshot_id": snapshot_id, "title":"Wrong side", "comment":"Explain.", "old":{"block_id":added["block_id"]}}),
            json!({"type":"diff", "snapshot_id": snapshot_id, "title":"Beyond block", "comment":"Explain.", "new":{"block_id":added["block_id"], "offset":2, "count":1}}),
        ]
        .into_iter()
        .enumerate()
        {
            let bad = ToolCallInfo {
                id: format!("bad-diff-{index}"),
                name: "explain_with_codebase".into(),
                arguments: json!({"steps":[step]}),
            };
            assert!(editor
                .begin_code_explanation(bad, None)
                .is_err());
            assert!(!editor.ai_chat_has_pending_code_explanation());
        }
        let call = ToolCallInfo {
            id: "walkthrough-diff".into(),
            name: "explain_with_codebase".into(),
            arguments: json!({"steps": [
                {"type": "concept", "title": "Change", "body": "One change."},
                {
                    "type": "diff", "snapshot_id": page["snapshot_id"],
                    "title": "First line", "comment": "This line changed.",
                    "old": {"block_id": removed["block_id"], "offset": 0, "count": 1},
                    "new": {"block_id": added["block_id"], "offset": 0, "count": 1}
                },
                {"type": "code", "path": "anchor.rs", "start_line": 1, "comment": "Now see the file."}
            ]}),
        };
        editor
            .begin_code_explanation(call.clone(), None)
            .unwrap_or_else(|(error, _)| panic!("walkthrough failed: {error:?}"));
        assert!(editor.move_code_explanation(true));
        assert!(matches!(editor.ai_code_explanation_view().unwrap().page,
            CodeExplanationPageView::Diff { ref title, .. } if title == "First line"));
        let displayed = editor.buffer().rope().to_string();
        assert!(displayed.contains("-old one"));
        assert!(displayed.contains("+new one"));
        assert!(!displayed.contains("-old two"));
        assert!(displayed.contains(" old two"));
        assert!(!displayed.contains("+new two"));
        assert!(displayed.contains(" new two"));
        assert!(editor.buffer().is_read_only());
        assert!(editor.move_code_explanation(true));
        assert!(matches!(
            editor.ai_code_explanation_view().unwrap().page,
            CodeExplanationPageView::Code { .. }
        ));
        assert!(editor.finish_code_explanation(false));
        assert_eq!(editor.buffer().id(), original_buffer_id);
        editor.set_last_layout(
            crate::Rect {
                x: 0,
                y: 0,
                width: 80,
                height: 11,
            },
            4,
            80,
            0,
        );
        let mut oversized = call.clone();
        oversized.id = "too-tall-diff".into();
        let error = editor
            .begin_code_explanation(oversized, None)
            .expect_err("diff page must fit the viewport")
            .0;
        assert!(
            matches!(error, ToolResult::Error(message) if message.contains("offset and count"))
        );
        editor.set_last_layout(
            crate::Rect {
                x: 0,
                y: 0,
                width: 80,
                height: 24,
            },
            4,
            80,
            0,
        );
        fs::write(&anchor, "different content\n").unwrap();
        for evict_cache in [false, true] {
            if evict_cache {
                let chat = editor.ai_state.chat.as_mut().unwrap();
                chat.code_explanation_cache.clear();
                chat.code_explanation_cache_bytes = 0;
            }
            editor
                .begin_code_explanation(call.clone(), None)
                .unwrap_or_else(|(error, _)| panic!("replay failed: {error:?}"));
            assert!(editor.move_code_explanation(true));
            assert_eq!(editor.buffer().rope().to_string(), displayed);
            assert!(editor.finish_code_explanation(false));
            assert_eq!(editor.buffer().id(), original_buffer_id);
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn href_walkthrough_keeps_frozen_context_visible_and_replays_after_source_deletion() {
        let repository = tempfile::tempdir().unwrap();
        let storage = tempfile::tempdir().unwrap();
        let repo = Repository::init(repository.path()).unwrap();
        let path = repository.path().join("nav.html");
        let before = (1..=20)
            .map(|line| format!("<!-- navigation {line} -->\n"))
            .collect::<String>()
            + "<nav>\n  <a href=\"/old\">Home</a>\n</nav>\n";
        fs::write(&path, &before).unwrap();
        commit(&repo);
        fs::write(&path, before.replace("/old", "/home")).unwrap();
        let layout = RunStorageLayout::new(storage.path().join("runs"));
        let mut editor = editor(&path, &layout);
        editor.set_viewport_height(32);
        editor.set_last_layout(
            crate::Rect {
                x: 0,
                y: 0,
                width: 80,
                height: 32,
            },
            4,
            76,
            0,
        );
        let ToolResult::Success(result) = editor.execute_read_diff_tool(&json!({})) else {
            panic!("could not capture navigation diff")
        };
        let captured: Value = serde_json::from_str(&result).unwrap();
        let block = |kind| {
            captured["blocks"]
                .as_array()
                .unwrap()
                .iter()
                .find(|block| block["kind"] == kind)
                .unwrap()["block_id"]
                .clone()
        };
        let call = ToolCallInfo {
            id: "href-with-context".into(),
            name: "explain_with_codebase".into(),
            arguments: json!({"steps": [{
                "type": "diff", "snapshot_id": captured["snapshot_id"],
                "title": "Home navigation", "comment": "Home now opens the new route.",
                "old": {"block_id": block("removed")},
                "new": {"block_id": block("added")}
            }]}),
        };
        fs::write(&path, "disk changed after capture\n").unwrap();
        editor
            .buffer_mut()
            .replace_content("unsaved buffer changed after capture\n");
        editor
            .begin_code_explanation(call.clone(), None)
            .unwrap_or_else(|(error, _)| panic!("walkthrough failed: {error:?}"));
        let displayed = editor.buffer().rope().to_string();
        assert!(displayed.contains(
            " <nav>\n-  <a href=\"/old\">Home</a>\n+  <a href=\"/home\">Home</a>\n </nav>\n"
        ));
        assert!(displayed.contains("<!-- navigation 20 -->"));
        assert!(!displayed.contains("changed after capture"));
        assert!(editor.ai_state.active_selection.is_none());
        assert_eq!(
            editor.scroll_offset(),
            0,
            "all context and source headers fit"
        );
        assert!(editor
            .buffer()
            .line_text(editor.buffer().cursor().line())
            .unwrap()
            .starts_with('-'));
        editor.position_code_explanation(6);
        let cursor = editor.buffer().cursor().line();
        assert!(cursor >= editor.scroll_offset() && cursor + 1 < editor.scroll_offset() + 6);
        editor.position_code_explanation(22);
        assert_eq!(editor.scroll_offset(), 0);
        assert!(editor.finish_code_explanation(false));
        fs::remove_file(&path).unwrap();
        for cached in [true, false] {
            if !cached {
                let chat = editor.ai_state.chat.as_mut().unwrap();
                chat.code_explanation_cache.clear();
                chat.code_explanation_cache_bytes = 0;
            }
            editor
                .begin_code_explanation(call.clone(), None)
                .unwrap_or_else(|(error, _)| panic!("frozen replay failed: {error:?}"));
            assert_eq!(editor.buffer().rope().to_string(), displayed);
            assert!(editor.finish_code_explanation(false));
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn saved_review_replays_after_restart_and_source_deletion() {
        let repository = tempfile::tempdir().unwrap();
        let storage = tempfile::tempdir().unwrap();
        let repo = Repository::init(repository.path()).unwrap();
        let anchor = repository.path().join("anchor.rs");
        let old = repository.path().join("old.rs");
        let new = repository.path().join("new.rs");
        fs::write(&anchor, "fn anchor() {}\n").unwrap();
        fs::write(&old, "fn moved() { println!(\"old\"); }\n").unwrap();
        commit(&repo);
        fs::remove_file(&old).unwrap();
        fs::write(&new, "fn moved() { println!(\"new\"); }\n").unwrap();
        let layout = RunStorageLayout::new(storage.path().join("runs"));

        let (call, snapshot_id) = {
            let mut editor = editor(&anchor, &layout);
            let output = editor.execute_read_diff_tool(&json!({}));
            let ToolResult::Success(output) = output else {
                panic!("read_diff failed: {output:?}")
            };
            let page: Value = serde_json::from_str(&output).unwrap();
            let snapshot_id = page["snapshot_id"].as_str().unwrap().to_owned();
            let blocks = page["blocks"].as_array().unwrap();
            let removed = blocks
                .iter()
                .find(|block| block["kind"] == "removed")
                .unwrap();
            let added = blocks
                .iter()
                .find(|block| block["kind"] == "added")
                .unwrap();
            let call = ToolCallInfo {
                id: "custom-review-call".into(),
                name: "show_custom_diff".into(),
                arguments: json!({
                    "snapshot_id": snapshot_id,
                    "title": "Moved function",
                    "pairings": [{
                        "label": "Function moved and edited",
                        "message": "The new function handles callers from both paths.",
                        "old": {"block_id": removed["block_id"]},
                        "new": {"block_id": added["block_id"]}
                    }]
                }),
            };
            let turn = editor
                .begin_ai_runtime_turn("Review the function move")
                .unwrap();
            let tool = editor.ai_runtime_record_tool_intent(&turn, &call).unwrap();
            editor.ai_runtime_start_tool(&turn, &tool).unwrap();
            let result = editor.execute_show_custom_diff_tool(&call);
            assert!(matches!(result, ToolResult::Success(_)), "{result:?}");
            let marker = json!([{"type": "text", "text": json!({
                "ovim_custom_diff_id": call.id
            }).to_string()}])
            .to_string();
            editor
                .bind_editor_custom_diff_result("provider-call", &marker)
                .unwrap();
            let aliases = DiffArtifacts::open(&editor).unwrap();
            let provider_review: SavedCustomDiff = aliases.load("review", "provider-call").unwrap();
            assert_eq!(provider_review.title, "Moved function");
            editor
                .ai_runtime_finish_tool(&turn, &tool, &result)
                .unwrap();
            editor.ai_state.agent_runtime.complete_turn(&turn).unwrap();
            (call, snapshot_id)
        };

        fs::remove_file(&new).unwrap();
        let mut restored = editor(&anchor, &layout);
        let artifacts = DiffArtifacts::open(&restored).unwrap();
        let saved: SavedCustomDiff = artifacts.load("review", &call.id).unwrap();
        assert_eq!(saved.snapshot.id, snapshot_id);
        assert_eq!(saved.title, "Moved function");
        assert_eq!(
            saved.pairings[0].message.as_deref(),
            Some("The new function handles callers from both paths.")
        );
        assert!(restored.ai_chat_tool_event_call(&call.id).is_some());
        assert!(restored.replay_custom_diff(&call.id));
        assert!(restored.is_diff_review_buffer());
        assert!(restored
            .buffer()
            .rope()
            .to_string()
            .contains("The new function handles callers from both paths."));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn read_diff_pages_long_lines_without_losing_bytes() {
        let repository = tempfile::tempdir().unwrap();
        let storage = tempfile::tempdir().unwrap();
        let repo = Repository::init(repository.path()).unwrap();
        let anchor = repository.path().join("anchor.txt");
        fs::write(&anchor, "before\n").unwrap();
        commit(&repo);
        let replacement = "x".repeat(MAX_PAGE_BYTES * 2 + 37);
        fs::write(&anchor, format!("{replacement}\n")).unwrap();
        let layout = RunStorageLayout::new(storage.path().join("runs"));
        let mut editor = editor(&anchor, &layout);

        let mut snapshot_id = None;
        let mut cursor = None;
        let mut reconstructed = String::new();
        for _ in 0..10 {
            let mut args = json!({"limit": 2});
            if let Some(id) = &snapshot_id {
                args["snapshot_id"] = json!(id);
            }
            if let Some(cursor) = &cursor {
                args["cursor"] = json!(cursor);
            }
            let ToolResult::Success(output) = editor.execute_read_diff_tool(&args) else {
                panic!("read_diff page failed")
            };
            let page: Value = serde_json::from_str(&output).unwrap();
            snapshot_id = Some(page["snapshot_id"].as_str().unwrap().to_owned());
            for block in page["blocks"].as_array().unwrap() {
                if block["kind"] == "added" {
                    reconstructed.push_str(block["content"].as_str().unwrap());
                }
            }
            let Some(next) = page["next_cursor"].as_str() else {
                break;
            };
            assert_ne!(cursor.as_deref(), Some(next));
            cursor = Some(next.to_owned());
        }
        assert_eq!(reconstructed, replacement);
    }
}
