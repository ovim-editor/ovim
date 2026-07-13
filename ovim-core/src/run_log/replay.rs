//! Read-only reconstruction of recorded workspace history.
//!
//! Replay consumes normalized file transitions. It never executes historical
//! tools or shell commands and never writes to the live checkout.

use super::{
    ArtifactAvailability, ArtifactId, ArtifactRecord, ArtifactRef, ArtifactState, ArtifactStore,
    BaseManifest, BranchId, EventEnvelope, EventId, EventKind, FileKind, FileMutationEvent,
    FileMutationState, GitBaseEntry, ManifestConfidence, ManifestLayer, Replayability, RepoPath,
    TurnId, WorkspaceSurface,
};
use git2::{ObjectType, Oid};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt;

/// Optional, read-only access to Git objects referenced by a base manifest.
/// Implementations must not update the object database or worktree.
pub trait ReplayGitResolver {
    fn read_blob(&self, object_id: &str) -> Result<Option<Vec<u8>>, String>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplayContent {
    Absent,
    Available {
        bytes: Vec<u8>,
        artifact_id: Option<ArtifactId>,
    },
    Unavailable {
        reason: String,
    },
}

impl ReplayContent {
    pub fn bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Available { bytes, .. } => Some(bytes),
            Self::Absent | Self::Unavailable { .. } => None,
        }
    }

    pub fn is_replayable(&self) -> bool {
        !matches!(self, Self::Unavailable { .. })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayFile {
    pub file_kind: FileKind,
    pub executable: bool,
    pub git_base: ReplayContent,
    pub index: ReplayContent,
    pub disk: ReplayContent,
    pub buffer: Option<ReplayContent>,
}

impl ReplayFile {
    /// The source an editor should show at this replay cursor.
    pub fn visible_content(&self) -> &ReplayContent {
        self.buffer.as_ref().unwrap_or(&self.disk)
    }

    fn surface(&self, surface: &WorkspaceSurface) -> &ReplayContent {
        match surface {
            WorkspaceSurface::Buffer { .. } => self.buffer.as_ref().unwrap_or(&self.disk),
            WorkspaceSurface::Disk => &self.disk,
            WorkspaceSurface::GitIndex => &self.index,
        }
    }

    fn set_surface(&mut self, surface: &WorkspaceSurface, content: ReplayContent) {
        match surface {
            WorkspaceSurface::Buffer { .. } => self.buffer = Some(content),
            WorkspaceSurface::Disk => self.disk = content,
            WorkspaceSurface::GitIndex => self.index = content,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayState {
    pub files: BTreeMap<RepoPath, ReplayFile>,
    pub unsaved_buffers: BTreeMap<ArtifactId, ReplayContent>,
    pub replayability: Replayability,
    pub issues: Vec<ReplayIssue>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayIssue {
    pub event_id: Option<EventId>,
    pub path: Option<RepoPath>,
    pub kind: ReplayIssueKind,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplayIssueKind {
    BaseIncomplete,
    GitObjectUnavailable,
    GitObjectMismatch,
    ArtifactUnavailable,
    ArtifactMetadataMissing,
    PreimageMismatch,
    CausalHistoryIncomplete,
    RecordedDivergence,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayStep {
    pub event_id: EventId,
    pub sequence: u64,
    pub turn_id: Option<TurnId>,
    pub changed_files: Vec<RepoPath>,
    pub replayability: Replayability,
    pub issues: Vec<ReplayIssue>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplayBoundary {
    Start,
    Event(EventId),
    Turn(TurnId),
    End,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservedFile {
    pub path: RepoPath,
    /// `None` means the path is absent on the observed surface.
    pub bytes: Option<Vec<u8>>,
    pub surface: WorkspaceSurface,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveComparison {
    pub replayability: Replayability,
    pub divergences: Vec<ReplayIssue>,
}

/// A deterministic, in-memory replay projection.
pub struct ReplaySession {
    initial: ReplayState,
    state: ReplayState,
    transitions: Vec<Transition>,
    transition_artifacts: HashMap<ArtifactId, ReplayContent>,
    event_sequences: HashMap<EventId, u64>,
    turn_sequences: HashMap<TurnId, u64>,
    divergences: Vec<(u64, ReplayIssue)>,
    divergence_cursor: usize,
    cursor: usize,
}

#[derive(Clone)]
struct Transition {
    envelope: EventEnvelope,
    mutation: FileMutationEvent,
}

impl ReplaySession {
    pub fn new(
        manifest: &BaseManifest,
        additional_artifacts: &[ArtifactRecord],
        events: &[EventEnvelope],
        store: &ArtifactStore,
        git: Option<&dyn ReplayGitResolver>,
    ) -> Result<Self, ReplayError> {
        ensure_unique_event_ids(events)?;
        let branches: HashSet<_> = events
            .iter()
            .filter_map(|event| event.branch_id.clone())
            .collect();
        if branches.len() > 1 {
            return Err(ReplayError::AmbiguousBranches);
        }
        Self::build(manifest, additional_artifacts, events, store, git)
    }

    /// Construct one branch trajectory from a forked run. The latest event on
    /// `branch_id` is the tip; only its transitive `caused_by` ancestry is
    /// included, so interleaved sibling events can never leak into the state.
    pub fn new_for_branch(
        manifest: &BaseManifest,
        additional_artifacts: &[ArtifactRecord],
        events: &[EventEnvelope],
        store: &ArtifactStore,
        git: Option<&dyn ReplayGitResolver>,
        branch_id: BranchId,
    ) -> Result<Self, ReplayError> {
        ensure_unique_event_ids(events)?;
        let selected = select_branch_ancestry(events, &branch_id)?;
        Self::build(manifest, additional_artifacts, &selected, store, git)
    }

    fn build(
        manifest: &BaseManifest,
        additional_artifacts: &[ArtifactRecord],
        events: &[EventEnvelope],
        store: &ArtifactStore,
        git: Option<&dyn ReplayGitResolver>,
    ) -> Result<Self, ReplayError> {
        let embedded_artifacts: Vec<_> = events
            .iter()
            .filter_map(|event| match &event.kind {
                EventKind::FileMutation(mutation) => Some(mutation.artifacts.iter()),
                _ => None,
            })
            .flatten()
            .collect();
        let artifacts = ArtifactCatalog::new(
            &manifest.artifacts,
            additional_artifacts,
            &embedded_artifacts,
        )?;
        let mut initial = materialize_base(manifest, &artifacts, store, git);
        let normalized = normalize_transitions(events, &mut initial)?;
        let mut transition_artifacts = HashMap::new();
        for transition in &normalized.transitions {
            for artifact in [
                transition.mutation.before_artifact.as_ref(),
                transition.mutation.after_artifact.as_ref(),
            ]
            .into_iter()
            .flatten()
            {
                transition_artifacts
                    .entry(artifact.artifact_id.clone())
                    .or_insert_with(|| match artifacts.resolve_ref(artifact, store) {
                        Ok(content) => content,
                        Err(reason) => ReplayContent::Unavailable { reason },
                    });
            }
        }
        initial.replayability = state_replayability(&initial);
        Ok(Self {
            state: initial.clone(),
            initial,
            transitions: normalized.transitions,
            transition_artifacts,
            event_sequences: normalized.event_sequences,
            turn_sequences: normalized.turn_sequences,
            divergences: normalized.divergences,
            divergence_cursor: 0,
            cursor: 0,
        })
    }

    pub fn state(&self) -> &ReplayState {
        &self.state
    }

    pub fn position(&self) -> usize {
        self.cursor
    }

    pub fn step_forward(&mut self) -> Option<ReplayStep> {
        let transition = self.transitions.get(self.cursor)?.clone();
        let divergence_start = self.divergence_cursor;
        self.apply_divergences_until(transition.envelope.sequence);
        let mut step = apply_transition(&mut self.state, &transition, &self.transition_artifacts);
        if self.divergence_cursor > divergence_start {
            step.replayability = Replayability::Diverged;
            step.issues.extend(
                self.divergences[divergence_start..self.divergence_cursor]
                    .iter()
                    .map(|(_, issue)| issue.clone()),
            );
        }
        self.cursor += 1;
        self.state.replayability = state_replayability(&self.state);
        Some(step)
    }

    /// Reconstruct from the immutable base instead of depending on prior cursor
    /// movement. Repeated calls for the same boundary are deterministic.
    pub fn reconstruct(&mut self, boundary: ReplayBoundary) -> Result<&ReplayState, ReplayError> {
        let target = self.boundary_cursor(&boundary)?;
        self.state = self.initial.clone();
        self.cursor = 0;
        self.divergence_cursor = 0;
        while self.cursor < target {
            let _ = self.step_forward();
        }
        let boundary_sequence = match boundary {
            ReplayBoundary::Start => 0,
            ReplayBoundary::End => u64::MAX,
            ReplayBoundary::Event(ref id) => self.event_sequences[id],
            ReplayBoundary::Turn(ref id) => self.turn_sequences[id],
        };
        self.apply_divergences_until(boundary_sequence);
        self.state.replayability = state_replayability(&self.state);
        Ok(&self.state)
    }

    pub fn steps_len(&self) -> usize {
        self.transitions.len()
    }

    /// Compare complete observed surface inventories with historical state.
    ///
    /// Every surface represented in `observed` is treated as a complete
    /// inventory, not a list of changed paths; omitted expected paths are
    /// therefore reported as deletions. An empty inventory means an empty disk
    /// workspace. This is observational and never changes either workspace.
    pub fn compare_observed_workspace(&self, observed: &[ObservedFile]) -> LiveComparison {
        let mut divergences = Vec::new();
        let mut inventory_surfaces: HashSet<SurfaceKind> = observed
            .iter()
            .map(|item| SurfaceKind::of(&item.surface))
            .collect();
        if inventory_surfaces.is_empty() {
            inventory_surfaces.insert(SurfaceKind::Disk);
        }
        let observed_keys: HashSet<_> = observed
            .iter()
            .map(|item| (item.path.clone(), SurfaceKind::of(&item.surface)))
            .collect();
        for item in observed {
            let expected = self
                .state
                .files
                .get(&item.path)
                .map(|file| file.surface(&item.surface))
                .unwrap_or(&ReplayContent::Absent);
            match expected {
                ReplayContent::Unavailable { reason } => divergences.push(ReplayIssue {
                    event_id: None,
                    path: Some(item.path.clone()),
                    kind: ReplayIssueKind::ArtifactUnavailable,
                    detail: format!("historical preimage is unavailable: {reason}"),
                }),
                ReplayContent::Absent if item.bytes.is_none() => {}
                ReplayContent::Available { bytes, .. }
                    if item.bytes.as_deref() == Some(bytes.as_slice()) => {}
                _ => divergences.push(ReplayIssue {
                    event_id: None,
                    path: Some(item.path.clone()),
                    kind: ReplayIssueKind::PreimageMismatch,
                    detail: "live workspace differs from the selected historical state".into(),
                }),
            }
        }
        for (path, file) in &self.state.files {
            for surface in &inventory_surfaces {
                if observed_keys.contains(&(path.clone(), *surface)) {
                    continue;
                }
                let expected = match surface {
                    SurfaceKind::Disk => &file.disk,
                    SurfaceKind::GitIndex => &file.index,
                    SurfaceKind::Buffer => {
                        let Some(buffer) = &file.buffer else {
                            continue;
                        };
                        buffer
                    }
                };
                if !matches!(expected, ReplayContent::Absent) {
                    divergences.push(ReplayIssue {
                        event_id: None,
                        path: Some(path.clone()),
                        kind: ReplayIssueKind::PreimageMismatch,
                        detail:
                            "expected historical path is absent from the complete live inventory"
                                .into(),
                    });
                }
            }
        }
        let replayability = if divergences.is_empty() {
            Replayability::Exact
        } else if divergences
            .iter()
            .any(|issue| issue.kind == ReplayIssueKind::PreimageMismatch)
        {
            Replayability::Diverged
        } else {
            Replayability::Partial
        };
        LiveComparison {
            replayability,
            divergences,
        }
    }

    fn boundary_cursor(&self, boundary: &ReplayBoundary) -> Result<usize, ReplayError> {
        match boundary {
            ReplayBoundary::Start => Ok(0),
            ReplayBoundary::End => Ok(self.transitions.len()),
            ReplayBoundary::Event(id) => {
                let sequence = *self
                    .event_sequences
                    .get(&id)
                    .ok_or_else(|| ReplayError::UnknownEventBoundary(id.clone()))?;
                Ok(self
                    .transitions
                    .partition_point(|transition| transition.envelope.sequence <= sequence))
            }
            ReplayBoundary::Turn(id) => {
                let sequence = *self
                    .turn_sequences
                    .get(&id)
                    .ok_or_else(|| ReplayError::UnknownTurnBoundary(id.clone()))?;
                Ok(self
                    .transitions
                    .partition_point(|transition| transition.envelope.sequence <= sequence))
            }
        }
    }

    fn apply_divergences_until(&mut self, sequence: u64) {
        while let Some((divergence_sequence, issue)) = self.divergences.get(self.divergence_cursor)
        {
            if *divergence_sequence > sequence {
                break;
            }
            self.state.issues.push(issue.clone());
            self.divergence_cursor += 1;
        }
    }
}

fn ensure_unique_event_ids(events: &[EventEnvelope]) -> Result<(), ReplayError> {
    let mut seen = HashSet::new();
    for event in events {
        if !seen.insert(event.event_id.clone()) {
            return Err(ReplayError::DuplicateEventId(event.event_id.clone()));
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum SurfaceKind {
    Buffer,
    Disk,
    GitIndex,
}

impl SurfaceKind {
    fn of(surface: &WorkspaceSurface) -> Self {
        match surface {
            WorkspaceSurface::Buffer { .. } => Self::Buffer,
            WorkspaceSurface::Disk => Self::Disk,
            WorkspaceSurface::GitIndex => Self::GitIndex,
        }
    }
}

fn select_branch_ancestry(
    events: &[EventEnvelope],
    branch_id: &BranchId,
) -> Result<Vec<EventEnvelope>, ReplayError> {
    let by_id: HashMap<_, _> = events
        .iter()
        .map(|event| (event.event_id.clone(), event))
        .collect();
    let mut current = events
        .iter()
        .filter(|event| event.branch_id.as_ref() == Some(branch_id))
        .max_by_key(|event| event.sequence)
        .ok_or_else(|| ReplayError::UnknownBranch(branch_id.clone()))?;
    let mut ancestry = HashSet::new();
    loop {
        if !ancestry.insert(current.event_id.clone()) {
            return Err(ReplayError::CauseNotPrior(current.event_id.clone()));
        }
        let Some(cause) = &current.caused_by else {
            break;
        };
        let Some(parent) = by_id.get(cause) else {
            break;
        };
        current = parent;
    }
    Ok(events
        .iter()
        .filter(|event| ancestry.contains(&event.event_id))
        .cloned()
        .collect())
}

struct ArtifactCatalog<'a> {
    records: HashMap<&'a ArtifactId, &'a ArtifactRecord>,
}

impl<'a> ArtifactCatalog<'a> {
    fn new(
        base: &'a [ArtifactRecord],
        additional: &'a [ArtifactRecord],
        embedded: &[&'a ArtifactRecord],
    ) -> Result<Self, ReplayError> {
        let mut records = HashMap::new();
        for record in base
            .iter()
            .chain(additional)
            .chain(embedded.iter().copied())
        {
            if let Some(existing) = records.insert(&record.artifact_id, record) {
                if existing != record {
                    return Err(ReplayError::ConflictingArtifactMetadata(
                        record.artifact_id.clone(),
                    ));
                }
            }
        }
        Ok(Self { records })
    }

    fn resolve(
        &self,
        artifact_id: &ArtifactId,
        store: &ArtifactStore,
    ) -> Result<ReplayContent, String> {
        let record = self
            .records
            .get(artifact_id)
            .ok_or_else(|| "artifact metadata is absent from the replay bundle".to_string())?;
        match &record.state {
            ArtifactState::Available { blob_id, byte_len } => {
                let bytes = store.read(*blob_id).map_err(|error| error.to_string())?;
                if bytes.len() as u64 != *byte_len {
                    return Err(format!(
                        "artifact length is {}, metadata declared {byte_len}",
                        bytes.len()
                    ));
                }
                Ok(ReplayContent::Available {
                    bytes,
                    artifact_id: Some(artifact_id.clone()),
                })
            }
            ArtifactState::Missing { reason }
            | ArtifactState::Excluded { reason }
            | ArtifactState::Redacted { reason, .. } => Err(reason.clone()),
        }
    }

    fn resolve_ref(
        &self,
        artifact: &ArtifactRef,
        store: &ArtifactStore,
    ) -> Result<ReplayContent, String> {
        if artifact.availability != ArtifactAvailability::Available {
            return Err(format!(
                "event marks artifact as {:?}",
                artifact.availability
            ));
        }
        let record = self
            .records
            .get(&artifact.artifact_id)
            .ok_or_else(|| "artifact metadata is absent from the replay bundle".to_string())?;
        if record.representation != artifact.representation {
            return Err("event and artifact metadata disagree about content representation".into());
        }
        self.resolve(&artifact.artifact_id, store)
    }
}

fn materialize_base(
    manifest: &BaseManifest,
    artifacts: &ArtifactCatalog<'_>,
    store: &ArtifactStore,
    git: Option<&dyn ReplayGitResolver>,
) -> ReplayState {
    let mut state = ReplayState {
        files: BTreeMap::new(),
        unsaved_buffers: BTreeMap::new(),
        replayability: Replayability::Exact,
        issues: Vec::new(),
    };
    if manifest.confidence == ManifestConfidence::Partial || !manifest.issues.is_empty() {
        state.issues.push(ReplayIssue {
            event_id: None,
            path: None,
            kind: ReplayIssueKind::BaseIncomplete,
            detail: "base manifest was captured partially".into(),
        });
    }

    for file in &manifest.files {
        let git_base = match &file.git_base {
            GitBaseEntry::Absent => ReplayContent::Absent,
            GitBaseEntry::Blob { object_id } => resolve_git(object_id, git, &file.path, &mut state),
        };
        let index = resolve_layer(
            &file.index,
            &git_base,
            artifacts,
            store,
            &file.path,
            &mut state,
        );
        let disk = resolve_layer(&file.disk, &index, artifacts, store, &file.path, &mut state);
        let buffer = file.editor.as_ref().map(|overlay| {
            resolve_artifact(
                &overlay.artifact_id,
                artifacts,
                store,
                Some(&file.path),
                None,
                &mut state,
            )
        });
        state.files.insert(
            file.path.clone(),
            ReplayFile {
                file_kind: file.file_kind.clone(),
                executable: file.executable,
                git_base,
                index,
                disk,
                buffer,
            },
        );
    }
    for unsaved in &manifest.unsaved_buffers {
        let content = resolve_artifact(
            &unsaved.artifact_id,
            artifacts,
            store,
            None,
            None,
            &mut state,
        );
        state
            .unsaved_buffers
            .insert(unsaved.entry_id.clone(), content);
    }
    state
}

fn resolve_git(
    object_id: &str,
    git: Option<&dyn ReplayGitResolver>,
    path: &RepoPath,
    state: &mut ReplayState,
) -> ReplayContent {
    let resolved = git
        .ok_or_else(|| "no read-only Git object resolver was supplied".to_string())
        .and_then(|resolver| {
            resolver
                .read_blob(object_id)?
                .ok_or_else(|| "Git object is unavailable".to_string())
        });
    match resolved {
        Ok(bytes) => {
            let matches = Oid::hash_object(ObjectType::Blob, &bytes)
                .map(|actual| actual.to_string() == object_id)
                .unwrap_or(false);
            if matches {
                ReplayContent::Available {
                    bytes,
                    artifact_id: None,
                }
            } else {
                state.issues.push(ReplayIssue {
                    event_id: None,
                    path: Some(path.clone()),
                    kind: ReplayIssueKind::GitObjectMismatch,
                    detail: format!("resolved Git blob does not hash to {object_id}"),
                });
                ReplayContent::Unavailable {
                    reason: "resolved Git blob failed identity verification".into(),
                }
            }
        }
        Err(reason) => {
            state.issues.push(ReplayIssue {
                event_id: None,
                path: Some(path.clone()),
                kind: ReplayIssueKind::GitObjectUnavailable,
                detail: reason.clone(),
            });
            ReplayContent::Unavailable { reason }
        }
    }
}

fn resolve_layer(
    layer: &ManifestLayer,
    inherited: &ReplayContent,
    artifacts: &ArtifactCatalog<'_>,
    store: &ArtifactStore,
    path: &RepoPath,
    state: &mut ReplayState,
) -> ReplayContent {
    match layer {
        ManifestLayer::Inherit => inherited.clone(),
        ManifestLayer::Deleted => ReplayContent::Absent,
        ManifestLayer::Artifact { artifact_id } => {
            resolve_artifact(artifact_id, artifacts, store, Some(path), None, state)
        }
    }
}

fn resolve_artifact(
    artifact_id: &ArtifactId,
    artifacts: &ArtifactCatalog<'_>,
    store: &ArtifactStore,
    path: Option<&RepoPath>,
    event_id: Option<&EventId>,
    state: &mut ReplayState,
) -> ReplayContent {
    match artifacts.resolve(artifact_id, store) {
        Ok(content) => content,
        Err(reason) => {
            state.issues.push(ReplayIssue {
                event_id: event_id.cloned(),
                path: path.cloned(),
                kind: if artifacts.records.contains_key(artifact_id) {
                    ReplayIssueKind::ArtifactUnavailable
                } else {
                    ReplayIssueKind::ArtifactMetadataMissing
                },
                detail: reason.clone(),
            });
            ReplayContent::Unavailable { reason }
        }
    }
}

struct NormalizedHistory {
    transitions: Vec<Transition>,
    event_sequences: HashMap<EventId, u64>,
    turn_sequences: HashMap<TurnId, u64>,
    divergences: Vec<(u64, ReplayIssue)>,
}

fn normalize_transitions(
    events: &[EventEnvelope],
    state: &mut ReplayState,
) -> Result<NormalizedHistory, ReplayError> {
    let mut ordered: Vec<_> = events.iter().collect();
    ordered.sort_by_key(|event| event.sequence);
    for pair in ordered.windows(2) {
        if pair[0].sequence == pair[1].sequence {
            return Err(ReplayError::DuplicateSequence(pair[0].sequence));
        }
    }
    let positions: HashMap<_, _> = ordered
        .iter()
        .enumerate()
        .map(|(position, event)| (&event.event_id, position))
        .collect();
    for (position, event) in ordered.iter().enumerate() {
        if let Some(cause) = &event.caused_by {
            match positions.get(cause) {
                Some(cause_position) if *cause_position < position => {}
                Some(_) => return Err(ReplayError::CauseNotPrior(event.event_id.clone())),
                None => state.issues.push(ReplayIssue {
                    event_id: Some(event.event_id.clone()),
                    path: None,
                    kind: ReplayIssueKind::CausalHistoryIncomplete,
                    detail: format!("causal predecessor {cause} is outside this replay slice"),
                }),
            }
        }
    }
    let event_sequences = ordered
        .iter()
        .map(|event| (event.event_id.clone(), event.sequence))
        .collect();
    let mut turn_sequences = HashMap::new();
    let mut divergences = Vec::new();
    for event in &ordered {
        if let Some(turn_id) = &event.turn_id {
            turn_sequences
                .entry(turn_id.clone())
                .and_modify(|sequence: &mut u64| *sequence = (*sequence).max(event.sequence))
                .or_insert(event.sequence);
        }
        if let EventKind::Divergence(divergence) = &event.kind {
            divergences.push((
                event.sequence,
                ReplayIssue {
                    event_id: Some(event.event_id.clone()),
                    path: RepoPath::parse(&divergence.scope).ok(),
                    kind: ReplayIssueKind::RecordedDivergence,
                    detail: divergence
                        .detail
                        .clone()
                        .unwrap_or_else(|| "workspace divergence was recorded".into()),
                },
            ));
        }
    }
    let transitions = ordered
        .into_iter()
        .filter_map(|event| match &event.kind {
            EventKind::FileMutation(mutation) if mutation.state == FileMutationState::Completed => {
                Some(Transition {
                    envelope: event.clone(),
                    mutation: mutation.clone(),
                })
            }
            _ => None,
        })
        .collect();
    Ok(NormalizedHistory {
        transitions,
        event_sequences,
        turn_sequences,
        divergences,
    })
}

fn apply_transition(
    state: &mut ReplayState,
    transition: &Transition,
    transition_artifacts: &HashMap<ArtifactId, ReplayContent>,
) -> ReplayStep {
    let event_id = &transition.envelope.event_id;
    let mutation = &transition.mutation;
    let mut issues = Vec::new();
    let path = match RepoPath::parse(&mutation.path) {
        Ok(path) => path,
        Err(error) => {
            issues.push(ReplayIssue {
                event_id: Some(event_id.clone()),
                path: None,
                kind: ReplayIssueKind::PreimageMismatch,
                detail: error.to_string(),
            });
            return finish_step(
                transition,
                Vec::new(),
                Replayability::NotReplayable,
                issues,
                state,
            );
        }
    };
    let previous_path = match &mutation.previous_path {
        Some(previous) => match RepoPath::parse(previous) {
            Ok(previous) => Some(previous),
            Err(error) => {
                issues.push(ReplayIssue {
                    event_id: Some(event_id.clone()),
                    path: None,
                    kind: ReplayIssueKind::PreimageMismatch,
                    detail: error.to_string(),
                });
                return finish_step(
                    transition,
                    vec![path],
                    Replayability::NotReplayable,
                    issues,
                    state,
                );
            }
        },
        None => None,
    };
    let source_path = previous_path.as_ref().unwrap_or(&path);
    let source_executable = state.files.get(source_path).map(|file| file.executable);
    let current = state
        .files
        .get(source_path)
        .map(|file| file.surface(&mutation.surface).clone())
        .unwrap_or(ReplayContent::Absent);
    let expected = resolve_event_ref(
        &mutation.before_artifact,
        event_id,
        source_path,
        state,
        transition_artifacts,
    );
    let after = resolve_event_ref(
        &mutation.after_artifact,
        event_id,
        &path,
        state,
        transition_artifacts,
    );

    let mut changed: BTreeSet<RepoPath> = BTreeSet::from([path.clone()]);
    if let Some(previous) = &previous_path {
        changed.insert(previous.clone());
    }

    let (expected, after) = match (expected, after) {
        (Ok(expected), Ok(after)) => (expected, after),
        _ => {
            issues.push(ReplayIssue {
                event_id: Some(event_id.clone()),
                path: Some(path.clone()),
                kind: ReplayIssueKind::ArtifactUnavailable,
                detail: "transition preimage or postimage is unavailable".into(),
            });
            poison_transition_paths(
                state,
                source_path,
                &path,
                mutation,
                "transition artifact unavailable",
            );
            return finish_step(
                transition,
                changed.into_iter().collect(),
                Replayability::NotReplayable,
                issues,
                state,
            );
        }
    };
    if !content_matches(&current, &expected) {
        issues.push(ReplayIssue {
            event_id: Some(event_id.clone()),
            path: Some(source_path.clone()),
            kind: ReplayIssueKind::PreimageMismatch,
            detail: "recorded preimage does not match the reconstructed predecessor state".into(),
        });
        poison_transition_paths(
            state,
            source_path,
            &path,
            mutation,
            "recorded preimage mismatch",
        );
        return finish_step(
            transition,
            changed.into_iter().collect(),
            Replayability::NotReplayable,
            issues,
            state,
        );
    }

    if previous_path
        .as_ref()
        .is_some_and(|previous| previous != &path)
    {
        set_surface(
            state,
            source_path,
            &mutation.surface,
            ReplayContent::Absent,
            mutation.file_kind.clone(),
        );
    }
    set_surface(
        state,
        &path,
        &mutation.surface,
        after,
        mutation.file_kind.clone(),
    );
    if previous_path
        .as_ref()
        .is_some_and(|previous| previous != &path)
    {
        if let (Some(executable), Some(destination)) =
            (source_executable, state.files.get_mut(&path))
        {
            destination.executable = executable;
        }
    }
    finish_step(
        transition,
        changed.into_iter().collect(),
        Replayability::Exact,
        issues,
        state,
    )
}

fn content_matches(left: &ReplayContent, right: &ReplayContent) -> bool {
    match (left, right) {
        (ReplayContent::Absent, ReplayContent::Absent) => true,
        (
            ReplayContent::Available { bytes: left, .. },
            ReplayContent::Available { bytes: right, .. },
        ) => left == right,
        _ => false,
    }
}

fn resolve_event_ref(
    artifact: &Option<ArtifactRef>,
    event_id: &EventId,
    path: &RepoPath,
    state: &mut ReplayState,
    transition_artifacts: &HashMap<ArtifactId, ReplayContent>,
) -> Result<ReplayContent, ()> {
    match artifact {
        None => Ok(ReplayContent::Absent),
        Some(artifact) => match transition_artifacts.get(&artifact.artifact_id).cloned() {
            Some(content) if content.is_replayable() => Ok(content),
            _ => {
                state.issues.push(ReplayIssue {
                    event_id: Some(event_id.clone()),
                    path: Some(path.clone()),
                    kind: ReplayIssueKind::ArtifactUnavailable,
                    detail: format!(
                        "transition artifact {} is unavailable",
                        artifact.artifact_id
                    ),
                });
                Err(())
            }
        },
    }
}

fn poison_transition_paths(
    state: &mut ReplayState,
    source: &RepoPath,
    destination: &RepoPath,
    mutation: &FileMutationEvent,
    reason: &str,
) {
    set_surface(
        state,
        source,
        &mutation.surface,
        ReplayContent::Unavailable {
            reason: reason.into(),
        },
        mutation.file_kind.clone(),
    );
    if source != destination {
        set_surface(
            state,
            destination,
            &mutation.surface,
            ReplayContent::Unavailable {
                reason: reason.into(),
            },
            mutation.file_kind.clone(),
        );
    }
}

fn set_surface(
    state: &mut ReplayState,
    path: &RepoPath,
    surface: &WorkspaceSurface,
    content: ReplayContent,
    file_kind: FileKind,
) {
    let file = state
        .files
        .entry(path.clone())
        .or_insert_with(|| ReplayFile {
            file_kind: file_kind.clone(),
            executable: false,
            git_base: ReplayContent::Absent,
            index: ReplayContent::Absent,
            disk: ReplayContent::Absent,
            buffer: None,
        });
    file.file_kind = file_kind;
    file.set_surface(surface, content);
}

fn finish_step(
    transition: &Transition,
    changed_files: Vec<RepoPath>,
    replayability: Replayability,
    issues: Vec<ReplayIssue>,
    state: &mut ReplayState,
) -> ReplayStep {
    state.issues.extend(issues.clone());
    ReplayStep {
        event_id: transition.envelope.event_id.clone(),
        sequence: transition.envelope.sequence,
        turn_id: transition.envelope.turn_id.clone(),
        changed_files,
        replayability,
        issues,
    }
}

fn state_replayability(state: &ReplayState) -> Replayability {
    if state
        .issues
        .iter()
        .any(|issue| issue.kind == ReplayIssueKind::RecordedDivergence)
    {
        return Replayability::Diverged;
    }
    if state
        .files
        .values()
        .any(|file| !file.visible_content().is_replayable())
        || state
            .unsaved_buffers
            .values()
            .any(|content| !content.is_replayable())
        || !state.issues.is_empty()
    {
        Replayability::Partial
    } else {
        Replayability::Exact
    }
}

#[derive(Debug)]
pub enum ReplayError {
    ConflictingArtifactMetadata(ArtifactId),
    DuplicateEventId(EventId),
    DuplicateSequence(u64),
    CauseNotPrior(EventId),
    UnknownEventBoundary(EventId),
    UnknownTurnBoundary(TurnId),
    AmbiguousBranches,
    UnknownBranch(BranchId),
}

impl fmt::Display for ReplayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConflictingArtifactMetadata(id) => {
                write!(formatter, "conflicting metadata for artifact {id}")
            }
            Self::DuplicateEventId(id) => write!(formatter, "duplicate event ID {id}"),
            Self::DuplicateSequence(sequence) => {
                write!(formatter, "duplicate event sequence {sequence}")
            }
            Self::CauseNotPrior(id) => write!(formatter, "event {id} names a non-prior cause"),
            Self::UnknownEventBoundary(id) => {
                write!(
                    formatter,
                    "event boundary {id} is not present in replay history"
                )
            }
            Self::UnknownTurnBoundary(id) => {
                write!(
                    formatter,
                    "turn boundary {id} is not present in replay history"
                )
            }
            Self::AmbiguousBranches => formatter
                .write_str("run contains multiple branches; select one explicitly for replay"),
            Self::UnknownBranch(id) => write!(formatter, "branch {id} has no events in this run"),
        }
    }
}

impl std::error::Error for ReplayError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run_log::{
        ArtifactExportPolicy, ArtifactRetention, ArtifactSource, BaseManifestId, BlobId,
        ContentRepresentation, DivergenceEvent, EventActor, ManifestFile, MessageEvent,
        MessageRole, RepositoryBase, RepositoryId, RunId, EVENT_PAYLOAD_VERSION,
        EVENT_SCHEMA_VERSION,
    };
    use tempfile::TempDir;

    struct Fixture {
        _directory: TempDir,
        store: ArtifactStore,
        base_records: Vec<ArtifactRecord>,
        additional_records: Vec<ArtifactRecord>,
    }

    impl Fixture {
        fn new() -> Self {
            let directory = tempfile::tempdir().unwrap();
            let store = ArtifactStore::open(directory.path()).unwrap();
            Self {
                _directory: directory,
                store,
                base_records: Vec::new(),
                additional_records: Vec::new(),
            }
        }

        fn base_artifact(&mut self, label: &str, bytes: &[u8]) -> ArtifactRef {
            let record = record(&self.store, label, bytes);
            let reference = record.as_ref();
            self.base_records.push(record);
            reference
        }

        fn transition_artifact(&mut self, label: &str, bytes: &[u8]) -> ArtifactRef {
            let record = record(&self.store, label, bytes);
            let reference = record.as_ref();
            self.additional_records.push(record);
            reference
        }

        fn manifest(&self, files: Vec<(&str, &ArtifactRef)>) -> BaseManifest {
            BaseManifest {
                base_manifest_id: BaseManifestId::parse("bsm_replay_test").unwrap(),
                captured_at: "2026-07-13T00:00:00Z".into(),
                repository: RepositoryBase {
                    repository_id: RepositoryId::parse("repo_replay_test").unwrap(),
                    head_commit: None,
                    index_tree: None,
                },
                files: files
                    .into_iter()
                    .map(|(path, artifact)| ManifestFile {
                        path: RepoPath::parse(path).unwrap(),
                        file_kind: FileKind::Regular,
                        executable: false,
                        git_base: GitBaseEntry::Absent,
                        index: ManifestLayer::Inherit,
                        disk: ManifestLayer::Artifact {
                            artifact_id: artifact.artifact_id.clone(),
                        },
                        editor: None,
                    })
                    .collect(),
                unsaved_buffers: Vec::new(),
                artifacts: self.base_records.clone(),
                captured_bytes: self
                    .base_records
                    .iter()
                    .filter_map(|record| match record.state {
                        ArtifactState::Available { byte_len, .. } => Some(byte_len),
                        _ => None,
                    })
                    .sum(),
                confidence: ManifestConfidence::Complete,
                issues: Vec::new(),
            }
        }
    }

    fn record(store: &ArtifactStore, label: &str, bytes: &[u8]) -> ArtifactRecord {
        let stored = store.put_bytes(bytes).unwrap();
        ArtifactRecord {
            artifact_id: ArtifactId::parse(format!("art_{label}")).unwrap(),
            state: ArtifactState::Available {
                blob_id: stored.blob_id,
                byte_len: stored.byte_len,
            },
            source: ArtifactSource::Imported {
                label: Some(label.into()),
            },
            representation: ContentRepresentation::RawBytes,
            media_type: None,
            retention: ArtifactRetention::Run,
            export_policy: ArtifactExportPolicy::Include,
        }
    }

    fn event(
        sequence: u64,
        kind: EventKind,
        turn: Option<&str>,
        cause: Option<u64>,
    ) -> EventEnvelope {
        EventEnvelope {
            schema_version: EVENT_SCHEMA_VERSION,
            payload_version: EVENT_PAYLOAD_VERSION,
            event_id: EventId::parse(format!("evt_{sequence}")).unwrap(),
            run_id: RunId::parse("run_replay_test").unwrap(),
            sequence,
            recorded_at: "2026-07-13T00:00:00Z".into(),
            caused_by: cause.map(|cause| EventId::parse(format!("evt_{cause}")).unwrap()),
            operation_id: None,
            provider_call_id: None,
            actor: EventActor::System("replay-test".into()),
            agent_id: None,
            turn_id: turn.map(|turn| TurnId::parse(turn).unwrap()),
            workspace_id: None,
            branch_id: None,
            kind,
        }
    }

    fn mutation(path: &str, before: Option<ArtifactRef>, after: Option<ArtifactRef>) -> EventKind {
        EventKind::FileMutation(FileMutationEvent {
            path: path.into(),
            previous_path: None,
            surface: WorkspaceSurface::Disk,
            file_kind: FileKind::Regular,
            before_artifact: before,
            after_artifact: after,
            artifacts: Vec::new(),
            state: FileMutationState::Completed,
        })
    }

    #[test]
    fn steps_forward_and_reconstructs_event_and_turn_boundaries_deterministically() {
        let mut fixture = Fixture::new();
        let base = fixture.base_artifact("base", b"one");
        let before_one = fixture.transition_artifact("before_one", b"one");
        let after_two = fixture.transition_artifact("after_two", b"two");
        let before_two = fixture.transition_artifact("before_two", b"two");
        let after_three = fixture.transition_artifact("after_three", b"three");
        let manifest = fixture.manifest(vec![("src/lib.rs", &base)]);
        let events = vec![
            event(
                3,
                mutation("src/lib.rs", Some(before_two), Some(after_three)),
                Some("trn_second"),
                Some(2),
            ),
            event(
                1,
                mutation("src/lib.rs", Some(before_one), Some(after_two)),
                Some("trn_first"),
                None,
            ),
            event(
                2,
                EventKind::Message(MessageEvent {
                    role: MessageRole::Agent,
                    content: "first edit complete".into(),
                }),
                Some("trn_first"),
                Some(1),
            ),
        ];
        let mut replay = ReplaySession::new(
            &manifest,
            &fixture.additional_records,
            &events,
            &fixture.store,
            None,
        )
        .unwrap();

        assert_eq!(replay.steps_len(), 2);
        let first = replay.step_forward().unwrap();
        assert_eq!(first.sequence, 1);
        assert_eq!(first.replayability, Replayability::Exact);
        assert_eq!(
            first.changed_files,
            vec![RepoPath::parse("src/lib.rs").unwrap()]
        );
        assert_eq!(visible(&replay, "src/lib.rs"), Some(b"two".as_slice()));

        replay
            .reconstruct(ReplayBoundary::Event(EventId::parse("evt_2").unwrap()))
            .unwrap();
        assert_eq!(visible(&replay, "src/lib.rs"), Some(b"two".as_slice()));
        replay
            .reconstruct(ReplayBoundary::Turn(TurnId::parse("trn_second").unwrap()))
            .unwrap();
        assert_eq!(visible(&replay, "src/lib.rs"), Some(b"three".as_slice()));
        replay.reconstruct(ReplayBoundary::Start).unwrap();
        assert_eq!(visible(&replay, "src/lib.rs"), Some(b"one".as_slice()));
        replay.reconstruct(ReplayBoundary::End).unwrap();
        assert_eq!(visible(&replay, "src/lib.rs"), Some(b"three".as_slice()));
    }

    #[test]
    fn mutation_embedded_artifact_metadata_is_replayable_without_side_channel_records() {
        let fixture = Fixture::new();
        let manifest = fixture.manifest(Vec::new());
        let record = record(&fixture.store, "embedded_created", b"created by shell");
        let reference = record.as_ref();
        let mutation = FileMutationEvent {
            path: "created.txt".into(),
            previous_path: None,
            surface: WorkspaceSurface::Disk,
            file_kind: FileKind::Regular,
            before_artifact: None,
            after_artifact: Some(reference),
            artifacts: vec![record],
            state: FileMutationState::Completed,
        };
        let events = vec![event(1, EventKind::FileMutation(mutation), None, None)];
        let mut replay = ReplaySession::new(&manifest, &[], &events, &fixture.store, None).unwrap();

        replay.reconstruct(ReplayBoundary::End).unwrap();
        let comparison = replay.compare_observed_workspace(&[ObservedFile {
            path: RepoPath::parse("created.txt").unwrap(),
            bytes: Some(b"created by shell".to_vec()),
            surface: WorkspaceSurface::Disk,
        }]);
        assert!(comparison.divergences.is_empty());
        assert_eq!(comparison.replayability, Replayability::Exact);
    }

    #[test]
    fn missing_postimage_only_poisoned_the_dependent_path() {
        let mut fixture = Fixture::new();
        let base_a = fixture.base_artifact("base_a", b"a");
        let base_b = fixture.base_artifact("base_b", b"b");
        let before = fixture.transition_artifact("before_a", b"a");
        let missing_id = ArtifactId::parse("art_missing_after").unwrap();
        fixture.additional_records.push(ArtifactRecord {
            artifact_id: missing_id.clone(),
            state: ArtifactState::Available {
                blob_id: BlobId::digest(b"not stored"),
                byte_len: 10,
            },
            source: ArtifactSource::Imported { label: None },
            representation: ContentRepresentation::RawBytes,
            media_type: None,
            retention: ArtifactRetention::Run,
            export_policy: ArtifactExportPolicy::Include,
        });
        let missing = ArtifactRef {
            artifact_id: missing_id,
            availability: ArtifactAvailability::Available,
            representation: ContentRepresentation::RawBytes,
        };
        let manifest = fixture.manifest(vec![("a.rs", &base_a), ("b.rs", &base_b)]);
        let events = vec![event(
            1,
            mutation("a.rs", Some(before), Some(missing)),
            None,
            None,
        )];
        let mut replay = ReplaySession::new(
            &manifest,
            &fixture.additional_records,
            &events,
            &fixture.store,
            None,
        )
        .unwrap();

        let step = replay.step_forward().unwrap();
        assert_eq!(step.replayability, Replayability::NotReplayable);
        assert_eq!(replay.state().replayability, Replayability::Partial);
        assert!(matches!(
            replay.state().files[&RepoPath::parse("a.rs").unwrap()].visible_content(),
            ReplayContent::Unavailable { .. }
        ));
        assert_eq!(visible(&replay, "b.rs"), Some(b"b".as_slice()));
    }

    #[test]
    fn preimage_mismatch_is_explicit_and_does_not_apply_the_postimage() {
        let mut fixture = Fixture::new();
        let base = fixture.base_artifact("base_mismatch", b"actual");
        let before = fixture.transition_artifact("wrong_before", b"expected");
        let after = fixture.transition_artifact("after_mismatch", b"new");
        let manifest = fixture.manifest(vec![("src/lib.rs", &base)]);
        let events = vec![event(
            1,
            mutation("src/lib.rs", Some(before), Some(after)),
            None,
            None,
        )];
        let mut replay = ReplaySession::new(
            &manifest,
            &fixture.additional_records,
            &events,
            &fixture.store,
            None,
        )
        .unwrap();

        let step = replay.step_forward().unwrap();
        assert_eq!(step.replayability, Replayability::NotReplayable);
        assert!(step
            .issues
            .iter()
            .any(|issue| issue.kind == ReplayIssueKind::PreimageMismatch));
        assert_ne!(visible(&replay, "src/lib.rs"), Some(b"new".as_slice()));
    }

    #[test]
    fn compares_live_bytes_without_mutating_or_claiming_external_edits_are_exact() {
        let mut fixture = Fixture::new();
        let base = fixture.base_artifact("base_compare", b"historical");
        let manifest = fixture.manifest(vec![("src/lib.rs", &base)]);
        let replay = ReplaySession::new(&manifest, &[], &[], &fixture.store, None).unwrap();
        let comparison = replay.compare_observed_workspace(&[ObservedFile {
            path: RepoPath::parse("src/lib.rs").unwrap(),
            bytes: Some(b"manual edit".to_vec()),
            surface: WorkspaceSurface::Disk,
        }]);
        assert_eq!(comparison.replayability, Replayability::Diverged);
        assert_eq!(comparison.divergences.len(), 1);
        assert_eq!(
            visible(&replay, "src/lib.rs"),
            Some(b"historical".as_slice())
        );
    }

    #[test]
    fn complete_live_inventory_detects_manual_deletion_and_an_extra_file() {
        let mut fixture = Fixture::new();
        let base = fixture.base_artifact("inventory_base", b"expected");
        let manifest = fixture.manifest(vec![("expected.rs", &base)]);
        let replay = ReplaySession::new(&manifest, &[], &[], &fixture.store, None).unwrap();
        let comparison = replay.compare_observed_workspace(&[ObservedFile {
            path: RepoPath::parse("extra.rs").unwrap(),
            bytes: Some(b"manual extra".to_vec()),
            surface: WorkspaceSurface::Disk,
        }]);

        assert_eq!(comparison.replayability, Replayability::Diverged);
        assert_eq!(comparison.divergences.len(), 2);
        for expected in ["expected.rs", "extra.rs"] {
            assert!(comparison.divergences.iter().any(|issue| issue
                .path
                .as_ref()
                .is_some_and(|path| path.as_str() == expected)));
        }
    }

    #[test]
    fn recorded_divergence_only_affects_boundaries_at_or_after_its_event() {
        let mut fixture = Fixture::new();
        let base = fixture.base_artifact("base_divergence", b"base");
        let manifest = fixture.manifest(vec![("src/lib.rs", &base)]);
        let events = vec![
            event(
                1,
                EventKind::Message(MessageEvent {
                    role: MessageRole::User,
                    content: "start".into(),
                }),
                None,
                None,
            ),
            event(
                2,
                EventKind::Divergence(DivergenceEvent {
                    scope: "src/lib.rs".into(),
                    expected_artifact: None,
                    actual_artifact: None,
                    replayability: Replayability::Diverged,
                    detail: Some("changed outside ovim".into()),
                }),
                None,
                Some(1),
            ),
        ];
        let mut replay = ReplaySession::new(&manifest, &[], &events, &fixture.store, None).unwrap();
        replay
            .reconstruct(ReplayBoundary::Event(EventId::parse("evt_1").unwrap()))
            .unwrap();
        assert_eq!(replay.state().replayability, Replayability::Exact);
        replay
            .reconstruct(ReplayBoundary::Event(EventId::parse("evt_2").unwrap()))
            .unwrap();
        assert_eq!(replay.state().replayability, Replayability::Diverged);
    }

    #[test]
    fn divergence_crossed_by_a_transition_marks_that_step_diverged() {
        let mut fixture = Fixture::new();
        let base = fixture.base_artifact("step_divergence_base", b"base");
        let before = fixture.transition_artifact("step_divergence_before", b"base");
        let after = fixture.transition_artifact("step_divergence_after", b"changed");
        let manifest = fixture.manifest(vec![("src/lib.rs", &base)]);
        let events = vec![
            event(
                1,
                EventKind::Divergence(DivergenceEvent {
                    scope: "src/lib.rs".into(),
                    expected_artifact: None,
                    actual_artifact: None,
                    replayability: Replayability::Diverged,
                    detail: Some("external edit observed".into()),
                }),
                None,
                None,
            ),
            event(
                2,
                mutation("src/lib.rs", Some(before), Some(after)),
                None,
                Some(1),
            ),
        ];
        let mut replay = ReplaySession::new(
            &manifest,
            &fixture.additional_records,
            &events,
            &fixture.store,
            None,
        )
        .unwrap();

        let step = replay.step_forward().unwrap();
        assert_eq!(step.replayability, Replayability::Diverged);
        assert!(step
            .issues
            .iter()
            .any(|issue| issue.kind == ReplayIssueKind::RecordedDivergence));
        assert_eq!(replay.state().replayability, Replayability::Diverged);
    }

    #[test]
    fn rename_moves_the_recorded_surface_and_reports_both_paths() {
        let mut fixture = Fixture::new();
        let base = fixture.base_artifact("rename_base", b"contents");
        let before = fixture.transition_artifact("rename_before", b"contents");
        let after = fixture.transition_artifact("rename_after", b"contents");
        let mut manifest = fixture.manifest(vec![("old.rs", &base)]);
        manifest.files[0].executable = true;
        let mut rename = match mutation("new.rs", Some(before), Some(after)) {
            EventKind::FileMutation(rename) => rename,
            _ => unreachable!(),
        };
        rename.previous_path = Some("old.rs".into());
        let events = vec![event(1, EventKind::FileMutation(rename), None, None)];
        let mut replay = ReplaySession::new(
            &manifest,
            &fixture.additional_records,
            &events,
            &fixture.store,
            None,
        )
        .unwrap();

        let step = replay.step_forward().unwrap();
        assert_eq!(step.replayability, Replayability::Exact);
        assert_eq!(
            step.changed_files,
            vec![
                RepoPath::parse("new.rs").unwrap(),
                RepoPath::parse("old.rs").unwrap()
            ]
        );
        assert_eq!(visible(&replay, "new.rs"), Some(b"contents".as_slice()));
        assert_eq!(visible(&replay, "old.rs"), None);
        assert!(replay.state().files[&RepoPath::parse("new.rs").unwrap()].executable);
    }

    #[test]
    fn explicit_branch_replay_follows_ancestry_without_merging_siblings() {
        let mut fixture = Fixture::new();
        let base = fixture.base_artifact("branch_base", b"base");
        let before_base = fixture.transition_artifact("branch_before_base", b"base");
        let parent_after = fixture.transition_artifact("branch_parent_after", b"parent");
        let before_a = fixture.transition_artifact("branch_before_a", b"parent");
        let after_a = fixture.transition_artifact("branch_after_a", b"sibling-a");
        let before_b = fixture.transition_artifact("branch_before_b", b"parent");
        let after_b = fixture.transition_artifact("branch_after_b", b"sibling-b");
        let manifest = fixture.manifest(vec![("src/lib.rs", &base)]);
        let parent = BranchId::parse("brn_parent").unwrap();
        let sibling_a = BranchId::parse("brn_sibling_a").unwrap();
        let sibling_b = BranchId::parse("brn_sibling_b").unwrap();
        let mut parent_event = event(
            1,
            mutation("src/lib.rs", Some(before_base), Some(parent_after)),
            None,
            None,
        );
        parent_event.branch_id = Some(parent);
        let mut event_a = event(
            2,
            mutation("src/lib.rs", Some(before_a), Some(after_a)),
            None,
            Some(1),
        );
        event_a.branch_id = Some(sibling_a.clone());
        let mut event_b = event(
            3,
            mutation("src/lib.rs", Some(before_b), Some(after_b)),
            None,
            Some(1),
        );
        event_b.branch_id = Some(sibling_b.clone());
        let events = vec![parent_event, event_a, event_b];

        assert!(matches!(
            ReplaySession::new(
                &manifest,
                &fixture.additional_records,
                &events,
                &fixture.store,
                None,
            ),
            Err(ReplayError::AmbiguousBranches)
        ));
        let mut replay_a = ReplaySession::new_for_branch(
            &manifest,
            &fixture.additional_records,
            &events,
            &fixture.store,
            None,
            sibling_a,
        )
        .unwrap();
        replay_a.reconstruct(ReplayBoundary::End).unwrap();
        assert_eq!(
            visible(&replay_a, "src/lib.rs"),
            Some(b"sibling-a".as_slice())
        );

        let mut replay_b = ReplaySession::new_for_branch(
            &manifest,
            &fixture.additional_records,
            &events,
            &fixture.store,
            None,
            sibling_b,
        )
        .unwrap();
        replay_b.reconstruct(ReplayBoundary::End).unwrap();
        assert_eq!(
            visible(&replay_b, "src/lib.rs"),
            Some(b"sibling-b".as_slice())
        );
    }

    #[test]
    fn verifies_git_base_bytes_instead_of_trusting_a_resolver() {
        struct Resolver {
            bytes: Vec<u8>,
        }
        impl ReplayGitResolver for Resolver {
            fn read_blob(&self, _object_id: &str) -> Result<Option<Vec<u8>>, String> {
                Ok(Some(self.bytes.clone()))
            }
        }

        let fixture = Fixture::new();
        let mut manifest = fixture.manifest(Vec::new());
        let expected = b"clean source";
        let object_id = Oid::hash_object(ObjectType::Blob, expected)
            .unwrap()
            .to_string();
        manifest.files.push(ManifestFile {
            path: RepoPath::parse("clean.rs").unwrap(),
            file_kind: FileKind::Regular,
            executable: false,
            git_base: GitBaseEntry::Blob {
                object_id: object_id.clone(),
            },
            index: ManifestLayer::Inherit,
            disk: ManifestLayer::Inherit,
            editor: None,
        });
        let exact = ReplaySession::new(
            &manifest,
            &[],
            &[],
            &fixture.store,
            Some(&Resolver {
                bytes: expected.to_vec(),
            }),
        )
        .unwrap();
        assert_eq!(exact.state().replayability, Replayability::Exact);

        let mismatch = ReplaySession::new(
            &manifest,
            &[],
            &[],
            &fixture.store,
            Some(&Resolver {
                bytes: b"different checkout".to_vec(),
            }),
        )
        .unwrap();
        assert_eq!(mismatch.state().replayability, Replayability::Partial);
        assert!(mismatch
            .state()
            .issues
            .iter()
            .any(|issue| issue.kind == ReplayIssueKind::GitObjectMismatch));
    }

    #[test]
    fn rejects_duplicate_sequences_and_non_prior_known_causes() {
        let fixture = Fixture::new();
        let manifest = fixture.manifest(Vec::new());
        let first = event(
            1,
            EventKind::Message(MessageEvent {
                role: MessageRole::User,
                content: "first".into(),
            }),
            None,
            None,
        );
        let mut reused_id = event(
            2,
            EventKind::Message(MessageEvent {
                role: MessageRole::Agent,
                content: "second".into(),
            }),
            None,
            Some(1),
        );
        reused_id.event_id = first.event_id.clone();
        assert!(matches!(
            ReplaySession::new(
                &manifest,
                &[],
                &[first, reused_id],
                &fixture.store,
                None,
            ),
            Err(ReplayError::DuplicateEventId(id)) if id.as_str() == "evt_1"
        ));

        let mut duplicates = vec![
            event(
                1,
                EventKind::Message(MessageEvent {
                    role: MessageRole::User,
                    content: "a".into(),
                }),
                None,
                None,
            ),
            event(
                1,
                EventKind::Message(MessageEvent {
                    role: MessageRole::Agent,
                    content: "b".into(),
                }),
                None,
                None,
            ),
        ];
        duplicates[1].event_id = EventId::parse("evt_duplicate_sequence").unwrap();
        assert!(matches!(
            ReplaySession::new(&manifest, &[], &duplicates, &fixture.store, None),
            Err(ReplayError::DuplicateSequence(1))
        ));
        let future_cause = vec![
            event(
                1,
                EventKind::Message(MessageEvent {
                    role: MessageRole::User,
                    content: "a".into(),
                }),
                None,
                Some(2),
            ),
            event(
                2,
                EventKind::Message(MessageEvent {
                    role: MessageRole::Agent,
                    content: "b".into(),
                }),
                None,
                None,
            ),
        ];
        assert!(matches!(
            ReplaySession::new(&manifest, &[], &future_cause, &fixture.store, None),
            Err(ReplayError::CauseNotPrior(_))
        ));
    }

    fn visible<'a>(replay: &'a ReplaySession, path: &str) -> Option<&'a [u8]> {
        replay.state().files[&RepoPath::parse(path).unwrap()]
            .visible_content()
            .bytes()
    }
}
