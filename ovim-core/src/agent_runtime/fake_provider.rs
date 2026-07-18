//! Deterministic, offline provider fixtures for subagent orchestration tests.
//!
//! This is test support for the supervisor and handoff phases. It deliberately
//! does not implement routing or launch a real provider. Time is represented by
//! monotonically increasing logical ticks, so fault tests do not sleep or use
//! the network.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;
use std::fmt;

const EMBEDDED_FIXTURES: &str = include_str!("fixtures/fake-provider-v1.json");

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct FakeProviderFixtureSet {
    pub version: u32,
    pub scenarios: Vec<FakeProviderScenario>,
}

impl FakeProviderFixtureSet {
    pub fn embedded() -> Result<Self, FakeProviderError> {
        let fixtures: Self = serde_json::from_str(EMBEDDED_FIXTURES)
            .map_err(|error| FakeProviderError::InvalidFixture(error.to_string()))?;
        fixtures.validate()?;
        Ok(fixtures)
    }

    pub fn scenario(&self, name: &str) -> Option<&FakeProviderScenario> {
        self.scenarios.iter().find(|scenario| scenario.name == name)
    }

    fn validate(&self) -> Result<(), FakeProviderError> {
        if self.version != 1 {
            return Err(FakeProviderError::UnsupportedVersion(self.version));
        }
        let mut scenario_names = BTreeSet::new();
        for scenario in &self.scenarios {
            if scenario.name.trim().is_empty() || !scenario_names.insert(&scenario.name) {
                return Err(FakeProviderError::InvalidFixture(format!(
                    "scenario name is empty or repeated: {:?}",
                    scenario.name
                )));
            }
            let mut call_ids = BTreeSet::new();
            for call in &scenario.calls {
                if call.call_id.trim().is_empty() || !call_ids.insert(&call.call_id) {
                    return Err(FakeProviderError::InvalidFixture(format!(
                        "call ID is empty or repeated in {}: {:?}",
                        scenario.name, call.call_id
                    )));
                }
                let mut last_tick = None;
                let mut terminal = false;
                for event in &call.events {
                    if last_tick.is_some_and(|tick| event.at_tick < tick) {
                        return Err(FakeProviderError::InvalidFixture(format!(
                            "events for {}/{} are not ordered by tick",
                            scenario.name, call.call_id
                        )));
                    }
                    if terminal {
                        return Err(FakeProviderError::InvalidFixture(format!(
                            "event follows terminal event for {}/{}",
                            scenario.name, call.call_id
                        )));
                    }
                    terminal = event.kind.is_terminal();
                    last_tick = Some(event.at_tick);
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct FakeProviderScenario {
    pub name: String,
    pub description: String,
    pub calls: Vec<FakeProviderCall>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct FakeProviderCall {
    pub call_id: String,
    pub events: Vec<FakeProviderEventSpec>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct FakeProviderEventSpec {
    pub at_tick: u64,
    #[serde(flatten)]
    pub kind: FakeProviderEventKind,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FakeProviderEventKind {
    Started,
    ToolStarted {
        tool_call_id: String,
        tool_name: String,
    },
    ToolFailed {
        tool_call_id: String,
        tool_name: String,
        error: String,
    },
    Handoff {
        payload: Value,
    },
    ProviderFailed {
        error: String,
    },
    Cancelled {
        reason: String,
    },
    TimedOut {
        timeout_ticks: u64,
    },
    Checkpoint {
        label: String,
    },
}

impl FakeProviderEventKind {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Handoff { .. }
                | Self::ProviderFailed { .. }
                | Self::Cancelled { .. }
                | Self::TimedOut { .. }
        )
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FakeProviderEvent {
    /// Stable within a scenario and independent of polling cadence.
    pub fixture_ordinal: usize,
    pub at_tick: u64,
    pub call_id: String,
    #[serde(flatten)]
    pub kind: FakeProviderEventKind,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct FakeProviderCheckpoint {
    version: u32,
    scenario_name: String,
    current_tick: u64,
    emitted_ordinals: BTreeSet<usize>,
}

/// A logical-clock provider that emits the embedded scenario exactly once.
pub struct FakeProvider {
    scenario: FakeProviderScenario,
    current_tick: u64,
    emitted_ordinals: BTreeSet<usize>,
}

impl FakeProvider {
    pub fn from_embedded(scenario_name: &str) -> Result<Self, FakeProviderError> {
        let fixtures = FakeProviderFixtureSet::embedded()?;
        let scenario = fixtures
            .scenario(scenario_name)
            .cloned()
            .ok_or_else(|| FakeProviderError::UnknownScenario(scenario_name.into()))?;
        Ok(Self {
            scenario,
            current_tick: 0,
            emitted_ordinals: BTreeSet::new(),
        })
    }

    pub fn scenario(&self) -> &FakeProviderScenario {
        &self.scenario
    }

    pub fn current_tick(&self) -> u64 {
        self.current_tick
    }

    /// Advance monotonically and return every newly due event in deterministic
    /// `(tick, declaration-order)` order.
    pub fn advance_to(&mut self, tick: u64) -> Result<Vec<FakeProviderEvent>, FakeProviderError> {
        if tick < self.current_tick {
            return Err(FakeProviderError::ClockMovedBackwards {
                current: self.current_tick,
                requested: tick,
            });
        }
        self.current_tick = tick;
        let mut due = self
            .flattened_events()
            .into_iter()
            .filter(|event| {
                event.at_tick <= tick && !self.emitted_ordinals.contains(&event.fixture_ordinal)
            })
            .collect::<Vec<_>>();
        due.sort_by_key(|event| (event.at_tick, event.fixture_ordinal));
        self.emitted_ordinals
            .extend(due.iter().map(|event| event.fixture_ordinal));
        Ok(due)
    }

    pub fn checkpoint(&self) -> Result<Vec<u8>, FakeProviderError> {
        serde_json::to_vec(&FakeProviderCheckpoint {
            version: 1,
            scenario_name: self.scenario.name.clone(),
            current_tick: self.current_tick,
            emitted_ordinals: self.emitted_ordinals.clone(),
        })
        .map_err(|error| FakeProviderError::InvalidCheckpoint(error.to_string()))
    }

    pub fn restore(checkpoint: &[u8]) -> Result<Self, FakeProviderError> {
        let checkpoint: FakeProviderCheckpoint = serde_json::from_slice(checkpoint)
            .map_err(|error| FakeProviderError::InvalidCheckpoint(error.to_string()))?;
        if checkpoint.version != 1 {
            return Err(FakeProviderError::UnsupportedVersion(checkpoint.version));
        }
        let mut provider = Self::from_embedded(&checkpoint.scenario_name)?;
        let valid_ordinals = provider
            .flattened_events()
            .into_iter()
            .map(|event| event.fixture_ordinal)
            .collect::<BTreeSet<_>>();
        if !checkpoint.emitted_ordinals.is_subset(&valid_ordinals) {
            return Err(FakeProviderError::InvalidCheckpoint(
                "checkpoint contains event ordinals outside its fixture".into(),
            ));
        }
        provider.current_tick = checkpoint.current_tick;
        provider.emitted_ordinals = checkpoint.emitted_ordinals;
        Ok(provider)
    }

    fn flattened_events(&self) -> Vec<FakeProviderEvent> {
        let mut ordinal = 0;
        let mut events = Vec::new();
        for call in &self.scenario.calls {
            for event in &call.events {
                events.push(FakeProviderEvent {
                    fixture_ordinal: ordinal,
                    at_tick: event.at_tick,
                    call_id: call.call_id.clone(),
                    kind: event.kind.clone(),
                });
                ordinal += 1;
            }
        }
        events
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FakeProviderError {
    UnsupportedVersion(u32),
    UnknownScenario(String),
    InvalidFixture(String),
    InvalidCheckpoint(String),
    ClockMovedBackwards { current: u64, requested: u64 },
}

impl fmt::Display for FakeProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported fake-provider version {version}")
            }
            Self::UnknownScenario(name) => {
                write!(formatter, "unknown fake-provider scenario {name}")
            }
            Self::InvalidFixture(detail) => {
                write!(formatter, "invalid fake-provider fixture: {detail}")
            }
            Self::InvalidCheckpoint(detail) => {
                write!(formatter, "invalid fake-provider checkpoint: {detail}")
            }
            Self::ClockMovedBackwards { current, requested } => write!(
                formatter,
                "fake-provider clock cannot move backwards from {current} to {requested}"
            ),
        }
    }
}

impl std::error::Error for FakeProviderError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn event_types(events: &[FakeProviderEvent]) -> Vec<&'static str> {
        events
            .iter()
            .map(|event| match event.kind {
                FakeProviderEventKind::Started => "started",
                FakeProviderEventKind::ToolStarted { .. } => "tool_started",
                FakeProviderEventKind::ToolFailed { .. } => "tool_failed",
                FakeProviderEventKind::Handoff { .. } => "handoff",
                FakeProviderEventKind::ProviderFailed { .. } => "provider_failed",
                FakeProviderEventKind::Cancelled { .. } => "cancelled",
                FakeProviderEventKind::TimedOut { .. } => "timed_out",
                FakeProviderEventKind::Checkpoint { .. } => "checkpoint",
            })
            .collect()
    }

    #[test]
    fn embedded_fixture_contract_is_valid_and_complete() {
        let fixtures = FakeProviderFixtureSet::embedded().unwrap();
        assert_eq!(fixtures.version, 1);
        assert_eq!(
            fixtures
                .scenarios
                .iter()
                .map(|scenario| scenario.name.as_str())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([
                "cancellation",
                "delayed_completion",
                "malformed_handoff",
                "out_of_order_completion",
                "restart",
                "timeout",
                "tool_failure",
            ])
        );
    }

    #[test]
    fn delayed_completion_uses_logical_time_without_sleeping() {
        let mut provider = FakeProvider::from_embedded("delayed_completion").unwrap();
        assert_eq!(event_types(&provider.advance_to(0).unwrap()), ["started"]);
        assert!(provider.advance_to(4).unwrap().is_empty());
        assert_eq!(event_types(&provider.advance_to(5).unwrap()), ["handoff"]);
        assert!(provider.advance_to(100).unwrap().is_empty());
    }

    #[test]
    fn calls_complete_out_of_declaration_order() {
        let mut provider = FakeProvider::from_embedded("out_of_order_completion").unwrap();
        let initial = provider.advance_to(0).unwrap();
        assert_eq!(initial.len(), 2);
        let fast = provider.advance_to(2).unwrap();
        assert_eq!(fast.len(), 1);
        assert_eq!(fast[0].call_id, "fast-second");
        assert!(matches!(
            fast[0].kind,
            FakeProviderEventKind::Handoff { .. }
        ));
        let slow = provider.advance_to(5).unwrap();
        assert_eq!(slow.len(), 1);
        assert_eq!(slow[0].call_id, "slow-first");
    }

    #[test]
    fn tool_failure_is_independent_and_explicit() {
        let mut provider = FakeProvider::from_embedded("tool_failure").unwrap();
        let events = provider.advance_to(3).unwrap();
        assert_eq!(
            event_types(&events),
            ["started", "tool_started", "tool_failed", "provider_failed"]
        );
        assert!(matches!(
            &events[2].kind,
            FakeProviderEventKind::ToolFailed { tool_name, error, .. }
                if tool_name == "read_file" && error == "injected read failure"
        ));
    }

    #[test]
    fn malformed_handoff_is_emitted_verbatim_for_validator_tests() {
        let mut provider = FakeProvider::from_embedded("malformed_handoff").unwrap();
        let events = provider.advance_to(1).unwrap();
        let FakeProviderEventKind::Handoff { payload } = &events[1].kind else {
            panic!("expected handoff")
        };
        assert_eq!(payload["version"], 99);
        assert!(payload.get("summary").is_none());
    }

    #[test]
    fn cancellation_and_timeout_have_distinct_terminal_events() {
        let mut cancelled = FakeProvider::from_embedded("cancellation").unwrap();
        assert_eq!(
            event_types(&cancelled.advance_to(2).unwrap()),
            ["started", "tool_started", "cancelled"]
        );
        let mut timed_out = FakeProvider::from_embedded("timeout").unwrap();
        assert_eq!(
            event_types(&timed_out.advance_to(3).unwrap()),
            ["started", "timed_out"]
        );
    }

    #[test]
    fn checkpoint_restore_emits_no_duplicates_and_preserves_future_events() {
        let mut original = FakeProvider::from_embedded("restart").unwrap();
        assert_eq!(
            event_types(&original.advance_to(1).unwrap()),
            ["started", "checkpoint"]
        );
        let checkpoint = original.checkpoint().unwrap();
        let mut restored = FakeProvider::restore(&checkpoint).unwrap();
        assert!(restored.advance_to(1).unwrap().is_empty());
        let remaining = restored.advance_to(4).unwrap();
        assert_eq!(event_types(&remaining), ["handoff"]);
        assert_eq!(remaining[0].fixture_ordinal, 2);
    }

    #[test]
    fn logical_clock_rejects_time_travel() {
        let mut provider = FakeProvider::from_embedded("timeout").unwrap();
        provider.advance_to(3).unwrap();
        assert_eq!(
            provider.advance_to(2).unwrap_err(),
            FakeProviderError::ClockMovedBackwards {
                current: 3,
                requested: 2,
            }
        );
    }
}
