use crate::ai::path_policy::{has_parent_traversal, sensitive_path_reason};
use std::path::{Path, PathBuf};

use super::ai_chat_tools::{ToolApprovalRequest, ToolPathResolution};
use super::Editor;

impl Editor {
    pub(super) fn active_chat_target_display_path(&self) -> String {
        let path = self
            .ai_state
            .chat
            .as_ref()
            .and_then(|c| self.get_buffer_by_id(c.active_buffer_id))
            .and_then(|b| b.file_path())
            .map(PathBuf::from)
            .or_else(|| self.buffer().file_path().map(PathBuf::from));

        let Some(path) = path else {
            return "[No Name]".to_string();
        };
        let absolute = self.absolutize_path(&path);
        if let Some(root) = self.ai_effective_project_root() {
            let rel = to_relative_path_for_boundary(&absolute, &root);
            return compact_tool_path(&rel);
        }
        compact_tool_path(&absolute.display().to_string())
    }

    pub(super) fn resolve_tool_path_policy(
        &self,
        raw_path: &str,
        treat_as_directory: bool,
        tool_name: &str,
        approved_once_root: Option<&PathBuf>,
    ) -> std::result::Result<ToolPathResolution, String> {
        if has_parent_traversal(Path::new(raw_path)) {
            return Err("path traversal (..) not allowed".to_string());
        }

        let boundary_root = self
            .ai_effective_project_root()
            .ok_or_else(|| self.no_project_root_error())?;
        let boundary_root = normalize_path(&boundary_root);

        let requested_path = {
            let path = Path::new(raw_path);
            if path.is_absolute() {
                self.absolutize_path(path)
            } else {
                let joined = boundary_root.join(path);
                joined
                    .canonicalize()
                    .unwrap_or_else(|_| normalize_path(&joined))
            }
        };
        let approved_once_root = approved_once_root.map(|p| normalize_path(p));
        let approved_once_match = approved_once_root
            .as_ref()
            .is_some_and(|root| requested_path.starts_with(root));
        let approved_session_match = self.current_session_approved_root_for(&requested_path);

        if requested_path.starts_with(&boundary_root) {
            if let Some(reason) = sensitive_path_reason(&requested_path) {
                let approved_sensitive = approved_once_match || approved_session_match.is_some();
                if !approved_sensitive {
                    return Ok(ToolPathResolution::NeedsApproval(ToolApprovalRequest {
                        requested_path: requested_path.clone(),
                        approval_root: requested_path.clone(),
                        reason: format!("sensitive-path access: {reason}"),
                        message: format!(
                            "Approval required: {} wants sensitive-path access to {} ({}). Press Ctrl-Y to allow once, Ctrl-A to allow for this chat session, Ctrl-N to deny.",
                            tool_name,
                            requested_path.display(),
                            reason
                        ),
                    }));
                }
            }
            return Ok(ToolPathResolution::Allowed {
                absolute_path: requested_path,
                boundary_root,
            });
        }

        if let Some(root) = approved_once_root {
            if requested_path.starts_with(&root) {
                return Ok(ToolPathResolution::Allowed {
                    absolute_path: requested_path,
                    boundary_root: root,
                });
            }
        }

        if let Some(root) = approved_session_match {
            return Ok(ToolPathResolution::Allowed {
                absolute_path: requested_path,
                boundary_root: root,
            });
        }

        let approval_root = if treat_as_directory {
            requested_path.clone()
        } else {
            requested_path
                .parent()
                .map(normalize_path)
                .unwrap_or_else(|| requested_path.clone())
        };

        Ok(ToolPathResolution::NeedsApproval(ToolApprovalRequest {
            requested_path: requested_path.clone(),
            approval_root: approval_root.clone(),
            reason: "the requested path is outside the active project".into(),
            message: format!(
                "Approval required: {} wants outside-project access to {}. Press Ctrl-Y to allow once, Ctrl-A to allow for this chat session, Ctrl-N to deny.",
                tool_name,
                requested_path.display()
            ),
        }))
    }

    pub(super) fn current_session_approved_root_for(&self, path: &Path) -> Option<PathBuf> {
        let chat = self.ai_state.chat.as_ref()?;
        for root in &chat.approved_external_roots {
            let root = normalize_path(root);
            if path.starts_with(&root) {
                return Some(root);
            }
        }
        None
    }

