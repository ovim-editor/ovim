use super::{
    AgentId, BaseManifestId, BranchId, EventId, OperationId, RepositoryId, RunId, TurnId,
    WorkspaceId,
};
use serde::{
    de::{self, DeserializeOwned},
    Deserialize, Deserializer, Serialize, Serializer,
};
use serde_json::Value;

pub const EVENT_SCHEMA_VERSION: u32 = 1;
pub const EVENT_PAYLOAD_VERSION: u32 = 1;

/// The principal responsible for an event. Provider sessions are deliberately
/// absent: they are replaceable adapter metadata, not ovim identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "id", rename_all = "snake_case")]
pub enum EventActor {
    User,
    Agent(AgentId),
    System(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub schema_version: u32,
    pub payload_version: u32,
    pub event_id: EventId,
    pub run_id: RunId,
    pub sequence: u64,
    /// RFC 3339 UTC timestamp. Sequence, rather than wall time, defines order.
    pub recorded_at: String,
    pub caused_by: Option<EventId>,
    pub operation_id: Option<OperationId>,
    /// Opaque provider/tool call identifier, never used as ovim identity.
    pub provider_call_id: Option<String>,
    pub actor: EventActor,
    pub agent_id: Option<AgentId>,
    pub turn_id: Option<TurnId>,
    pub workspace_id: Option<WorkspaceId>,
    /// Durable conversation trajectory. Editor tree/node identifiers remain a
    /// transient projection and must not be persisted here.
    #[serde(default)]
    pub branch_id: Option<BranchId>,
    pub kind: EventKind,
}

/// An event before store-owned ordering and recording metadata is assigned.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NewRunEvent {
    pub run_id: RunId,
    pub caused_by: Option<EventId>,
    pub operation_id: Option<OperationId>,
    pub provider_call_id: Option<String>,
    pub actor: EventActor,
    pub agent_id: Option<AgentId>,
    pub turn_id: Option<TurnId>,
    pub workspace_id: Option<WorkspaceId>,
    #[serde(default)]
    pub branch_id: Option<BranchId>,
    pub kind: EventKind,
}

impl NewRunEvent {
    pub fn new(run_id: RunId, actor: EventActor, kind: EventKind) -> Self {
        Self {
            run_id,
            caused_by: None,
            operation_id: None,
            provider_call_id: None,
            actor,
            agent_id: None,
            turn_id: None,
            workspace_id: None,
            branch_id: None,
            kind,
        }
    }

    pub fn in_turn(mut self, turn_id: TurnId) -> Self {
        self.turn_id = Some(turn_id);
        self
    }

    pub fn caused_by(mut self, event_id: EventId) -> Self {
        self.caused_by = Some(event_id);
        self
    }

    pub fn for_provider_call(mut self, provider_call_id: impl Into<String>) -> Self {
        self.provider_call_id = Some(provider_call_id.into());
        self
    }

    pub fn for_operation(mut self, operation_id: OperationId) -> Self {
        self.operation_id = Some(operation_id);
        self
    }

    pub fn for_agent(mut self, agent_id: AgentId) -> Self {
        self.agent_id = Some(agent_id);
        self
    }

    pub fn in_workspace(mut self, workspace_id: WorkspaceId) -> Self {
        self.workspace_id = Some(workspace_id);
        self
    }

    pub fn in_branch(mut self, branch_id: BranchId) -> Self {
        self.branch_id = Some(branch_id);
        self
    }
}

/// Version-one normalized event payload. New semantic shapes should normally be
/// added as variants; incompatible changes require a new payload version.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum EventKind {
    RunLifecycle(RunLifecycleEvent),
    BranchLifecycle(BranchLifecycleEvent),
    AgentLifecycle(AgentLifecycleEvent),
    TurnLifecycle(TurnLifecycleEvent),
    Message(MessageEvent),
    ToolIntent(ToolIntentEvent),
    ToolDecision(ToolDecisionEvent),
    ToolStarted(ToolStartedEvent),
    ToolResult(ToolResultEvent),
    FileMutation(FileMutationEvent),
    Checkpoint(CheckpointEvent),
    Divergence(DivergenceEvent),
    /// A future or extension event retained without semantic interpretation.
    Unknown {
        name: String,
        payload: Value,
    },
}

