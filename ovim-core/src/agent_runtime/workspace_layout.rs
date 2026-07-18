//! Durable storage layout and ownership records for Ovim-managed workspaces.
//!
//! Workspace storage deliberately does not share the run-log directory. Run
//! history and a write worktree have different retention semantics: removing
//! history must never silently remove the only copy of unresolved agent work.

use crate::ai::AiSubagentWorkspaceConfig;
use crate::run_log::{AgentId, RepositoryId, RunId};
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

pub const OVIM_WORKSPACES_DIR_ENV: &str = "OVIM_WORKSPACES_DIR";
pub const WORKSPACE_MARKER_FILE: &str = "workspace.json";
pub const WORKSPACE_CHECKOUT_DIRECTORY: &str = "checkout";
pub const WORKSPACE_MARKER_VERSION: u32 = 1;
const MEBIBYTE: u64 = 1024 * 1024;
const MAX_PATH_COMPONENT_BYTES: usize = 255;
static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(0);

/// Why a workspace root was selected.
///
/// Only environment and configuration roots count as explicit overrides. A
/// repo-adjacent root is accepted only with one of those origins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspaceRootOrigin {
    Environment,
    Configuration,
    PlatformDefault,
}

impl WorkspaceRootOrigin {
    pub fn is_explicit(self) -> bool {
        matches!(self, Self::Environment | Self::Configuration)
    }
}

/// Unvalidated workspace root plus allocation policy.
///
/// Call [`Self::preflight`] before using the root for durable state. Keeping
/// this type distinct from [`ValidatedWorkspaceStorageLayout`] makes it hard
/// for later worktree code to accidentally skip canonicalization and Git-aware
/// containment checks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceStorageLayout {
    root: PathBuf,
    origin: WorkspaceRootOrigin,
    minimum_free_space_bytes: u64,
}

impl WorkspaceStorageLayout {
    pub fn new(
        root: impl Into<PathBuf>,
        origin: WorkspaceRootOrigin,
        minimum_free_space_mb: u64,
    ) -> Self {
        Self {
            root: root.into(),
            origin,
            minimum_free_space_bytes: minimum_free_space_mb.saturating_mul(MEBIBYTE),
        }
    }

    /// Resolve production storage using environment, config, platform order.
    pub fn discover(config: &AiSubagentWorkspaceConfig) -> Result<Self, WorkspaceLayoutError> {
        Self::from_locations(
            std::env::var_os(OVIM_WORKSPACES_DIR_ENV),
            config,
            dirs::data_local_dir(),
        )
    }

    /// Injectable discovery that never reads or mutates process-global state.
    pub fn from_locations(
        environment_root: Option<OsString>,
        config: &AiSubagentWorkspaceConfig,
        data_local_dir: Option<PathBuf>,
    ) -> Result<Self, WorkspaceLayoutError> {
        if let Some(root) = environment_root.filter(|value| !value.is_empty()) {
            return Ok(Self::new(
                PathBuf::from(root),
                WorkspaceRootOrigin::Environment,
                config.minimum_free_space_mb,
            ));
        }
        if let Some(root) = config
            .root
            .as_ref()
            .filter(|root| !root.as_os_str().is_empty())
        {
            return Ok(Self::new(
                root,
                WorkspaceRootOrigin::Configuration,
                config.minimum_free_space_mb,
            ));
        }
        data_local_dir
            .map(|directory| {
                Self::new(
                    directory.join("ovim").join("workspaces"),
                    WorkspaceRootOrigin::PlatformDefault,
                    config.minimum_free_space_mb,
                )
            })
            .ok_or(WorkspaceLayoutError::PlatformDataUnavailable)
    }

    pub fn root_candidate(&self) -> &Path {
        &self.root
    }

    pub fn origin(&self) -> WorkspaceRootOrigin {
        self.origin
    }

    pub fn minimum_free_space_bytes(&self) -> u64 {
        self.minimum_free_space_bytes
    }

    pub fn candidate_paths(&self, coordinates: &WorkspaceCoordinates) -> WorkspacePaths {
        WorkspacePaths::under(&self.root, coordinates)
    }

    /// Validate and prepare the root for one exact future allocation.
    pub fn preflight(
        &self,
        source_repository: &Path,
        coordinates: &WorkspaceCoordinates,
    ) -> Result<WorkspacePreflight, WorkspaceLayoutError> {
        self.preflight_with(source_repository, coordinates, &NativeWorkspaceProbe)
    }

