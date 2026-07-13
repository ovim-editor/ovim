//! Replay-oriented workspace base manifests.
//!
//! A manifest records only overlays that cannot be reconstructed from Git.
//! Content access is injected so capture policy can reject a candidate before
//! potentially sensitive bytes are read.

use super::{
    ArtifactExportPolicy, ArtifactId, ArtifactRecord, ArtifactRetention, ArtifactSource,
    ArtifactState, ArtifactStore, BaseManifestId, ContentRepresentation, FileKind, RepositoryId,
    WorkspaceSurface,
};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RepoPath(String);

impl RepoPath {
    pub fn parse(value: impl Into<String>) -> Result<Self, InvalidRepoPath> {
        let value = value.into();
        let invalid = value.is_empty()
            || value.starts_with('/')
            || value.contains('\0')
            || value.contains('\\')
            || value
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
            || value.as_bytes().get(1) == Some(&b':');
        if invalid {
            Err(InvalidRepoPath(value))
        } else {
            Ok(Self(value))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RepoPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidRepoPath(String);

impl fmt::Display for InvalidRepoPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "path is not normalized repository-relative UTF-8: {:?}",
            self.0
        )
    }
}

impl std::error::Error for InvalidRepoPath {}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryBase {
    pub repository_id: RepositoryId,
    pub head_commit: Option<String>,
    pub index_tree: Option<String>,
}

/// A content locator is opaque to the manifest layer. Adapters may use a path,
/// buffer handle, Git object ID, or an entry in a synthetic test map.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentRequest {
    pub locator: String,
    pub declared_bytes: Option<u64>,
}

