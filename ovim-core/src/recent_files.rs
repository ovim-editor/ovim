//! Recently opened files, remembered across sessions per project.
//!
//! One small JSON document under the user's data directory holds every
//! project's list. Several ovim processes may run at once, so each update
//! re-reads the file under an advisory lock, merges, and replaces it
//! atomically. Bare `Editor`s (tests, embedders) never touch the disk: a
//! frontend opts in with `Editor::enable_recent_files`. That store writes on a
//! background thread and ignores projects under the system temp directory.

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

const VERSION: u32 = 1;
/// Files remembered per project.
const MAX_PER_PROJECT: usize = 200;
/// Projects remembered (least recently used are dropped).
const MAX_PROJECTS: usize = 40;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentEntry {
    pub path: String,
    /// Unix seconds of the last visit.
    pub opened: u64,
    /// Cursor when the file was last left (0-based).
    pub line: usize,
    pub col: usize,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Document {
    #[serde(default)]
    version: u32,
    /// Project root -> entries, most recent first.
    #[serde(default)]
    projects: std::collections::BTreeMap<String, Vec<RecentEntry>>,
}

type Job = Box<dyn FnOnce() + Send>;

/// Handle to the on-disk store.
#[derive(Debug, Clone)]
pub struct RecentFiles {
    path: PathBuf,
    /// Present for the user's store: updates run off the editor loop, in order.
    writer: Option<Arc<Writer>>,
    /// Scratch checkouts and test fixtures would crowd out real projects.
    skip_temporary: bool,
}

#[derive(Debug)]
struct Writer(std::sync::Mutex<mpsc::Sender<Job>>);

impl Writer {
    fn spawn() -> Option<Self> {
        let (sender, receiver) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("ovim-recent-files".into())
            .spawn(move || receiver.into_iter().for_each(|job| job()))
            .ok()?;
        Some(Self(std::sync::Mutex::new(sender)))
    }

    fn send(&self, job: Job) {
        if let Ok(sender) = self.0.lock() {
            let _ = sender.send(job);
        }
    }

    /// Waits until every update queued so far has been written.
    fn flush(&self) {
        let (done, wait) = mpsc::channel();
        self.send(Box::new(move || {
            let _ = done.send(());
        }));
        let _ = wait.recv();
    }
}

impl RecentFiles {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            writer: None,
            skip_temporary: false,
        }
    }

    /// The user's store: `$OVIM_RECENT_FILES`, else
    /// `<data dir>/ovim/recent-files.json`.
    pub fn discover() -> Option<Self> {
        let path = std::env::var_os("OVIM_RECENT_FILES")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .or_else(|| dirs::data_local_dir().map(|root| root.join("ovim/recent-files.json")))?;
        Some(Self {
            writer: Writer::spawn().map(Arc::new),
            skip_temporary: true,
            ..Self::new(path)
        })
    }

    fn ignores(&self, project: &Path) -> bool {
        self.skip_temporary && is_temporary(project)
    }

    fn update(&self, job: impl FnOnce(&Self) + Send + 'static) {
        match &self.writer {
            Some(writer) => {
                let store = Self::new(self.path.clone());
                writer.send(Box::new(move || job(&store)));
            }
            None => job(self),
        }
    }

    fn load(&self) -> Document {
        let Ok(bytes) = fs::read(&self.path) else {
            return Document::default();
        };
        match serde_json::from_slice::<Document>(&bytes) {
            Ok(document) if document.version == VERSION => document,
            _ => Document::default(),
        }
    }

    fn lock(&self) -> Option<File> {
        let directory = self.path.parent()?;
        fs::create_dir_all(directory).ok()?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.path.with_extension("lock"))
            .ok()?;
        lock.lock_exclusive().ok()?;
        Some(lock)
    }

    fn save(&self, mut document: Document) {
        document.version = VERSION;
        // Forget projects that no longer exist (deleted checkouts, temp dirs).
        document
            .projects
            .retain(|project, _| Path::new(project).is_dir());
        // Drop the oldest projects beyond the cap.
        if document.projects.len() > MAX_PROJECTS {
            let mut by_recency: Vec<(String, u64)> = document
                .projects
                .iter()
                .map(|(project, entries)| {
                    (
                        project.clone(),
                        entries.first().map(|entry| entry.opened).unwrap_or(0),
                    )
                })
                .collect();
            by_recency.sort_by_key(|(_, opened)| std::cmp::Reverse(*opened));
            for (project, _) in by_recency.into_iter().skip(MAX_PROJECTS) {
                document.projects.remove(&project);
            }
        }
        let Ok(bytes) = serde_json::to_vec_pretty(&document) else {
            return;
        };
        // Rename keeps readers from seeing a partial file; a lost update after
        // a power cut is acceptable for this list, so skip the fsync.
        let temp = self.path.with_extension("tmp");
        let written = File::create(&temp)
            .and_then(|mut file| file.write_all(&bytes))
            .is_ok();
        if written {
            let _ = fs::rename(&temp, &self.path);
        }
    }

    /// Notes a visit to `path` (moving it to the front). `cursor` overrides the
    /// remembered position when given.
    pub fn record(&self, project: &Path, path: &Path, cursor: Option<(usize, usize)>) {
        if self.ignores(project) {
            return;
        }
        let (project, path) = (project.to_path_buf(), path.to_path_buf());
        self.update(move |store| store.write_record(&project, &path, cursor));
    }

    fn write_record(&self, project: &Path, path: &Path, cursor: Option<(usize, usize)>) {
        let Some(_lock) = self.lock() else { return };
        let mut document = self.load();
        let entries = document
            .projects
            .entry(project.to_string_lossy().to_string())
            .or_default();
        let path = path.to_string_lossy().to_string();
        let previous = entries
            .iter()
            .position(|entry| entry.path == path)
            .map(|index| entries.remove(index));
        let (line, col) = cursor
            .or_else(|| previous.as_ref().map(|entry| (entry.line, entry.col)))
            .unwrap_or((0, 0));
        entries.insert(
            0,
            RecentEntry {
                path,
                opened: now(),
                line,
                col,
            },
        );
        entries.truncate(MAX_PER_PROJECT);
        self.save(document);
    }

    /// Updates the remembered cursor of a file without changing its rank.
    pub fn update_cursor(&self, project: &Path, path: &Path, cursor: (usize, usize)) {
        if self.ignores(project) {
            return;
        }
        let (project, path) = (project.to_path_buf(), path.to_path_buf());
        self.update(move |store| store.write_cursor(&project, &path, cursor));
    }

    fn write_cursor(&self, project: &Path, path: &Path, cursor: (usize, usize)) {
        let Some(_lock) = self.lock() else { return };
        let mut document = self.load();
        let path = path.to_string_lossy();
        if let Some(entry) = document
            .projects
            .get_mut(project.to_string_lossy().as_ref())
            .and_then(|entries| entries.iter_mut().find(|entry| entry.path == path))
        {
            entry.line = cursor.0;
            entry.col = cursor.1;
            self.save(document);
        }
    }

    /// The project's files, most recent first, without files that no longer exist.
    pub fn list(&self, project: &Path) -> Vec<RecentEntry> {
        if let Some(writer) = &self.writer {
            writer.flush();
        }
        let document = self.load();
        document
            .projects
            .get(project.to_string_lossy().as_ref())
            .map(|entries| {
                entries
                    .iter()
                    .filter(|entry| Path::new(&entry.path).is_file())
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Under the system temp directory (compared after resolving symlinks).
fn is_temporary(path: &Path) -> bool {
    let temp = std::env::temp_dir();
    let temp = temp.canonicalize().unwrap_or(temp);
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .starts_with(temp)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_most_recent_first_per_project_and_survives_reload() {
        let directory = tempfile::tempdir().unwrap();
        let store = RecentFiles::new(directory.path().join("recent.json"));
        let a = directory.path().join("a.txt");
        let b = directory.path().join("b.txt");
        let c = directory.path().join("c.txt");
        for file in [&a, &b, &c] {
            fs::write(file, "x").unwrap();
        }
        let project = directory.path().join("proj");
        let other = directory.path().join("other");
        fs::create_dir(&project).unwrap();
        fs::create_dir(&other).unwrap();

        store.record(&project, &a, None);
        store.record(&project, &b, None);
        store.record(&other, &c, None);
        store.record(&project, &a, Some((3, 4)));

        let reopened = RecentFiles::new(directory.path().join("recent.json"));
        let list = reopened.list(&project);
        assert_eq!(
            list.iter().map(|e| e.path.clone()).collect::<Vec<_>>(),
            vec![
                a.to_string_lossy().to_string(),
                b.to_string_lossy().to_string()
            ]
        );
        assert_eq!((list[0].line, list[0].col), (3, 4));
        assert_eq!(reopened.list(&other).len(), 1);
    }

    #[test]
    fn deleted_files_are_not_listed_and_the_list_is_capped() {
        let directory = tempfile::tempdir().unwrap();
        let store = RecentFiles::new(directory.path().join("recent.json"));
        let project = directory.path().join("proj");
        fs::create_dir(&project).unwrap();
        let gone = directory.path().join("gone.txt");
        fs::write(&gone, "x").unwrap();
        store.record(&project, &gone, None);
        fs::remove_file(&gone).unwrap();
        assert!(store.list(&project).is_empty());

        for index in 0..(MAX_PER_PROJECT + 20) {
            let file = directory.path().join(format!("f{index}.txt"));
            fs::write(&file, "x").unwrap();
            store.record(&project, &file, None);
        }
        assert_eq!(store.list(&project).len(), MAX_PER_PROJECT);
    }

    #[test]
    fn update_cursor_keeps_the_rank() {
        let directory = tempfile::tempdir().unwrap();
        let store = RecentFiles::new(directory.path().join("recent.json"));
        let project = directory.path().join("proj");
        fs::create_dir(&project).unwrap();
        let a = directory.path().join("a.txt");
        let b = directory.path().join("b.txt");
        fs::write(&a, "x").unwrap();
        fs::write(&b, "x").unwrap();
        store.record(&project, &a, None);
        store.record(&project, &b, None);
        store.update_cursor(&project, &a, (7, 1));
        let list = store.list(&project);
        assert_eq!(list[0].path, b.to_string_lossy());
        assert_eq!((list[1].line, list[1].col), (7, 1));
    }

    #[test]
    fn background_updates_land_in_order_before_listing() {
        let directory = tempfile::tempdir().unwrap();
        let store = RecentFiles {
            writer: Writer::spawn().map(Arc::new),
            ..RecentFiles::new(directory.path().join("recent.json"))
        };
        let project = directory.path().join("proj");
        fs::create_dir(&project).unwrap();
        let a = directory.path().join("a.txt");
        let b = directory.path().join("b.txt");
        fs::write(&a, "x").unwrap();
        fs::write(&b, "x").unwrap();
        store.record(&project, &a, None);
        store.record(&project, &b, None);
        store.update_cursor(&project, &a, (7, 1));
        let list = store.list(&project);
        assert_eq!(list[0].path, b.to_string_lossy());
        assert_eq!((list[1].line, list[1].col), (7, 1));
    }

    #[test]
    fn the_users_store_ignores_projects_in_the_temp_directory() {
        let directory = tempfile::tempdir().unwrap();
        let store = RecentFiles {
            skip_temporary: true,
            ..RecentFiles::new(directory.path().join("recent.json"))
        };
        let file = directory.path().join("a.txt");
        fs::write(&file, "x").unwrap();
        store.record(directory.path(), &file, None);
        assert!(store.list(directory.path()).is_empty());
        assert!(!directory.path().join("recent.json").exists());
    }

    #[test]
    fn corrupt_files_are_ignored() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("recent.json");
        fs::write(&path, "not json").unwrap();
        let store = RecentFiles::new(path);
        assert!(store.list(directory.path()).is_empty());
        let file = directory.path().join("a.txt");
        fs::write(&file, "x").unwrap();
        store.record(directory.path(), &file, None);
        assert_eq!(store.list(directory.path()).len(), 1);
    }
}
