//! Git and filesystem adapter for base-manifest capture.
//!
//! Enumeration is eager, but workspace and index bytes remain behind opaque
//! locators. This preserves the policy-before-read boundary in `manifest`.

use super::{
    BaseManifestId, ContentRequest, EditorOverlayInput, FileKind, FileSnapshotInput, LayerInput,
    RepoPath, RepositoryBase, RepositoryId, RepositorySnapshotInput, SnapshotContentReader,
    UnsavedBufferInput,
};
use git2::{ErrorCode, Oid, Repository, Status, StatusOptions};
use std::collections::HashMap;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitManifestMetadata {
    pub repository_id: RepositoryId,
    pub base_manifest_id: BaseManifestId,
    pub captured_at: String,
}

/// A Git-derived manifest input plus the reader for its opaque locators.
/// Callers may attach editor overlays and unsaved buffers to `input` before
/// passing both values to `capture_base_manifest`.
#[derive(Debug)]
pub struct GitManifestSnapshot {
    pub input: RepositorySnapshotInput,
    pub reader: GitSnapshotContentReader,
    pub issues: Vec<GitManifestAdapterIssue>,
}

impl GitManifestSnapshot {
    pub fn set_editor_overlay(&mut self, path: &str, overlay: EditorOverlayInput) -> bool {
        let Some(file) = self.input.files.iter_mut().find(|file| file.path == path) else {
            return false;
        };
        file.editor = Some(overlay);
        true
    }

    pub fn add_unsaved_buffer(&mut self, buffer: UnsavedBufferInput) {
        self.input.unsaved_buffers.push(buffer);
    }

    /// Attaches an editor buffer, adding an otherwise-omitted clean Git path
    /// without reading its disk or Git content.
    pub fn attach_editor_overlay(
        &mut self,
        path: &str,
        overlay: EditorOverlayInput,
    ) -> Result<(), String> {
        if self.set_editor_overlay(path, overlay.clone()) {
            return Ok(());
        }
        let path = RepoPath::parse(path).map_err(|error| error.to_string())?;
        if is_git_internal(&path) {
            return Err("refusing to attach Git administrative data".into());
        }
        let repository =
            Repository::open(&self.reader.git_dir).map_err(|error| error.to_string())?;
        let relative = Path::new(path.as_str());
        let head_entry = repository
            .head()
            .ok()
            .and_then(|head| head.peel_to_tree().ok())
            .and_then(|tree| {
                tree.get_path(relative)
                    .ok()
                    .map(|entry| (entry.id(), entry.filemode() as u32))
            });
        let index_entry = repository
            .index()
            .ok()
            .and_then(|index| index.get_path(relative, 0));
        if head_entry.is_none()
            && index_entry.is_none()
            && fs::symlink_metadata(self.reader.workdir.join(path.as_str())).is_err()
        {
            return Err("editor path was absent from Git, index, and workspace".into());
        }
        let fallback_mode = index_entry
            .as_ref()
            .map(|entry| entry.mode)
            .or_else(|| head_entry.as_ref().map(|(_, mode)| *mode));
        let (file_kind, executable) = inspect_file_mode(&self.reader.workdir, &path, fallback_mode);
        self.input.files.push(FileSnapshotInput {
            path: path.to_string(),
            file_kind,
            executable,
            git_blob: head_entry.map(|(oid, _)| oid.to_string()),
            index: LayerInput::Inherit,
            disk: LayerInput::Inherit,
            editor: Some(overlay),
        });
        self.input
            .files
            .sort_by(|left, right| left.path.cmp(&right.path));
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitManifestAdapterIssue {
    pub path: Option<String>,
    pub detail: String,
}

#[derive(Debug)]
pub enum GitManifestAdapterError {
    NotRepository(PathBuf),
    BareRepository(PathBuf),
    Git(git2::Error),
    Io(io::Error),
}

impl fmt::Display for GitManifestAdapterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotRepository(path) => {
                write!(
                    formatter,
                    "{} is not inside a Git repository",
                    path.display()
                )
            }
            Self::BareRepository(path) => write!(
                formatter,
                "{} is a bare repository without a workspace",
                path.display()
            ),
            Self::Git(error) => write!(formatter, "Git manifest inspection failed: {error}"),
            Self::Io(error) => write!(formatter, "workspace manifest inspection failed: {error}"),
        }
    }
}