    /// Effective project boundary for AI project-level tools.
    ///
    /// Prefers git repository root. Outside git, falls back to a
    /// session-approved folder root.
    pub(crate) fn ai_effective_project_root(&self) -> Option<PathBuf> {
        self.ai_repo_root().or_else(|| {
            self.ai_state
                .no_repo_session_allowed_root
                .as_ref()
                .map(|p| normalize_path(p))
        })
    }

    pub(super) fn ai_project_start_path(&self) -> Option<PathBuf> {
        let active_target_file = self
            .ai_state
            .chat
            .as_ref()
            .and_then(|chat| self.get_buffer_by_id(chat.active_buffer_id))
            .and_then(|buf| buf.file_path())
            .map(PathBuf::from);
        let origin_file = self
            .ai_state
            .chat
            .as_ref()
            .and_then(|chat| self.get_buffer_by_id(chat.origin_buffer_id))
            .and_then(|buf| buf.file_path())
            .map(PathBuf::from);
        let current_file = self.buffer().file_path().map(PathBuf::from);

        if let Some(file) = active_target_file.or(origin_file).or(current_file) {
            Some(self.absolutize_path(&file))
        } else {
            std::env::current_dir().ok()
        }
    }

    pub(super) fn ai_no_repo_candidate_root(&self) -> Option<PathBuf> {
        let start = self.ai_project_start_path()?;
        if start.is_dir() {
            Some(normalize_path(&start))
        } else {
            start.parent().map(normalize_path)
        }
    }

    pub(super) fn no_project_root_error(&self) -> String {
        "No project boundary available. You're not in a git repo and no folder access was approved for this session.".to_string()
    }

    /// Repository root for AI project-level tools.
    ///
    /// Resolves from current file (if available) or current working directory.
    pub(crate) fn ai_repo_root(&self) -> Option<PathBuf> {
        let start = self.ai_project_start_path()?;
        discover_repo_root_from_start(&start)
    }

    pub(super) fn absolutize_path(&self, path: &Path) -> PathBuf {
        let joined = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .map(|cwd| cwd.join(path))
                .unwrap_or_else(|_| path.to_path_buf())
        };
        joined
            .canonicalize()
            .unwrap_or_else(|_| normalize_path(&joined))
    }
}

pub(super) fn normalize_path(path: &Path) -> PathBuf {
    crate::ai::path_policy::normalize_path(path)
}

pub(super) fn discover_repo_root_from_start(start: &Path) -> Option<PathBuf> {
    let probe = if start.is_dir() {
        start.to_path_buf()
    } else {
        start.parent()?.to_path_buf()
    };

    if let Ok(repo) = git2::Repository::discover(&probe) {
        if let Some(workdir) = repo.workdir() {
            return Some(normalize_path(workdir));
        }
        if let Some(parent) = repo.path().parent() {
            return Some(normalize_path(parent));
        }
        return Some(normalize_path(repo.path()));
    }

    // Fallback for marker-only setups (e.g. tests, partial repos).
    let mut dir = probe;
    loop {
        if dir.join(".git").exists() {
            return Some(normalize_path(&dir));
        }
        if !dir.pop() {
            break;
        }
    }
    None
}

pub(super) fn to_relative_path_for_boundary(path: &Path, boundary_root: &Path) -> String {
    let rel = path.strip_prefix(boundary_root).unwrap_or(path);
    if rel.as_os_str().is_empty() {
        ".".to_string()
    } else {
        rel.to_string_lossy().to_string()
    }
}

pub(super) fn compact_tool_path(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    let parts: Vec<&str> = normalized.split('/').filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return ".".to_string();
    }

    let keep = 3usize.min(parts.len());
    let tail = parts[parts.len() - keep..].join("/");
    let max_chars = 42usize;
    if tail.chars().count() <= max_chars {
        return tail;
    }

    let mut out: String = tail.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('\u{2026}');
    out
}

pub(super) fn compact_tool_label(text: &str) -> String {
    let single_line = text
        .replace('\n', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let max_chars = 72;
    if single_line.chars().count() <= max_chars {
        return single_line;
    }
    let mut out: String = single_line
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect();
    out.push('\u{2026}');
    out
}
