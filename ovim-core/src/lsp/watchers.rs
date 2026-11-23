//! `workspace/didChangeWatchedFiles`: server-registered file watchers.
//!
//! Servers ask the client to watch files through dynamic registration
//! (`client/registerCapability` with method `workspace/didChangeWatchedFiles`)
//! and expect a `workspace/didChangeWatchedFiles` notification whenever a
//! matching file is created, changed or deleted on disk - including files that
//! are not open in any buffer (git checkout, build tools, other editors). The
//! editor's file-system watcher feeds events into
//! [`LspManager::send_watched_file_changes`], which routes each event to the
//! servers whose registered globs match it.

use super::*;
use globset::{Glob, GlobBuilder, GlobMatcher};
use lsp_types::{
    DidChangeWatchedFilesParams, FileChangeType, FileEvent, GlobPattern, OneOf, WatchKind,
};

/// A change observed on disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WatchedFileEvent {
    pub path: PathBuf,
    pub change: WatchedChange,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WatchedChange {
    Created,
    Changed,
    Deleted,
}

impl WatchedChange {
    fn to_lsp(self) -> FileChangeType {
        match self {
            WatchedChange::Created => FileChangeType::CREATED,
            WatchedChange::Changed => FileChangeType::CHANGED,
            WatchedChange::Deleted => FileChangeType::DELETED,
        }
    }

    fn watch_kind(self) -> WatchKind {
        match self {
            WatchedChange::Created => WatchKind::Create,
            WatchedChange::Changed => WatchKind::Change,
            WatchedChange::Deleted => WatchKind::Delete,
        }
    }
}

#[derive(Debug)]
struct CompiledWatcher {
    matcher: GlobMatcher,
    /// Relative patterns match paths relative to this directory.
    base: Option<PathBuf>,
    /// Bitmask of `WatchKind` (create=1, change=2, delete=4).
    kinds: u8,
}

#[derive(Debug)]
pub(super) struct WatcherRegistration {
    id: String,
    watchers: Vec<CompiledWatcher>,
}

fn compile_glob(pattern: &str) -> Option<GlobMatcher> {
    let glob: Glob = GlobBuilder::new(pattern)
        .literal_separator(true)
        .build()
        .ok()?;
    Some(glob.compile_matcher())
}

/// Parses the registration options of a `workspace/didChangeWatchedFiles`
/// registration. Watchers with unparsable globs are dropped.
pub(super) fn parse_registration(
    id: &str,
    options: Option<&serde_json::Value>,
) -> Option<WatcherRegistration> {
    let options: lsp_types::DidChangeWatchedFilesRegistrationOptions =
        serde_json::from_value(options?.clone()).ok()?;
    let watchers = options
        .watchers
        .iter()
        .filter_map(|watcher| {
            let (pattern, base) = match &watcher.glob_pattern {
                GlobPattern::String(pattern) => (pattern.clone(), None),
                GlobPattern::Relative(relative) => {
                    let base_uri = match &relative.base_uri {
                        OneOf::Left(folder) => &folder.uri,
                        OneOf::Right(uri) => uri,
                    };
                    (relative.pattern.clone(), uri_to_file_path(base_uri))
                }
            };
            Some(CompiledWatcher {
                matcher: compile_glob(&pattern)?,
                base,
                kinds: watcher.kind.map_or(7, |kind| {
                    let mut mask = 0;
                    if kind.contains(WatchKind::Create) {
                        mask |= 1;
                    }
                    if kind.contains(WatchKind::Change) {
                        mask |= 2;
                    }
                    if kind.contains(WatchKind::Delete) {
                        mask |= 4;
                    }
                    mask
                }),
            })
        })
        .collect();
    Some(WatcherRegistration {
        id: id.to_string(),
        watchers,
    })
}

