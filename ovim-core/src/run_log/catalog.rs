use super::{
    AgentId, BranchId, ConversationId, RepositoryId, RunId, RunLogError, RunStorageLayout,
    WorkspaceId,
};
use rusqlite::{params, Connection, ErrorCode, OpenFlags, OptionalExtension, TransactionBehavior};
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const CATALOG_DATABASE: &str = "index.sqlite";
const LATEST_MIGRATION: u32 = 1;
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

pub trait CatalogClock: Send + Sync {
    fn now_millis(&self) -> i64;
}

#[derive(Default)]
pub struct SystemCatalogClock;

impl CatalogClock for SystemCatalogClock {
    fn now_millis(&self) -> i64 {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        i64::try_from(millis).unwrap_or(i64::MAX)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositoryRegistration {
    /// Canonical Git common directory. Linked worktrees of one repository
    /// share this value; independent clones do not, even with the same remote.
    pub common_git_dir: PathBuf,
    /// Canonical worktree roots and other caller-established aliases.
    pub worktree_aliases: Vec<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositoryRecord {
    pub repository_id: RepositoryId,
    pub common_git_dir: PathBuf,
    pub worktree_aliases: Vec<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ConversationScope {
    NoFile,
    RepositoryPath(PathBuf),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ConversationKey {
    pub repository_id: RepositoryId,
    pub scope: ConversationScope,
    pub logical_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversationBinding {
    pub key: ConversationKey,
    pub conversation_id: ConversationId,
    pub run_id: RunId,
    pub root_agent_id: AgentId,
    pub workspace_id: WorkspaceId,
    pub selected_branch_id: BranchId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeaseOwner {
    /// Opaque identity generated once per live ovim process. PID is not an
    /// identity because operating systems reuse it.
    pub instance_id: String,
    pub pid_marker: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunLease {
    pub run_id: RunId,
    pub owner: LeaseOwner,
    pub heartbeat_at_millis: i64,
    pub expires_at_millis: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LeaseStatus {
    Missing,
    Active(RunLease),
    Stale(RunLease),
}

#[derive(Debug)]
pub enum CatalogError {
    RunLog(RunLogError),
    InvalidRepositoryPath(String),
    InvalidConversationKey(String),
    RepositoryAliasConflict,
    UnknownRepository(RepositoryId),
    LeaseHeld(RunLease),
    LeaseNotOwned(RunLease),
    InvalidLeaseDuration,
}

impl fmt::Display for CatalogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RunLog(error) => write!(formatter, "catalog storage: {error}"),
            Self::InvalidRepositoryPath(detail) => {
                write!(formatter, "invalid repository path: {detail}")
            }
            Self::InvalidConversationKey(detail) => {
                write!(formatter, "invalid conversation key: {detail}")
            }
            Self::RepositoryAliasConflict => {
                formatter.write_str("repository aliases identify different repositories")
            }
            Self::UnknownRepository(id) => write!(formatter, "unknown repository {id}"),
            Self::LeaseHeld(lease) => write!(
                formatter,
                "run {} is leased by {} until {}",
                lease.run_id, lease.owner.instance_id, lease.expires_at_millis
            ),
            Self::LeaseNotOwned(lease) => write!(
                formatter,
                "run {} lease belongs to {}",
                lease.run_id, lease.owner.instance_id
            ),
            Self::InvalidLeaseDuration => {
                formatter.write_str("lease duration must be positive and fit in milliseconds")
            }
        }
    }
}

impl std::error::Error for CatalogError {}

impl From<RunLogError> for CatalogError {
    fn from(value: RunLogError) -> Self {
        Self::RunLog(value)
    }
}

pub struct RunCatalog {
    connection: Mutex<Connection>,
    clock: Arc<dyn CatalogClock>,
}

impl RunCatalog {
    pub fn open(layout: &RunStorageLayout) -> Result<Self, CatalogError> {
        Self::open_with_clock(layout, Arc::new(SystemCatalogClock))
    }

    pub fn open_with_clock(
        layout: &RunStorageLayout,
        clock: Arc<dyn CatalogClock>,
    ) -> Result<Self, CatalogError> {
        layout.ensure_root()?;
        let path = layout.root().join(CATALOG_DATABASE);
        let mut connection = Connection::open_with_flags(
            &path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|error| storage("open catalog", error))?;
        connection
            .busy_timeout(BUSY_TIMEOUT)
            .map_err(|error| storage("set catalog busy timeout", error))?;
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .map_err(|error| storage("enable catalog foreign keys", error))?;
        configure_wal(&connection)?;
        connection
            .pragma_update(None, "synchronous", "FULL")
            .map_err(|error| storage("enable catalog full synchronization", error))?;
        migrate(&mut connection)?;
        set_owner_only(&path)?;
        Ok(Self {
            connection: Mutex::new(connection),
            clock,
        })
    }

    fn connection(&self) -> Result<MutexGuard<'_, Connection>, CatalogError> {
        self.connection
            .lock()
            .map_err(|_| CatalogError::RunLog(RunLogError::Poisoned))
    }

    pub fn register_repository(
        &self,
        registration: RepositoryRegistration,
    ) -> Result<RepositoryRecord, CatalogError> {
        let common = path_key(&registration.common_git_dir)?;
        let mut aliases = registration
            .worktree_aliases
            .iter()
            .map(|path| path_key(path))
            .collect::<Result<Vec<_>, _>>()?;
        aliases.sort();
        aliases.dedup();
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| storage("begin repository registration", error))?;

        let mut candidates = Vec::new();
        if let Some(id) = transaction
            .query_row(
                "SELECT repository_id FROM repositories WHERE common_git_dir = ?1",
                [&common],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| storage("resolve repository common directory", error))?
        {
            candidates.push(id);
        }
        for alias in &aliases {
            if let Some(id) = transaction
                .query_row(
                    "SELECT repository_id FROM repository_aliases WHERE alias_path = ?1",
                    [alias],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|error| storage("resolve repository alias", error))?
            {
                candidates.push(id);
            }
        }
        candidates.sort();
        candidates.dedup();
        if candidates.len() > 1 {
            return Err(CatalogError::RepositoryAliasConflict);
        }
        let repository_id = match candidates.pop() {
            Some(value) => parse_repository_id(value)?,
            None => {
                let id = RepositoryId::new();
                transaction
                    .execute(
                        "INSERT INTO repositories(repository_id, common_git_dir, created_at_millis) VALUES (?1, ?2, ?3)",
                        params![id.as_str(), common, self.clock.now_millis()],
                    )
                    .map_err(|error| storage("insert repository", error))?;
                id
            }
        };
        for alias in &aliases {
            transaction
                .execute(
                    "INSERT INTO repository_aliases(alias_path, repository_id) VALUES (?1, ?2) \
                     ON CONFLICT(alias_path) DO UPDATE SET repository_id = excluded.repository_id \
                     WHERE repository_aliases.repository_id = excluded.repository_id",
                    params![alias, repository_id.as_str()],
                )
                .map_err(|error| storage("register repository alias", error))?;
        }
        transaction
            .commit()
            .map_err(|error| storage("commit repository registration", error))?;
        drop(connection);
        self.repository(&repository_id)?.ok_or_else(|| {
            CatalogError::RunLog(RunLogError::Corruption {
                detail: format!("registered repository {repository_id} disappeared"),
            })
        })
    }

    pub fn repository(
        &self,
        repository_id: &RepositoryId,
    ) -> Result<Option<RepositoryRecord>, CatalogError> {
        let connection = self.connection()?;
        let common: Option<Vec<u8>> = connection
            .query_row(
                "SELECT common_git_dir FROM repositories WHERE repository_id = ?1",
                [repository_id.as_str()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| storage("read repository", error))?;
        let Some(common) = common else {
            return Ok(None);
        };
        let mut statement = connection
            .prepare("SELECT alias_path FROM repository_aliases WHERE repository_id = ?1 ORDER BY alias_path")
            .map_err(|error| storage("prepare repository alias read", error))?;
        let aliases = statement
            .query_map([repository_id.as_str()], |row| row.get::<_, Vec<u8>>(0))
            .map_err(|error| storage("read repository aliases", error))?
            .map(|row| {
                row.map(bytes_path)
                    .map_err(|error| storage("decode repository alias", error))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Some(RepositoryRecord {
            repository_id: repository_id.clone(),
            common_git_dir: bytes_path(common),
            worktree_aliases: aliases,
        }))
    }

    pub fn open_conversation(
        &self,
        key: ConversationKey,
        initial_branch_id: BranchId,
    ) -> Result<ConversationBinding, CatalogError> {
        let (scope_kind, scope_path) = normalized_scope(&key.scope)?;
        if key.logical_name.trim().is_empty() {
            return Err(CatalogError::InvalidConversationKey(
                "logical name is empty".into(),
            ));
        }
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| storage("begin conversation open", error))?;
        let exists: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM repositories WHERE repository_id = ?1)",
                [key.repository_id.as_str()],
                |row| row.get(0),
            )
            .map_err(|error| storage("validate conversation repository", error))?;
        if !exists {
            return Err(CatalogError::UnknownRepository(key.repository_id));
        }
        let existing = read_binding(&transaction, &key, scope_kind, &scope_path)?;
        let binding = match existing {
            Some(binding) => binding,
            None => {
                let binding = ConversationBinding {
                    key: key.clone(),
                    conversation_id: ConversationId::new(),
                    run_id: RunId::new(),
                    root_agent_id: AgentId::new(),
                    workspace_id: WorkspaceId::new(),
                    selected_branch_id: initial_branch_id,
                };
                transaction
                    .execute(
                        "INSERT INTO conversation_bindings(\
                         repository_id, scope_kind, scope_path, logical_name, conversation_id, \
                         run_id, root_agent_id, workspace_id, selected_branch_id, updated_at_millis) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                        params![
                            binding.key.repository_id.as_str(), scope_kind, scope_path,
                            binding.key.logical_name, binding.conversation_id.as_str(),
                            binding.run_id.as_str(), binding.root_agent_id.as_str(),
                            binding.workspace_id.as_str(), binding.selected_branch_id.as_str(),
                            self.clock.now_millis(),
                        ],
                    )
                    .map_err(|error| storage("insert conversation binding", error))?;
                binding
            }
        };
        transaction
            .commit()
            .map_err(|error| storage("commit conversation open", error))?;
        Ok(binding)
    }

    pub fn update_selected_branch(
        &self,
        key: &ConversationKey,
        branch_id: BranchId,
    ) -> Result<Option<ConversationBinding>, CatalogError> {
        let (scope_kind, scope_path) = normalized_scope(&key.scope)?;
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| storage("begin selected branch update", error))?;
        let changed = transaction
            .execute(
                "UPDATE conversation_bindings SET selected_branch_id = ?5, updated_at_millis = ?6 \
                 WHERE repository_id = ?1 AND scope_kind = ?2 AND scope_path = ?3 AND logical_name = ?4",
                params![key.repository_id.as_str(), scope_kind, scope_path, key.logical_name, branch_id.as_str(), self.clock.now_millis()],
            )
            .map_err(|error| storage("update selected branch", error))?;
        if changed == 0 {
            return Ok(None);
        }
        let binding = read_binding(&transaction, key, scope_kind, &scope_path)?;
        transaction
            .commit()
            .map_err(|error| storage("commit selected branch update", error))?;
        Ok(binding)
    }

    pub fn acquire_lease(
        &self,
        run_id: RunId,
        owner: LeaseOwner,
        duration: Duration,
    ) -> Result<RunLease, CatalogError> {
        validate_owner(&owner)?;
        let now = self.clock.now_millis();
        let expires = lease_expiry(now, duration)?;
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| storage("begin lease acquisition", error))?;
        if let Some(current) = read_lease(&transaction, &run_id)? {
            if current.expires_at_millis > now && current.owner.instance_id != owner.instance_id {
                return Err(CatalogError::LeaseHeld(current));
            }
        }
        transaction
            .execute(
                "INSERT INTO run_leases(run_id, instance_id, pid_marker, heartbeat_at_millis, expires_at_millis) \
                 VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(run_id) DO UPDATE SET \
                 instance_id=excluded.instance_id, pid_marker=excluded.pid_marker, \
                 heartbeat_at_millis=excluded.heartbeat_at_millis, expires_at_millis=excluded.expires_at_millis",
                params![run_id.as_str(), owner.instance_id, owner.pid_marker, now, expires],
            )
            .map_err(|error| storage("acquire run lease", error))?;
        transaction
            .commit()
            .map_err(|error| storage("commit lease acquisition", error))?;
        Ok(RunLease {
            run_id,
            owner,
            heartbeat_at_millis: now,
            expires_at_millis: expires,
        })
    }

