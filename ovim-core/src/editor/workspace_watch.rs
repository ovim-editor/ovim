//! File-system watching for files changed outside the editor.
//!
//! Language servers that register `workspace/didChangeWatchedFiles` must hear
//! about creates/changes/deletes anywhere in the workspace (git checkout, build
//! tools, other editors), not only for open buffers.
//!
//! Design constraints (rust-analyzer registers watchers for every Rust
//! project, and `target/` can hold hundreds of thousands of directories):
//! - Directories are walked with a `.gitignore`-aware walker plus a hard skip
//!   list, and watched NON-recursively; a directory created later is added as
//!   it appears. The number of watched directories is capped.
//! - All walking and watch registration happens on a worker thread. The editor
//!   tick only sends commands and drains channels; it never blocks on I/O.
//! - Raw events are filtered by the servers' registered globs before they are
//!   stored. A burst larger than the pending cap is released early, never
//!   dropped.

use crate::lsp::{WatchedChange, WatchedFileEvent};
use ignore::WalkBuilder;
use notify::event::{ModifyKind, RenameMode};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Quiet period after the last event before a batch is released.
const QUIET_PERIOD: Duration = Duration::from_millis(150);
/// A batch is released after this long even if events keep arriving.
const MAX_BATCH_AGE: Duration = Duration::from_secs(1);
/// Most distinct paths held before a batch is released without waiting for
/// the burst to settle.
const MAX_PENDING: usize = 10_000;
/// Most directories watched at once. Each is an inotify watch, a limited
/// per-user resource shared with every other program.
const MAX_WATCHED_DIRS: usize = 20_000;

/// Directory names that are never watched, wherever they appear.
const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    ".gradle",
    ".idea",
    "__pycache__",
    ".venv",
];

/// Build output directories, skipped directly under a watch root only: deeper
/// down the same names are ordinary source directories (a `build` package),
/// and generated ones are left out by `.gitignore`.
const ROOT_SKIP_DIRS: &[&str] = &["target", "build", "out", "dist"];

/// Whether `relative` (a path below a watch root) is in a skipped directory.
fn is_skipped_relative(relative: &Path) -> bool {
    relative.components().enumerate().any(|(depth, part)| {
        let name = part.as_os_str();
        SKIP_DIRS.iter().any(|skip| name == *skip)
            || (depth == 0 && ROOT_SKIP_DIRS.iter().any(|skip| name == *skip))
    })
}

#[derive(Default, Clone, Copy)]
struct Seen {
    created: bool,
    removed: bool,
}

enum Command {
    /// Make the watched roots equal to this set.
    Sync(Vec<PathBuf>),
    /// Watch a newly created directory (and its non-ignored subdirectories).
    AddDir(PathBuf),
}

type WatchedSet = Arc<Mutex<HashSet<PathBuf>>>;