impl CompiledWatcher {
    fn matches(&self, path: &Path, root: Option<&Path>, change: WatchedChange) -> bool {
        let bit = match change.watch_kind() {
            WatchKind::Create => 1,
            WatchKind::Change => 2,
            _ => 4,
        };
        if self.kinds & bit == 0 {
            return false;
        }
        if let Some(base) = &self.base {
            return path
                .strip_prefix(base)
                .is_ok_and(|relative| self.matcher.is_match(relative));
        }
        // Plain patterns are usually `**/*.ext` (match anywhere) or absolute;
        // servers also write them relative to the workspace root.
        self.matcher.is_match(path)
            || root
                .and_then(|root| path.strip_prefix(root).ok())
                .is_some_and(|relative| self.matcher.is_match(relative))
    }
}

impl LspManager {
    /// Records a dynamic `workspace/didChangeWatchedFiles` registration.
    pub(super) fn register_file_watchers(
        &self,
        server_id: &str,
        registration: WatcherRegistration,
    ) {
        let mut entry = self
            .file_watch_registrations
            .entry(server_id.to_string())
            .or_default();
        entry.retain(|existing| existing.id != registration.id);
        entry.push(registration);
    }

    pub(super) fn unregister_file_watchers(&self, server_id: &str, id: &str) {
        if let Some(mut entry) = self.file_watch_registrations.get_mut(server_id) {
            entry.retain(|existing| existing.id != id);
        }
    }

    /// Directories the editor must watch so registered servers hear about
    /// changes: the root of every server that registered file watchers, plus
    /// the base directory of any relative-pattern watcher. A fallback root
    /// (no project marker found), the home directory and the filesystem root
    /// are never watched: they would claim a watch for every directory below.
    pub fn watched_file_roots(&self) -> Vec<PathBuf> {
        let mut roots = Vec::new();
        for entry in self.file_watch_registrations.iter() {
            if entry.value().iter().all(|reg| reg.watchers.is_empty()) {
                continue;
            }
            if !self.fallback_root_servers.contains(entry.key()) {
                if let Some(root) = self.server_roots.get(entry.key()) {
                    roots.push(root.value().clone());
                }
            }
            for registration in entry.value() {
                for watcher in &registration.watchers {
                    if let Some(base) = &watcher.base {
                        roots.push(base.clone());
                    }
                }
            }
        }
        roots.retain(|root| !crate::project_root::is_too_broad_to_watch(root));
        roots.sort();
        roots.dedup();
        // A root nested in another root is already covered by the recursive watch.
        let all = roots.clone();
        roots.retain(|root| {
            !all.iter()
                .any(|other| other != root && root.starts_with(other))
        });
        roots
    }

    /// Cheap pre-filter for raw file events: does any registered watcher glob
    /// match `path` for some kind of change?
    pub fn watched_path_matches(&self, path: &Path) -> bool {
        self.file_watch_registrations.iter().any(|entry| {
            let root = self
                .server_roots
                .get(entry.key())
                .map(|r| r.value().clone());
            entry.value().iter().any(|reg| {
                reg.watchers.iter().any(|w| {
                    [
                        WatchedChange::Created,
                        WatchedChange::Changed,
                        WatchedChange::Deleted,
                    ]
                    .into_iter()
                    .any(|c| w.matches(path, root.as_deref(), c))
                })
            })
        })
    }

    /// True when at least one server asked to be told about file changes.
    pub fn has_file_watchers(&self) -> bool {
        self.file_watch_registrations
            .iter()
            .any(|entry| entry.value().iter().any(|reg| !reg.watchers.is_empty()))
    }

