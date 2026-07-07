//! Defines structured, rate-limited diagnostics for lifecycle recovery and conservative handling.
//!
//! Records contain only allowlisted lifecycle evidence and never command text, output, prompt
//! contents, working directories, or other UGC. `TerminalModel` emits records through an internal
//! terminal event, and `ModelEventDispatcher` forwards them to the feature-specific telemetry
//! event.

use std::collections::HashMap;
use std::time::Duration;

use instant::Instant;
use serde_json::{json, Value};
use strum_macros::{EnumDiscriminants, EnumIter};
use warp_core::telemetry::{EnablementState, TelemetryEvent, TelemetryEventDesc};

use super::transition::{LifecycleAction, LifecycleInputKind, LifecyclePhase, LifecycleSnapshot};
use crate::terminal::model::block::BlockState;

/// Limits identical lifecycle diagnostics to one emitted aggregate per minute.
const TRANSITION_TELEMETRY_INTERVAL: Duration = Duration::from_secs(60);

/// Captures the allowlisted evidence for one lifecycle diagnostic.
///
/// The first occurrence is emitted immediately. A later aggregate includes the number of
/// equivalent records suppressed during the same rate-limit interval.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LifecycleRecoveryRecord {
    pub(in crate::terminal) previous_phase: LifecyclePhase,
    pub(in crate::terminal) next_phase: LifecyclePhase,
    pub(in crate::terminal) input_kind: LifecycleInputKind,
    pub(in crate::terminal) action: LifecycleAction,
    pub(in crate::terminal) active_block_id: String,
    pub(in crate::terminal) supplied_next_block_id: Option<String>,
    pub(in crate::terminal) active_session_id: Option<u64>,
    pub(in crate::terminal) hook_session_id: Option<u64>,
    pub(in crate::terminal) block_state: BlockState,
    pub(in crate::terminal) started: bool,
    pub(in crate::terminal) finished: bool,
    pub(in crate::terminal) received_precmd: bool,
    pub(in crate::terminal) is_in_band: bool,
    pub(in crate::terminal) is_bootstrapped: bool,
    pub(in crate::terminal) is_bootstrap_done: bool,
    pub(in crate::terminal) is_alt_screen_active: bool,
    pub(in crate::terminal) completion_mismatch: bool,
    pub(in crate::terminal) suppressed_repeats: u64,
}

impl LifecycleRecoveryRecord {
    /// Builds a diagnostic record from the planned transition and its live evidence.
    pub(super) fn new(
        previous_phase: LifecyclePhase,
        next_phase: LifecyclePhase,
        input_kind: LifecycleInputKind,
        action: LifecycleAction,
        snapshot: &LifecycleSnapshot,
    ) -> Self {
        Self {
            previous_phase,
            next_phase,
            input_kind,
            action,
            active_block_id: snapshot.active_block_id.clone(),
            supplied_next_block_id: snapshot.supplied_next_block_id.clone(),
            active_session_id: snapshot.active_session_id,
            hook_session_id: snapshot.hook_session_id,
            block_state: snapshot.block_state,
            started: snapshot.started,
            finished: snapshot.finished,
            received_precmd: snapshot.received_precmd,
            is_in_band: snapshot.is_in_band,
            is_bootstrapped: snapshot.is_bootstrapped,
            is_bootstrap_done: snapshot.is_bootstrap_done,
            is_alt_screen_active: snapshot.is_alt_screen_active,
            completion_mismatch: snapshot.completion_mismatch,
            suppressed_repeats: 0,
        }
    }
}

/// Keys rate limiting by the policy decision rather than input-specific identifiers.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct TransitionKey {
    previous_phase: LifecyclePhase,
    input_kind: LifecycleInputKind,
    action: LifecycleAction,
}

/// Tracks the last emitted aggregate and the repeats suppressed since then.
struct RateLimitState {
    last_emitted: Instant,
    suppressed_repeats: u64,
}

/// Rate-limits lifecycle diagnostics independently for one terminal coordinator.
#[derive(Default)]
pub(super) struct LifecycleTelemetryLimiter {
    transitions: HashMap<TransitionKey, RateLimitState>,
}