/// Owns the `notify` watcher; runs all blocking work.
fn worker(
    commands: Receiver<Command>,
    events: Sender<notify::Result<notify::Event>>,
    errors: Sender<String>,
    watched: WatchedSet,
    max_dirs: usize,
) {
    let callback_events = events;
    let mut watcher: RecommendedWatcher = match notify::recommended_watcher(move |event| {
        let _ = callback_events.send(event);
    }) {
        Ok(watcher) => watcher,
        Err(error) => {
            let _ = errors.send(format!(
                "cannot watch files for the language server: {error}"
            ));
            return;
        }
    };
    let mut roots: BTreeSet<PathBuf> = BTreeSet::new();

    // Watches `top` and the directories below it; `root` is the watch root
    // `top` belongs to, which skip rules are relative to.
    let watch_tree = |watcher: &mut RecommendedWatcher,
                      root: &Path,
                      top: &Path,
                      first_error: &mut Option<String>| {
        let root = root.to_path_buf();
        let walker = WalkBuilder::new(top)
            .hidden(false)
            .require_git(false)
            .follow_links(false)
            .filter_entry(move |entry| {
                entry
                    .path()
                    .strip_prefix(&root)
                    .map_or(true, |relative| !is_skipped_relative(relative))
            })
            .build();
        for entry in walker.flatten() {
            if !entry.file_type().is_some_and(|t| t.is_dir()) {
                continue;
            }
            let dir = entry.into_path();
            let count = match watched.lock() {
                Ok(watched) if watched.contains(&dir) => continue,
                Ok(watched) => watched.len(),
                Err(_) => continue,
            };
            if count >= max_dirs {
                first_error.get_or_insert(format!(
                    "watching only {max_dirs} directories for the language server; \
                     changes elsewhere in the project will be missed"
                ));
                break;
            }
            match watcher.watch(&dir, RecursiveMode::NonRecursive) {
                Ok(()) => {
                    if let Ok(mut w) = watched.lock() {
                        w.insert(dir);
                    }
                }
                Err(error) => {
                    first_error.get_or_insert(format!(
                        "cannot watch {} for the language server: {error}",
                        dir.display()
                    ));
                }
            }
        }
    };

    while let Ok(command) = commands.recv() {
        let mut error = None;
        match command {
            Command::Sync(wanted) => {
                let wanted: BTreeSet<PathBuf> = wanted.into_iter().collect();
                for stale in roots.difference(&wanted) {
                    if let Ok(mut w) = watched.lock() {
                        let gone: Vec<PathBuf> =
                            w.iter().filter(|d| d.starts_with(stale)).cloned().collect();
                        for dir in gone {
                            let _ = watcher.unwatch(&dir);
                            w.remove(&dir);
                        }
                    }
                }
                for added in wanted.difference(&roots) {
                    watch_tree(&mut watcher, added, added, &mut error);
                }
                roots = wanted;
            }
            Command::AddDir(dir) => {
                if let Some(root) = roots.iter().find(|root| dir.starts_with(root)) {
                    watch_tree(&mut watcher, root, &dir, &mut error);
                }
            }
        }
        if let Some(message) = error {
            let _ = errors.send(message);
        }
    }
}

pub struct WorkspaceWatcher {
    commands: Option<Sender<Command>>,
    events: Option<Receiver<notify::Result<notify::Event>>>,
    errors: Option<Receiver<String>>,
    watched: WatchedSet,
    roots: BTreeSet<PathBuf>,
    pending: HashMap<PathBuf, Seen>,
    first_event: Option<Instant>,
    last_event: Option<Instant>,
    /// More paths are pending than one batch should hold: release it now.
    release_now: bool,
    max_dirs: usize,
    max_pending: usize,
    /// Last watcher setup error, surfaced once to the user.
    pub last_error: Option<String>,
}

impl Default for WorkspaceWatcher {
    fn default() -> Self {
        Self::with_limits(MAX_WATCHED_DIRS, MAX_PENDING)
    }
}

impl WorkspaceWatcher {
    /// A watcher that watches at most `max_dirs` directories and releases a
    /// batch early once `max_pending` distinct paths are waiting.
    pub fn with_limits(max_dirs: usize, max_pending: usize) -> Self {
        Self {
            commands: None,
            events: None,
            errors: None,
            watched: WatchedSet::default(),
            roots: BTreeSet::new(),
            pending: HashMap::new(),
            first_event: None,
            last_event: None,
            release_now: false,
            max_dirs,
            max_pending,
            last_error: None,
        }
    }

    pub fn roots(&self) -> &BTreeSet<PathBuf> {
        &self.roots
    }

    /// Directories currently watched (for diagnostics and tests).
    pub fn watched_dirs(&self) -> HashSet<PathBuf> {
        self.watched.lock().map(|w| w.clone()).unwrap_or_default()
    }

