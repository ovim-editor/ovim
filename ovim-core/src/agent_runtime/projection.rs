//! Restart-safe usage and progress projection for delegated agents.

use crate::run_log::{
    AgentId, AgentProgressEvent, AgentReported, AgentUsageCost, AgentUsageEvent, EventEnvelope,
    EventId, EventKind, TurnId, AGENT_PROGRESS_EVENT_VERSION, AGENT_USAGE_EVENT_VERSION,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// Stable key for metrics that reset when a retained agent starts a follow-up
/// generation. `turn_id` is the durable causal turn carried by the event
/// envelope; generation remains authoritative for initial and follow-up work.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AgentTurnKey {
    pub agent_id: AgentId,
    pub turn_generation: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<TurnId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTurnMetrics {
    pub key: AgentTurnKey,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_event_id: Option<EventId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<AgentUsageEvent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress_event_id: Option<EventId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<AgentProgressEvent>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentRuntimeProjection {
    turns: BTreeMap<AgentTurnKey, AgentTurnMetrics>,
}

impl AgentRuntimeProjection {
    pub fn rehydrate(events: &[EventEnvelope]) -> Result<Self, AgentProjectionError> {
        let mut sorted = events.iter().collect::<Vec<_>>();
        sorted.sort_by_key(|event| event.sequence);
        let mut projection = Self::default();
        for event in sorted {
            match &event.kind {
                EventKind::AgentUsage(usage) => projection.apply_usage(event, usage)?,
                EventKind::AgentProgress(progress) => projection.apply_progress(event, progress)?,
                _ => {}
            }
        }
        Ok(projection)
    }

    pub fn turns(&self) -> impl Iterator<Item = &AgentTurnMetrics> {
        self.turns.values()
    }

    pub fn get(&self, key: &AgentTurnKey) -> Option<&AgentTurnMetrics> {
        self.turns.get(key)
    }

    pub fn latest_for_agent(&self, agent_id: &AgentId) -> Option<&AgentTurnMetrics> {
        self.turns
            .values()
            .filter(|metrics| &metrics.key.agent_id == agent_id)
            .max_by_key(|metrics| metrics.key.turn_generation)
    }

    fn apply_usage(
        &mut self,
        event: &EventEnvelope,
        usage: &AgentUsageEvent,
    ) -> Result<(), AgentProjectionError> {
        if usage.version != AGENT_USAGE_EVENT_VERSION {
            return Err(AgentProjectionError::UnsupportedUsageVersion(usage.version));
        }
        let key = event_key(event, usage.turn_generation)?;
        let metrics = self.entry(key.clone());
        if let Some(previous) = metrics.usage.as_ref() {
            validate_usage_monotonic(previous, usage)?;
        }
        metrics.usage = Some(usage.clone());
        metrics.usage_event_id = Some(event.event_id.clone());
        Ok(())
    }

    fn apply_progress(
        &mut self,
        event: &EventEnvelope,
        progress: &AgentProgressEvent,
    ) -> Result<(), AgentProjectionError> {
        if progress.version != AGENT_PROGRESS_EVENT_VERSION {
            return Err(AgentProjectionError::UnsupportedProgressVersion(
                progress.version,
            ));
        }
        let key = event_key(event, progress.turn_generation)?;
        let metrics = self.entry(key.clone());
        if metrics
            .progress
            .as_ref()
            .is_some_and(|previous| progress.elapsed_millis < previous.elapsed_millis)
        {
            return Err(AgentProjectionError::ElapsedMovedBackwards(key));
        }
        metrics.progress = Some(progress.clone());
        metrics.progress_event_id = Some(event.event_id.clone());
        Ok(())
    }

    fn entry(&mut self, key: AgentTurnKey) -> &mut AgentTurnMetrics {
        self.turns
            .entry(key.clone())
            .or_insert_with(|| AgentTurnMetrics {
                key,
                usage_event_id: None,
                usage: None,
                progress_event_id: None,
                progress: None,
            })
    }
}

fn event_key(
    event: &EventEnvelope,
    turn_generation: u32,
) -> Result<AgentTurnKey, AgentProjectionError> {
    let agent_id = event
        .agent_id
        .clone()
        .ok_or_else(|| AgentProjectionError::MissingAgent(event.event_id.clone()))?;
    Ok(AgentTurnKey {
        agent_id,
        turn_generation,
        turn_id: event.turn_id.clone(),
    })
}

fn validate_usage_monotonic(
    previous: &AgentUsageEvent,
    next: &AgentUsageEvent,
) -> Result<(), AgentProjectionError> {
    if next.provider_calls < previous.provider_calls || next.tool_calls < previous.tool_calls {
        return Err(AgentProjectionError::UsageMovedBackwards);
    }
    validate_count(&previous.input_tokens, &next.input_tokens)?;
    validate_count(&previous.output_tokens, &next.output_tokens)?;
    validate_count(&previous.cached_input_tokens, &next.cached_input_tokens)?;
    validate_cost(&previous.cost, &next.cost)
}

fn validate_count(
    previous: &AgentReported<u64>,
    next: &AgentReported<u64>,
) -> Result<(), AgentProjectionError> {
    match (previous, next) {
        (AgentReported::Reported(previous), AgentReported::Reported(next)) if next < previous => {
            Err(AgentProjectionError::UsageMovedBackwards)
        }
        (AgentReported::Reported(_), AgentReported::NotReported) => {
            Err(AgentProjectionError::ReportedMetricBecameUnknown)
        }
        _ => Ok(()),
    }
}

fn validate_cost(
    previous: &AgentReported<AgentUsageCost>,
    next: &AgentReported<AgentUsageCost>,
) -> Result<(), AgentProjectionError> {
    match (previous, next) {
        (AgentReported::Reported(previous), AgentReported::Reported(next))
            if previous.currency != next.currency
                || next.amount_micros < previous.amount_micros =>
        {
            Err(AgentProjectionError::UsageMovedBackwards)
        }
        (AgentReported::Reported(_), AgentReported::NotReported) => {
            Err(AgentProjectionError::ReportedMetricBecameUnknown)
        }
        _ => Ok(()),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentProjectionError {
    MissingAgent(EventId),
    UnsupportedUsageVersion(u32),
    UnsupportedProgressVersion(u32),
    UsageMovedBackwards,
    ReportedMetricBecameUnknown,
    ElapsedMovedBackwards(AgentTurnKey),
}

impl fmt::Display for AgentProjectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingAgent(event) => {
                write!(formatter, "agent metric event {event} has no agent")
            }
            Self::UnsupportedUsageVersion(version) => {
                write!(formatter, "unsupported agent usage event version {version}")
            }
            Self::UnsupportedProgressVersion(version) => {
                write!(
                    formatter,
                    "unsupported agent progress event version {version}"
                )
            }
            Self::UsageMovedBackwards => {
                formatter.write_str("cumulative agent usage moved backwards")
            }
            Self::ReportedMetricBecameUnknown => {
                formatter.write_str("a reported agent metric became not-reported")
            }
            Self::ElapsedMovedBackwards(key) => write!(
                formatter,
                "elapsed agent progress moved backwards for {} generation {}",
                key.agent_id, key.turn_generation
            ),
        }
    }
}

impl std::error::Error for AgentProjectionError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run_log::{AgentProgressActivity, EventActor, NewRunEvent, RunEventSink, RunId};
    use std::sync::Arc;

    #[test]
    fn rehydrates_reported_and_unknown_usage_without_estimation() {
        let sink = Arc::new(crate::run_log::InMemoryRunEventSink::new());
        let run_id = RunId::new();
        let agent_id = AgentId::new();
        let turn_id = TurnId::new();
        let usage = AgentUsageEvent {
            version: AGENT_USAGE_EVENT_VERSION,
            turn_generation: 0,
            provider_calls: 1,
            tool_calls: 0,
            input_tokens: AgentReported::Reported(120),
            output_tokens: AgentReported::NotReported,
            cached_input_tokens: AgentReported::NotReported,
            cost: AgentReported::NotReported,
        };
        let append = |kind| {
            sink.append(NewRunEvent {
                run_id: run_id.clone(),
                caused_by: None,
                operation_id: None,
                provider_call_id: None,
                actor: EventActor::Agent(agent_id.clone()),
                agent_id: Some(agent_id.clone()),
                turn_id: Some(turn_id.clone()),
                workspace_id: None,
                branch_id: None,
                kind,
            })
            .unwrap()
        };
        append(EventKind::AgentUsage(usage.clone()));
        append(EventKind::AgentProgress(AgentProgressEvent {
            version: AGENT_PROGRESS_EVENT_VERSION,
            turn_generation: 0,
            activity: AgentProgressActivity::ProviderCall,
            elapsed_millis: 8,
            current_tool: None,
            detail: Some("round 1".into()),
        }));

        let restored = AgentRuntimeProjection::rehydrate(&sink.events(&run_id).unwrap()).unwrap();
        let metrics = restored.latest_for_agent(&agent_id).unwrap();
        assert_eq!(metrics.usage.as_ref(), Some(&usage));
        assert_eq!(
            metrics.usage.as_ref().unwrap().output_tokens,
            AgentReported::NotReported
        );
        assert_eq!(metrics.progress.as_ref().unwrap().elapsed_millis, 8);
    }
}