impl std::error::Error for GitManifestAdapterError {}

impl From<git2::Error> for GitManifestAdapterError {
    fn from(error: git2::Error) -> Self {
        Self::Git(error)
    }
}

impl From<io::Error> for GitManifestAdapterError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Debug)]
pub struct GitSnapshotContentReader {
    workdir: PathBuf,
    git_dir: PathBuf,
    sources: HashMap<String, GitContentSource>,
}

impl GitSnapshotContentReader {
    pub fn workdir(&self) -> &Path {
        &self.workdir
    }
}

#[derive(Debug)]
enum GitContentSource {
    Disk(RepoPath),
    Object(Oid),
}

impl SnapshotContentReader for GitSnapshotContentReader {
    fn read(&self, request: &ContentRequest, max_bytes: u64) -> Result<Vec<u8>, String> {
        let source = self
            .sources
            .get(&request.locator)
            .ok_or_else(|| "unknown Git manifest content locator".to_string())?;
        match source {
            GitContentSource::Disk(path) => read_disk(&self.workdir, path, max_bytes),
            GitContentSource::Object(oid) => {
                let repository =
                    Repository::open(&self.git_dir).map_err(|error| error.to_string())?;
                let blob = repository
                    .find_blob(*oid)
                    .map_err(|error| error.to_string())?;
                bounded_copy(blob.content(), max_bytes)
            }
        }
    }
}