    /// Asks the worker to make the watched roots equal `wanted`. Returns
    /// immediately; setup errors arrive later through [`Self::poll`].
    pub fn sync_roots(&mut self, wanted: &[PathBuf]) {
        // The home directory and above are never a project; watching them
        // would claim a watch for every directory in the user's account.
        let wanted: BTreeSet<PathBuf> = wanted
            .iter()
            .filter(|root| !crate::project_root::is_too_broad_to_watch(root))
            .cloned()
            .collect();
        if wanted == self.roots {
            return;
        }
        if wanted.is_empty() {
            // Dropping the command channel stops the worker and its watcher.
            *self = Self::with_limits(self.max_dirs, self.max_pending);
            return;
        }
        if self.commands.is_none() {
            let (cmd_tx, cmd_rx) = channel();
            let (ev_tx, ev_rx) = channel();
            let (err_tx, err_rx) = channel();
            let watched = self.watched.clone();
            let max_dirs = self.max_dirs;
            std::thread::Builder::new()
                .name("ovim-workspace-watch".into())
                .spawn(move || worker(cmd_rx, ev_tx, err_tx, watched, max_dirs))
                .ok();
            self.commands = Some(cmd_tx);
            self.events = Some(ev_rx);
            self.errors = Some(err_rx);
        }
        if let Some(commands) = &self.commands {
            let _ = commands.send(Command::Sync(wanted.iter().cloned().collect()));
        }
        self.roots = wanted;
    }

    fn is_ignored(&self, path: &Path) -> bool {
        let relative = self
            .roots
            .iter()
            .find_map(|root| path.strip_prefix(root).ok())
            .unwrap_or(path);
        is_skipped_relative(relative)
    }

    fn record(&mut self, event: notify::Event, now: Instant, wanted: &dyn Fn(&Path) -> bool) {
        let created_dirs: Vec<PathBuf> = match event.kind {
            EventKind::Create(_)
            | EventKind::Modify(ModifyKind::Name(RenameMode::To | RenameMode::Both)) => event
                .paths
                .iter()
                .filter(|p| !self.is_ignored(p) && p.is_dir())
                .cloned()
                .collect(),
            _ => Vec::new(),
        };
        if let Some(commands) = &self.commands {
            for dir in created_dirs {
                let _ = commands.send(Command::AddDir(dir));
            }
        }

        let mark = |this: &mut Self, path: &PathBuf, created: bool, removed: bool| {
            if this.is_ignored(path) || !wanted(path) {
                return;
            }
            if this.pending.len() >= this.max_pending {
                // A burst this large (a checkout, a generator) is released
                // now instead of growing without bound; nothing is dropped.
                this.release_now = true;
            }
            let seen = this.pending.entry(path.clone()).or_default();
            seen.created |= created;
            seen.removed |= removed;
            this.first_event.get_or_insert(now);
            this.last_event = Some(now);
        };
        match event.kind {
            EventKind::Create(_) => event.paths.iter().for_each(|p| mark(self, p, true, false)),
            EventKind::Remove(_) => event.paths.iter().for_each(|p| mark(self, p, false, true)),
            EventKind::Modify(ModifyKind::Name(RenameMode::From)) => {
                event.paths.iter().for_each(|p| mark(self, p, false, true))
            }
            EventKind::Modify(ModifyKind::Name(RenameMode::To)) => {
                event.paths.iter().for_each(|p| mark(self, p, true, false))
            }
            EventKind::Modify(ModifyKind::Name(RenameMode::Both)) => {
                if let [from, to] = event.paths.as_slice() {
                    mark(self, from, false, true);
                    mark(self, to, true, false);
                }
            }
            EventKind::Modify(_) | EventKind::Any => {
                event.paths.iter().for_each(|p| mark(self, p, false, false))
            }
            EventKind::Access(_) | EventKind::Other => {}
        }
    }

