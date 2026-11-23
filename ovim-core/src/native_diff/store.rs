//! Worktree-scoped refinements shared by all frontends, independent of chat history.
//! Immutable comparison keys, atomic publication and a per-worktree lock keep
//! concurrent processes from publishing partial reviews or racing retention.
use super::{CustomReview, DiffPairing, ReviewPatch, ReviewSnapshot};
use anyhow::{ensure, Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

const VERSION: u32 = 1;
const MAX_RECORD_BYTES: u64 = 32 * 1024 * 1024;
const MAX_WORKTREE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_REVIEWS: usize = 20;

/// Same contract used by chat replay. Sections are derived from the frozen
/// canonical snapshot and pairings instead of persisting a second arrangement.
#[derive(Serialize, Deserialize)]
pub struct SavedReview {
    pub title: String,
    pub snapshot: ReviewSnapshot,
    pub pairings: Vec<DiffPairing>,
}
impl SavedReview {
    pub fn into_custom(self) -> Result<(String, CustomReview)> {
        ensure!(
            !self.title.trim().is_empty() && !self.title.contains(['\n', '\r']),
            "Invalid saved review title"
        );
        ensure!(
            self.pairings
                .iter()
                .filter_map(|pairing| pairing.label.as_deref())
                .all(|label| !label.contains(['\n', '\r'])),
            "Invalid saved pairing label"
        );
        ensure!(
            self.snapshot.patch.lines.iter().all(|line| line
                .file
                .is_none_or(|file| file < self.snapshot.patch.files.len())),
            "Saved patch line references a missing file"
        );
        let canonical = ReviewSnapshot::from_patch(self.snapshot.patch.clone())?;
        ensure!(
            canonical.blocks == self.snapshot.blocks,
            "Saved review block map is invalid"
        );
        for file in &self.snapshot.patch.files {
            ensure!(
                safe_path(&file.path) && file.old_path.as_deref().is_none_or(safe_path),
                "Invalid saved review source path"
            );
        }
        let mut source_files = std::collections::HashSet::new();
        for file in &self.snapshot.sources {
            ensure!(
                file.file < self.snapshot.patch.files.len(),
                "Invalid saved source file"
            );
            ensure!(
                source_files.insert(file.file),
                "Duplicate saved source file"
            );
            let patch_file = &self.snapshot.patch.files[file.file];
            for (source, expected_path) in [
                (
                    &file.old,
                    patch_file.old_path.as_deref().unwrap_or(&patch_file.path),
                ),
                (&file.new, patch_file.path.as_str()),
            ] {
                let Some(source) = source else { continue };
                ensure!(
                    safe_path(&source.path) && source.path == expected_path,
                    "Invalid saved source path"
                );
                let mut end = 0;
                for window in &source.windows {
                    ensure!(window.start_line > end, "Overlapping saved source windows");
                    end = window
                        .start_line
                        .checked_add(window.lines.len())
                        .context("Invalid saved source window")?
                        .saturating_sub(1);
                }
            }
        }
        let custom = self.snapshot.reassign(&self.pairings)?;
        Ok((self.title, custom))
    }
}

fn safe_path(path: &str) -> bool {
    !path.is_empty()
        && Path::new(path)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

pub fn same_patch(left: &ReviewPatch, right: &ReviewPatch) -> bool {
    left.root == right.root
        && left.base.spec == right.base.spec
        && left.comparison_base_oid == right.comparison_base_oid
        && left.text == right.text
        && left.lines == right.lines
        && left.files == right.files
        && left.truncated == right.truncated
}

/// No mtime, abbreviated Git object ID, or whitespace-normalized comparison is
/// sufficient. Old/partial captures without a full byte fingerprint fail closed.
pub fn same_comparison(saved: &ReviewSnapshot, live: &ReviewSnapshot) -> bool {
    !live.patch.truncated
        && same_patch(&saved.patch, &live.patch)
        && saved.content_fingerprint.is_some()
        && saved.content_fingerprint == live.content_fingerprint
        && saved.sources == live.sources
}

fn comparison_key(snapshot: &ReviewSnapshot) -> Result<String> {
    let patch = &snapshot.patch;
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(
            &patch.base.spec,
            &patch.comparison_base_oid,
            &patch.text,
            &patch.lines,
            &patch.files,
            patch.truncated,
            &snapshot.content_fingerprint,
        ))?)
    ))
}

