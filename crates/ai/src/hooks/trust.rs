use crate::hooks::model::{HookSourceId, HookSurface};

/// Whether the run's execution profile already lets the agent run commands without asking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandExecutionPosture {
    AutoApproves,
    RequiresApproval,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrustDecision {
    Activate,
    /// The source is loaded and reported but none of its hooks run.
    Inert {
        reason: String,
    },
}

/// Decides whether a discovered source may run its hooks on a surface. Where a file lives never
/// grants permission to execute it; this policy does.
pub trait TrustPolicy: Send + Sync {
    fn may_activate(
        &self,
        source: &HookSourceId,
        surface: HookSurface,
        posture: CommandExecutionPosture,
    ) -> TrustDecision;
}

/// Keeps every source inert. Stands in until the static Phase 1 table exists.
pub struct DenyAllTrustPolicy;

impl TrustPolicy for DenyAllTrustPolicy {
    fn may_activate(
        &self,
        _source: &HookSourceId,
        _surface: HookSurface,
        _posture: CommandExecutionPosture,
    ) -> TrustDecision {
        TrustDecision::Inert {
            reason: "hook sources are not activated by this build".to_owned(),
        }
    }
}

#[cfg(test)]
#[path = "trust_tests.rs"]
mod tests;
