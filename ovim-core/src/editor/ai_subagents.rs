//! Editor-owned assembly for the read-only delegated-agent preview.
//!
//! Root chat remains in its existing orchestration path. This service owns a
//! separate per-run supervisor and immutable snapshot stack, while sharing the
//! exact durable run sink and configured provider profiles.

use crate::agent_runtime::{
    AgentSupervisor, AgentSupervisorConfig, AgentWorkspaceManager, ProfileAgentProvider,
    SnapshotAgentLoopInputFactory, SubagentModelCatalog,
};
use crate::ai::{AiConfig, AiSubagentConfig};
use crate::run_log::{
    AgentId, ArtifactStore, BaseManifest, LocalRunStore, ManifestId, RepositoryId, RunEventSink,
    RunId,
};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub(crate) struct AiSubagentService {
    startup_policy: AiSubagentConfig,
    config_fingerprint: String,
    catalog: Result<Arc<SubagentModelCatalog>, String>,
    provider: Arc<ProfileAgentProvider>,
    runs: Mutex<HashMap<RunId, Arc<AiSubagentRun>>>,
}

pub(crate) struct AiSubagentRun {
    pub root_agent_id: AgentId,
    pub repository_id: RepositoryId,
    pub supervisor: AgentSupervisor,
    pub workspace_manager: AgentWorkspaceManager,
    pub input_factory: Arc<SnapshotAgentLoopInputFactory>,
    manifest_registry: BaseManifestRegistry,
}

impl AiSubagentService {
    pub fn new(config: &AiConfig) -> Self {
        Self {
            startup_policy: config.subagents.clone(),
            config_fingerprint: subagent_config_fingerprint(config),
            catalog: SubagentModelCatalog::from_config(config)
                .map(Arc::new)
                .map_err(|error| error.to_string()),
            provider: Arc::new(ProfileAgentProvider::new(config)),
            runs: Mutex::new(HashMap::new()),
        }
    }

    pub fn policy(&self) -> &AiSubagentConfig {
        &self.startup_policy
    }

    pub fn catalog(&self) -> Result<Arc<SubagentModelCatalog>, String> {
        self.catalog.clone()
    }

    /// Running supervisors keep the exact startup policy/profile snapshot.
    /// A mutable Lua/config reload must rebuild AiState rather than silently
    /// changing routing or authority beneath queued children.
    pub fn ensure_compatible(&self, current: &AiConfig) -> Result<(), String> {
        if !self.startup_policy.enabled {
            return Err("the subagent preview is disabled".into());
        }
        if current.subagents != self.startup_policy
            || subagent_config_fingerprint(current) != self.config_fingerprint
        {
            return Err(
                "AI subagent configuration changed after editor startup; restart Ovim before dispatching"
                    .into(),
            );
        }
        let catalog = self.catalog()?;
        if catalog.entries().next().is_none() {
            return Err("the subagent preview has no allowed model routes".into());
        }
        Ok(())
    }

    pub fn run(
        &self,
        store: Arc<LocalRunStore>,
        run_id: RunId,
        root_agent_id: AgentId,
        repository_id: RepositoryId,
    ) -> Result<Arc<AiSubagentRun>, String> {
        let mut runs = self
            .runs
            .lock()
            .map_err(|_| "subagent run registry is poisoned".to_string())?;
        if let Some(run) = runs.get(&run_id) {
            if run.root_agent_id != root_agent_id || run.repository_id != repository_id {
                return Err("subagent run identity changed after initialization".into());
            }
            return Ok(run.clone());
        }

        store
            .layout()
            .ensure_run_directory(&run_id)
            .map_err(|error| error.to_string())?;
        let artifact_store = ArtifactStore::open(store.layout().artifact_directory(&run_id))
            .map_err(|error| error.to_string())?;
        let workspace_manager = AgentWorkspaceManager::new(artifact_store);
        let input_factory = Arc::new(SnapshotAgentLoopInputFactory::new(
            workspace_manager.clone(),
            self.provider.clone(),
        ));
        let sink: Arc<dyn RunEventSink> = store.clone();
        let supervisor = AgentSupervisor::new(
            run_id.clone(),
            root_agent_id.clone(),
            sink,
            self.catalog()?,
            input_factory.clone(),
            supervisor_config(&self.startup_policy),
        )
        .map_err(|error| error.to_string())?;
        let run = Arc::new(AiSubagentRun {
            root_agent_id,
            repository_id,
            supervisor,
            workspace_manager,
            input_factory,
            manifest_registry: BaseManifestRegistry::new(
                store.layout().run_directory(&run_id).join("manifests"),
            ),
        });
        runs.insert(run_id, run.clone());
        Ok(run)
    }
}