    /// Injectable preflight for deterministic disk/path/platform tests.
    pub fn preflight_with(
        &self,
        source_repository: &Path,
        coordinates: &WorkspaceCoordinates,
        probe: &dyn WorkspaceProbe,
    ) -> Result<WorkspacePreflight, WorkspaceLayoutError> {
        preflight_layout(self, source_repository, coordinates, probe)
    }
}

/// Identities that alone determine an Ovim workspace path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceCoordinates {
    pub repository_id: RepositoryId,
    pub run_id: RunId,
    pub agent_id: AgentId,
}

impl WorkspaceCoordinates {
    pub fn new(repository_id: RepositoryId, run_id: RunId, agent_id: AgentId) -> Self {
        Self {
            repository_id,
            run_id,
            agent_id,
        }
    }
}

/// Exact derived locations. Display labels, task text and model output never
/// participate in this derivation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspacePaths {
    pub repository_directory: PathBuf,
    pub run_directory: PathBuf,
    pub agent_directory: PathBuf,
    pub marker: PathBuf,
    pub checkout: PathBuf,
}

impl WorkspacePaths {
    fn under(root: &Path, coordinates: &WorkspaceCoordinates) -> Self {
        let repository_directory = root.join(encoded_identity_component(
            "repo",
            coordinates.repository_id.as_str(),
        ));
        let run_directory = repository_directory.join(encoded_identity_component(
            "run",
            coordinates.run_id.as_str(),
        ));
        let agent_directory = run_directory.join(encoded_identity_component(
            "agent",
            coordinates.agent_id.as_str(),
        ));
        Self {
            marker: agent_directory.join(WORKSPACE_MARKER_FILE),
            checkout: agent_directory.join(WORKSPACE_CHECKOUT_DIRECTORY),
            repository_directory,
            run_directory,
            agent_directory,
        }
    }
}

fn encoded_identity_component(prefix: &str, value: &str) -> String {
    let mut encoded = String::with_capacity(prefix.len() + 1 + value.len() * 2);
    encoded.push_str(prefix);
    encoded.push('-');
    for byte in value.as_bytes() {
        use fmt::Write as _;
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

/// Canonical, Git-aware layout proven safe for an exact identity tuple.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidatedWorkspaceStorageLayout {
    root: PathBuf,
    origin: WorkspaceRootOrigin,
    minimum_free_space_bytes: u64,
}

impl ValidatedWorkspaceStorageLayout {
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn origin(&self) -> WorkspaceRootOrigin {
        self.origin
    }

    pub fn minimum_free_space_bytes(&self) -> u64 {
        self.minimum_free_space_bytes
    }

    pub fn paths(&self, coordinates: &WorkspaceCoordinates) -> WorkspacePaths {
        WorkspacePaths::under(&self.root, coordinates)
    }
}

/// Read-only diagnostics captured by preflight.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspacePreflight {
    pub layout: ValidatedWorkspaceStorageLayout,
    pub canonical_source: PathBuf,
    pub canonical_git_common_directory: PathBuf,
    pub registered_worktrees: Vec<RegisteredGitWorktree>,
    pub available_space_bytes: u64,
    pub same_filesystem_as_source: Option<bool>,
    pub repo_adjacent: bool,
    pub path_limit_bytes: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegisteredGitWorktree {
    pub path: PathBuf,
}

/// OS facts are injectable so tests need neither global environment changes
/// nor an actually full filesystem.
pub trait WorkspaceProbe {
    fn available_space(&self, path: &Path) -> io::Result<u64>;
    fn path_limit(&self, path: &Path) -> usize;
    fn same_filesystem(&self, left: &Path, right: &Path) -> io::Result<Option<bool>>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NativeWorkspaceProbe;

impl WorkspaceProbe for NativeWorkspaceProbe {
    fn available_space(&self, path: &Path) -> io::Result<u64> {
        fs2::available_space(path)
    }

    fn path_limit(&self, _path: &Path) -> usize {
        #[cfg(windows)]
        {
            // Leave room for Git administrative suffixes on systems without
            // long-path support enabled.
            240
        }
        #[cfg(not(windows))]
        {
            4_096
        }
    }

    fn same_filesystem(&self, left: &Path, right: &Path) -> io::Result<Option<bool>> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            Ok(Some(
                fs::metadata(left)?.dev() == fs::metadata(right)?.dev(),
            ))
        }
        #[cfg(not(unix))]
        {
            let _ = (left, right);
            Ok(None)
        }
    }
}