#[derive(Serialize, Deserialize)]
struct RawEventKind {
    kind: String,
    #[serde(default)]
    data: Value,
}

impl Serialize for EventKind {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let (kind, data) = match self {
            Self::RunLifecycle(value) => ("run_lifecycle", serde_json::to_value(value)),
            Self::BranchLifecycle(value) => ("branch_lifecycle", serde_json::to_value(value)),
            Self::AgentLifecycle(value) => ("agent_lifecycle", serde_json::to_value(value)),
            Self::TurnLifecycle(value) => ("turn_lifecycle", serde_json::to_value(value)),
            Self::Message(value) => ("message", serde_json::to_value(value)),
            Self::ToolIntent(value) => ("tool_intent", serde_json::to_value(value)),
            Self::ToolDecision(value) => ("tool_decision", serde_json::to_value(value)),
            Self::ToolStarted(value) => ("tool_started", serde_json::to_value(value)),
            Self::ToolResult(value) => ("tool_result", serde_json::to_value(value)),
            Self::FileMutation(value) => ("file_mutation", serde_json::to_value(value)),
            Self::Checkpoint(value) => ("checkpoint", serde_json::to_value(value)),
            Self::Divergence(value) => ("divergence", serde_json::to_value(value)),
            Self::Unknown {
                name,
                payload: data,
            } => {
                return RawEventKind {
                    kind: name.clone(),
                    data: data.clone(),
                }
                .serialize(serializer)
            }
        };
        RawEventKind {
            kind: kind.into(),
            data: data.map_err(serde::ser::Error::custom)?,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for EventKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawEventKind::deserialize(deserializer)?;
        match raw.kind.as_str() {
            "run_lifecycle" => decode(raw.data).map(Self::RunLifecycle),
            "branch_lifecycle" => decode(raw.data).map(Self::BranchLifecycle),
            "agent_lifecycle" => decode(raw.data).map(Self::AgentLifecycle),
            "turn_lifecycle" => decode(raw.data).map(Self::TurnLifecycle),
            "message" => decode(raw.data).map(Self::Message),
            "tool_intent" => decode(raw.data).map(Self::ToolIntent),
            "tool_decision" => decode(raw.data).map(Self::ToolDecision),
            "tool_started" => decode(raw.data).map(Self::ToolStarted),
            "tool_result" => decode(raw.data).map(Self::ToolResult),
            "file_mutation" => decode(raw.data).map(Self::FileMutation),
            "checkpoint" => decode(raw.data).map(Self::Checkpoint),
            "divergence" => decode(raw.data).map(Self::Divergence),
            _ => Ok(Self::Unknown {
                name: raw.kind,
                payload: raw.data,
            }),
        }
    }
}