impl AiSubagentRun {
    /// Publish manifest metadata before making the projection available to a
    /// queue runner. The content blobs were already fsynced by ArtifactStore.
    pub fn register_manifest(
        &self,
        manifest_id: ManifestId,
        manifest: BaseManifest,
        repository_start: &Path,
    ) -> Result<(), String> {
        if manifest.repository.repository_id != self.repository_id {
            return Err("captured manifest belongs to another repository".into());
        }
        self.manifest_registry
            .register(&manifest_id, &manifest)
            .map_err(|error| error.to_string())?;
        self.workspace_manager
            .register_read_only(manifest_id, manifest, repository_start)
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    #[cfg(test)]
    fn manifest(&self, manifest_id: &ManifestId) -> Result<BaseManifest, String> {
        self.manifest_registry
            .load(manifest_id)
            .map_err(|error| error.to_string())
    }
}

fn supervisor_config(policy: &AiSubagentConfig) -> AgentSupervisorConfig {
    AgentSupervisorConfig {
        max_concurrent: policy.max_concurrent,
        max_queued: policy.max_queued,
        max_children_per_parent: policy.max_children_per_parent,
        max_total_per_run: policy.max_total_per_run,
        max_depth: policy.max_depth,
        child_budget: crate::agent_runtime::AgentLoopBudget {
            timeout: std::time::Duration::from_secs(policy.default_timeout_seconds),
            max_provider_events: policy.budgets.max_provider_events_per_agent,
            max_tool_calls: policy.budgets.max_tool_calls_per_agent,
        },
        root_max_provider_events: policy.budgets.max_total_provider_events,
        root_max_tool_calls: policy.budgets.max_total_tool_calls,
    }
}

fn subagent_config_fingerprint(config: &AiConfig) -> String {
    let mut digest = Sha256::new();
    digest.update(b"ovim-editor-subagents-v1\0");
    let mut profiles = config.profiles.values().collect::<Vec<_>>();
    profiles.sort_by(|left, right| left.name.cmp(&right.name));
    for profile in profiles {
        for value in [
            profile.name.as_str(),
            &profile.provider.to_string(),
            profile.model.as_str(),
            profile.base_url.as_deref().unwrap_or_default(),
            profile.reasoning_effort.as_deref().unwrap_or_default(),
        ] {
            digest.update(value.as_bytes());
            digest.update(b"\0");
        }
        for tool in &profile.tools {
            digest.update(tool.as_bytes());
            digest.update(b"\0");
        }
        digest.update([
            profile.scope.files as u8,
            u8::from(profile.scope.shell),
            u8::from(profile.scope.network),
        ]);
        // Detect credential-source changes without retaining them in service
        // diagnostics or exposing them to child context.
        for secret_source in [profile.api_key.as_deref(), profile.api_key_env.as_deref()] {
            if let Some(secret_source) = secret_source {
                digest.update(Sha256::digest(secret_source.as_bytes()));
            }
            digest.update(b"\0");
        }
    }
    format!("sha256:{:x}", digest.finalize())
}

#[derive(Clone)]
struct BaseManifestRegistry {
    root: PathBuf,
}

impl BaseManifestRegistry {
    fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn register(
        &self,
        manifest_id: &ManifestId,
        manifest: &BaseManifest,
    ) -> Result<(), ManifestRegistryError> {
        ensure_private_directory(&self.root)?;
        let destination = self.path(manifest_id);
        if destination.exists() {
            let existing = self.load(manifest_id)?;
            return if &existing == manifest {
                Ok(())
            } else {
                Err(ManifestRegistryError::Conflict(manifest_id.clone()))
            };
        }
        let bytes = serde_json::to_vec(manifest)
            .map_err(|error| ManifestRegistryError::Serialization(error.to_string()))?;
        let temporary = self.root.join(format!(
            ".{}.{}.tmp",
            encoded_manifest_component(manifest_id),
            std::process::id()
        ));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(ManifestRegistryError::Io)?;
        let result = (|| {
            file.write_all(&bytes)?;
            file.flush()?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temporary, &destination)?;
            let directory = OpenOptions::new().read(true).open(&self.root)?;
            directory.sync_all()?;
            Ok::<_, std::io::Error>(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result.map_err(ManifestRegistryError::Io)
    }

    fn load(&self, manifest_id: &ManifestId) -> Result<BaseManifest, ManifestRegistryError> {
        let bytes = fs::read(self.path(manifest_id)).map_err(ManifestRegistryError::Io)?;
        serde_json::from_slice(&bytes)
            .map_err(|error| ManifestRegistryError::Serialization(error.to_string()))
    }

    fn path(&self, manifest_id: &ManifestId) -> PathBuf {
        self.root
            .join(format!("{}.json", encoded_manifest_component(manifest_id)))
    }
}

fn encoded_manifest_component(manifest_id: &ManifestId) -> String {
    let mut encoded = String::with_capacity(manifest_id.as_str().len() * 2);
    for byte in manifest_id.as_str().as_bytes() {
        use std::fmt::Write;
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

fn ensure_private_directory(path: &Path) -> Result<(), ManifestRegistryError> {
    fs::create_dir_all(path).map_err(ManifestRegistryError::Io)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(ManifestRegistryError::Io)?;
    }
    Ok(())
}

#[derive(Debug)]
enum ManifestRegistryError {
    Io(std::io::Error),
    Serialization(String),
    Conflict(ManifestId),
}

impl std::fmt::Display for ManifestRegistryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "manifest registry I/O: {error}"),
            Self::Serialization(error) => write!(formatter, "manifest registry JSON: {error}"),
            Self::Conflict(id) => write!(
                formatter,
                "manifest {id} was already registered differently"
            ),
        }
    }
}

