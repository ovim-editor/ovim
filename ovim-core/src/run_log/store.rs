use super::{
    EventEnvelope, EventId, NewRunEvent, RunId, EVENT_PAYLOAD_VERSION, EVENT_SCHEMA_VERSION,
};
use chrono::Utc;
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::sync::{Arc, Mutex};

/// The minimal append/read interface shared by transient and durable stores.
pub trait RunEventSink: Send + Sync {
    fn append(&self, event: NewRunEvent) -> Result<EventEnvelope, RunLogError>;
    fn event(
        &self,
        run_id: &RunId,
        event_id: &EventId,
    ) -> Result<Option<EventEnvelope>, RunLogError>;
    fn events(&self, run_id: &RunId) -> Result<Vec<EventEnvelope>, RunLogError>;
    fn last_sequence(&self, run_id: &RunId) -> Result<Option<u64>, RunLogError>;
    /// Discovers runs known to the store in stable creation order.
    fn runs(&self) -> Result<Vec<RunId>, RunLogError>;
}

/// Injectable sources keep ordering tests deterministic without weakening the
/// production ID and UTC timestamp defaults.
pub trait EventIdGenerator: Send + Sync {
    fn next_event_id(&self) -> EventId;
}

pub trait EventClock: Send + Sync {
    /// Returns a portable RFC 3339 UTC timestamp.
    fn now(&self) -> String;
}

#[derive(Default)]
pub struct SystemEventIdGenerator;

impl EventIdGenerator for SystemEventIdGenerator {
    fn next_event_id(&self) -> EventId {
        EventId::new()
    }
}

#[derive(Default)]
pub struct SystemEventClock;

impl EventClock for SystemEventClock {
    fn now(&self) -> String {
        Utc::now().to_rfc3339()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunLogError {
    MissingCause { run_id: RunId, event_id: EventId },
    DuplicateEventId { event_id: EventId },
    SequenceExhausted { run_id: RunId },
    Storage { operation: String, detail: String },
    Serialization { operation: String, detail: String },
    Corruption { detail: String },
    Migration { version: u32, detail: String },
    Poisoned,
}

impl fmt::Display for RunLogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingCause { run_id, event_id } => {
                write!(
                    formatter,
                    "event {event_id} is not an earlier event in run {run_id}"
                )
            }
            Self::DuplicateEventId { event_id } => {
                write!(formatter, "event identifier {event_id} was generated twice")
            }
            Self::SequenceExhausted { run_id } => {
                write!(formatter, "event sequence exhausted for run {run_id}")
            }
            Self::Storage { operation, detail } => {
                write!(
                    formatter,
                    "run log storage failed during {operation}: {detail}"
                )
            }
            Self::Serialization { operation, detail } => {
                write!(
                    formatter,
                    "run log serialization failed during {operation}: {detail}"
                )
            }
            Self::Corruption { detail } => write!(formatter, "run log is corrupt: {detail}"),
            Self::Migration { version, detail } => {
                write!(formatter, "run log migration {version} failed: {detail}")
            }
            Self::Poisoned => formatter.write_str("run event store lock was poisoned"),
        }
    }
}

impl std::error::Error for RunLogError {}

#[derive(Default)]
struct InMemoryState {
    runs: HashMap<RunId, RunEvents>,
    event_runs: HashMap<EventId, RunId>,
    next_run_order: u64,
}

#[derive(Default)]
struct RunEvents {
    by_sequence: BTreeMap<u64, EventEnvelope>,
    sequence_by_id: HashMap<EventId, u64>,
    last_sequence: u64,
    created_order: u64,
}

/// Thread-safe reference implementation. A single lock makes allocation and
/// insertion atomic and therefore preserves deterministic per-run ordering.
#[derive(Clone)]
pub struct InMemoryRunEventSink {
    state: Arc<Mutex<InMemoryState>>,
    id_generator: Arc<dyn EventIdGenerator>,
    clock: Arc<dyn EventClock>,
}

impl InMemoryRunEventSink {
    pub fn new() -> Self {
        Self::with_sources(Arc::new(SystemEventIdGenerator), Arc::new(SystemEventClock))
    }

    pub fn with_sources(
        id_generator: Arc<dyn EventIdGenerator>,
        clock: Arc<dyn EventClock>,
    ) -> Self {
        Self {
            state: Arc::new(Mutex::new(InMemoryState::default())),
            id_generator,
            clock,
        }
    }
}