impl LifecycleTelemetryLimiter {
    /// Records an occurrence and returns it only when it should be emitted immediately.
    pub(super) fn record(
        &mut self,
        record: LifecycleRecoveryRecord,
    ) -> Option<LifecycleRecoveryRecord> {
        self.record_at(record, Instant::now())
    }

    /// Records an occurrence at a supplied instant for deterministic rate-limit evaluation.
    pub(super) fn record_at(
        &mut self,
        mut record: LifecycleRecoveryRecord,
        now: Instant,
    ) -> Option<LifecycleRecoveryRecord> {
        let key = TransitionKey {
            previous_phase: record.previous_phase,
            input_kind: record.input_kind,
            action: record.action,
        };
        let Some(state) = self.transitions.get_mut(&key) else {
            self.transitions.insert(
                key,
                RateLimitState {
                    last_emitted: now,
                    suppressed_repeats: 0,
                },
            );
            return Some(record);
        };

        if now.duration_since(state.last_emitted) < TRANSITION_TELEMETRY_INTERVAL {
            state.suppressed_repeats += 1;
            return None;
        }

        record.suppressed_repeats = state.suppressed_repeats;
        state.last_emitted = now;
        state.suppressed_repeats = 0;
        Some(record)
    }
}

/// Defines feature-specific telemetry events emitted by the lifecycle coordinator.
#[derive(Debug, EnumDiscriminants)]
#[strum_discriminants(derive(EnumIter))]
pub(in crate::terminal) enum LifecycleTelemetryEvent {
    /// Reports a conservative or recovery lifecycle transition.
    Recovery(LifecycleRecoveryRecord),
}

impl TelemetryEvent for LifecycleTelemetryEvent {
    fn name(&self) -> &'static str {
        LifecycleTelemetryEventDiscriminants::from(self).name()
    }

    fn payload(&self) -> Option<Value> {
        match self {
            LifecycleTelemetryEvent::Recovery(record) => Some(json!({
                "previous_phase": format!("{:?}", record.previous_phase),
                "next_phase": format!("{:?}", record.next_phase),
                "input_kind": format!("{:?}", record.input_kind),
                "action": format!("{:?}", record.action),
                "active_block_id": record.active_block_id,
                "supplied_next_block_id": record.supplied_next_block_id,
                "active_session_id": record.active_session_id,
                "hook_session_id": record.hook_session_id,
                "block_state": format!("{:?}", record.block_state),
                "started": record.started,
                "finished": record.finished,
                "received_precmd": record.received_precmd,
                "is_in_band": record.is_in_band,
                "is_bootstrapped": record.is_bootstrapped,
                "is_bootstrap_done": record.is_bootstrap_done,
                "is_alt_screen_active": record.is_alt_screen_active,
                "completion_mismatch": record.completion_mismatch,
                "suppressed_repeats": record.suppressed_repeats,
            })),
        }
    }

    fn description(&self) -> &'static str {
        LifecycleTelemetryEventDiscriminants::from(self).description()
    }

    fn enablement_state(&self) -> EnablementState {
        LifecycleTelemetryEventDiscriminants::from(self).enablement_state()
    }

    fn contains_ugc(&self) -> bool {
        false
    }

    fn event_descs() -> impl Iterator<Item = Box<dyn TelemetryEventDesc>> {
        warp_core::telemetry::enum_events::<Self>()
    }
}

impl TelemetryEventDesc for LifecycleTelemetryEventDiscriminants {
    fn name(&self) -> &'static str {
        match self {
            LifecycleTelemetryEventDiscriminants::Recovery => "Terminal Lifecycle Recovery",
        }
    }

    fn description(&self) -> &'static str {
        match self {
            LifecycleTelemetryEventDiscriminants::Recovery => {
                "A terminal lifecycle transition required conservative handling or recovery"
            }
        }
    }

    fn enablement_state(&self) -> EnablementState {
        EnablementState::Always
    }
}

warp_core::register_telemetry_event!(LifecycleTelemetryEvent);
