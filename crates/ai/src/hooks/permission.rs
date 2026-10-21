use crate::hooks::model::{HookReason, HookVerdict, MergedEffects};

/// Text used when a hook denies without saying why.
pub const DEFAULT_DENY_REASON: &str = "An agent hook denied this action.";

/// Text used when a hook asks for a rewrite, which cannot be honored yet. Running the original
/// input instead would silently drop a control effect, so the action is denied.
pub const REWRITE_UNSUPPORTED_DENY_REASON: &str = "An agent hook asked to rewrite this action. Rewrites are not supported yet, so the action \
     was not run.";

/// The facts Warp's own permission check produced for an action, before hooks are consulted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WarpPermissionResult {
    /// Warp's per-tool check allows the action without asking.
    pub can_auto_execute: bool,
    /// The user already approved this action in a confirmation card.
    pub is_user_initiated: bool,
    /// An autonomous run is executing a shell command, which Warp lets through to a denylist
    /// check instead of asking.
    pub is_autonomous_shell_exception: bool,
}

impl WarpPermissionResult {
    /// What Warp decides on its own, with no hooks involved.
    fn decision_without_hooks(&self) -> PermissionDecision {
        if self.is_user_initiated || self.can_auto_execute || self.is_autonomous_shell_exception {
            PermissionDecision::Execute
        } else {
            PermissionDecision::RequireConfirmation { reason: None }
        }
    }
}

/// Limits a hook can never loosen. Empty until the floors are named.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PermissionFloors {}

/// What the loop does with an action after hooks and Warp's own check are combined.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionDecision {
    Execute,
    RequireConfirmation { reason: Option<HookReason> },
    Deny { reason: HookReason },
}

/// Composes a merged hook verdict with Warp's permission result. Hooks can only tighten: a deny
/// always denies, an input rewrite denies because it cannot be honored yet, and every other
/// verdict leaves Warp's own decision in place.
pub struct HookPermissionPolicy;

impl HookPermissionPolicy {
    pub fn compose(
        merged: &MergedEffects,
        warp: WarpPermissionResult,
        floors: &PermissionFloors,
    ) -> PermissionDecision {
        let PermissionFloors {} = floors;
        if merged.replacement_input.is_some() && merged.verdict != HookVerdict::Deny {
            return PermissionDecision::Deny {
                reason: HookReason {
                    text: REWRITE_UNSUPPORTED_DENY_REASON.to_owned(),
                    binding: None,
                },
            };
        }
        match merged.verdict {
            HookVerdict::Deny => PermissionDecision::Deny {
                reason: deny_reason(merged),
            },
            HookVerdict::NoOpinion | HookVerdict::Allow | HookVerdict::Ask | HookVerdict::Defer => {
                warp.decision_without_hooks()
            }
        }
    }
}

/// The reason to report for a merged deny: the denying binding's own reason when it gave one,
/// otherwise the first reason any binding gave, otherwise the default text.
fn deny_reason(merged: &MergedEffects) -> HookReason {
    merged
        .reasons
        .iter()
        .find(|reason| reason.binding.is_some() && reason.binding == merged.verdict_binding)
        .or_else(|| merged.reasons.first())
        .cloned()
        .unwrap_or_else(|| HookReason {
            text: DEFAULT_DENY_REASON.to_owned(),
            binding: merged.verdict_binding.clone(),
        })
}

#[cfg(test)]
#[path = "permission_tests.rs"]
mod tests;