    pub fn renew_lease(
        &self,
        run_id: &RunId,
        owner: &LeaseOwner,
        duration: Duration,
    ) -> Result<RunLease, CatalogError> {
        validate_owner(owner)?;
        let now = self.clock.now_millis();
        let expires = lease_expiry(now, duration)?;
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| storage("begin lease renewal", error))?;
        if let Some(current) = read_lease(&transaction, run_id)? {
            if current.owner.instance_id != owner.instance_id {
                return Err(CatalogError::LeaseNotOwned(current));
            }
        } else {
            return Err(CatalogError::InvalidConversationKey(
                "run has no lease".into(),
            ));
        }
        let changed = transaction
            .execute(
                "UPDATE run_leases SET pid_marker=?3, heartbeat_at_millis=?4, expires_at_millis=?5 \
                 WHERE run_id=?1 AND instance_id=?2",
                params![run_id.as_str(), owner.instance_id, owner.pid_marker, now, expires],
            )
            .map_err(|error| storage("renew run lease", error))?;
        if changed != 1 {
            return Err(CatalogError::RunLog(RunLogError::Corruption {
                detail: format!("lease for run {run_id} disappeared during renewal"),
            }));
        }
        transaction
            .commit()
            .map_err(|error| storage("commit lease renewal", error))?;
        Ok(RunLease {
            run_id: run_id.clone(),
            owner: owner.clone(),
            heartbeat_at_millis: now,
            expires_at_millis: expires,
        })
    }

    pub fn release_lease(&self, run_id: &RunId, owner: &LeaseOwner) -> Result<bool, CatalogError> {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| storage("begin lease release", error))?;
        if let Some(current) = read_lease(&transaction, run_id)? {
            if current.owner.instance_id != owner.instance_id {
                return Err(CatalogError::LeaseNotOwned(current));
            }
        } else {
            return Ok(false);
        }
        let changed = transaction
            .execute(
                "DELETE FROM run_leases WHERE run_id=?1 AND instance_id=?2",
                params![run_id.as_str(), owner.instance_id],
            )
            .map_err(|error| storage("release run lease", error))?;
        if changed != 1 {
            return Err(CatalogError::RunLog(RunLogError::Corruption {
                detail: format!("lease for run {run_id} disappeared during release"),
            }));
        }
        transaction
            .commit()
            .map_err(|error| storage("commit lease release", error))?;
        Ok(true)
    }

    pub fn lease_status(&self, run_id: &RunId) -> Result<LeaseStatus, CatalogError> {
        let connection = self.connection()?;
        Ok(match read_lease(&connection, run_id)? {
            None => LeaseStatus::Missing,
            Some(lease) if lease.expires_at_millis <= self.clock.now_millis() => {
                LeaseStatus::Stale(lease)
            }
            Some(lease) => LeaseStatus::Active(lease),
        })
    }
}