#[derive(Debug)]
pub enum WorkspaceLayoutError {
    PlatformDataUnavailable,
    UnsafeRoot {
        path: PathBuf,
        reason: String,
    },
    NotGitRepository {
        path: PathBuf,
        detail: String,
    },
    GitUnsupported {
        operation: String,
        detail: String,
    },
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    LowDiskSpace {
        available: u64,
        required: u64,
    },
    PathTooLong {
        path: PathBuf,
        limit: usize,
    },
    PathComponentTooLong {
        component_bytes: usize,
        limit: usize,
    },
    WorkspaceCollision {
        path: PathBuf,
    },
    MarkerMissing {
        path: PathBuf,
    },
    MarkerInvalid {
        path: PathBuf,
        detail: String,
    },
    MarkerMismatch {
        field: &'static str,
        detail: String,
    },
}

impl fmt::Display for WorkspaceLayoutError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PlatformDataUnavailable => {
                formatter.write_str("the platform does not provide a local data directory")
            }
            Self::UnsafeRoot { path, reason } => {
                write!(
                    formatter,
                    "unsafe workspace root {}: {reason}",
                    path.display()
                )
            }
            Self::NotGitRepository { path, detail } => write!(
                formatter,
                "{} is not a supported Git worktree: {detail}",
                path.display()
            ),
            Self::GitUnsupported { operation, detail } => {
                write!(formatter, "Git cannot {operation}: {detail}")
            }
            Self::Io {
                operation,
                path,
                source,
            } => write!(formatter, "{operation} {}: {source}", path.display()),
            Self::LowDiskSpace {
                available,
                required,
            } => write!(
                formatter,
                "workspace root has {available} bytes available; {required} bytes required"
            ),
            Self::PathTooLong { path, limit } => write!(
                formatter,
                "workspace path {} exceeds the platform limit of {limit} bytes",
                path.display()
            ),
            Self::PathComponentTooLong {
                component_bytes,
                limit,
            } => write!(
                formatter,
                "workspace path component is {component_bytes} bytes; limit is {limit}"
            ),
            Self::WorkspaceCollision { path } => {
                write!(
                    formatter,
                    "workspace allocation already exists at {}",
                    path.display()
                )
            }
            Self::MarkerMissing { path } => {
                write!(
                    formatter,
                    "workspace marker is missing at {}",
                    path.display()
                )
            }
            Self::MarkerInvalid { path, detail } => {
                write!(
                    formatter,
                    "workspace marker {} is invalid: {detail}",
                    path.display()
                )
            }
            Self::MarkerMismatch { field, detail } => {
                write!(formatter, "workspace marker {field} mismatch: {detail}")
            }
        }
    }
}

impl std::error::Error for WorkspaceLayoutError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

fn io_error(operation: &'static str, path: &Path, source: io::Error) -> WorkspaceLayoutError {
    WorkspaceLayoutError::Io {
        operation,
        path: path.to_path_buf(),
        source,
    }
}

struct GitWorkspaceContext {
    canonical_source: PathBuf,
    canonical_common_directory: PathBuf,
    registered_worktrees: Vec<RegisteredGitWorktree>,
}