impl Default for InMemoryRunEventSink {
    fn default() -> Self {
        Self::new()
    }
}

impl RunEventSink for InMemoryRunEventSink {
    fn append(&self, event: NewRunEvent) -> Result<EventEnvelope, RunLogError> {
        let mut state = self.state.lock().map_err(|_| RunLogError::Poisoned)?;
        if let Some(cause) = &event.caused_by {
            if state.event_runs.get(cause) != Some(&event.run_id) {
                return Err(RunLogError::MissingCause {
                    run_id: event.run_id,
                    event_id: cause.clone(),
                });
            }
        }

        let event_id = self.id_generator.next_event_id();
        if state.event_runs.contains_key(&event_id) {
            return Err(RunLogError::DuplicateEventId { event_id });
        }

        if !state.runs.contains_key(&event.run_id) {
            state.next_run_order = state.next_run_order.saturating_add(1);
            let created_order = state.next_run_order;
            state.runs.insert(
                event.run_id.clone(),
                RunEvents {
                    created_order,
                    ..RunEvents::default()
                },
            );
        }
        let run = state
            .runs
            .get_mut(&event.run_id)
            .expect("run was inserted above");

        let sequence =
            run.last_sequence
                .checked_add(1)
                .ok_or_else(|| RunLogError::SequenceExhausted {
                    run_id: event.run_id.clone(),
                })?;
        let envelope = EventEnvelope {
            schema_version: EVENT_SCHEMA_VERSION,
            payload_version: EVENT_PAYLOAD_VERSION,
            event_id,
            run_id: event.run_id,
            sequence,
            recorded_at: self.clock.now(),
            caused_by: event.caused_by,
            operation_id: event.operation_id,
            provider_call_id: event.provider_call_id,
            actor: event.actor,
            agent_id: event.agent_id,
            turn_id: event.turn_id,
            workspace_id: event.workspace_id,
            kind: event.kind,
        };

        run.last_sequence = sequence;
        run.sequence_by_id
            .insert(envelope.event_id.clone(), sequence);
        run.by_sequence.insert(sequence, envelope.clone());
        state
            .event_runs
            .insert(envelope.event_id.clone(), envelope.run_id.clone());
        Ok(envelope)
    }

    fn event(
        &self,
        run_id: &RunId,
        event_id: &EventId,
    ) -> Result<Option<EventEnvelope>, RunLogError> {
        let state = self.state.lock().map_err(|_| RunLogError::Poisoned)?;
        let Some(run) = state.runs.get(run_id) else {
            return Ok(None);
        };
        Ok(run
            .sequence_by_id
            .get(event_id)
            .and_then(|sequence| run.by_sequence.get(sequence))
            .cloned())
    }

    fn events(&self, run_id: &RunId) -> Result<Vec<EventEnvelope>, RunLogError> {
        let state = self.state.lock().map_err(|_| RunLogError::Poisoned)?;
        Ok(state
            .runs
            .get(run_id)
            .map(|run| run.by_sequence.values().cloned().collect())
            .unwrap_or_default())
    }

    fn last_sequence(&self, run_id: &RunId) -> Result<Option<u64>, RunLogError> {
        let state = self.state.lock().map_err(|_| RunLogError::Poisoned)?;
        Ok(state.runs.get(run_id).map(|run| run.last_sequence))
    }

