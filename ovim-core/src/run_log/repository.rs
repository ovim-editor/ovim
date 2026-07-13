use super::{BlobId, RepositoryId};
use git2::{ErrorCode, Repository};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// A durable anchor for the repository state at the start of a run.
///
/// `repository_id` is an ovim identity supplied by the caller. It must not be
/// derived solely from a remote URL: repositories without remotes and distinct
/// local clones are both valid identities.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositorySnapshot {
    pub repository_id: RepositoryId,
    pub local_paths: LocalRepositoryPaths,
    pub head: RepositoryHead,
    pub remote: Option<SanitizedRemote>,
}

impl RepositorySnapshot {
    /// Discovers the containing repository from `start` and captures its
    /// current identity and HEAD state without changing repository state.
    pub fn capture(
        start: impl AsRef<Path>,
        repository_id: RepositoryId,
    ) -> Result<Self, RepositorySnapshotError> {
        let start = start.as_ref();
        let repository = Repository::discover(start).map_err(|error| {
            if error.code() == ErrorCode::NotFound {
                RepositorySnapshotError::NotRepository {
                    start: start.to_owned(),
                }
            } else {
                RepositorySnapshotError::Git(error)
            }
        })?;

        let git_dir = canonicalize_existing(repository.path());
        let workdir = repository.workdir().map(canonicalize_existing);
        let common_git_dir = discover_common_git_dir(&git_dir)?;
        let head = capture_head(&repository)?;
        let remote = capture_remote(&repository)?;

        Ok(Self {
            repository_id,
            local_paths: LocalRepositoryPaths {
                workdir,
                git_dir,
                common_git_dir,
            },
            head,
            remote,
        })
    }
}

/// Machine-local paths used to re-open a run. These are useful in the local
/// store but must be omitted or explicitly scrubbed from exported bundles.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalRepositoryPaths {
    pub workdir: Option<PathBuf>,
    pub git_dir: PathBuf,
    pub common_git_dir: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryHead {
    pub oid: Option<String>,
    /// The full symbolic reference, such as `refs/heads/main`, when present.
    pub reference: Option<String>,
    pub branch: Option<String>,
    pub detached: bool,
    pub unborn: bool,
}

/// Credential-free remote identity suitable for persistence and display.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SanitizedRemote {
    pub display: String,
    /// SHA-256 of `display`, so credentials and incidental query parameters do
    /// not affect identity or leak through a stable fingerprint.
    pub fingerprint: BlobId,
}

#[derive(Debug)]
pub enum RepositorySnapshotError {
    NotRepository { start: PathBuf },
    Git(git2::Error),
    Io(io::Error),
}

impl fmt::Display for RepositorySnapshotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotRepository { start } => {
                write!(
                    formatter,
                    "{} is not inside a Git repository",
                    start.display()
                )
            }
            Self::Git(error) => write!(formatter, "could not capture Git repository: {error}"),
            Self::Io(error) => write!(formatter, "could not inspect Git repository: {error}"),
        }
    }
}

impl std::error::Error for RepositorySnapshotError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Git(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::NotRepository { .. } => None,
        }
    }
}

impl From<git2::Error> for RepositorySnapshotError {
    fn from(error: git2::Error) -> Self {
        Self::Git(error)
    }
}

impl From<io::Error> for RepositorySnapshotError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

fn capture_head(repository: &Repository) -> Result<RepositoryHead, RepositorySnapshotError> {
    let detached = repository.head_detached()?;
    let raw_head = repository.find_reference("HEAD")?;
    let symbolic_reference = raw_head.symbolic_target().map(ToOwned::to_owned);

    let (oid, unborn) = match repository.head() {
        Ok(head) => (head.target().map(|oid| oid.to_string()), false),
        Err(error) if matches!(error.code(), ErrorCode::UnbornBranch | ErrorCode::NotFound) => {
            (None, true)
        }
        Err(error) => return Err(error.into()),
    };

    let branch = symbolic_reference
        .as_deref()
        .and_then(|name| name.strip_prefix("refs/heads/"))
        .map(ToOwned::to_owned);

    Ok(RepositoryHead {
        oid,
        reference: symbolic_reference,
        branch,
        detached,
        unborn,
    })
}