fn decode<T, E>(value: Value) -> Result<T, E>
where
    T: DeserializeOwned,
    E: de::Error,
{
    serde_json::from_value(value).map_err(E::custom)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunLifecycleEvent {
    pub state: RunLifecycleState,
    pub objective: Option<String>,
    pub detail: Option<String>,
    /// Present on `Created`. Optional/defaulted so historical envelopes remain
    /// readable and non-creation lifecycle records stay compact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creation: Option<RunCreationMetadata>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunCreationMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository_id: Option<RepositoryId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_manifest_id: Option<BaseManifestId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_branch_id: Option<BranchId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchLifecycleEvent {
    pub state: BranchLifecycleState,
    pub branch_id: BranchId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_branch_id: Option<BranchId>,
    /// Exact event in the parent trajectory from which this branch diverged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forked_at: Option<EventId>,
    /// Human-facing locator at the time of the event; never used as identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BranchLifecycleState {
    Created,
    Forked,
    Selected,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunLifecycleState {
    Created,
    ObjectiveUpdated,
    Completed,
    Interrupted,
    Failed,
    Abandoned,
    Recovered,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentLifecycleEvent {
    pub agent_id: AgentId,
    pub parent_agent_id: Option<AgentId>,
    pub state: AgentLifecycleState,
    pub kind: String,
    pub objective: Option<String>,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentLifecycleState {
    Dispatched,
    Started,
    Waiting,
    Blocked,
    Completed,
    Interrupted,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnLifecycleEvent {
    pub state: TurnLifecycleState,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnLifecycleState {
    Started,
    Steered,
    Completed,
    Interrupted,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageEvent {
    pub role: MessageRole,
    pub content: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    User,
    Agent,
    ReasoningSummary,
    System,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolIntentEvent {
    pub tool_name: String,
    pub arguments: Value,
    pub side_effect: ToolSideEffect,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolSideEffect {
    Read,
    Navigation,
    Mutation,
    External,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolDecisionEvent {
    pub decision: ToolDecision,
    pub decided_by: String,
    pub reason_summary: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolDecision {
    Allowed,
    Denied,
    Escalated,
    RequiresUser,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolStartedEvent {
    pub tool_name: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolResultEvent {
    pub outcome: ToolOutcome,
    pub summary: Option<String>,
    pub result: Option<Value>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolOutcome {
    Completed,
    Failed,
    Interrupted,
    /// The process stopped after an effect may have begun but before a durable
    /// terminal result was recorded. Recovery must not guess success/failure.
    UnknownAfterCrash,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileMutationEvent {
    pub path: String,
    pub previous_path: Option<String>,
    pub surface: WorkspaceSurface,
    pub file_kind: FileKind,
    /// `None` represents a file that did not exist before the transition.
    pub before_artifact: Option<ArtifactRef>,
    /// `None` represents deletion by the transition.
    pub after_artifact: Option<ArtifactRef>,
    pub state: FileMutationState,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkspaceSurface {
    Buffer { version: Option<u64> },
    Disk,
    GitIndex,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    Regular,
    Symlink,
    Directory,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactRef {
    pub artifact_id: super::ArtifactId,
    pub availability: ArtifactAvailability,
    pub representation: ContentRepresentation,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentRepresentation {
    RawBytes,
    EditorText {
        encoding: Option<String>,
        line_endings: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactAvailability {
    Available,
    Missing,
    Redacted,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileMutationState {
    Proposed,
    Completed,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointEvent {
    pub checkpoint_id: String,
    pub label: Option<String>,
    pub workspace_manifest: Option<super::ManifestId>,
    pub git_commit: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DivergenceEvent {
    pub scope: String,
    pub expected_artifact: Option<String>,
    pub actual_artifact: Option<String>,
    pub replayability: Replayability,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Replayability {
    Exact,
    Compatible,
    Diverged,
    Partial,
    NotReplayable,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn preserves_unknown_event_payloads_across_json_round_trip() {
        let raw = json!({
            "kind": "workspace.future_transition",
            "data": { "revision": 7, "paths": ["src/lib.rs"] }
        });

        let event: EventKind = serde_json::from_value(raw.clone()).unwrap();
        assert_eq!(
            event,
            EventKind::Unknown {
                name: "workspace.future_transition".into(),
                payload: json!({ "revision": 7, "paths": ["src/lib.rs"] }),
            }
        );
        assert_eq!(serde_json::to_value(event).unwrap(), raw);
    }

    #[test]
    fn known_event_payloads_round_trip_through_the_same_wire_shape() {
        let event = EventKind::Message(MessageEvent {
            role: MessageRole::ReasoningSummary,
            content: "Inspect the workspace first".into(),
        });

        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(serde_json::from_value::<EventKind>(json).unwrap(), event);
    }
}