    /// Drains raw events (keeping only paths `wanted` accepts) and returns a
    /// coalesced batch once the burst has settled. Never blocks.
    pub fn poll(
        &mut self,
        now: Instant,
        wanted: &dyn Fn(&Path) -> bool,
    ) -> Option<Vec<WatchedFileEvent>> {
        if let Some(errors) = &self.errors {
            while let Ok(message) = errors.try_recv() {
                self.last_error = Some(message);
            }
        }
        let mut raw = Vec::new();
        if let Some(rx) = &self.events {
            while let Ok(Ok(event)) = rx.try_recv() {
                raw.push(event);
                if raw.len() >= 4096 {
                    break; // keep the tick short; the rest is drained next tick
                }
            }
        }
        for event in raw {
            self.record(event, now, wanted);
        }
        let (first, last) = (self.first_event?, self.last_event?);
        if !self.release_now
            && now.duration_since(last) < QUIET_PERIOD
            && now.duration_since(first) < MAX_BATCH_AGE
        {
            return None;
        }
        self.first_event = None;
        self.last_event = None;
        self.release_now = false;
        let mut events: Vec<WatchedFileEvent> = std::mem::take(&mut self.pending)
            .into_iter()
            .filter_map(|(path, seen)| {
                let exists = path.exists();
                if exists && path.is_dir() {
                    return None;
                }
                let change = if !exists {
                    WatchedChange::Deleted
                } else if seen.created && !seen.removed {
                    WatchedChange::Created
                } else {
                    WatchedChange::Changed
                };
                Some(WatchedFileEvent { path, change })
            })
            .collect();
        events.sort_by(|a, b| a.path.cmp(&b.path));
        (!events.is_empty()).then_some(events)
    }