fn capture_remote(
    repository: &Repository,
) -> Result<Option<SanitizedRemote>, RepositorySnapshotError> {
    let selected_name = if repository.find_remote("origin").is_ok() {
        Some("origin".to_owned())
    } else {
        let mut names: Vec<_> = repository
            .remotes()?
            .iter()
            .flatten()
            .map(ToOwned::to_owned)
            .collect();
        names.sort();
        names.into_iter().next()
    };

    let Some(name) = selected_name else {
        return Ok(None);
    };
    let remote = repository.find_remote(&name)?;
    let Some(raw_url) = remote.url() else {
        return Ok(None);
    };
    let display = sanitize_remote_url(raw_url);
    if display.is_empty() {
        return Ok(None);
    }
    let fingerprint = BlobId::digest(display.as_bytes());
    Ok(Some(SanitizedRemote {
        display,
        fingerprint,
    }))
}

fn sanitize_remote_url(raw: &str) -> String {
    if raw.contains("://") {
        if let Ok(mut parsed) = url::Url::parse(raw) {
            // Hierarchical URLs (https, ssh, and file) are safest to sanitize via
            // their parsed components. Never fall back to the raw URL after seeing
            // userinfo.
            let had_userinfo = !parsed.username().is_empty() || parsed.password().is_some();
            let username_removed = parsed.set_username("").is_ok();
            let password_removed = parsed.set_password(None).is_ok();
            parsed.set_query(None);
            parsed.set_fragment(None);
            if !had_userinfo || (username_removed && password_removed) {
                return parsed.to_string();
            }
        }
    }

    sanitize_scp_like(raw)
}

fn sanitize_scp_like(raw: &str) -> String {
    let suffix = raw
        .char_indices()
        .find_map(|(index, character)| matches!(character, '?' | '#').then_some(index));
    let without_suffix = suffix.map_or(raw, |index| &raw[..index]);
    if let Some((scheme, remainder)) = without_suffix.split_once("://") {
        let (authority, path) = remainder
            .find('/')
            .map_or((remainder, ""), |index| remainder.split_at(index));
        let host = authority
            .rsplit_once('@')
            .map_or(authority, |(_, host)| host);
        return format!("{scheme}://{host}{path}");
    }
    if !without_suffix.contains("://") {
        if let Some((_, host_and_path)) = without_suffix.rsplit_once('@') {
            if host_and_path.contains(':') {
                return host_and_path.to_owned();
            }
        }
    }
    without_suffix.to_owned()
}

fn canonicalize_existing(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_owned())
}