fn preflight_layout(
    requested: &WorkspaceStorageLayout,
    source_repository: &Path,
    coordinates: &WorkspaceCoordinates,
    probe: &dyn WorkspaceProbe,
) -> Result<WorkspacePreflight, WorkspaceLayoutError> {
    let git = inspect_git_workspace(source_repository)?;
    let candidate = canonicalize_allow_missing(&requested.root)
        .map_err(|source| io_error("resolve workspace root", &requested.root, source))?;

    validate_root(
        &candidate,
        requested.origin,
        &git.canonical_source,
        &git.canonical_common_directory,
        &git.registered_worktrees,
    )?;
    ensure_private_directory(&candidate)?;
    // Canonicalize again after creation to close the missing-tail/symlink gap.
    let canonical_root = fs::canonicalize(&candidate)
        .map_err(|source| io_error("canonicalize workspace root", &candidate, source))?;
    validate_root(
        &canonical_root,
        requested.origin,
        &git.canonical_source,
        &git.canonical_common_directory,
        &git.registered_worktrees,
    )?;
    verify_writable(&canonical_root)?;

    let available_space_bytes = probe
        .available_space(&canonical_root)
        .map_err(|source| io_error("measure workspace free space", &canonical_root, source))?;
    if available_space_bytes < requested.minimum_free_space_bytes {
        return Err(WorkspaceLayoutError::LowDiskSpace {
            available: available_space_bytes,
            required: requested.minimum_free_space_bytes,
        });
    }

    let layout = ValidatedWorkspaceStorageLayout {
        root: canonical_root.clone(),
        origin: requested.origin,
        minimum_free_space_bytes: requested.minimum_free_space_bytes,
    };
    let paths = layout.paths(coordinates);
    validate_path_limits(&paths.checkout, probe.path_limit(&canonical_root))?;
    if paths.agent_directory.exists() {
        return Err(WorkspaceLayoutError::WorkspaceCollision {
            path: paths.agent_directory,
        });
    }

    let same_filesystem_as_source = probe
        .same_filesystem(&canonical_root, &git.canonical_source)
        .map_err(|source| {
            io_error(
                "compare workspace and source filesystems",
                &canonical_root,
                source,
            )
        })?;
    let repo_adjacent = is_repo_adjacent(&canonical_root, &git.canonical_source);

    Ok(WorkspacePreflight {
        layout,
        canonical_source: git.canonical_source,
        canonical_git_common_directory: git.canonical_common_directory,
        registered_worktrees: git.registered_worktrees,
        available_space_bytes,
        same_filesystem_as_source,
        repo_adjacent,
        path_limit_bytes: probe.path_limit(&canonical_root),
    })
}

fn inspect_git_workspace(source: &Path) -> Result<GitWorkspaceContext, WorkspaceLayoutError> {
    let top_level = run_git(
        source,
        &["rev-parse", "--show-toplevel"],
        "find repository root",
    )?;
    let canonical_source = parse_single_git_path(&top_level, source, "repository root")?;
    let common = run_git(
        &canonical_source,
        &["rev-parse", "--git-common-dir"],
        "find common Git directory",
    )?;
    let common_path = parse_single_git_path(&common, &canonical_source, "common Git directory")?;
    let canonical_common_directory = canonicalize_allow_missing(&common_path)
        .map_err(|source| io_error("canonicalize common Git directory", &common_path, source))?;

    let output = Command::new("git")
        .arg("-C")
        .arg(&canonical_source)
        .args(["worktree", "list", "--porcelain", "-z"])
        .output()
        .map_err(|error| WorkspaceLayoutError::GitUnsupported {
            operation: "list registered worktrees".into(),
            detail: error.to_string(),
        })?;
    if !output.status.success() {
        return Err(WorkspaceLayoutError::GitUnsupported {
            operation: "list registered worktrees".into(),
            detail: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    let mut registered_worktrees = Vec::new();
    for field in output.stdout.split(|byte| *byte == 0) {
        let Some(path) = field.strip_prefix(b"worktree ") else {
            continue;
        };
        let path =
            path_from_git_bytes(path).map_err(|detail| WorkspaceLayoutError::GitUnsupported {
                operation: "decode registered worktree path".into(),
                detail,
            })?;
        let path = canonicalize_allow_missing(&path)
            .map_err(|source| io_error("canonicalize registered worktree", &path, source))?;
        registered_worktrees.push(RegisteredGitWorktree { path });
    }
    if !registered_worktrees
        .iter()
        .any(|worktree| worktree.path == canonical_source)
    {
        return Err(WorkspaceLayoutError::GitUnsupported {
            operation: "verify registered source worktree".into(),
            detail: format!(
                "Git did not report {} in `git worktree list --porcelain -z`",
                canonical_source.display()
            ),
        });
    }

    Ok(GitWorkspaceContext {
        canonical_source,
        canonical_common_directory,
        registered_worktrees,
    })
}

fn run_git(source: &Path, args: &[&str], operation: &str) -> Result<Vec<u8>, WorkspaceLayoutError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(source)
        .args(args)
        .output()
        .map_err(|error| WorkspaceLayoutError::NotGitRepository {
            path: source.to_path_buf(),
            detail: error.to_string(),
        })?;
    if !output.status.success() {
        return Err(WorkspaceLayoutError::NotGitRepository {
            path: source.to_path_buf(),
            detail: format!(
                "{operation}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        });
    }
    Ok(output.stdout)
}

fn parse_single_git_path(
    bytes: &[u8],
    relative_to: &Path,
    description: &str,
) -> Result<PathBuf, WorkspaceLayoutError> {
    let raw = bytes.strip_suffix(b"\n").unwrap_or(bytes);
    let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
    let mut path =
        path_from_git_bytes(raw).map_err(|detail| WorkspaceLayoutError::GitUnsupported {
            operation: format!("decode {description}"),
            detail,
        })?;
    if path.is_relative() {
        path = relative_to.join(path);
    }
    canonicalize_allow_missing(&path)
        .map_err(|source| io_error("canonicalize Git-reported path", &path, source))
}

#[cfg(unix)]
fn path_from_git_bytes(bytes: &[u8]) -> Result<PathBuf, String> {
    use std::os::unix::ffi::OsStringExt;
    Ok(PathBuf::from(OsString::from_vec(bytes.to_vec())))
}

#[cfg(not(unix))]
fn path_from_git_bytes(bytes: &[u8]) -> Result<PathBuf, String> {
    String::from_utf8(bytes.to_vec())
        .map(PathBuf::from)
        .map_err(|error| error.to_string())
}

fn canonicalize_allow_missing(path: &Path) -> io::Result<PathBuf> {
    if !path.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "workspace paths must be absolute",
        ));
    }
    let mut existing = path;
    let mut missing = Vec::new();
    while !existing.exists() {
        let component = existing
            .file_name()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no existing path ancestor"))?;
        if component == OsStr::new(".") || component == OsStr::new("..") {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "dot path components are not accepted",
            ));
        }
        missing.push(component.to_os_string());
        existing = existing
            .parent()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no existing path ancestor"))?;
    }
    let mut canonical = fs::canonicalize(existing)?;
    for component in missing.into_iter().rev() {
        canonical.push(component);
    }
    Ok(canonical)
}