#[derive(Serialize, Deserialize)]
struct Document {
    version: u32,
    review: SavedReview,
}

#[derive(Clone, Debug)]
pub struct ReviewStore {
    root: PathBuf,
}
impl ReviewStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
    pub fn discover() -> Result<Self> {
        let root = std::env::var_os("OVIM_DIFF_REVIEWS_DIR")
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .or_else(|| dirs::data_local_dir().map(|root| root.join("ovim/diff-reviews")))
            .context("No local data directory for saved diff reviews")?;
        Ok(Self::new(root))
    }
    fn worktree_directory(&self, root: &Path) -> Result<PathBuf> {
        let root = fs::canonicalize(root).context("Resolve diff worktree")?;
        Ok(self.root.join(format!(
            "{:x}",
            Sha256::digest(root.as_os_str().as_encoded_bytes())
        )))
    }
    fn lock(&self, directory: &Path) -> Result<File> {
        private_directory(&self.root)?;
        private_directory(directory)?;
        let mut options = OpenOptions::new();
        options.create(true).read(true).write(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let lock = options.open(directory.join(".lock"))?;
        lock.lock_exclusive()?;
        Ok(lock)
    }

    pub fn save(&self, title: &str, custom: &CustomReview) -> Result<()> {
        custom.validate_coverage()?;
        ensure!(
            custom.snapshot.reassign(&custom.pairings)? == *custom,
            "Review sections do not match their saved pairing contract"
        );
        let review = SavedReview {
            title: title.to_string(),
            snapshot: custom.snapshot.clone(),
            pairings: custom.pairings.clone(),
        };
        let key = comparison_key(&review.snapshot)?;
        let directory = self.worktree_directory(&review.snapshot.patch.root)?;
        let bytes = serde_json::to_vec(&Document {
            version: VERSION,
            review,
        })?;
        ensure!(
            bytes.len() as u64 <= MAX_RECORD_BYTES,
            "Saved diff review exceeds the 32 MiB storage limit"
        );
        let _lock = self.lock(&directory)?;
        atomic_write(&directory.join(format!("{key}.json")), &bytes)?;
        atomic_write(&directory.join("latest"), key.as_bytes())?;
        let mut records = fs::read_dir(&directory)?
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let path = entry.path();
                let stem = path.file_stem()?.to_str()?;
                if path.extension()? != "json" || !valid_key(stem) {
                    return None;
                }
                let metadata = entry.metadata().ok()?;
                Some((path, metadata.modified().ok()?, metadata.len()))
            })
            .collect::<Vec<_>>();
        records.sort_by_key(|record| std::cmp::Reverse(record.1));
        let current = directory.join(format!("{key}.json"));
        let mut retained = 1;
        let mut size = bytes.len() as u64;
        for (path, _, bytes) in records {
            if path == current {
                continue;
            }
            if retained < MAX_REVIEWS && size + bytes <= MAX_WORKTREE_BYTES {
                retained += 1;
                size += bytes;
            } else {
                fs::remove_file(path)?;
            }
        }
        Ok(())
    }

    /// Prefer this exact comparison, otherwise expose the latest frozen review
    /// as stale. Loading never changes which review is considered latest.
    pub fn load(&self, live: &ReviewSnapshot) -> Result<Option<(String, CustomReview)>> {
        let directory = self.worktree_directory(&live.patch.root)?;
        if !directory.exists() {
            return Ok(None);
        }
        let _lock = self.lock(&directory)?;
        let exact = directory.join(format!("{}.json", comparison_key(live)?));
        let path = if exact.exists() {
            exact
        } else {
            let latest = match read_bounded(&directory.join("latest"), 64) {
                Ok(bytes) => String::from_utf8(bytes)?,
                Err(error)
                    if error
                        .downcast_ref::<std::io::Error>()
                        .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
                {
                    return Ok(None)
                }
                Err(error) => return Err(error),
            };
            ensure!(valid_key(&latest), "Invalid saved diff reference");
            directory.join(format!("{latest}.json"))
        };
        let document: Document = serde_json::from_slice(&read_bounded(&path, MAX_RECORD_BYTES)?)?;
        ensure!(
            document.version == VERSION,
            "Unsupported saved diff version {}",
            document.version
        );
        ensure!(
            document.review.snapshot.patch.root == live.patch.root,
            "Saved diff belongs to another worktree"
        );
        ensure!(
            path.file_stem().and_then(|stem| stem.to_str())
                == Some(comparison_key(&document.review.snapshot)?.as_str()),
            "Saved diff identity is invalid"
        );
        Ok(Some(document.review.into_custom()?))
    }
}
fn valid_key(key: &str) -> bool {
    key.len() == 64 && key.bytes().all(|byte| byte.is_ascii_hexdigit())
}
fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= limit,
        "Saved diff exceeds its storage limit"
    );
    Ok(bytes)
}
fn private_directory(path: &Path) -> Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("Saved diff path has no parent")?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path).context("Publish saved diff")?;
    #[cfg(unix)]
    File::open(parent)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_diff::{review_snapshot, ReviewBase};
    use git2::{Repository, Signature};

    fn fixture() -> (tempfile::TempDir, Repository, ReviewStore) {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("worktree");
        fs::create_dir(&root).unwrap();
        let repo = Repository::init(&root).unwrap();
        repo.set_head("refs/heads/main").unwrap();
        fs::write(root.join("a.rs"), "fn before() {}\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("a.rs")).unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let signature = Signature::now("Ovim", "ovim@example.com").unwrap();
        repo.commit(Some("HEAD"), &signature, &signature, "base", &tree, &[])
            .unwrap();
        drop(tree);
        let store = ReviewStore::new(directory.path().join("saved"));
        (directory, repo, store)
    }
    fn snapshot(repo: &Repository, version: usize) -> ReviewSnapshot {
        fs::write(
            repo.workdir().unwrap().join("a.rs"),
            format!("fn after_{version}() {{}}\n"),
        )
        .unwrap();
        review_snapshot(repo.workdir().unwrap(), &ReviewBase::explicit("HEAD")).unwrap()
    }

    #[test]
    fn independent_writers_publish_complete_records_without_losing_other_comparisons() {
        let (_directory, repo, store) = fixture();
        let snapshots: Vec<_> = (0..4).map(|i| snapshot(&repo, i)).collect();
        let writers: Vec<_> = snapshots
            .iter()
            .cloned()
            .enumerate()
            .map(|(index, snapshot)| {
                let store = store.clone();
                std::thread::spawn(move || {
                    store
                        .save(&format!("Review {index}"), &snapshot.reassign(&[]).unwrap())
                        .unwrap()
                })
            })
            .collect();
        for writer in writers {
            writer.join().unwrap();
        }
        for (index, snapshot) in snapshots.iter().enumerate() {
            let (title, custom) = store.load(snapshot).unwrap().unwrap();
            assert_eq!(title, format!("Review {index}"));
            assert!(same_comparison(&custom.snapshot, snapshot));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let directory = store.worktree_directory(repo.workdir().unwrap()).unwrap();
            assert_eq!(
                fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
                0o700
            );
            let key = comparison_key(&snapshots[0]).unwrap();
            assert_eq!(
                fs::metadata(directory.join(format!("{key}.json")))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn corrupt_unknown_version_and_tampered_block_maps_fail_closed() {
        let (_directory, repo, store) = fixture();
        let snapshot = snapshot(&repo, 1);
        let custom = snapshot.reassign(&[]).unwrap();
        store.save("Refinement", &custom).unwrap();
        let directory = store.worktree_directory(repo.workdir().unwrap()).unwrap();
        let path = directory.join(format!("{}.json", comparison_key(&snapshot).unwrap()));
        let valid = fs::read(&path).unwrap();
        fs::write(&path, b"{partial").unwrap();
        assert!(store.load(&snapshot).is_err());
        let mut document: serde_json::Value = serde_json::from_slice(&valid).unwrap();
        document["version"] = 99.into();
        fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
        assert!(store.load(&snapshot).is_err());
        document["version"] = VERSION.into();
        document["review"]["snapshot"]["blocks"][0]["patchLineStart"] = u64::MAX.into();
        fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
        assert!(store.load(&snapshot).is_err());
        fs::remove_file(&path).unwrap();
        fs::write(directory.join("latest"), "../../outside").unwrap();
        assert!(store.load(&snapshot).is_err());
    }

    #[test]
    fn saved_payload_validates_metadata_and_frozen_source_ownership() {
        let (_directory, repo, _) = fixture();
        let original = snapshot(&repo, 1);
        let saved = |snapshot| SavedReview {
            title: "Review".into(),
            snapshot,
            pairings: vec![],
        };
        assert!(saved(original.clone()).into_custom().is_ok());
        let mut invalid = original.clone();
        invalid.patch.lines[0].file = Some(usize::MAX);
        assert!(saved(invalid).into_custom().is_err());
        let mut invalid = original.clone();
        invalid.sources[0].new.as_mut().unwrap().path = "unrelated.rs".into();
        assert!(saved(invalid).into_custom().is_err());
        let mut invalid = original.clone();
        invalid.sources.push(invalid.sources[0].clone());
        assert!(saved(invalid).into_custom().is_err());
        let mut invalid = original;
        invalid.sources[0].new.as_mut().unwrap().windows[0].start_line = usize::MAX;
        assert!(saved(invalid).into_custom().is_err());
    }

    #[test]
    fn retention_is_bounded_and_preserves_the_newest_review() {
        let (_directory, repo, store) = fixture();
        let mut latest = None;
        for version in 0..MAX_REVIEWS + 3 {
            let snapshot = snapshot(&repo, version);
            store
                .save(
                    &format!("Review {version}"),
                    &snapshot.reassign(&[]).unwrap(),
                )
                .unwrap();
            latest = Some(snapshot);
        }
        let directory = store.worktree_directory(repo.workdir().unwrap()).unwrap();
        let count = fs::read_dir(directory)
            .unwrap()
            .flatten()
            .filter(|entry| {
                entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "json")
            })
            .count();
        assert_eq!(count, MAX_REVIEWS);
        let (title, _) = store.load(&latest.unwrap()).unwrap().unwrap();
        assert_eq!(title, format!("Review {}", MAX_REVIEWS + 2));
    }
    #[cfg(unix)]
    #[test]
    fn path_aliases_share_reviews_but_linked_worktrees_do_not() {
        let (directory, repo, store) = fixture();
        let snapshot = snapshot(&repo, 1);
        store
            .save("Primary worktree", &snapshot.reassign(&[]).unwrap())
            .unwrap();
        let alias = directory.path().join("alias");
        std::os::unix::fs::symlink(repo.workdir().unwrap(), &alias).unwrap();
        let alias_snapshot = review_snapshot(&alias, &ReviewBase::explicit("HEAD")).unwrap();
        let (_, saved) = store.load(&alias_snapshot).unwrap().unwrap();
        assert!(same_comparison(&saved.snapshot, &alias_snapshot));
        let secondary = directory.path().join("secondary");
        repo.worktree("secondary", &secondary, None).unwrap();
        fs::write(secondary.join("a.rs"), "fn after_1() {}\n").unwrap();
        let other = review_snapshot(&secondary, &ReviewBase::explicit("HEAD")).unwrap();
        assert!(store.load(&other).unwrap().is_none());
    }
}
