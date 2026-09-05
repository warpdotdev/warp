//! What a restart's resume did, reported without reporting what the user was doing.
//!
//! One event per restored pane that carried recorded state. It is the only place the field
//! behavior of the feature becomes visible: whether resume works, which of the gate's rules the
//! misses trip on, and how old the recordings behind them were. R22's freshness window is a
//! provisional constant until this last part comes back from the field.
//!
//! The payload is four closed values — an agent kind, an outcome, a flag, an age band — and
//! nothing else may join them. The invocation the pane would have run, the flags recorded off
//! the user's own command, the session identifier and the directory are all things this feature
//! touches and none of them belongs in an event: a resume's diagnostics live in the `full:` arm
//! of a `safe_*` macro, where they stay on the machine that produced them.

use std::time::Duration;

use chrono::NaiveDateTime;
use serde::Serialize;
use serde_json::{Value, json};
use strum_macros::{EnumDiscriminants, EnumIter};
use warp_core::features::FeatureFlag;
use warp_core::telemetry::{EnablementState, TelemetryEvent, TelemetryEventDesc};

use crate::app_state::RecordedAgentSession;
use crate::pane_group::ResumeIneligibility;
use crate::server::telemetry::CLIAgentType;
use crate::terminal::cli_agent_resume::PermissionPosture;

#[derive(Debug, EnumDiscriminants)]
#[strum_discriminants(derive(EnumIter))]
pub(crate) enum AgentSessionResumeTelemetryEvent {
    /// A restored pane that carried recorded state reported what became of it.
    PaneRestored {
        agent: CLIAgentType,
        outcome: ResumeOutcome,
        /// Whether the resume ran at the permission posture the user's own invocation had.
        /// True says the recording was recent enough for R22 to let its posture flags ride
        /// along, not that the user had recorded any: which flags survived validation is a
        /// question about one invocation, and belongs nowhere near an event.
        permission_flags_carried: bool,
        recorded_age: RecordedAgeBucket,
    },
}

impl AgentSessionResumeTelemetryEvent {
    /// What a pane holding `recorded` reports for `outcome`, judged at `now`.
    pub(crate) fn pane_restored(
        recorded: &RecordedAgentSession,
        outcome: ResumeOutcome,
        now: NaiveDateTime,
    ) -> Self {
        let posture = PermissionPosture::for_observation(recorded.observed_at, now);
        Self::PaneRestored {
            agent: recorded.agent.into(),
            outcome,
            // Only a pane that launched can have carried anything: for every other outcome the
            // posture never got the chance to apply, however fresh the recording was.
            permission_flags_carried: outcome == ResumeOutcome::Resumed
                && posture == PermissionPosture::Carry,
            recorded_age: RecordedAgeBucket::for_observation(recorded.observed_at, now),
        }
    }
}

/// What became of one restored pane that carried recorded state.
///
/// Every rejection keeps its own value. A single "not eligible" would say that resume did not
/// happen without saying which rule stopped it, and the rules fail for unrelated reasons: a
/// deleted worktree is a fact about the user's machine, an undeclared agent is a gap in Warp.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ResumeOutcome {
    /// The pane came back with a resume invocation armed for it.
    Resumed,
    /// The pane was eligible, but nothing could be armed for it.
    Failed,
    NotStartupRestore,
    NoSessionIdentifier,
    AgentNotDeclared,
    SharedSessionViewer,
    SessionNotLocal,
    RecordedDirectoryMissing,
    RestoredElsewhere,
    IdentifierClaimedByAnotherPane,
}

impl ResumeOutcome {
    /// The outcome a pane reports for the gate's `verdict`, having armed an invocation or not, or
    /// `None` when the pane has nothing to say about this feature.
    pub(crate) fn for_verdict(
        verdict: &Result<&RecordedAgentSession, ResumeIneligibility>,
        resume_armed: bool,
    ) -> Option<Self> {
        match verdict {
            Ok(_) if resume_armed => Some(Self::Resumed),
            // Cleared by the gate and still holding nothing to run: what was recorded did not
            // survive validation, which is the one outcome that says the feature itself failed.
            Ok(_) => Some(Self::Failed),
            Err(reason) => Self::for_ineligibility(*reason),
        }
    }

    /// The outcome for a pane the gate turned down, or `None` for the rejection that is not
    /// about this feature at all: a pane that was never running an agent, which is most of them.
    fn for_ineligibility(reason: ResumeIneligibility) -> Option<Self> {
        match reason {
            ResumeIneligibility::NoRecordedSession => None,
            ResumeIneligibility::NotStartupRestore => Some(Self::NotStartupRestore),
            ResumeIneligibility::NoSessionIdentifier => Some(Self::NoSessionIdentifier),
            ResumeIneligibility::AgentNotDeclared => Some(Self::AgentNotDeclared),
            ResumeIneligibility::SharedSessionViewer => Some(Self::SharedSessionViewer),
            ResumeIneligibility::SessionNotLocal => Some(Self::SessionNotLocal),
            ResumeIneligibility::RecordedDirectoryMissing => Some(Self::RecordedDirectoryMissing),
            ResumeIneligibility::RestoredElsewhere => Some(Self::RestoredElsewhere),
            ResumeIneligibility::IdentifierClaimedByAnotherPane => {
                Some(Self::IdentifierClaimedByAnotherPane)
            }
        }
    }
}