fn validate_root(
    root: &Path,
    origin: WorkspaceRootOrigin,
    source: &Path,
    common_git_directory: &Path,
    registered_worktrees: &[RegisteredGitWorktree],
) -> Result<(), WorkspaceLayoutError> {
    let normal_components = root
        .components()
        .filter(|component| matches!(component, Component::Normal(_)))
        .count();
    if root.parent().is_none() || normal_components < 2 {
        return Err(unsafe_root(root, "it is an unsafe broad filesystem root"));
    }
    if dirs::home_dir().as_deref() == Some(root) {
        return Err(unsafe_root(root, "it is the user's home directory"));
    }
    if is_equal_or_descendant(root, source) {
        return Err(unsafe_root(
            root,
            "it is equal to or inside the source worktree",
        ));
    }
    if is_equal_or_descendant(root, common_git_directory) {
        return Err(unsafe_root(
            root,
            "it is equal to or inside the common Git directory",
        ));
    }
    for registered in registered_worktrees {
        if is_equal_or_descendant(root, &registered.path) {
            return Err(unsafe_root(
                root,
                &format!(
                    "it is equal to or inside registered worktree {}",
                    registered.path.display()
                ),
            ));
        }
    }
    if is_repo_adjacent(root, source) && !origin.is_explicit() {
        return Err(unsafe_root(
            root,
            "repo-adjacent storage requires an explicit environment or configuration override",
        ));
    }
    Ok(())
}

fn unsafe_root(path: &Path, reason: &str) -> WorkspaceLayoutError {
    WorkspaceLayoutError::UnsafeRoot {
        path: path.to_path_buf(),
        reason: reason.into(),
    }
}

fn is_equal_or_descendant(path: &Path, ancestor: &Path) -> bool {
    path == ancestor || path.starts_with(ancestor)
}

fn is_repo_adjacent(root: &Path, source: &Path) -> bool {
    let Some(source_name) = source.file_name() else {
        return false;
    };
    let mut expected_name = source_name.to_os_string();
    expected_name.push("-worktrees");
    root.parent() == source.parent() && root.file_name() == Some(expected_name.as_os_str())
}