fn read_binding(
    connection: &Connection,
    key: &ConversationKey,
    scope_kind: i64,
    scope_path: &[u8],
) -> Result<Option<ConversationBinding>, CatalogError> {
    let row: Option<(String, String, String, String, String)> = connection
        .query_row(
            "SELECT conversation_id, run_id, root_agent_id, workspace_id, selected_branch_id \
             FROM conversation_bindings WHERE repository_id=?1 AND scope_kind=?2 AND scope_path=?3 AND logical_name=?4",
            params![key.repository_id.as_str(), scope_kind, scope_path, key.logical_name],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )
        .optional()
        .map_err(|error| storage("read conversation binding", error))?;
    row.map(|(conversation, run, agent, workspace, branch)| {
        Ok(ConversationBinding {
            key: key.clone(),
            conversation_id: ConversationId::parse(conversation).map_err(corrupt_id)?,
            run_id: RunId::parse(run).map_err(corrupt_id)?,
            root_agent_id: AgentId::parse(agent).map_err(corrupt_id)?,
            workspace_id: WorkspaceId::parse(workspace).map_err(corrupt_id)?,
            selected_branch_id: BranchId::parse(branch).map_err(corrupt_id)?,
        })
    })
    .transpose()
}

fn read_lease(connection: &Connection, run_id: &RunId) -> Result<Option<RunLease>, CatalogError> {
    connection
        .query_row(
            "SELECT instance_id, pid_marker, heartbeat_at_millis, expires_at_millis FROM run_leases WHERE run_id=?1",
            [run_id.as_str()],
            |row| Ok(RunLease {
                run_id: run_id.clone(),
                owner: LeaseOwner { instance_id: row.get(0)?, pid_marker: row.get(1)? },
                heartbeat_at_millis: row.get(2)?,
                expires_at_millis: row.get(3)?,
            }),
        )
        .optional()
        .map_err(|error| storage("read run lease", error))
}