    /// Sends `workspace/didChangeWatchedFiles` to every server whose
    /// registered watchers match some of `events`. Returns how many
    /// notifications were sent.
    pub async fn send_watched_file_changes(&self, events: &[WatchedFileEvent]) -> usize {
        let mut sent = 0;
        let targets: Vec<String> = self
            .file_watch_registrations
            .iter()
            .map(|entry| entry.key().clone())
            .collect();
        for server_id in targets {
            let root = self.server_roots.get(&server_id).map(|r| r.value().clone());
            let changes: Vec<FileEvent> = {
                let Some(registrations) = self.file_watch_registrations.get(&server_id) else {
                    continue;
                };
                events
                    .iter()
                    .filter(|event| {
                        registrations.iter().any(|reg| {
                            reg.watchers.iter().any(|watcher| {
                                watcher.matches(&event.path, root.as_deref(), event.change)
                            })
                        })
                    })
                    .filter_map(|event| {
                        Some(FileEvent::new(
                            uri_from_file_path(&event.path)?,
                            event.change.to_lsp(),
                        ))
                    })
                    .collect()
            };
            if changes.is_empty() {
                continue;
            }
            let Some(server) = self.servers.get(&server_id).map(|s| s.value().clone()) else {
                continue;
            };
            let params = match serde_json::to_value(DidChangeWatchedFilesParams { changes }) {
                Ok(params) => params,
                Err(_) => continue,
            };
            match server
                .notify("workspace/didChangeWatchedFiles", params)
                .await
            {
                Ok(()) => sent += 1,
                Err(error) => lsp_warn!(
                    "LspManager",
                    "didChangeWatchedFiles to {} failed: {}",
                    server_id,
                    error
                ),
            }
        }
        sent
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn watch_everything() -> WatcherRegistration {
        registration(serde_json::json!({"watchers": [{"globPattern": "**/*"}]}))
    }

    #[test]
    fn fallback_and_overly_broad_roots_are_not_watched() {
        let manager = LspManager::new();
        let project = std::env::temp_dir().join("ovim-watch-project");
        let loose = std::env::temp_dir().join("ovim-watch-loose-file-dir");
        for (server, root) in [
            ("project", project.clone()),
            ("loose", loose),
            ("home", dirs::home_dir().unwrap_or_else(|| "/home".into())),
            ("top", PathBuf::from("/")),
        ] {
            manager.server_roots.insert(server.to_string(), root);
            manager.register_file_watchers(server, watch_everything());
        }
        manager.mark_fallback_root("loose");

        assert_eq!(manager.watched_file_roots(), vec![project]);
    }

    fn registration(json: serde_json::Value) -> WatcherRegistration {
        parse_registration("r1", Some(&json)).expect("registration")
    }

    #[test]
    fn globs_match_absolute_and_root_relative_paths() {
        let reg = registration(serde_json::json!({
            "watchers": [
                {"globPattern": "**/*.{java,kt}"},
                {"globPattern": "build.gradle"},
                {"globPattern": "src/**/*.xml", "kind": 2},
            ]
        }));
        let root = Path::new("/ws");
        let matches = |path: &str, change| {
            reg.watchers
                .iter()
                .any(|w| w.matches(Path::new(path), Some(root), change))
        };
        assert!(matches("/ws/a/b/C.java", WatchedChange::Changed));
        assert!(matches("/ws/C.kt", WatchedChange::Deleted));
        assert!(!matches("/ws/C.txt", WatchedChange::Changed));
        assert!(matches("/ws/build.gradle", WatchedChange::Changed));
        assert!(!matches("/ws/sub/build.gradle", WatchedChange::Changed));
        assert!(matches("/ws/src/main/x.xml", WatchedChange::Changed));
        assert!(
            !matches("/ws/src/main/x.xml", WatchedChange::Created),
            "kind=2 only watches changes"
        );
    }

    #[test]
    fn relative_patterns_match_below_their_base() {
        let reg = registration(serde_json::json!({
            "watchers": [{"globPattern": {"baseUri": "file:///ws/app", "pattern": "**/*.java"}}]
        }));
        let w = &reg.watchers[0];
        assert!(w.matches(
            Path::new("/ws/app/src/A.java"),
            None,
            WatchedChange::Created
        ));
        assert!(!w.matches(Path::new("/ws/other/A.java"), None, WatchedChange::Created));
    }
}