fn ensure_private_directory(path: &Path) -> Result<(), WorkspaceLayoutError> {
    fs::create_dir_all(path)
        .map_err(|source| io_error("create workspace directory", path, source))?;
    let metadata = fs::metadata(path)
        .map_err(|source| io_error("inspect workspace directory", path, source))?;
    if !metadata.is_dir() {
        return Err(unsafe_root(path, "it is not a directory"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|source| io_error("set private workspace permissions", path, source))?;
    }
    Ok(())
}

fn verify_writable(root: &Path) -> Result<(), WorkspaceLayoutError> {
    for _ in 0..100 {
        let temporary = root.join(format!(
            ".ovim-write-probe-{}-{}",
            std::process::id(),
            NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed)
        ));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&temporary) {
            Ok(file) => {
                drop(file);
                fs::remove_file(&temporary).map_err(|source| {
                    io_error("remove workspace write probe", &temporary, source)
                })?;
                return Ok(());
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(source) => {
                return Err(io_error("verify workspace root is writable", root, source));
            }
        }
    }
    Err(io_error(
        "verify workspace root is writable",
        root,
        io::Error::new(io::ErrorKind::AlreadyExists, "temporary name exhaustion"),
    ))
}

fn validate_path_limits(path: &Path, path_limit: usize) -> Result<(), WorkspaceLayoutError> {
    for component in path.components() {
        if let Component::Normal(component) = component {
            let bytes = component.as_encoded_bytes().len();
            if bytes > MAX_PATH_COMPONENT_BYTES {
                return Err(WorkspaceLayoutError::PathComponentTooLong {
                    component_bytes: bytes,
                    limit: MAX_PATH_COMPONENT_BYTES,
                });
            }
        }
    }
    let path_bytes = path.as_os_str().as_encoded_bytes().len();
    if path_bytes > path_limit {
        return Err(WorkspaceLayoutError::PathTooLong {
            path: path.to_path_buf(),
            limit: path_limit,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeProbe {
        available: io::Result<u64>,
        path_limit: usize,
        same_filesystem: io::Result<Option<bool>>,
    }

    impl FakeProbe {
        fn permissive() -> Self {
            Self {
                available: Ok(u64::MAX),
                path_limit: 4_096,
                same_filesystem: Ok(Some(true)),
            }
        }
    }

    impl WorkspaceProbe for FakeProbe {
        fn available_space(&self, _path: &Path) -> io::Result<u64> {
            clone_io_result(&self.available)
        }

        fn path_limit(&self, _path: &Path) -> usize {
            self.path_limit
        }

        fn same_filesystem(&self, _left: &Path, _right: &Path) -> io::Result<Option<bool>> {
            clone_io_result(&self.same_filesystem)
        }
    }

    fn clone_io_result<T: Copy>(result: &io::Result<T>) -> io::Result<T> {
        match result {
            Ok(value) => Ok(*value),
            Err(error) => Err(io::Error::new(error.kind(), error.to_string())),
        }
    }

    fn coordinates(suffix: &str) -> WorkspaceCoordinates {
        WorkspaceCoordinates::new(
            RepositoryId::parse(format!("repo_{suffix}")).unwrap(),
            RunId::parse(format!("run_{suffix}")).unwrap(),
            AgentId::parse(format!("agt_{suffix}")).unwrap(),
        )
    }

    fn git_repository() -> tempfile::TempDir {
        let repository = tempfile::tempdir().unwrap();
        git(repository.path(), &["init", "-q"]);
        fs::write(repository.path().join("README"), "workspace layout test\n").unwrap();
        git(repository.path(), &["add", "README"]);
        git(
            repository.path(),
            &[
                "-c",
                "user.name=Ovim Test",
                "-c",
                "user.email=ovim@example.invalid",
                "commit",
                "-qm",
                "initial",
            ],
        );
        repository
    }

    fn git(repository: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(repository)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn environment_then_config_then_platform_default_precedence_is_injectable() {
        let mut config = AiSubagentWorkspaceConfig::default();
        config.root = Some(PathBuf::from("/configured/workspaces"));

        let environment = WorkspaceStorageLayout::from_locations(
            Some(OsString::from("/environment/workspaces")),
            &config,
            Some(PathBuf::from("/platform/data")),
        )
        .unwrap();
        assert_eq!(
            environment.root_candidate(),
            Path::new("/environment/workspaces")
        );
        assert_eq!(environment.origin(), WorkspaceRootOrigin::Environment);

        let configured = WorkspaceStorageLayout::from_locations(
            None,
            &config,
            Some(PathBuf::from("/platform/data")),
        )
        .unwrap();
        assert_eq!(
            configured.root_candidate(),
            Path::new("/configured/workspaces")
        );
        assert_eq!(configured.origin(), WorkspaceRootOrigin::Configuration);

        config.root = None;
        let platform = WorkspaceStorageLayout::from_locations(
            None,
            &config,
            Some(PathBuf::from("/platform/data")),
        )
        .unwrap();
        assert_eq!(
            platform.root_candidate(),
            Path::new("/platform/data/ovim/workspaces")
        );
        assert_eq!(platform.origin(), WorkspaceRootOrigin::PlatformDefault);
    }

    #[test]
    fn opaque_hostile_identities_cannot_escape_or_collide() {
        let root = Path::new("/safe/ovim/workspaces");
        let layout = WorkspaceStorageLayout::new(root, WorkspaceRootOrigin::Configuration, 0);
        let hostile = WorkspaceCoordinates::new(
            RepositoryId::parse("repo_../../same").unwrap(),
            RunId::parse("run_..\\../same").unwrap(),
            AgentId::parse("agt_task/model prose /../same").unwrap(),
        );
        let similar = WorkspaceCoordinates::new(
            RepositoryId::parse("repo_..-..-same").unwrap(),
            RunId::parse("run_..-..-same").unwrap(),
            AgentId::parse("agt_task-model-prose-same").unwrap(),
        );

        let hostile_paths = layout.candidate_paths(&hostile);
        let similar_paths = layout.candidate_paths(&similar);
        assert!(hostile_paths.checkout.starts_with(root));
        assert_eq!(hostile_paths.checkout.ancestors().nth(4), Some(root));
        assert_ne!(hostile_paths.agent_directory, similar_paths.agent_directory);
        for component in hostile_paths
            .checkout
            .strip_prefix(root)
            .unwrap()
            .components()
        {
            let Component::Normal(component) = component else {
                panic!("derived path contained traversal");
            };
            assert!(!component.to_string_lossy().contains('/'));
        }
    }

    #[test]
    fn preflight_canonicalizes_root_and_reports_locality() {
        let repository = git_repository();
        let storage = tempfile::tempdir().unwrap();
        let requested = storage.path().join("nested").join("workspaces");
        let layout = WorkspaceStorageLayout::new(&requested, WorkspaceRootOrigin::Configuration, 1);

        let result = layout
            .preflight_with(
                repository.path(),
                &coordinates("canonical"),
                &FakeProbe::permissive(),
            )
            .unwrap();

        assert_eq!(result.layout.root(), fs::canonicalize(requested).unwrap());
        assert_eq!(result.same_filesystem_as_source, Some(true));
        assert!(!result.repo_adjacent);
        assert!(result
            .registered_worktrees
            .iter()
            .any(|worktree| worktree.path == fs::canonicalize(repository.path()).unwrap()));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_override_is_validated_at_its_target() {
        use std::os::unix::fs::symlink;
        let repository = git_repository();
        let inside = repository.path().join("agent-storage");
        fs::create_dir(&inside).unwrap();
        let links = tempfile::tempdir().unwrap();
        let link = links.path().join("workspaces");
        symlink(&inside, &link).unwrap();
        let layout = WorkspaceStorageLayout::new(link, WorkspaceRootOrigin::Configuration, 0);

        let error = layout
            .preflight_with(
                repository.path(),
                &coordinates("symlink"),
                &FakeProbe::permissive(),
            )
            .unwrap_err();
        assert!(matches!(error, WorkspaceLayoutError::UnsafeRoot { .. }));
    }

    #[test]
    fn source_and_git_administration_roots_are_rejected() {
        let repository = git_repository();
        for root in [
            repository.path().to_path_buf(),
            repository.path().join("nested"),
            repository.path().join(".git").join("ovim-workspaces"),
        ] {
            let layout = WorkspaceStorageLayout::new(root, WorkspaceRootOrigin::Configuration, 0);
            assert!(matches!(
                layout.preflight_with(
                    repository.path(),
                    &coordinates("source-rejection"),
                    &FakeProbe::permissive()
                ),
                Err(WorkspaceLayoutError::UnsafeRoot { .. })
            ));
        }
    }

    #[test]
    fn any_registered_worktree_root_is_rejected() {
        let repository = git_repository();
        let linked_parent = tempfile::tempdir().unwrap();
        let linked = linked_parent.path().join("linked");
        git(
            repository.path(),
            &[
                "worktree",
                "add",
                "-q",
                "--detach",
                linked.to_str().unwrap(),
            ],
        );
        let layout = WorkspaceStorageLayout::new(
            linked.join("storage"),
            WorkspaceRootOrigin::Configuration,
            0,
        );

        let error = layout
            .preflight_with(
                repository.path(),
                &coordinates("registered"),
                &FakeProbe::permissive(),
            )
            .unwrap_err();
        assert!(matches!(error, WorkspaceLayoutError::UnsafeRoot { .. }));
    }

    #[test]
    fn repo_adjacent_layout_requires_explicit_override() {
        let repository = git_repository();
        let source = fs::canonicalize(repository.path()).unwrap();
        let adjacent = source.parent().unwrap().join(format!(
            "{}-worktrees",
            source.file_name().unwrap().to_string_lossy()
        ));

        let implicit =
            WorkspaceStorageLayout::new(&adjacent, WorkspaceRootOrigin::PlatformDefault, 0);
        assert!(matches!(
            implicit.preflight_with(
                repository.path(),
                &coordinates("implicit-adjacent"),
                &FakeProbe::permissive()
            ),
            Err(WorkspaceLayoutError::UnsafeRoot { .. })
        ));

        let explicit =
            WorkspaceStorageLayout::new(&adjacent, WorkspaceRootOrigin::Configuration, 0);
        let result = explicit
            .preflight_with(
                repository.path(),
                &coordinates("explicit-adjacent"),
                &FakeProbe::permissive(),
            )
            .unwrap();
        assert!(result.repo_adjacent);
    }

    #[test]
    fn low_disk_and_path_limits_fail_before_allocation() {
        let repository = git_repository();
        let storage = tempfile::tempdir().unwrap();
        let layout = WorkspaceStorageLayout::new(
            storage.path().join("workspaces"),
            WorkspaceRootOrigin::Configuration,
            2,
        );
        let low_disk = FakeProbe {
            available: Ok(MEBIBYTE),
            ..FakeProbe::permissive()
        };
        assert!(matches!(
            layout.preflight_with(repository.path(), &coordinates("disk"), &low_disk),
            Err(WorkspaceLayoutError::LowDiskSpace { .. })
        ));

        let short_path = FakeProbe {
            path_limit: 10,
            ..FakeProbe::permissive()
        };
        assert!(matches!(
            layout.preflight_with(repository.path(), &coordinates("path"), &short_path),
            Err(WorkspaceLayoutError::PathTooLong { .. })
        ));
    }

    #[test]
    fn path_collision_is_never_reused() {
        let repository = git_repository();
        let storage = tempfile::tempdir().unwrap();
        let coordinates = coordinates("collision");
        let layout = WorkspaceStorageLayout::new(
            storage.path().join("workspaces"),
            WorkspaceRootOrigin::Configuration,
            0,
        );
        let paths = layout.candidate_paths(&coordinates);
        fs::create_dir_all(&paths.agent_directory).unwrap();

        assert!(matches!(
            layout.preflight_with(repository.path(), &coordinates, &FakeProbe::permissive()),
            Err(WorkspaceLayoutError::WorkspaceCollision { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn prepared_root_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let repository = git_repository();
        let storage = tempfile::tempdir().unwrap();
        let layout = WorkspaceStorageLayout::new(
            storage.path().join("workspaces"),
            WorkspaceRootOrigin::Configuration,
            0,
        );
        let result = layout
            .preflight_with(
                repository.path(),
                &coordinates("permissions"),
                &FakeProbe::permissive(),
            )
            .unwrap();

        assert_eq!(
            fs::metadata(result.layout.root())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }

    #[test]
    fn non_directory_ancestor_fails_writability_preflight() {
        let repository = git_repository();
        let storage = tempfile::tempdir().unwrap();
        let blocker = storage.path().join("not-a-directory");
        fs::write(&blocker, "blocked").unwrap();
        let layout = WorkspaceStorageLayout::new(
            blocker.join("workspaces"),
            WorkspaceRootOrigin::Configuration,
            0,
        );

        assert!(matches!(
            layout.preflight_with(
                repository.path(),
                &coordinates("writability"),
                &FakeProbe::permissive()
            ),
            Err(WorkspaceLayoutError::Io { .. })
        ));
    }
}