fn normalized_scope(scope: &ConversationScope) -> Result<(i64, Vec<u8>), CatalogError> {
    match scope {
        ConversationScope::NoFile => Ok((0, Vec::new())),
        ConversationScope::RepositoryPath(path) => {
            if path.as_os_str().is_empty() || path.is_absolute() {
                return Err(CatalogError::InvalidConversationKey(
                    "repository path must be non-empty and relative".into(),
                ));
            }
            let mut normalized = PathBuf::new();
            for component in path.components() {
                match component {
                    Component::Normal(value) => normalized.push(value),
                    Component::CurDir => {}
                    Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                        return Err(CatalogError::InvalidConversationKey(
                            "repository path may not escape its root".into(),
                        ));
                    }
                }
            }
            if normalized.as_os_str().is_empty() {
                return Err(CatalogError::InvalidConversationKey(
                    "repository path normalizes to empty".into(),
                ));
            }
            Ok((1, os_bytes(normalized.as_os_str())))
        }
    }
}

fn path_key(path: &Path) -> Result<Vec<u8>, CatalogError> {
    if path.as_os_str().is_empty() || !path.is_absolute() {
        return Err(CatalogError::InvalidRepositoryPath(
            "caller must supply a non-empty canonical absolute path".into(),
        ));
    }
    Ok(os_bytes(path.as_os_str()))
}

