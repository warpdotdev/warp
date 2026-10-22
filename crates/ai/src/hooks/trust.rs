use crate::hooks::model::{HookSourceId, HookSourceKind, HookSurface};

/// Whether the run's execution profile already lets the agent run commands without asking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandExecutionPosture {
    AutoApproves,
    RequiresApproval,
    Unknown,
}

impl CommandExecutionPosture {
    /// The more cautious of two postures: needing approval beats not knowing, which beats
    /// auto-approving.
    pub fn strictest(self, other: Self) -> Self {
        match (self, other) {
            (Self::RequiresApproval, _) | (_, Self::RequiresApproval) => Self::RequiresApproval,
            (Self::Unknown, _) | (_, Self::Unknown) => Self::Unknown,
            (Self::AutoApproves, Self::AutoApproves) => Self::AutoApproves,
        }
    }

    /// The strictest posture across `postures`, or [`Self::Unknown`] when `postures` is empty.
    pub fn strictest_of(postures: impl IntoIterator<Item = Self>) -> Self {
        postures
            .into_iter()
            .reduce(Self::strictest)
            .unwrap_or(Self::Unknown)
    }
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

/// Keeps every source inert.
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

/// The posture rule as a fixed table. Sources delivered with the run were chosen by whoever
/// configured the run, so they activate everywhere; their loader activates them without asking
/// this table, and the rows are here so the table is complete and says the same thing. Only
/// project files are decided here: one activates only in a headless run whose execution profile
/// already auto-approves command execution, where the agent could run the repository's scripts
/// on its own anyway. On interactive surfaces a project file stays inert until the per-root
/// approval flow exists, and a local child waits for its parent's approved set.
pub struct StaticTrustPolicy;

impl TrustPolicy for StaticTrustPolicy {
    fn may_activate(
        &self,
        source: &HookSourceId,
        surface: HookSurface,
        posture: CommandExecutionPosture,
    ) -> TrustDecision {
        use CommandExecutionPosture::{AutoApproves, RequiresApproval, Unknown};
        use HookSourceKind::{FactoryAgent, FactoryWide, ProjectCursorFile, RunDelivered};
        use HookSurface::{CliRun, Gui, LocalChild, OzRun, Tui};

        match (surface, source.kind, posture) {
            (
                OzRun | CliRun | LocalChild | Gui | Tui,
                RunDelivered | FactoryAgent | FactoryWide,
                AutoApproves | RequiresApproval | Unknown,
            ) => TrustDecision::Activate,
            (OzRun | CliRun, ProjectCursorFile, AutoApproves) => TrustDecision::Activate,
            (OzRun | CliRun, ProjectCursorFile, RequiresApproval) => TrustDecision::Inert {
                reason: "the run's execution profile does not auto-approve command execution, \
                         so project hooks only run when the run names them as an explicit source"
                    .to_owned(),
            },
            (OzRun | CliRun, ProjectCursorFile, Unknown) => TrustDecision::Inert {
                reason: "the run's command execution posture is not known yet".to_owned(),
            },
            (LocalChild, ProjectCursorFile, AutoApproves | RequiresApproval | Unknown) => {
                TrustDecision::Inert {
                    reason: "a local child runs under its parent's approved project hooks, which \
                             do not exist yet"
                        .to_owned(),
                }
            }
            (Gui | Tui, ProjectCursorFile, AutoApproves | RequiresApproval | Unknown) => {
                TrustDecision::Inert {
                    reason: "project hooks only run in headless runs for now; interactive \
                             approval is not available yet"
                        .to_owned(),
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "trust_tests.rs"]
mod tests;