    fn runs(&self) -> Result<Vec<RunId>, RunLogError> {
        let state = self.state.lock().map_err(|_| RunLogError::Poisoned)?;
        let mut runs: Vec<_> = state
            .runs
            .iter()
            .map(|(run_id, run)| (run.created_order, run_id.clone()))
            .collect();
        runs.sort();
        Ok(runs.into_iter().map(|(_, run_id)| run_id).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run_log::{
        EventActor, EventKind, MessageEvent, MessageRole, RunLifecycleEvent, RunLifecycleState,
    };
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;
    use std::thread;

    fn created(run_id: RunId) -> NewRunEvent {
        NewRunEvent::new(
            run_id,
            EventActor::User,
            EventKind::RunLifecycle(RunLifecycleEvent {
                state: RunLifecycleState::Created,
                objective: Some("ship it".into()),
                detail: None,
            }),
        )
    }

    struct SequentialIds(AtomicU64);

    impl EventIdGenerator for SequentialIds {
        fn next_event_id(&self) -> EventId {
            EventId::parse(format!(
                "evt_deterministic_{:04}",
                self.0.fetch_add(1, Ordering::Relaxed)
            ))
            .unwrap()
        }
    }

    struct FixedClock;

    impl EventClock for FixedClock {
        fn now(&self) -> String {
            "2026-07-13T12:00:00Z".into()
        }
    }

    #[test]
    fn allocates_independent_monotonic_sequences_per_run() {
        let sink = InMemoryRunEventSink::new();
        let first_run = RunId::new();
        let second_run = RunId::new();

        assert_eq!(sink.append(created(first_run.clone())).unwrap().sequence, 1);
        assert_eq!(sink.append(created(second_run)).unwrap().sequence, 1);
        assert_eq!(sink.append(created(first_run)).unwrap().sequence, 2);
    }

    #[test]
    fn rejects_causal_references_that_are_not_prior_in_the_same_run() {
        let sink = InMemoryRunEventSink::new();
        let first_run = RunId::new();
        let second_run = RunId::new();
        let cause = sink.append(created(first_run)).unwrap();

        let error = sink
            .append(created(second_run.clone()).caused_by(cause.event_id.clone()))
            .unwrap_err();

        assert_eq!(
            error,
            RunLogError::MissingCause {
                run_id: second_run,
                event_id: cause.event_id,
            }
        );
    }

    #[test]
    fn reads_are_ordered_and_snapshots_cannot_mutate_the_store() {
        let sink = InMemoryRunEventSink::new();
        let run_id = RunId::new();
        let first = sink.append(created(run_id.clone())).unwrap();
        sink.append(NewRunEvent::new(
            run_id.clone(),
            EventActor::User,
            EventKind::Message(MessageEvent {
                role: MessageRole::User,
                content: "continue".into(),
            }),
        ))
        .unwrap();

        let mut snapshot = sink.events(&run_id).unwrap();
        snapshot.clear();

        assert_eq!(sink.events(&run_id).unwrap().len(), 2);
        assert_eq!(sink.event(&run_id, &first.event_id).unwrap(), Some(first));
    }

    #[test]
    fn concurrent_appends_allocate_every_sequence_once() {
        let sink = Arc::new(InMemoryRunEventSink::new());
        let run_id = RunId::new();
        let handles: Vec<_> = (0..16)
            .map(|index| {
                let sink = Arc::clone(&sink);
                let run_id = run_id.clone();
                thread::spawn(move || {
                    sink.append(NewRunEvent::new(
                        run_id,
                        EventActor::System("test".into()),
                        EventKind::Message(MessageEvent {
                            role: MessageRole::System,
                            content: index.to_string(),
                        }),
                    ))
                    .unwrap()
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }

        let events = sink.events(&run_id).unwrap();
        assert_eq!(
            events
                .iter()
                .map(|event| event.sequence)
                .collect::<Vec<_>>(),
            (1..=16).collect::<Vec<_>>()
        );
    }

    #[test]
    fn empty_runs_have_no_events_or_last_sequence() {
        let sink = InMemoryRunEventSink::new();
        let run_id = RunId::new();
        assert!(sink.events(&run_id).unwrap().is_empty());
        assert_eq!(sink.last_sequence(&run_id).unwrap(), None);
    }

    #[test]
    fn injected_metadata_sources_make_envelopes_deterministic() {
        let sink = InMemoryRunEventSink::with_sources(
            Arc::new(SequentialIds(AtomicU64::new(1))),
            Arc::new(FixedClock),
        );
        let run_id = RunId::parse("run_deterministic").unwrap();

        let envelope = sink.append(created(run_id)).unwrap();

        assert_eq!(envelope.event_id.as_str(), "evt_deterministic_0001");
        assert_eq!(envelope.sequence, 1);
        assert_eq!(envelope.recorded_at, "2026-07-13T12:00:00Z");
    }

    #[test]
    fn discovers_runs_in_stable_creation_order() {
        let sink = InMemoryRunEventSink::with_sources(
            Arc::new(SequentialIds(AtomicU64::new(1))),
            Arc::new(FixedClock),
        );
        let second = RunId::parse("run_second").unwrap();
        let first = RunId::parse("run_first").unwrap();
        sink.append(created(second.clone())).unwrap();
        sink.append(created(first.clone())).unwrap();

        assert_eq!(sink.runs().unwrap(), vec![second, first]);
    }
}