/// Discovers the containing worktree and captures all nonignored status
/// overlays. Clean paths are intentionally omitted: HEAD plus the index tree
/// reconstruct them without duplicating blobs.
pub fn discover_git_manifest(
    start: impl AsRef<Path>,
    metadata: GitManifestMetadata,
) -> Result<GitManifestSnapshot, GitManifestAdapterError> {
    let start = start.as_ref();
    let repository = Repository::discover(start).map_err(|error| {
        if error.code() == ErrorCode::NotFound {
            GitManifestAdapterError::NotRepository(start.to_owned())
        } else {
            GitManifestAdapterError::Git(error)
        }
    })?;
    let workdir = repository
        .workdir()
        .ok_or_else(|| GitManifestAdapterError::BareRepository(repository.path().to_owned()))?;
    let workdir = fs::canonicalize(workdir)?;
    let git_dir = fs::canonicalize(repository.path()).unwrap_or_else(|_| repository.path().into());

    let (head_commit, head_tree) = match repository.head().and_then(|head| head.peel_to_commit()) {
        Ok(commit) => {
            let oid = commit.id().to_string();
            (Some(oid), Some(commit.tree()?))
        }
        Err(error) if matches!(error.code(), ErrorCode::UnbornBranch | ErrorCode::NotFound) => {
            (None, None)
        }
        Err(error) => return Err(error.into()),
    };

    let index = repository.index()?;
    // Computing an index tree through libgit2 writes missing tree objects.
    // Base capture is observational, so the MVP reconstructs staged state
    // from per-path index overlays instead.
    let index_tree = None;
    let mut options = StatusOptions::new();
    options
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_ignored(false)
        .exclude_submodules(true)
        .renames_head_to_index(true)
        .renames_index_to_workdir(true);
    let statuses = repository.statuses(Some(&mut options))?;

    let mut sources = HashMap::new();
    let mut files = Vec::new();
    let mut issues = Vec::new();

    for entry in statuses.iter() {
        let status = entry.status();
        let Some(path_text) = entry.path() else {
            issues.push(GitManifestAdapterIssue {
                path: None,
                detail: "Git status path was not valid UTF-8".into(),
            });
            continue;
        };
        let path = match RepoPath::parse(path_text) {
            Ok(path) if !is_git_internal(&path) => path,
            Ok(_) => continue,
            Err(error) => {
                issues.push(GitManifestAdapterIssue {
                    path: Some(path_text.into()),
                    detail: error.to_string(),
                });
                continue;
            }
        };
        if status.contains(Status::CONFLICTED) {
            issues.push(GitManifestAdapterIssue {
                path: Some(path.to_string()),
                detail: "index conflict stages are not captured by the MVP adapter".into(),
            });
            continue;
        }
        if status.intersects(Status::INDEX_RENAMED | Status::WT_RENAMED) {
            issues.push(GitManifestAdapterIssue {
                path: Some(path.to_string()),
                detail:
                    "rename old-path provenance is not captured; treat this manifest as partial"
                        .into(),
            });
        }

        let path_buf = Path::new(path.as_str());
        let head_entry = head_tree
            .as_ref()
            .and_then(|tree| tree.get_path(path_buf).ok());
        let git_blob = head_entry.as_ref().map(|entry| entry.id().to_string());
        let index_entry = index.get_path(path_buf, 0);

        let index_layer = if status.contains(Status::INDEX_DELETED) {
            LayerInput::Deleted
        } else if has_index_change(status) {
            match index_entry.as_ref() {
                Some(entry) if !is_gitlink(entry.mode) => {
                    let oid = entry.id;
                    // Index metadata supplies the bound without loading blob
                    // content before capture policy has run.
                    let declared_bytes = Some(entry.file_size as u64);
                    LayerInput::Content(register_source(
                        &mut sources,
                        GitContentSource::Object(oid),
                        declared_bytes,
                    ))
                }
                Some(_) => {
                    issues.push(GitManifestAdapterIssue {
                        path: Some(path.to_string()),
                        detail: "submodule index entries are not captured by the MVP adapter"
                            .into(),
                    });
                    LayerInput::Inherit
                }
                None => {
                    issues.push(GitManifestAdapterIssue {
                        path: Some(path.to_string()),
                        detail: "changed path was absent from index".into(),
                    });
                    LayerInput::Inherit
                }
            }
        } else {
            LayerInput::Inherit
        };

        let disk_layer = if status.contains(Status::WT_DELETED) {
            LayerInput::Deleted
        } else if has_worktree_change(status) {
            let declared_bytes = fs::symlink_metadata(workdir.join(path.as_str()))
                .ok()
                .map(|metadata| metadata.len());
            LayerInput::Content(register_source(
                &mut sources,
                GitContentSource::Disk(path.clone()),
                declared_bytes,
            ))
        } else {
            LayerInput::Inherit
        };

        let fallback_mode = index_entry
            .as_ref()
            .map(|entry| entry.mode)
            .or_else(|| head_entry.as_ref().map(|entry| entry.filemode() as u32));
        let (file_kind, executable) = inspect_file_mode(&workdir, &path, fallback_mode);

        files.push(FileSnapshotInput {
            path: path.to_string(),
            file_kind,
            executable,
            git_blob,
            index: index_layer,
            disk: disk_layer,
            editor: None,
        });
    }

    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(GitManifestSnapshot {
        input: RepositorySnapshotInput {
            base_manifest_id: metadata.base_manifest_id,
            captured_at: metadata.captured_at,
            base: RepositoryBase {
                repository_id: metadata.repository_id,
                head_commit,
                index_tree,
            },
            files,
            unsaved_buffers: Vec::new(),
        },
        reader: GitSnapshotContentReader {
            workdir,
            git_dir,
            sources,
        },
        issues,
    })
}

fn register_source(
    sources: &mut HashMap<String, GitContentSource>,
    source: GitContentSource,
    declared_bytes: Option<u64>,
) -> ContentRequest {
    let locator = format!("git-snapshot:{}", sources.len());
    sources.insert(locator.clone(), source);
    ContentRequest {
        locator,
        declared_bytes,
    }
}