#[cfg(unix)]
fn os_bytes(value: &std::ffi::OsStr) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    value.as_bytes().to_vec()
}

#[cfg(not(unix))]
fn os_bytes(value: &std::ffi::OsStr) -> Vec<u8> {
    value.to_string_lossy().as_bytes().to_vec()
}

#[cfg(unix)]
fn bytes_path(value: Vec<u8>) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;
    PathBuf::from(std::ffi::OsString::from_vec(value))
}

#[cfg(not(unix))]
fn bytes_path(value: Vec<u8>) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(&value).into_owned())
}

fn validate_owner(owner: &LeaseOwner) -> Result<(), CatalogError> {
    if owner.instance_id.trim().is_empty() {
        Err(CatalogError::InvalidConversationKey(
            "lease instance identity is empty".into(),
        ))
    } else {
        Ok(())
    }
}

fn lease_expiry(now: i64, duration: Duration) -> Result<i64, CatalogError> {
    let millis =
        i64::try_from(duration.as_millis()).map_err(|_| CatalogError::InvalidLeaseDuration)?;
    if millis <= 0 {
        return Err(CatalogError::InvalidLeaseDuration);
    }
    now.checked_add(millis)
        .ok_or(CatalogError::InvalidLeaseDuration)
}

fn migrate(connection: &mut Connection) -> Result<(), CatalogError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| storage("begin catalog migration", error))?;
    let current: u32 = transaction
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|error| migration(0, error))?;
    if current > LATEST_MIGRATION {
        return Err(CatalogError::RunLog(RunLogError::Migration {
            version: current,
            detail: format!("catalog schema is newer than supported version {LATEST_MIGRATION}"),
        }));
    }
    if current < 1 {
        transaction.execute_batch(
            "CREATE TABLE repositories(\
                 repository_id TEXT PRIMARY KEY, common_git_dir BLOB NOT NULL UNIQUE, created_at_millis INTEGER NOT NULL);\
             CREATE TABLE repository_aliases(\
                 alias_path BLOB PRIMARY KEY, repository_id TEXT NOT NULL REFERENCES repositories(repository_id) ON DELETE CASCADE);\
             CREATE INDEX repository_aliases_repository ON repository_aliases(repository_id);\
             CREATE TABLE conversation_bindings(\
                 repository_id TEXT NOT NULL REFERENCES repositories(repository_id) ON DELETE CASCADE,\
                 scope_kind INTEGER NOT NULL, scope_path BLOB NOT NULL, logical_name TEXT NOT NULL,\
                 conversation_id TEXT NOT NULL UNIQUE, run_id TEXT NOT NULL UNIQUE, root_agent_id TEXT NOT NULL,\
                 workspace_id TEXT NOT NULL, selected_branch_id TEXT NOT NULL, updated_at_millis INTEGER NOT NULL,\
                 PRIMARY KEY(repository_id, scope_kind, scope_path, logical_name));\
             CREATE TABLE run_leases(\
                 run_id TEXT PRIMARY KEY, instance_id TEXT NOT NULL, pid_marker INTEGER,\
                 heartbeat_at_millis INTEGER NOT NULL, expires_at_millis INTEGER NOT NULL);\
             PRAGMA user_version=1;"
        ).map_err(|error| migration(1, error))?;
    }
    transaction.commit().map_err(|error| migration(0, error))
}