    /// Takes the last setup error so it is reported once.
    pub fn take_error(&mut self) -> Option<String> {
        self.last_error.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait_until(mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done() {
            assert!(Instant::now() < deadline, "timed out");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn drain(watcher: &mut WorkspaceWatcher, wait: Duration) -> Vec<WatchedFileEvent> {
        let end = Instant::now() + wait;
        let mut all = Vec::new();
        while Instant::now() < end {
            if let Some(batch) = watcher.poll(Instant::now(), &|_| true) {
                all.extend(batch);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        all
    }

    #[test]
    fn skips_target_watches_source_dirs_and_follows_new_directories() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(root.join("src/a")).unwrap();
        for n in 0..300 {
            std::fs::create_dir_all(root.join(format!("target/debug/deps/d{n}"))).unwrap();
        }
        std::fs::create_dir_all(root.join("node_modules/x")).unwrap();

        let mut watcher = WorkspaceWatcher::default();
        let started = Instant::now();
        watcher.sync_roots(std::slice::from_ref(&root));
        assert!(
            started.elapsed() < Duration::from_millis(100),
            "sync_roots must not walk on the caller's thread"
        );
        wait_until(|| watcher.watched_dirs().contains(&root.join("src/a")));
        let watched = watcher.watched_dirs();
        assert!(watched.contains(&root) && watched.contains(&root.join("src")));
        assert!(
            watched.iter().all(|d| !d.starts_with(root.join("target"))
                && !d.starts_with(root.join("node_modules"))),
            "skip-listed directories must not be watched: {watched:?}"
        );
        assert!(watched.len() < 10, "{}", watched.len());

        // Changes under target/ produce no events; changes in src do.
        std::fs::write(root.join("target/debug/deps/d1/x.rs"), "x").unwrap();
        std::fs::write(root.join("src/a/lib.rs"), "fn a() {}").unwrap();
        let events = drain(&mut watcher, Duration::from_millis(800));
        assert!(
            events.iter().any(|e| e.path == root.join("src/a/lib.rs")),
            "{events:?}"
        );
        assert!(events
            .iter()
            .all(|e| !e.path.starts_with(root.join("target"))));

        // A directory created later gets its own watch.
        std::fs::create_dir_all(root.join("src/newdir")).unwrap();
        // The tick (poll) is what notices the Create event and asks for the watch.
        wait_until(|| {
            watcher.poll(Instant::now(), &|_| true);
            watcher.watched_dirs().contains(&root.join("src/newdir"))
        });
        std::fs::write(root.join("src/newdir/b.rs"), "fn b() {}").unwrap();
        let events = drain(&mut watcher, Duration::from_millis(800));
        assert!(
            events
                .iter()
                .any(|e| e.path == root.join("src/newdir/b.rs")),
            "{events:?}"
        );
    }

    #[test]
    fn build_named_source_directories_below_the_root_are_watched() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(root.join("src/build/inner")).unwrap();
        std::fs::create_dir_all(root.join("lib/out")).unwrap();
        std::fs::create_dir_all(root.join("build/generated")).unwrap();
        std::fs::create_dir_all(root.join("dist")).unwrap();

        let mut watcher = WorkspaceWatcher::default();
        watcher.sync_roots(std::slice::from_ref(&root));
        wait_until(|| {
            watcher
                .watched_dirs()
                .contains(&root.join("src/build/inner"))
        });
        let watched = watcher.watched_dirs();
        assert!(watched.contains(&root.join("lib/out")), "{watched:?}");
        assert!(
            watched
                .iter()
                .all(|d| !d.starts_with(root.join("build")) && !d.starts_with(root.join("dist"))),
            "build output at the root must stay unwatched: {watched:?}"
        );

        // A source file below `src/build` is reported; one in the root-level
        // `build` is not.
        std::fs::write(root.join("src/build/B.java"), "class B {}").unwrap();
        std::fs::write(root.join("build/generated/G.java"), "class G {}").unwrap();
        let events = drain(&mut watcher, Duration::from_millis(800));
        assert!(events.iter().any(|e| e.path.ends_with("src/build/B.java")));
        assert!(events
            .iter()
            .all(|e| !e.path.starts_with(root.join("build"))));
    }

    #[test]
    fn watching_stops_at_the_directory_cap_and_reports_it() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        for n in 0..40 {
            std::fs::create_dir_all(root.join(format!("d{n}"))).unwrap();
        }

        let mut watcher = WorkspaceWatcher::with_limits(5, MAX_PENDING);
        watcher.sync_roots(std::slice::from_ref(&root));
        wait_until(|| {
            watcher.poll(Instant::now(), &|_| true);
            watcher.last_error.is_some()
        });
        assert_eq!(watcher.watched_dirs().len(), 5);
        assert!(watcher
            .take_error()
            .is_some_and(|message| message.contains("only 5 directories")),);
    }

    #[test]
    fn a_burst_over_the_pending_cap_is_released_in_pieces_not_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let mut watcher = WorkspaceWatcher::with_limits(MAX_WATCHED_DIRS, 5);
        watcher.sync_roots(std::slice::from_ref(&root));
        wait_until(|| watcher.watched_dirs().contains(&root));

        for n in 0..30 {
            std::fs::write(root.join(format!("f{n}.rs")), "").unwrap();
        }
        let events = drain(&mut watcher, Duration::from_millis(1500));
        let reported: HashSet<PathBuf> = events.into_iter().map(|e| e.path).collect();
        for n in 0..30 {
            assert!(
                reported.contains(&root.join(format!("f{n}.rs"))),
                "f{n}.rs was lost from a large burst"
            );
        }
    }

    #[test]
    fn the_filesystem_root_and_home_are_never_watched() {
        let mut roots = vec![PathBuf::from("/")];
        roots.extend(dirs::home_dir());
        let mut watcher = WorkspaceWatcher::default();
        watcher.sync_roots(&roots);
        std::thread::sleep(Duration::from_millis(300));
        assert!(watcher.roots().is_empty());
        assert!(watcher.watched_dirs().is_empty());
    }

    #[test]
    fn events_are_filtered_by_registered_globs_before_being_stored() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let mut watcher = WorkspaceWatcher::default();
        watcher.sync_roots(std::slice::from_ref(&root));
        wait_until(|| watcher.watched_dirs().contains(&root));
        std::fs::write(root.join("a.rs"), "").unwrap();
        std::fs::write(root.join("a.txt"), "").unwrap();
        let end = Instant::now() + Duration::from_millis(800);
        let mut events = Vec::new();
        while Instant::now() < end {
            if let Some(b) = watcher.poll(Instant::now(), &|p| {
                p.extension().is_some_and(|e| e == "rs")
            }) {
                events.extend(b);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(events.iter().any(|e| e.path.ends_with("a.rs")));
        assert!(events.iter().all(|e| !e.path.ends_with("a.txt")));
    }
}