fn discover_common_git_dir(git_dir: &Path) -> Result<Option<PathBuf>, RepositorySnapshotError> {
    let marker = git_dir.join("commondir");
    let common = match fs::read_to_string(marker) {
        Ok(value) => {
            let value = value.trim();
            if value.is_empty() {
                return Ok(None);
            }
            let path = Path::new(value);
            if path.is_absolute() {
                path.to_owned()
            } else {
                git_dir.join(path)
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => git_dir.to_owned(),
        Err(error) => return Err(error.into()),
    };
    Ok(Some(canonicalize_existing(&common)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use git2::{Oid, Signature};

    fn repository_id() -> RepositoryId {
        RepositoryId::parse("repo_test").unwrap()
    }

    fn commit(repository: &Repository, message: &str) -> Oid {
        let workdir = repository.workdir().unwrap();
        fs::write(workdir.join("file.txt"), message).unwrap();
        let mut index = repository.index().unwrap();
        index.add_path(Path::new("file.txt")).unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repository.find_tree(tree_id).unwrap();
        let signature = Signature::now("ovim test", "ovim@example.invalid").unwrap();
        let parents: Vec<_> = repository
            .head()
            .ok()
            .and_then(|head| head.target())
            .map(|oid| repository.find_commit(oid).unwrap())
            .into_iter()
            .collect();
        let parent_refs: Vec<_> = parents.iter().collect();
        repository
            .commit(
                Some("HEAD"),
                &signature,
                &signature,
                message,
                &tree,
                &parent_refs,
            )
            .unwrap()
    }

    #[test]
    fn captures_normal_branch_and_local_paths() {
        let directory = tempfile::tempdir().unwrap();
        let repository = Repository::init(directory.path()).unwrap();
        repository.set_head("refs/heads/main").unwrap();
        let oid = commit(&repository, "initial");
        let oid_string = oid.to_string();

        let snapshot = RepositorySnapshot::capture(directory.path(), repository_id()).unwrap();
        assert_eq!(snapshot.head.oid.as_deref(), Some(oid_string.as_str()));
        assert_eq!(snapshot.head.branch.as_deref(), Some("main"));
        assert_eq!(snapshot.head.reference.as_deref(), Some("refs/heads/main"));
        assert!(!snapshot.head.detached);
        assert!(!snapshot.head.unborn);
        assert_eq!(
            snapshot.local_paths.workdir,
            Some(canonicalize_existing(directory.path()))
        );
        assert_eq!(
            snapshot.local_paths.git_dir,
            canonicalize_existing(repository.path())
        );
        assert_eq!(
            snapshot.local_paths.common_git_dir,
            Some(snapshot.local_paths.git_dir.clone())
        );
    }

    #[test]
    fn captures_detached_head() {
        let directory = tempfile::tempdir().unwrap();
        let repository = Repository::init(directory.path()).unwrap();
        repository.set_head("refs/heads/main").unwrap();
        let oid = commit(&repository, "initial");
        repository.set_head_detached(oid).unwrap();
        let oid_string = oid.to_string();

        let snapshot = RepositorySnapshot::capture(directory.path(), repository_id()).unwrap();
        assert_eq!(snapshot.head.oid.as_deref(), Some(oid_string.as_str()));
        assert!(snapshot.head.detached);
        assert!(!snapshot.head.unborn);
        assert_eq!(snapshot.head.branch, None);
        assert_eq!(snapshot.head.reference, None);
    }

    #[test]
    fn captures_unborn_branch() {
        let directory = tempfile::tempdir().unwrap();
        let repository = Repository::init(directory.path()).unwrap();
        repository.set_head("refs/heads/main").unwrap();

        let snapshot = RepositorySnapshot::capture(directory.path(), repository_id()).unwrap();
        assert!(snapshot.head.unborn);
        assert!(!snapshot.head.detached);
        assert_eq!(snapshot.head.oid, None);
        assert_eq!(snapshot.head.branch.as_deref(), Some("main"));
    }

    #[test]
    fn credentialed_remote_is_redacted_and_fingerprint_is_stable() {
        let directory = tempfile::tempdir().unwrap();
        let repository = Repository::init(directory.path()).unwrap();
        repository
            .remote(
                "origin",
                "https://alice:secret@example.com/org/repo.git?token=first#fragment",
            )
            .unwrap();

        let first = RepositorySnapshot::capture(directory.path(), repository_id())
            .unwrap()
            .remote
            .unwrap();
        assert_eq!(first.display, "https://example.com/org/repo.git");
        assert!(!first.display.contains("alice"));
        assert!(!first.display.contains("secret"));
        assert!(!first.display.contains("token"));

        repository
            .remote_set_url(
                "origin",
                "https://bob:different@example.com/org/repo.git?token=second",
            )
            .unwrap();
        let second = RepositorySnapshot::capture(directory.path(), repository_id())
            .unwrap()
            .remote
            .unwrap();
        assert_eq!(second.display, first.display);
        assert_eq!(second.fingerprint, first.fingerprint);
    }

    #[test]
    fn strips_scp_and_ssh_userinfo() {
        assert_eq!(
            sanitize_remote_url("git@github.com:openai/codex.git"),
            "github.com:openai/codex.git"
        );
        assert_eq!(
            sanitize_remote_url("ssh://git:secret@example.com/openai/codex.git?x=1#y"),
            "ssh://example.com/openai/codex.git"
        );
    }

    #[test]
    fn reports_non_repository_as_a_typed_error() {
        let directory = tempfile::tempdir().unwrap();
        let error = RepositorySnapshot::capture(directory.path(), repository_id()).unwrap_err();
        assert!(matches!(
            error,
            RepositorySnapshotError::NotRepository { .. }
        ));
    }
}