fn has_index_change(status: Status) -> bool {
    status.intersects(
        Status::INDEX_NEW
            | Status::INDEX_MODIFIED
            | Status::INDEX_RENAMED
            | Status::INDEX_TYPECHANGE,
    )
}

fn has_worktree_change(status: Status) -> bool {
    status.intersects(
        Status::WT_NEW | Status::WT_MODIFIED | Status::WT_RENAMED | Status::WT_TYPECHANGE,
    )
}

fn is_git_internal(path: &RepoPath) -> bool {
    path.as_str().split('/').next() == Some(".git")
}

fn is_gitlink(mode: u32) -> bool {
    mode & 0o170000 == 0o160000
}

fn inspect_file_mode(
    workdir: &Path,
    path: &RepoPath,
    fallback_mode: Option<u32>,
) -> (FileKind, bool) {
    if let Ok(metadata) = fs::symlink_metadata(workdir.join(path.as_str())) {
        let kind = if metadata.file_type().is_symlink() {
            FileKind::Symlink
        } else if metadata.is_dir() {
            FileKind::Directory
        } else {
            FileKind::Regular
        };
        #[cfg(unix)]
        let executable = {
            use std::os::unix::fs::PermissionsExt;
            metadata.permissions().mode() & 0o111 != 0
        };
        #[cfg(not(unix))]
        let executable = fallback_mode.is_some_and(|mode| mode & 0o111 != 0);
        return (kind, executable);
    }
    let mode = fallback_mode.unwrap_or(0o100644);
    let kind = match mode & 0o170000 {
        0o120000 => FileKind::Symlink,
        0o040000 | 0o160000 => FileKind::Directory,
        _ => FileKind::Regular,
    };
    (kind, mode & 0o111 != 0)
}

fn read_disk(root: &Path, path: &RepoPath, max_bytes: u64) -> Result<Vec<u8>, String> {
    validate_disk_path(root, path)?;
    let full_path = root.join(path.as_str());
    let metadata = fs::symlink_metadata(&full_path).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink() {
        let target = fs::read_link(&full_path).map_err(|error| error.to_string())?;
        return bounded_copy(&os_path_bytes(&target), max_bytes);
    }
    if !metadata.is_file() {
        return Err("workspace content is not a regular file or symlink".into());
    }
    if metadata.len() > max_bytes {
        return Err(format!(
            "workspace file exceeds {max_bytes} byte read limit"
        ));
    }
    let file = open_regular_without_following(&full_path).map_err(|error| error.to_string())?;
    bounded_reader(file, max_bytes)
}

fn validate_disk_path(root: &Path, path: &RepoPath) -> Result<(), String> {
    if is_git_internal(path) {
        return Err("refusing to read Git administrative data".into());
    }
    let relative = Path::new(path.as_str());
    if relative
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err("workspace path is not lexically contained".into());
    }
    let mut parent = root.to_owned();
    if let Some(components) = relative.parent() {
        for component in components.components() {
            let Component::Normal(component) = component else {
                return Err("workspace path is not lexically contained".into());
            };
            parent.push(component);
            let metadata = fs::symlink_metadata(&parent).map_err(|error| error.to_string())?;
            if metadata.file_type().is_symlink() {
                return Err("refusing to traverse a symlinked workspace directory".into());
            }
        }
    }
    Ok(())
}

fn bounded_reader(mut reader: impl Read, max_bytes: u64) -> Result<Vec<u8>, String> {
    let limit = max_bytes.saturating_add(1);
    let mut bytes = Vec::new();
    reader
        .by_ref()
        .take(limit)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > max_bytes {
        Err(format!("content exceeds {max_bytes} byte read limit"))
    } else {
        Ok(bytes)
    }
}

#[cfg(unix)]
fn open_regular_without_following(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
}

#[cfg(not(unix))]
fn open_regular_without_following(path: &Path) -> io::Result<File> {
    File::open(path)
}

