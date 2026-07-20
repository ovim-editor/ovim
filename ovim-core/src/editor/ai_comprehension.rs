use sha2::{Digest, Sha256};

use crate::ai::tools::ToolResult;

use super::ai_chat_state::{ComprehensionCheckpoint, ComprehensionPolicy};
use super::Editor;

pub const RECORD_COMPREHENSION_CHECKPOINT_TOOL: &str = "record_comprehension_checkpoint";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ComprehensionBoundary {
    Commit,
    Publish,
}

impl ComprehensionBoundary {
    fn label(self) -> &'static str {
        match self {
            Self::Commit => "local commit",
            Self::Publish => "publication",
        }
    }
}

impl Editor {
    pub(crate) fn comprehension_system_prompt(&self) -> Option<&'static str> {
        (self.ai_chat_comprehension_policy() != ComprehensionPolicy::Off).then_some(
            "## Required comprehension workflow\n\nComprehension mode is active and is independent from YOLO. Before the configured commit/publication boundary, the user must demonstrate minimum sufficient comprehension of the exact current change set. Do not treat viewing an explanation or expressing confidence as mastery.\n\nWhen a boundary is approaching:\n1. Inspect the exact accumulated change scope and separate relevant behavior from incidental edits.\n2. State the user-visible goal, then establish a small end-to-end mental map before zooming in.\n3. Select only critical mastery criteria: the central mechanism/invariant, principal realistic failure mode, design rationale where material, and how verification addresses the risk.\n4. Teach one coherent relationship at a time. Remove incidental complexity without removing essential complexity. Use explain_with_codebase when code anchoring materially helps.\n5. Ask exactly one active-recall, contrast, transfer, or failure-analysis question at a time. Do not reveal the answer before the user's first attempt. Never test line numbers, spelling, or syntax trivia.\n6. Diagnose the specific gap. Keep mastery criteria fixed while adapting granularity, prerequisites, examples, hints, and code anchors. A wrong answer is diagnostic, not a pass. After directly teaching an answer, use a fresh transfer question rather than asking for repetition.\n7. Call record_comprehension_checkpoint only after every critical criterion has been demonstrated. Supporting or enriching context must not block. If the user explicitly waives comprehension, explain that the current implementation has no agent-side waiver tool; do not falsely record mastery.\n\nThe repository fingerprint invalidates stale checkpoints automatically. A blocked shell result means the current state is not covered; conduct the workflow over subsequent user turns, record the checkpoint, and then retry."
        )
    }

    pub fn ai_chat_comprehension_policy(&self) -> ComprehensionPolicy {
        self.ai_state
            .chat
            .as_ref()
            .map(|chat| chat.comprehension_policy)
            .unwrap_or_default()
    }

    pub fn set_ai_chat_comprehension_policy(&mut self, policy: ComprehensionPolicy) -> bool {
        let Some(chat) = self.ai_state.chat.as_mut() else {
            return false;
        };
        if chat.comprehension_policy == policy {
            return false;
        }
        chat.comprehension_policy = policy;
        self.set_lsp_status(match policy {
            ComprehensionPolicy::Off => "Comprehension checkpoints disabled".into(),
            ComprehensionPolicy::Publish => {
                "Comprehension required before the agent publishes changes".into()
            }
            ComprehensionPolicy::Commit => {
                "Comprehension required before agent commits or publishes changes".into()
            }
        });
        true
    }

    /// The header is intentionally a simple toggle. More stringent commit
    /// gating remains available through `/comprehension commit`.
    pub fn toggle_ai_chat_comprehension_policy(&mut self) -> ComprehensionPolicy {
        let next = match self.ai_chat_comprehension_policy() {
            ComprehensionPolicy::Off => ComprehensionPolicy::Publish,
            ComprehensionPolicy::Publish | ComprehensionPolicy::Commit => ComprehensionPolicy::Off,
        };
        self.set_ai_chat_comprehension_policy(next);
        next
    }

    pub fn ai_chat_comprehension_checkpoint_summary(&self) -> Option<&str> {
        let checkpoint = self
            .ai_state
            .chat
            .as_ref()?
            .comprehension_checkpoint
            .as_ref()?;
        (self.current_repository_fingerprint().ok().as_deref()
            == Some(checkpoint.repository_fingerprint.as_str()))
        .then_some(checkpoint.summary.as_str())
    }

    pub(crate) fn execute_record_comprehension_checkpoint(
        &mut self,
        args: &serde_json::Value,
    ) -> ToolResult {
        if self.ai_chat_comprehension_policy() == ComprehensionPolicy::Off {
            return ToolResult::Error(
                "comprehension mode is off; do not record a checkpoint".into(),
            );
        }
        let summary = match args.get("summary").and_then(|value| value.as_str()) {
            Some(summary) if !summary.trim().is_empty() => summary.trim().to_string(),
            _ => return ToolResult::Error("'summary' is required and must be non-empty".into()),
        };
        let critical_concepts = args
            .get("critical_concepts")
            .and_then(|value| value.as_array())
            .map(|values| {
                values
                    .iter()
                    .filter_map(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if critical_concepts.is_empty() {
            return ToolResult::Error(
                "'critical_concepts' must contain at least one demonstrated concept".into(),
            );
        }
        let fingerprint = match self.current_repository_fingerprint() {
            Ok(fingerprint) => fingerprint,
            Err(error) => return ToolResult::Error(error),
        };
        let Some(chat) = self.ai_state.chat.as_mut() else {
            return ToolResult::Error("AI chat is not open".into());
        };
        chat.comprehension_checkpoint = Some(ComprehensionCheckpoint {
            repository_fingerprint: fingerprint,
            summary,
            critical_concepts: critical_concepts.clone(),
        });
        ToolResult::Success(format!(
            "Comprehension checkpoint recorded for the current repository state ({} critical concept{}). Publication remains subject to its normal approval policy.",
            critical_concepts.len(),
            if critical_concepts.len() == 1 { "" } else { "s" }
        ))
    }

    pub(crate) fn comprehension_gate_for_bash(&self, command: &str) -> Option<String> {
        let boundary = classify_comprehension_boundary(command)?;
        let policy = self.ai_chat_comprehension_policy();
        let covered = match (policy, boundary) {
            (ComprehensionPolicy::Off, _) => return None,
            (ComprehensionPolicy::Publish, ComprehensionBoundary::Commit) => return None,
            (ComprehensionPolicy::Publish | ComprehensionPolicy::Commit, _) => self
                .current_repository_fingerprint()
                .ok()
                .zip(
                    self.ai_state
                        .chat
                        .as_ref()
                        .and_then(|chat| chat.comprehension_checkpoint.as_ref()),
                )
                .is_some_and(|(current, checkpoint)| current == checkpoint.repository_fingerprint),
        };
        if covered {
            return None;
        }
        Some(format!(
            "Blocked by COMPREHENSION: {} requires demonstrated understanding of the current repository state before {}. The entire shell invocation was stopped before any segment ran; retry earlier permitted work (such as a local commit under publish policy) in a separate command. Teach a concise mental map, invariant, principal risk, and relevant verification; ask one active-recall/application question at a time without revealing answers first. Keep mastery criteria fixed while adapting scaffolding. After every critical concept is demonstrated, call {} and retry this exact action. YOLO does not bypass this gate.",
            policy.as_str(),
            boundary.label(),
            RECORD_COMPREHENSION_CHECKPOINT_TOOL,
        ))
    }

    fn current_repository_fingerprint(&self) -> Result<String, String> {
        let root = self.ai_effective_project_root().ok_or_else(|| {
            "cannot fingerprint comprehension scope without a project root".to_string()
        })?;
        repository_fingerprint(&root)
    }
}

fn repository_fingerprint(root: &std::path::Path) -> Result<String, String> {
    let repo = git2::Repository::discover(root)
        .map_err(|error| format!("cannot fingerprint repository: {error}"))?;
    let mut digest = Sha256::new();
    digest.update(b"ovim-comprehension-v1\0");

    // Deliberately fingerprint repository content rather than commit identity.
    // A checkpoint taken immediately before `git commit` must remain valid for
    // the subsequent push when the commit changes only HEAD, not the tree.
    let index = repo
        .index()
        .map_err(|error| format!("cannot read repository index: {error}"))?;
    for entry in index.iter() {
        digest.update(&entry.path);
        digest.update(entry.id.as_bytes());
        digest.update(entry.mode.to_le_bytes());
    }

    let mut options = git2::StatusOptions::new();
    options
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_ignored(false)
        .renames_head_to_index(true)
        .renames_index_to_workdir(true);
    let statuses = repo
        .statuses(Some(&mut options))
        .map_err(|error| format!("cannot inspect repository status: {error}"))?;
    let workdir = repo
        .workdir()
        .ok_or_else(|| "bare repositories cannot use comprehension checkpoints".to_string())?;
    let worktree_status = git2::Status::WT_NEW
        | git2::Status::WT_MODIFIED
        | git2::Status::WT_DELETED
        | git2::Status::WT_RENAMED
        | git2::Status::WT_TYPECHANGE
        | git2::Status::CONFLICTED;
    let mut changed = statuses
        .iter()
        .filter_map(|entry| {
            let status = entry.status() & worktree_status;
            (!status.is_empty())
                .then(|| entry.path().map(|path| (path.to_string(), status.bits())))
                .flatten()
        })
        .collect::<Vec<_>>();
    changed.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    for (path, status) in changed {
        digest.update(path.as_bytes());
        digest.update(status.to_le_bytes());
        let absolute = workdir.join(&path);
        match std::fs::read(&absolute) {
            Ok(bytes) => digest.update(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => digest.update(b"deleted"),
            Err(error) => {
                return Err(format!(
                    "cannot fingerprint {}: {error}",
                    absolute.display()
                ))
            }
        }
    }
    Ok(format!("{:x}", digest.finalize()))
}

pub(crate) fn classify_comprehension_boundary(command: &str) -> Option<ComprehensionBoundary> {
    let mut boundary = None;
    for segment in split_shell_segments(command)? {
        if let Some(words) = shlex::split(segment) {
            match classify_command_segment(&words) {
                Some(ComprehensionBoundary::Publish) => {
                    boundary = Some(ComprehensionBoundary::Publish)
                }
                Some(ComprehensionBoundary::Commit) if boundary.is_none() => {
                    boundary = Some(ComprehensionBoundary::Commit)
                }
                _ => {}
            }
        }
    }
    boundary
}

/// Split command-list operators before asking `shlex` to parse each command.
/// `shlex::split` treats an adjacent operator such as the semicolon in
/// `git push; echo done` as part of the preceding word, which would otherwise
/// let a normal shell spelling bypass the boundary classifier.
fn split_shell_segments(command: &str) -> Option<Vec<&str>> {
    let bytes = command.as_bytes();
    let mut segments = Vec::new();
    let mut start = 0;
    let mut index = 0;
    let mut quote = None;
    let mut escaped = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        match quote {
            Some(b'\'') if byte == b'\'' => quote = None,
            Some(b'"') if byte == b'"' => quote = None,
            Some(b'"') if byte == b'\\' => escaped = true,
            Some(_) => {}
            None if matches!(byte, b'\'' | b'"') => quote = Some(byte),
            None if byte == b'\\' => escaped = true,
            None if matches!(byte, b';' | b'\n' | b'|' | b'&') => {
                segments.push(&command[start..index]);
                index += usize::from(bytes.get(index + 1) == Some(&byte));
                start = index + 1;
            }
            None => {}
        }
        index += 1;
    }
    if quote.is_some() || escaped {
        return None;
    }
    segments.push(&command[start..]);
    Some(segments)
}

fn classify_command_segment(words: &[String]) -> Option<ComprehensionBoundary> {
    let mut index = 0;
    loop {
        while words.get(index).is_some_and(|word| word.starts_with('-')) {
            index += 1;
        }
        while words
            .get(index)
            .is_some_and(|word| word.contains('=') && !word.starts_with('-'))
        {
            index += 1;
        }
        if matches!(
            words.get(index).map(String::as_str),
            Some("env" | "command" | "sudo")
        ) {
            index += 1;
        } else {
            break;
        }
    }
    let executable = words.get(index)?.rsplit('/').next()?;
    let args = &words[index + 1..];
    match executable {
        "git" => classify_git(args),
        "gh" => classify_gh(args),
        "bash" | "sh" | "zsh" => args
            .windows(2)
            .find(|pair| pair[0] == "-c" || pair[0] == "-lc")
            .and_then(|pair| classify_comprehension_boundary(&pair[1])),
        _ => None,
    }
}

fn classify_git(args: &[String]) -> Option<ComprehensionBoundary> {
    let mut index = 0;
    while let Some(arg) = args.get(index) {
        if matches!(
            arg.as_str(),
            "-C" | "-c" | "--git-dir" | "--work-tree" | "--namespace"
        ) {
            index += 2;
        } else if arg.starts_with('-') {
            index += 1;
        } else {
            break;
        }
    }
    match args.get(index).map(String::as_str) {
        Some("push") => Some(ComprehensionBoundary::Publish),
        Some("commit") => Some(ComprehensionBoundary::Commit),
        _ => None,
    }
}

fn classify_gh(args: &[String]) -> Option<ComprehensionBoundary> {
    let significant = args
        .iter()
        .filter(|arg| !arg.starts_with('-'))
        .take(2)
        .map(String::as_str)
        .collect::<Vec<_>>();
    match significant.as_slice() {
        ["pr", "create" | "edit" | "ready" | "reopen" | "merge"]
        | ["release", "create" | "edit" | "upload"] => Some(ComprehensionBoundary::Publish),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_git_boundaries_in_compound_commands() {
        assert_eq!(
            classify_comprehension_boundary("cd app && git push origin main"),
            Some(ComprehensionBoundary::Publish)
        );
        assert_eq!(
            classify_comprehension_boundary("git -C app commit -m 'done'"),
            Some(ComprehensionBoundary::Commit)
        );
        assert_eq!(classify_comprehension_boundary("echo 'git push'"), None);
        assert_eq!(
            classify_comprehension_boundary("git commit -m done && git push"),
            Some(ComprehensionBoundary::Publish)
        );
        assert_eq!(
            classify_comprehension_boundary("git push; echo published"),
            Some(ComprehensionBoundary::Publish)
        );
        assert_eq!(
            classify_comprehension_boundary("env GH_TOKEN=secret git push"),
            Some(ComprehensionBoundary::Publish)
        );
        assert_eq!(
            classify_comprehension_boundary("printf '%s' 'git push; still quoted'"),
            None
        );
    }

    #[test]
    fn recognizes_github_publication_but_not_read_only_commands() {
        assert_eq!(
            classify_comprehension_boundary("gh pr create --fill"),
            Some(ComprehensionBoundary::Publish)
        );
        assert_eq!(classify_comprehension_boundary("gh pr view 42"), None);
    }

    #[test]
    fn repository_content_checkpoint_survives_commit_but_not_an_edit() {
        let directory = tempfile::tempdir().unwrap();
        let repository = git2::Repository::init(directory.path()).unwrap();
        std::fs::write(directory.path().join("app.txt"), "understood\n").unwrap();
        let mut index = repository.index().unwrap();
        index.add_path(std::path::Path::new("app.txt")).unwrap();
        index.write().unwrap();

        let before_commit = repository_fingerprint(directory.path()).unwrap();
        let tree_id = index.write_tree().unwrap();
        drop(index);
        let tree = repository.find_tree(tree_id).unwrap();
        let signature = git2::Signature::now("Ovim Test", "ovim@example.com").unwrap();
        repository
            .commit(
                Some("HEAD"),
                &signature,
                &signature,
                "checkpointed change",
                &tree,
                &[],
            )
            .unwrap();
        drop(tree);

        assert_eq!(
            repository_fingerprint(directory.path()).unwrap(),
            before_commit,
            "changing only commit identity must not force a duplicate push drill"
        );
        std::fs::write(directory.path().join("app.txt"), "changed afterward\n").unwrap();
        assert_ne!(
            repository_fingerprint(directory.path()).unwrap(),
            before_commit,
            "new content must invalidate demonstrated comprehension"
        );
    }

    #[test]
    fn publish_policy_blocks_until_a_current_checkpoint_and_ignores_yolo() {
        let directory = tempfile::tempdir().unwrap();
        git2::Repository::init(directory.path()).unwrap();
        let file = directory.path().join("app.txt");
        std::fs::write(&file, "first state\n").unwrap();

        let mut editor = Editor::default();
        editor
            .buffer_mut()
            .set_file_path(file.to_string_lossy().to_string());
        editor
            .open_ai_chat(crate::ai::chat_types::ChatOpts::default())
            .unwrap();
        editor.set_ai_chat_comprehension_policy(ComprehensionPolicy::Publish);
        editor.set_ai_chat_yolo_mode(true);

        assert!(editor
            .comprehension_gate_for_bash("git commit -am test")
            .is_none());
        assert!(editor.comprehension_gate_for_bash("git push").is_some());
        let result = editor.execute_record_comprehension_checkpoint(&serde_json::json!({
            "summary": "The user demonstrated the state transition and its failure mode.",
            "critical_concepts": ["state transition", "failure mode"]
        }));
        assert!(matches!(result, ToolResult::Success(_)));
        assert!(editor.comprehension_gate_for_bash("git push").is_none());

        std::fs::write(&file, "second state\n").unwrap();
        assert!(editor.comprehension_gate_for_bash("git push").is_some());
    }
}