impl ContentRequest {
    pub fn new(locator: impl Into<String>) -> Self {
        Self {
            locator: locator.into(),
            declared_bytes: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LayerInput {
    Inherit,
    Content(ContentRequest),
    Deleted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorOverlayInput {
    pub content: ContentRequest,
    pub version: Option<u64>,
    pub modified: bool,
    pub encoding: Option<String>,
    pub line_endings: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileSnapshotInput {
    pub path: String,
    pub file_kind: FileKind,
    pub executable: bool,
    /// Git blob at HEAD, or `None` when the path is absent at the Git base.
    pub git_blob: Option<String>,
    pub index: LayerInput,
    pub disk: LayerInput,
    /// An open editor buffer is authoritative even when marked unmodified.
    pub editor: Option<EditorOverlayInput>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnsavedBufferInput {
    /// Process-local editor handle used only while capturing this snapshot.
    pub ephemeral_buffer_id: Option<String>,
    pub display_name: Option<String>,
    pub content: ContentRequest,
    pub version: Option<u64>,
    pub modified: bool,
    pub encoding: Option<String>,
    pub line_endings: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositorySnapshotInput {
    pub base_manifest_id: BaseManifestId,
    /// RFC 3339 timestamp supplied by the run coordinator.
    pub captured_at: String,
    pub base: RepositoryBase,
    pub files: Vec<FileSnapshotInput>,
    pub unsaved_buffers: Vec<UnsavedBufferInput>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureLimits {
    pub max_file_bytes: u64,
    pub max_total_bytes: u64,
}

impl Default for CaptureLimits {
    fn default() -> Self {
        Self {
            max_file_bytes: 8 * 1024 * 1024,
            max_total_bytes: 64 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaptureDecision {
    Include,
    Exclude { reason: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureKind {
    Index,
    Disk,
    Editor,
    UnsavedBuffer,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureSubject<'a> {
    pub path: Option<&'a RepoPath>,
    pub ephemeral_buffer_id: Option<&'a str>,
    pub kind: CaptureKind,
    pub locator: &'a str,
    pub declared_bytes: Option<u64>,
}

pub trait CapturePolicy {
    fn decide(&self, subject: &CaptureSubject<'_>) -> CaptureDecision;
}

pub trait SnapshotContentReader {
    /// Return at most `max_bytes` bytes. Implementations should fail instead
    /// of allocating beyond this bound.
    fn read(&self, request: &ContentRequest, max_bytes: u64) -> Result<Vec<u8>, String>;
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaseManifest {
    pub base_manifest_id: BaseManifestId,
    pub captured_at: String,
    pub repository: RepositoryBase,
    pub files: Vec<ManifestFile>,
    pub unsaved_buffers: Vec<UnsavedBuffer>,
    pub artifacts: Vec<ArtifactRecord>,
    pub captured_bytes: u64,
    pub confidence: ManifestConfidence,
    pub issues: Vec<ManifestIssue>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestFile {
    pub path: RepoPath,
    pub file_kind: FileKind,
    pub executable: bool,
    pub git_base: GitBaseEntry,
    pub index: ManifestLayer,
    pub disk: ManifestLayer,
    pub editor: Option<EditorOverlay>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum GitBaseEntry {
    Blob { object_id: String },
    Absent,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ManifestLayer {
    Inherit,
    Artifact { artifact_id: ArtifactId },
    Deleted,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditorOverlay {
    pub artifact_id: ArtifactId,
    pub version: Option<u64>,
    pub modified: bool,
    pub encoding: Option<String>,
    pub line_endings: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnsavedBuffer {
    /// Stable identity for this pathless manifest entry.
    pub entry_id: ArtifactId,
    /// Local editor projection only; never written to a manifest.
    #[serde(skip)]
    pub ephemeral_buffer_id: Option<String>,
    pub display_name: Option<String>,
    pub artifact_id: ArtifactId,
    pub version: Option<u64>,
    pub modified: bool,
    pub encoding: Option<String>,
    pub line_endings: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManifestConfidence {
    Complete,
    Partial,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestIssue {
    pub path: Option<RepoPath>,
    /// Diagnostic routing for the current process only.
    #[serde(skip)]
    pub ephemeral_buffer_id: Option<String>,
    pub kind: CaptureKind,
    pub detail: String,
}

pub fn capture_base_manifest(
    input: RepositorySnapshotInput,
    store: &ArtifactStore,
    policy: &dyn CapturePolicy,
    reader: &dyn SnapshotContentReader,
    limits: CaptureLimits,
) -> BaseManifest {
    let mut capture = Capture {
        store,
        policy,
        reader,
        limits,
        total: 0,
        artifacts: Vec::new(),
        issues: Vec::new(),
    };
    let mut files = Vec::with_capacity(input.files.len());

    for file in input.files {
        let path = match RepoPath::parse(file.path) {
            Ok(path) => path,
            Err(error) => {
                capture.issues.push(ManifestIssue {
                    path: None,
                    ephemeral_buffer_id: None,
                    kind: CaptureKind::Disk,
                    detail: error.to_string(),
                });
                continue;
            }
        };
        let git_base = file
            .git_blob
            .map_or(GitBaseEntry::Absent, |object_id| GitBaseEntry::Blob {
                object_id,
            });
        let index = capture.layer(&path, CaptureKind::Index, file.index);
        let disk = capture.layer(&path, CaptureKind::Disk, file.disk);
        let editor = file.editor.map(|overlay| {
            let artifact_id = capture.content(
                Some(&path),
                None,
                None,
                CaptureKind::Editor,
                &overlay.content,
                WorkspaceSurface::Buffer {
                    version: overlay.version,
                },
                ContentRepresentation::EditorText {
                    encoding: overlay.encoding.clone(),
                    line_endings: overlay.line_endings.clone(),
                },
            );
            EditorOverlay {
                artifact_id,
                version: overlay.version,
                modified: overlay.modified,
                encoding: overlay.encoding,
                line_endings: overlay.line_endings,
            }
        });
        files.push(ManifestFile {
            path,
            file_kind: file.file_kind,
            executable: file.executable,
            git_base,
            index,
            disk,
            editor,
        });
    }

    let mut unsaved_buffers = Vec::with_capacity(input.unsaved_buffers.len());
    for buffer in input.unsaved_buffers {
        let entry_id = ArtifactId::new();
        let artifact_id = capture.content(
            None,
            buffer.ephemeral_buffer_id.as_deref(),
            Some(format!("unsaved:{entry_id}")),
            CaptureKind::UnsavedBuffer,
            &buffer.content,
            WorkspaceSurface::Buffer {
                version: buffer.version,
            },
            ContentRepresentation::EditorText {
                encoding: buffer.encoding.clone(),
                line_endings: buffer.line_endings.clone(),
            },
        );
        unsaved_buffers.push(UnsavedBuffer {
            entry_id,
            ephemeral_buffer_id: buffer.ephemeral_buffer_id,
            display_name: buffer.display_name,
            artifact_id,
            version: buffer.version,
            modified: buffer.modified,
            encoding: buffer.encoding,
            line_endings: buffer.line_endings,
        });
    }

    BaseManifest {
        base_manifest_id: input.base_manifest_id,
        captured_at: input.captured_at,
        repository: input.base,
        files,
        unsaved_buffers,
        captured_bytes: capture.total,
        confidence: if capture.issues.is_empty() {
            ManifestConfidence::Complete
        } else {
            ManifestConfidence::Partial
        },
        artifacts: capture.artifacts,
        issues: capture.issues,
    }
}

struct Capture<'a> {
    store: &'a ArtifactStore,
    policy: &'a dyn CapturePolicy,
    reader: &'a dyn SnapshotContentReader,
    limits: CaptureLimits,
    total: u64,
    artifacts: Vec<ArtifactRecord>,
    issues: Vec<ManifestIssue>,
}

impl Capture<'_> {
    fn layer(&mut self, path: &RepoPath, kind: CaptureKind, layer: LayerInput) -> ManifestLayer {
        match layer {
            LayerInput::Inherit => ManifestLayer::Inherit,
            LayerInput::Deleted => ManifestLayer::Deleted,
            LayerInput::Content(request) => {
                let surface = match kind {
                    CaptureKind::Index => WorkspaceSurface::GitIndex,
                    _ => WorkspaceSurface::Disk,
                };
                ManifestLayer::Artifact {
                    artifact_id: self.content(
                        Some(path),
                        None,
                        None,
                        kind,
                        &request,
                        surface,
                        ContentRepresentation::RawBytes,
                    ),
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn content(
        &mut self,
        path: Option<&RepoPath>,
        ephemeral_buffer_id: Option<&str>,
        source_path_override: Option<String>,
        kind: CaptureKind,
        request: &ContentRequest,
        surface: WorkspaceSurface,
        representation: ContentRepresentation,
    ) -> ArtifactId {
        let artifact_id = ArtifactId::new();
        let subject = CaptureSubject {
            path,
            ephemeral_buffer_id,
            kind: kind.clone(),
            locator: &request.locator,
            declared_bytes: request.declared_bytes,
        };
        let decision = self.policy.decide(&subject);
        let source_path = source_path_override
            .or_else(|| path.map(ToString::to_string))
            .unwrap_or_else(|| "unsaved:unknown".into());
        let source = ArtifactSource::Workspace {
            path: source_path,
            surface,
        };

        let state = match decision {
            CaptureDecision::Exclude { reason } => {
                self.issue(&subject, reason.clone());
                ArtifactState::Excluded { reason }
            }
            CaptureDecision::Include => self.capture_included(&subject, request),
        };
        let export_policy = if matches!(state, ArtifactState::Excluded { .. }) {
            ArtifactExportPolicy::Omit
        } else {
            ArtifactExportPolicy::Include
        };
        self.artifacts.push(ArtifactRecord {
            artifact_id: artifact_id.clone(),
            state,
            source,
            representation,
            media_type: None,
            retention: ArtifactRetention::Run,
            export_policy,
        });
        artifact_id
    }

    fn capture_included(
        &mut self,
        subject: &CaptureSubject<'_>,
        request: &ContentRequest,
    ) -> ArtifactState {
        let remaining = self.limits.max_total_bytes.saturating_sub(self.total);
        let permitted = self.limits.max_file_bytes.min(remaining);
        if request.declared_bytes.is_some_and(|size| size > permitted) || permitted == 0 {
            let reason = "capture size limit exceeded".to_string();
            self.issue(subject, reason.clone());
            return ArtifactState::Excluded { reason };
        }
        let bytes = match self.reader.read(request, permitted) {
            Ok(bytes) => bytes,
            Err(reason) => {
                self.issue(subject, reason.clone());
                return ArtifactState::Missing { reason };
            }
        };
        if bytes.len() as u64 > permitted {
            let reason = "content reader exceeded capture size limit".to_string();
            self.issue(subject, reason.clone());
            return ArtifactState::Excluded { reason };
        }
        if matches!(
            subject.kind,
            CaptureKind::Editor | CaptureKind::UnsavedBuffer
        ) && (std::str::from_utf8(&bytes).is_err() || bytes.contains(&b'\r'))
        {
            let reason = "editor content was not normalized UTF-8 with LF line endings".to_string();
            self.issue(subject, reason.clone());
            return ArtifactState::Missing { reason };
        }
        match self.store.put_bytes(&bytes) {
            Ok(stored) => {
                self.total += stored.byte_len;
                ArtifactState::Available {
                    blob_id: stored.blob_id,
                    byte_len: stored.byte_len,
                }
            }
            Err(error) => {
                let reason = error.to_string();
                self.issue(subject, reason.clone());
                ArtifactState::Missing { reason }
            }
        }
    }

    fn issue(&mut self, subject: &CaptureSubject<'_>, detail: String) {
        self.issues.push(ManifestIssue {
            path: subject.path.cloned(),
            ephemeral_buffer_id: subject.ephemeral_buffer_id.map(str::to_owned),
            kind: subject.kind.clone(),
            detail,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};
    use std::sync::Mutex;
    use tempfile::tempdir;

    struct Allow;
    impl CapturePolicy for Allow {
        fn decide(&self, _: &CaptureSubject<'_>) -> CaptureDecision {
            CaptureDecision::Include
        }
    }

    struct Reader {
        values: HashMap<String, Result<Vec<u8>, String>>,
        reads: Mutex<Vec<String>>,
    }
    impl SnapshotContentReader for Reader {
        fn read(&self, request: &ContentRequest, _: u64) -> Result<Vec<u8>, String> {
            self.reads.lock().unwrap().push(request.locator.clone());
            self.values.get(&request.locator).cloned().unwrap()
        }
    }

    fn base() -> RepositoryBase {
        RepositoryBase {
            repository_id: RepositoryId::parse("repo_test").unwrap(),
            head_commit: Some("abc123".into()),
            index_tree: Some("tree123".into()),
        }
    }

    fn capture(files: Vec<FileSnapshotInput>, reader: &Reader) -> BaseManifest {
        let dir = tempdir().unwrap();
        let store = ArtifactStore::open(dir.path()).unwrap();
        capture_base_manifest(
            RepositorySnapshotInput {
                base_manifest_id: BaseManifestId::parse("bsm_test").unwrap(),
                captured_at: "2026-07-13T12:00:00Z".into(),
                base: base(),
                files,
                unsaved_buffers: vec![],
            },
            &store,
            &Allow,
            reader,
            CaptureLimits::default(),
        )
    }

    #[test]
    fn models_clean_staged_dirty_untracked_and_deleted_layers() {
        let reader = Reader {
            values: HashMap::from([
                ("staged".into(), Ok(b"staged".to_vec())),
                ("dirty".into(), Ok(b"dirty".to_vec())),
                ("new".into(), Ok(b"new".to_vec())),
            ]),
            reads: Mutex::new(vec![]),
        };
        let files = vec![
            FileSnapshotInput {
                path: "clean.rs".into(),
                file_kind: FileKind::Regular,
                executable: false,
                git_blob: Some("g1".into()),
                index: LayerInput::Inherit,
                disk: LayerInput::Inherit,
                editor: None,
            },
            FileSnapshotInput {
                path: "staged.rs".into(),
                file_kind: FileKind::Regular,
                executable: false,
                git_blob: Some("g2".into()),
                index: LayerInput::Content(ContentRequest::new("staged")),
                disk: LayerInput::Inherit,
                editor: None,
            },
            FileSnapshotInput {
                path: "dirty.rs".into(),
                file_kind: FileKind::Regular,
                executable: false,
                git_blob: Some("g3".into()),
                index: LayerInput::Inherit,
                disk: LayerInput::Content(ContentRequest::new("dirty")),
                editor: None,
            },
            FileSnapshotInput {
                path: "new.rs".into(),
                file_kind: FileKind::Regular,
                executable: true,
                git_blob: None,
                index: LayerInput::Inherit,
                disk: LayerInput::Content(ContentRequest::new("new")),
                editor: None,
            },
            FileSnapshotInput {
                path: "gone.rs".into(),
                file_kind: FileKind::Symlink,
                executable: false,
                git_blob: Some("g4".into()),
                index: LayerInput::Inherit,
                disk: LayerInput::Deleted,
                editor: None,
            },
        ];
        let manifest = capture(files, &reader);
        assert_eq!(manifest.files.len(), 5);
        assert!(manifest.artifacts.len() == 3);
        assert!(matches!(
            manifest.files[0].git_base,
            GitBaseEntry::Blob { .. }
        ));
        assert_eq!(manifest.files[0].index, ManifestLayer::Inherit);
        assert_eq!(manifest.files[0].disk, ManifestLayer::Inherit);
        assert!(matches!(
            manifest.files[1].index,
            ManifestLayer::Artifact { .. }
        ));
        assert!(matches!(
            manifest.files[2].disk,
            ManifestLayer::Artifact { .. }
        ));
        assert_eq!(manifest.files[4].disk, ManifestLayer::Deleted);
        assert!(manifest.files[3].executable);
        assert_eq!(manifest.files[4].file_kind, FileKind::Symlink);
    }

    #[test]
    fn raw_crlf_disk_and_normalized_editor_overlay_are_distinct() {
        let reader = Reader {
            values: HashMap::from([
                ("disk".into(), Ok(b"a\r\n".to_vec())),
                ("editor".into(), Ok(b"a\n".to_vec())),
            ]),
            reads: Mutex::new(vec![]),
        };
        let manifest = capture(
            vec![FileSnapshotInput {
                path: "a.txt".into(),
                file_kind: FileKind::Regular,
                executable: false,
                git_blob: Some("g".into()),
                index: LayerInput::Inherit,
                disk: LayerInput::Content(ContentRequest::new("disk")),
                editor: Some(EditorOverlayInput {
                    content: ContentRequest::new("editor"),
                    version: Some(7),
                    modified: false,
                    encoding: Some("utf-8".into()),
                    line_endings: Some("lf".into()),
                }),
            }],
            &reader,
        );
        assert_eq!(manifest.artifacts.len(), 2);
        assert_ne!(manifest.artifacts[0].state, manifest.artifacts[1].state);
        assert!(matches!(
            manifest.artifacts[0].representation,
            ContentRepresentation::RawBytes
        ));
        assert!(matches!(
            manifest.artifacts[1].representation,
            ContentRepresentation::EditorText { .. }
        ));
        assert!(!manifest.files[0].editor.as_ref().unwrap().modified);
    }

    #[test]
    fn captures_pathless_unsaved_buffer() {
        let reader = Reader {
            values: HashMap::from([("buf".into(), Ok(b"draft".to_vec()))]),
            reads: Mutex::new(vec![]),
        };
        let dir = tempdir().unwrap();
        let manifest = capture_base_manifest(
            RepositorySnapshotInput {
                base_manifest_id: BaseManifestId::parse("bsm_unsaved").unwrap(),
                captured_at: "2026-07-13T12:00:00Z".into(),
                base: base(),
                files: vec![],
                unsaved_buffers: vec![UnsavedBufferInput {
                    ephemeral_buffer_id: Some("42".into()),
                    display_name: None,
                    content: ContentRequest::new("buf"),
                    version: Some(2),
                    modified: true,
                    encoding: Some("utf-8".into()),
                    line_endings: Some("lf".into()),
                }],
            },
            &ArtifactStore::open(dir.path()).unwrap(),
            &Allow,
            &reader,
            CaptureLimits::default(),
        );
        assert_eq!(manifest.unsaved_buffers.len(), 1);
        assert_eq!(manifest.artifacts.len(), 1);
        assert_eq!(manifest.base_manifest_id.as_str(), "bsm_unsaved");
        assert_eq!(manifest.captured_at, "2026-07-13T12:00:00Z");
        let persisted = serde_json::to_value(&manifest).unwrap();
        assert!(persisted["unsaved_buffers"][0]
            .get("ephemeral_buffer_id")
            .is_none());
        let restored: BaseManifest = serde_json::from_value(persisted).unwrap();
        assert_eq!(restored.unsaved_buffers[0].ephemeral_buffer_id, None);
        assert_eq!(
            restored.unsaved_buffers[0].entry_id,
            manifest.unsaved_buffers[0].entry_id
        );
    }

    #[test]
    fn exclusion_policy_runs_before_sensitive_reader() {
        struct ExcludeSecret;
        impl CapturePolicy for ExcludeSecret {
            fn decide(&self, subject: &CaptureSubject<'_>) -> CaptureDecision {
                if subject.path.is_some_and(|p| p.as_str() == ".env") {
                    CaptureDecision::Exclude {
                        reason: "secret policy".into(),
                    }
                } else {
                    CaptureDecision::Include
                }
            }
        }
        let reader = Reader {
            values: HashMap::from([("secret".into(), Ok(b"password".to_vec()))]),
            reads: Mutex::new(vec![]),
        };
        let dir = tempdir().unwrap();
        let manifest = capture_base_manifest(
            RepositorySnapshotInput {
                base_manifest_id: BaseManifestId::parse("bsm_secret").unwrap(),
                captured_at: "2026-07-13T12:00:00Z".into(),
                base: base(),
                files: vec![FileSnapshotInput {
                    path: ".env".into(),
                    file_kind: FileKind::Regular,
                    executable: false,
                    git_blob: None,
                    index: LayerInput::Inherit,
                    disk: LayerInput::Content(ContentRequest::new("secret")),
                    editor: None,
                }],
                unsaved_buffers: vec![],
            },
            &ArtifactStore::open(dir.path()).unwrap(),
            &ExcludeSecret,
            &reader,
            CaptureLimits::default(),
        );
        assert!(reader.reads.lock().unwrap().is_empty());
        assert!(matches!(
            manifest.artifacts[0].state,
            ArtifactState::Excluded { .. }
        ));
    }

    #[test]
    fn rejects_traversal_and_localizes_reader_failure() {
        let reader = Reader {
            values: HashMap::from([
                ("bad".into(), Err("file vanished".into())),
                ("ok".into(), Ok(b"ok".to_vec())),
            ]),
            reads: Mutex::new(vec![]),
        };
        let manifest = capture(
            vec![
                FileSnapshotInput {
                    path: "../secret".into(),
                    file_kind: FileKind::Regular,
                    executable: false,
                    git_blob: None,
                    index: LayerInput::Inherit,
                    disk: LayerInput::Content(ContentRequest::new("never")),
                    editor: None,
                },
                FileSnapshotInput {
                    path: "bad.rs".into(),
                    file_kind: FileKind::Regular,
                    executable: false,
                    git_blob: None,
                    index: LayerInput::Inherit,
                    disk: LayerInput::Content(ContentRequest::new("bad")),
                    editor: None,
                },
                FileSnapshotInput {
                    path: "ok.rs".into(),
                    file_kind: FileKind::Regular,
                    executable: false,
                    git_blob: None,
                    index: LayerInput::Inherit,
                    disk: LayerInput::Content(ContentRequest::new("ok")),
                    editor: None,
                },
            ],
            &reader,
        );
        assert_eq!(manifest.files.len(), 2);
        assert_eq!(manifest.confidence, ManifestConfidence::Partial);
        assert!(matches!(
            manifest.artifacts[0].state,
            ArtifactState::Missing { .. }
        ));
        assert!(matches!(
            manifest.artifacts[1].state,
            ArtifactState::Available { .. }
        ));
        let reads: HashSet<_> = reader.reads.lock().unwrap().iter().cloned().collect();
        assert_eq!(reads, HashSet::from(["bad".into(), "ok".into()]));
    }

    #[test]
    fn rejects_all_non_normalized_path_forms() {
        for path in [
            "/absolute",
            "../escape",
            "nested/../escape",
            "./local",
            "double//slash",
            "windows\\path",
            "C:/absolute",
            "nul\0byte",
        ] {
            assert!(RepoPath::parse(path).is_err(), "accepted {path:?}");
        }
        assert_eq!(
            RepoPath::parse("src/lib.rs").unwrap().as_str(),
            "src/lib.rs"
        );
    }

    #[test]
    fn declared_size_limit_excludes_without_reading() {
        let reader = Reader {
            values: HashMap::from([("large".into(), Ok(vec![0; 20]))]),
            reads: Mutex::new(vec![]),
        };
        let dir = tempdir().unwrap();
        let mut request = ContentRequest::new("large");
        request.declared_bytes = Some(20);
        let manifest = capture_base_manifest(
            RepositorySnapshotInput {
                base_manifest_id: BaseManifestId::parse("bsm_large").unwrap(),
                captured_at: "2026-07-13T12:00:00Z".into(),
                base: base(),
                files: vec![FileSnapshotInput {
                    path: "large.bin".into(),
                    file_kind: FileKind::Regular,
                    executable: false,
                    git_blob: None,
                    index: LayerInput::Inherit,
                    disk: LayerInput::Content(request),
                    editor: None,
                }],
                unsaved_buffers: vec![],
            },
            &ArtifactStore::open(dir.path()).unwrap(),
            &Allow,
            &reader,
            CaptureLimits {
                max_file_bytes: 10,
                max_total_bytes: 100,
            },
        );
        assert!(reader.reads.lock().unwrap().is_empty());
        assert!(matches!(
            manifest.artifacts[0].state,
            ArtifactState::Excluded { .. }
        ));
        assert_eq!(manifest.confidence, ManifestConfidence::Partial);
    }
}