/// How old the recorded state was when the pane came back, in bands coarse enough that no value
/// is a fact about one user's day.
///
/// The edges bracket the values R22's freshness window could take — the provisional twelve hours
/// is an edge rather than a band — so the field distribution answers what moving the window to
/// six or twenty-four hours would cost, instead of only how the current guess did. Each band is
/// closed at its upper edge, matching the rule the window itself is applied with, so everything
/// up to and including [`RecordedAgeBucket::SixToTwelveHours`] is exactly what carries the
/// posture flags today.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(crate) enum RecordedAgeBucket {
    #[serde(rename = "up_to_1h")]
    UpToOneHour,
    #[serde(rename = "1h_to_6h")]
    OneToSixHours,
    #[serde(rename = "6h_to_12h")]
    SixToTwelveHours,
    #[serde(rename = "12h_to_24h")]
    TwelveToTwentyFourHours,
    #[serde(rename = "1d_to_7d")]
    OneToSevenDays,
    #[serde(rename = "over_7d")]
    OverSevenDays,
    /// The recording is dated after the restart that read it, so the clock moved backwards
    /// between the two and nothing here can vouch for an age. Kept as a band of its own rather
    /// than folded into the oldest one, which would read as evidence for a shorter window.
    #[serde(rename = "unverifiable")]
    Unverifiable,
}

/// The upper edge of each age band, in the order they are tried.
const AGE_BAND_EDGES: [(Duration, RecordedAgeBucket); 5] = [
    (Duration::from_secs(60 * 60), RecordedAgeBucket::UpToOneHour),
    (
        Duration::from_secs(6 * 60 * 60),
        RecordedAgeBucket::OneToSixHours,
    ),
    (
        Duration::from_secs(12 * 60 * 60),
        RecordedAgeBucket::SixToTwelveHours,
    ),
    (
        Duration::from_secs(24 * 60 * 60),
        RecordedAgeBucket::TwelveToTwentyFourHours,
    ),
    (
        Duration::from_secs(7 * 24 * 60 * 60),
        RecordedAgeBucket::OneToSevenDays,
    ),
];

impl RecordedAgeBucket {
    /// The band state observed at `observed_at` falls in when the pane restores at `now`.
    pub(crate) fn for_observation(observed_at: NaiveDateTime, now: NaiveDateTime) -> Self {
        let Ok(age) = (now - observed_at).to_std() else {
            return Self::Unverifiable;
        };
        AGE_BAND_EDGES
            .iter()
            .find_map(|(edge, bucket)| (age <= *edge).then_some(*bucket))
            .unwrap_or(Self::OverSevenDays)
    }
}

impl TelemetryEvent for AgentSessionResumeTelemetryEvent {
    fn name(&self) -> &'static str {
        AgentSessionResumeTelemetryEventDiscriminants::from(self).name()
    }

    fn payload(&self) -> Option<Value> {
        match self {
            Self::PaneRestored {
                agent,
                outcome,
                permission_flags_carried,
                recorded_age,
            } => Some(json!({
                "agent": agent,
                "outcome": outcome,
                "permission_flags_carried": permission_flags_carried,
                "recorded_age": recorded_age,
            })),
        }
    }

    fn description(&self) -> &'static str {
        AgentSessionResumeTelemetryEventDiscriminants::from(self).description()
    }

    fn enablement_state(&self) -> EnablementState {
        AgentSessionResumeTelemetryEventDiscriminants::from(self).enablement_state()
    }

    fn contains_ugc(&self) -> bool {
        match self {
            Self::PaneRestored { .. } => false,
        }
    }

    fn event_descs() -> impl Iterator<Item = Box<dyn TelemetryEventDesc>> {
        warp_core::telemetry::enum_events::<Self>()
    }
}

impl TelemetryEventDesc for AgentSessionResumeTelemetryEventDiscriminants {
    fn name(&self) -> &'static str {
        match self {
            Self::PaneRestored => "AgentSessionResume.PaneRestore.Outcome",
        }
    }

    fn description(&self) -> &'static str {
        match self {
            Self::PaneRestored => {
                "A restored pane that had a recorded agent session reported whether it resumed, \
                 and how old the recording was"
            }
        }
    }

    fn enablement_state(&self) -> EnablementState {
        match self {
            Self::PaneRestored => EnablementState::Flag(FeatureFlag::AgentSessionResume),
        }
    }
}

warp_core::register_telemetry_event!(AgentSessionResumeTelemetryEvent);