fn configure_wal(connection: &Connection) -> Result<(), CatalogError> {
    let deadline = std::time::Instant::now() + BUSY_TIMEOUT;
    loop {
        match connection.pragma_update(None, "journal_mode", "WAL") {
            Ok(()) => return Ok(()),
            Err(error) if is_busy(&error) && std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10))
            }
            Err(error) => return Err(storage("enable catalog WAL", error)),
        }
    }
}

fn set_owner_only(path: &Path) -> Result<(), CatalogError> {
    #[cfg(unix)]
    {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|error| {
            CatalogError::RunLog(RunLogError::Storage {
                operation: "set private catalog permissions".into(),
                detail: format!("{}: {error}", path.display()),
            })
        })?;
    }
    Ok(())
}

fn parse_repository_id(value: String) -> Result<RepositoryId, CatalogError> {
    RepositoryId::parse(value).map_err(corrupt_id)
}

fn corrupt_id(error: super::InvalidId) -> CatalogError {
    CatalogError::RunLog(RunLogError::Corruption {
        detail: error.to_string(),
    })
}

fn storage(operation: &str, error: rusqlite::Error) -> CatalogError {
    CatalogError::RunLog(RunLogError::Storage {
        operation: operation.into(),
        detail: error.to_string(),
    })
}