fn bounded_copy(bytes: &[u8], max_bytes: u64) -> Result<Vec<u8>, String> {
    if bytes.len() as u64 > max_bytes {
        Err(format!("content exceeds {max_bytes} byte read limit"))
    } else {
        Ok(bytes.to_vec())
    }
}

#[cfg(unix)]
fn os_path_bytes(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes().to_vec()
}

#[cfg(not(unix))]
fn os_path_bytes(path: &Path) -> Vec<u8> {
    path.to_string_lossy().as_bytes().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use git2::Signature;
    use std::collections::HashSet;

    fn metadata() -> GitManifestMetadata {
        GitManifestMetadata {
            repository_id: RepositoryId::parse("repo_git_adapter").unwrap(),
            base_manifest_id: BaseManifestId::parse("bsm_git_adapter").unwrap(),
            captured_at: "2026-07-13T14:00:00Z".into(),
        }
    }

    fn commit_all(repository: &Repository) {
        let mut index = repository.index().unwrap();
        index
            .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repository.find_tree(tree_id).unwrap();
        let signature = Signature::now("ovim", "ovim@example.invalid").unwrap();
        repository
            .commit(Some("HEAD"), &signature, &signature, "base", &tree, &[])
            .unwrap();
    }

    fn file<'a>(snapshot: &'a GitManifestSnapshot, path: &str) -> &'a FileSnapshotInput {
        snapshot
            .input
            .files
            .iter()
            .find(|file| file.path == path)
            .unwrap()
    }

    fn object_ids(repository: &Repository) -> HashSet<Oid> {
        let mut ids = HashSet::new();
        repository
            .odb()
            .unwrap()
            .foreach(|oid| {
                ids.insert(*oid);
                true
            })
            .unwrap();
        ids
    }

    fn read_layer(snapshot: &GitManifestSnapshot, layer: &LayerInput) -> Vec<u8> {
        let LayerInput::Content(request) = layer else {
            panic!("expected content layer")
        };
        snapshot.reader.read(request, 1024).unwrap()
    }

    #[test]
    fn captures_dirty_staged_untracked_and_deleted_but_not_ignored() {
        let directory = tempfile::tempdir().unwrap();
        let repository = Repository::init(directory.path()).unwrap();
        fs::write(directory.path().join("dirty"), b"base dirty").unwrap();
        fs::write(directory.path().join("staged"), b"base staged").unwrap();
        fs::write(directory.path().join("deleted"), b"base deleted").unwrap();
        fs::write(directory.path().join(".gitignore"), b"ignored\n").unwrap();
        commit_all(&repository);

        fs::write(directory.path().join("dirty"), b"worktree dirty").unwrap();
        fs::write(directory.path().join("staged"), b"index staged").unwrap();
        let mut index = repository.index().unwrap();
        index.add_path(Path::new("staged")).unwrap();
        index.write().unwrap();
        fs::write(directory.path().join("untracked"), b"new bytes").unwrap();
        fs::write(directory.path().join("ignored"), b"secret").unwrap();
        fs::remove_file(directory.path().join("deleted")).unwrap();

        let objects_before_capture = object_ids(&repository);
        let snapshot = discover_git_manifest(directory.path(), metadata()).unwrap();
        assert_eq!(
            read_layer(&snapshot, &file(&snapshot, "dirty").disk),
            b"worktree dirty"
        );
        assert_eq!(
            read_layer(&snapshot, &file(&snapshot, "staged").index),
            b"index staged"
        );
        assert_eq!(file(&snapshot, "staged").disk, LayerInput::Inherit);
        assert_eq!(
            read_layer(&snapshot, &file(&snapshot, "untracked").disk),
            b"new bytes"
        );
        assert_eq!(file(&snapshot, "deleted").disk, LayerInput::Deleted);
        assert!(snapshot
            .input
            .files
            .iter()
            .all(|file| file.path != "ignored"));
        assert!(snapshot.input.base.head_commit.is_some());
        assert_eq!(snapshot.input.base.index_tree, None);
        assert_eq!(object_ids(&repository), objects_before_capture);
    }

    #[cfg(unix)]
    #[test]
    fn reads_symlink_target_bytes_without_following_outside_target() {
        use std::os::unix::fs::symlink;
        let directory = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret"), b"must not be read").unwrap();
        let repository = Repository::init(directory.path()).unwrap();
        fs::write(directory.path().join("base"), b"base").unwrap();
        commit_all(&repository);
        let target = outside.path().join("secret");
        symlink(&target, directory.path().join("link")).unwrap();

        let snapshot = discover_git_manifest(directory.path(), metadata()).unwrap();
        let link = file(&snapshot, "link");
        assert_eq!(link.file_kind, FileKind::Symlink);
        assert_eq!(read_layer(&snapshot, &link.disk), os_path_bytes(&target));
        assert_ne!(read_layer(&snapshot, &link.disk), b"must not be read");
    }

    #[cfg(unix)]
    #[test]
    fn records_executable_mode() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let repository = Repository::init(directory.path()).unwrap();
        let script = directory.path().join("script");
        fs::write(&script, b"#!/bin/sh\n").unwrap();
        let mut permissions = fs::metadata(&script).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script, permissions).unwrap();
        commit_all(&repository);
        fs::write(&script, b"#!/bin/sh\necho ok\n").unwrap();

        let snapshot = discover_git_manifest(directory.path(), metadata()).unwrap();
        assert!(file(&snapshot, "script").executable);
    }

    #[test]
    fn reader_enforces_bounds_and_rejects_unknown_locators() {
        let directory = tempfile::tempdir().unwrap();
        let repository = Repository::init(directory.path()).unwrap();
        fs::write(directory.path().join("base"), b"base").unwrap();
        commit_all(&repository);
        fs::write(directory.path().join("new"), b"12345").unwrap();
        let snapshot = discover_git_manifest(directory.path(), metadata()).unwrap();
        let LayerInput::Content(request) = &file(&snapshot, "new").disk else {
            panic!()
        };
        assert!(snapshot.reader.read(request, 4).is_err());
        assert!(snapshot
            .reader
            .read(&ContentRequest::new("git-snapshot:missing"), 100)
            .is_err());
    }

    #[test]
    fn clean_omitted_path_can_receive_editor_overlay_later() {
        let directory = tempfile::tempdir().unwrap();
        let repository = Repository::init(directory.path()).unwrap();
        fs::write(directory.path().join("clean"), b"clean").unwrap();
        commit_all(&repository);
        let mut snapshot = discover_git_manifest(directory.path(), metadata()).unwrap();
        assert!(snapshot.input.files.is_empty());

        snapshot
            .attach_editor_overlay(
                "clean",
                EditorOverlayInput {
                    content: ContentRequest::new("editor-owned:1"),
                    version: Some(3),
                    modified: false,
                    encoding: Some("utf-8".into()),
                    line_endings: Some("lf".into()),
                },
            )
            .unwrap();
        let clean = file(&snapshot, "clean");
        assert!(clean.git_blob.is_some());
        assert_eq!(clean.disk, LayerInput::Inherit);
        assert!(!clean.editor.as_ref().unwrap().modified);
    }

    #[test]
    fn detected_rename_is_explicitly_partial() {
        let directory = tempfile::tempdir().unwrap();
        let repository = Repository::init(directory.path()).unwrap();
        fs::write(directory.path().join("old"), b"same bytes").unwrap();
        commit_all(&repository);
        fs::rename(directory.path().join("old"), directory.path().join("new")).unwrap();
        let mut index = repository.index().unwrap();
        index.remove_path(Path::new("old")).unwrap();
        index.add_path(Path::new("new")).unwrap();
        index.write().unwrap();

        let snapshot = discover_git_manifest(directory.path(), metadata()).unwrap();
        assert!(snapshot
            .issues
            .iter()
            .any(|issue| issue.detail.contains("rename old-path provenance")));
    }
}