impl std::error::Error for ManifestRegistryError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run_log::{BaseManifestId, ManifestConfidence, RepositoryBase};

    fn manifest(repository_id: RepositoryId) -> BaseManifest {
        BaseManifest {
            base_manifest_id: BaseManifestId::new(),
            captured_at: "2026-07-18T10:00:00Z".into(),
            repository: RepositoryBase {
                repository_id,
                head_commit: None,
                index_tree: None,
            },
            files: Vec::new(),
            unsaved_buffers: Vec::new(),
            artifacts: Vec::new(),
            captured_bytes: 0,
            confidence: ManifestConfidence::Complete,
            issues: Vec::new(),
        }
    }

    #[test]
    fn manifest_registry_is_durable_idempotent_and_conflict_safe() {
        let directory = tempfile::tempdir().unwrap();
        let registry = BaseManifestRegistry::new(directory.path().join("manifests"));
        let id = ManifestId::new();
        let repository = RepositoryId::new();
        let first = manifest(repository.clone());
        registry.register(&id, &first).unwrap();
        registry.register(&id, &first).unwrap();
        assert_eq!(registry.load(&id).unwrap(), first);

        let mut changed = manifest(repository);
        changed.captured_at = "later".into();
        assert!(matches!(
            registry.register(&id, &changed),
            Err(ManifestRegistryError::Conflict(conflict)) if conflict == id
        ));
    }

    #[test]
    fn config_changes_require_service_rebuild() {
        let mut config = AiConfig::default();
        config.subagents.enabled = true;
        let service = AiSubagentService::new(&config);
        assert!(service.ensure_compatible(&config).is_ok());

        config.subagents.max_concurrent = 2;
        assert!(service.ensure_compatible(&config).is_err());
    }

    #[test]
    fn service_owns_one_snapshot_stack_per_durable_root_run() {
        let directory = tempfile::tempdir().unwrap();
        git2::Repository::init(directory.path()).unwrap();
        let layout = crate::run_log::RunStorageLayout::new(directory.path().join("runs"));
        let store = Arc::new(LocalRunStore::new(layout));
        let mut config = AiConfig::default();
        config.subagents.enabled = true;
        let service = AiSubagentService::new(&config);
        let run_id = RunId::new();
        let root = AgentId::new();
        let repository = RepositoryId::new();
        let first = service
            .run(
                store.clone(),
                run_id.clone(),
                root.clone(),
                repository.clone(),
            )
            .unwrap();
        let reopened = service
            .run(store.clone(), run_id.clone(), root, repository.clone())
            .unwrap();
        assert!(Arc::ptr_eq(&first, &reopened));

        let manifest_id = ManifestId::new();
        first
            .register_manifest(manifest_id.clone(), manifest(repository), directory.path())
            .unwrap();
        assert_eq!(
            first.manifest(&manifest_id).unwrap().captured_at,
            "2026-07-18T10:00:00Z"
        );
        assert!(first
            .workspace_manager
            .read_only(&manifest_id)
            .unwrap()
            .is_some());

        assert!(service
            .run(store, run_id, AgentId::new(), RepositoryId::new())
            .is_err());
    }
}