fn migration(version: u32, error: rusqlite::Error) -> CatalogError {
    CatalogError::RunLog(RunLogError::Migration {
        version,
        detail: error.to_string(),
    })
}

fn is_busy(error: &rusqlite::Error) -> bool {
    matches!(error, rusqlite::Error::SqliteFailure(details, _) if matches!(details.code, ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicI64, Ordering};
    use std::sync::Barrier;
    use std::thread;

    struct TestClock(AtomicI64);

    impl TestClock {
        fn new(now: i64) -> Self {
            Self(AtomicI64::new(now))
        }

        fn set(&self, now: i64) {
            self.0.store(now, Ordering::SeqCst);
        }
    }

    impl CatalogClock for TestClock {
        fn now_millis(&self) -> i64 {
            self.0.load(Ordering::SeqCst)
        }
    }

    fn registration(clone: &str) -> RepositoryRegistration {
        RepositoryRegistration {
            common_git_dir: PathBuf::from(format!("/{clone}/.git")),
            worktree_aliases: vec![PathBuf::from(format!("/{clone}"))],
        }
    }

    #[test]
    fn repository_identity_survives_reopen_and_distinguishes_clones() {
        let temporary = tempfile::tempdir().unwrap();
        let layout = RunStorageLayout::new(temporary.path().join("runs"));
        let first = RunCatalog::open(&layout).unwrap();
        let original = first.register_repository(registration("clone-a")).unwrap();
        let other = first.register_repository(registration("clone-b")).unwrap();
        assert_ne!(original.repository_id, other.repository_id);
        drop(first);

        let reopened = RunCatalog::open(&layout).unwrap();
        assert_eq!(
            reopened
                .register_repository(registration("clone-a"))
                .unwrap()
                .repository_id,
            original.repository_id
        );
    }

    #[test]
    fn file_and_no_file_conversations_have_stable_distinct_bindings() {
        let temporary = tempfile::tempdir().unwrap();
        let layout = RunStorageLayout::new(temporary.path().join("runs"));
        let catalog = RunCatalog::open(&layout).unwrap();
        let repository = catalog.register_repository(registration("repo")).unwrap();
        let no_file = ConversationKey {
            repository_id: repository.repository_id.clone(),
            scope: ConversationScope::NoFile,
            logical_name: "chat".into(),
        };
        let file = ConversationKey {
            repository_id: repository.repository_id,
            scope: ConversationScope::RepositoryPath("src/lib.rs".into()),
            logical_name: "chat".into(),
        };
        let no_file_binding = catalog
            .open_conversation(no_file.clone(), BranchId::new())
            .unwrap();
        let file_binding = catalog
            .open_conversation(file.clone(), BranchId::new())
            .unwrap();
        assert_ne!(no_file_binding.run_id, file_binding.run_id);
        assert_eq!(
            catalog
                .open_conversation(no_file, BranchId::new())
                .unwrap()
                .run_id,
            no_file_binding.run_id
        );
        assert_eq!(
            catalog
                .open_conversation(file, BranchId::new())
                .unwrap()
                .conversation_id,
            file_binding.conversation_id
        );
    }

    #[test]
    fn selected_branch_update_is_durable() {
        let temporary = tempfile::tempdir().unwrap();
        let layout = RunStorageLayout::new(temporary.path().join("runs"));
        let catalog = RunCatalog::open(&layout).unwrap();
        let repository = catalog.register_repository(registration("repo")).unwrap();
        let key = ConversationKey {
            repository_id: repository.repository_id,
            scope: ConversationScope::NoFile,
            logical_name: "default".into(),
        };
        catalog
            .open_conversation(key.clone(), BranchId::new())
            .unwrap();
        let selected = BranchId::new();
        assert_eq!(
            catalog
                .update_selected_branch(&key, selected.clone())
                .unwrap()
                .unwrap()
                .selected_branch_id,
            selected
        );
        drop(catalog);
        assert_eq!(
            RunCatalog::open(&layout)
                .unwrap()
                .open_conversation(key, BranchId::new())
                .unwrap()
                .selected_branch_id,
            selected
        );
    }

    #[test]
    fn leases_contend_expire_transfer_and_survive_reopen() {
        let temporary = tempfile::tempdir().unwrap();
        let layout = RunStorageLayout::new(temporary.path().join("runs"));
        let clock = Arc::new(TestClock::new(1_000));
        let catalog = RunCatalog::open_with_clock(&layout, clock.clone()).unwrap();
        let run_id = RunId::new();
        let first = LeaseOwner {
            instance_id: "instance-a".into(),
            pid_marker: Some(42),
        };
        let second = LeaseOwner {
            instance_id: "instance-b".into(),
            pid_marker: Some(42), // same PID does not make this the same owner
        };
        catalog
            .acquire_lease(run_id.clone(), first.clone(), Duration::from_secs(1))
            .unwrap();
        assert!(matches!(
            catalog.acquire_lease(run_id.clone(), second.clone(), Duration::from_secs(1)),
            Err(CatalogError::LeaseHeld(_))
        ));
        clock.set(2_000);
        assert!(matches!(
            catalog.lease_status(&run_id).unwrap(),
            LeaseStatus::Stale(_)
        ));
        catalog
            .acquire_lease(run_id.clone(), second.clone(), Duration::from_secs(2))
            .unwrap();
        assert!(matches!(
            catalog.renew_lease(&run_id, &first, Duration::from_secs(1)),
            Err(CatalogError::LeaseNotOwned(_))
        ));
        assert!(matches!(
            catalog.release_lease(&run_id, &first),
            Err(CatalogError::LeaseNotOwned(_))
        ));
        clock.set(2_500);
        let renewed = catalog
            .renew_lease(&run_id, &second, Duration::from_secs(2))
            .unwrap();
        assert_eq!(renewed.heartbeat_at_millis, 2_500);
        assert_eq!(renewed.expires_at_millis, 4_500);
        drop(catalog);

        let reopened = RunCatalog::open_with_clock(&layout, clock).unwrap();
        assert!(matches!(
            reopened.lease_status(&run_id).unwrap(),
            LeaseStatus::Active(RunLease { owner, .. }) if owner == second
        ));
        assert!(reopened.release_lease(&run_id, &second).unwrap());
        assert_eq!(
            reopened.lease_status(&run_id).unwrap(),
            LeaseStatus::Missing
        );
    }

    #[test]
    fn concurrent_registration_converges_on_one_repository_id() {
        let temporary = tempfile::tempdir().unwrap();
        let layout = RunStorageLayout::new(temporary.path().join("runs"));
        let barrier = Arc::new(Barrier::new(8));
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let layout = layout.clone();
                let barrier = barrier.clone();
                thread::spawn(move || {
                    barrier.wait();
                    RunCatalog::open(&layout)
                        .unwrap()
                        .register_repository(registration("shared"))
                        .unwrap()
                        .repository_id
                })
            })
            .collect();
        let ids: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect();
        assert!(ids.iter().all(|id| id == &ids[0]));
    }

    #[cfg(unix)]
    #[test]
    fn catalog_database_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let temporary = tempfile::tempdir().unwrap();
        let layout = RunStorageLayout::new(temporary.path().join("runs"));
        RunCatalog::open(&layout).unwrap();
        assert_eq!(
            std::fs::metadata(layout.root().join(CATALOG_DATABASE))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}
