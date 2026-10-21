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

/// Facts about the run that change how a hook verdict is applied.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RunFacts {
    /// No one can answer a confirmation request: an Oz run, a command-line run, or a hidden
    /// local child.
    pub headless: bool,
    /// The strict hook policy is on for this run.
    pub is_strict: bool,
}

/// Limits a hook can never loosen. Each one is a fact Warp's own permission check already
/// established for this action; the policy may add restrictions on top of them but never
/// removes one.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PermissionFloors {
    /// A command denylist that applies to this run matched the command: the organization's
    /// list, or the user's list when per-conversation auto-approve is not allowed to bypass it.
    pub command_denylist_hit: bool,
    /// A file edit touches a protected path: an MCP configuration file outside a sandbox, or a
    /// hook file or a script it names anywhere.
    pub protected_write_path: bool,
    /// The MCP server is on the active profile's denylist.
    pub mcp_server_denylisted: bool,
    /// The active profile's setting for this action family is "always ask" and nothing on the
    /// allowlist let the action through.
    pub always_ask_setting: bool,
}

impl PermissionFloors {
    pub fn any(&self) -> bool {
        let Self {
            command_denylist_hit,
            protected_write_path,
            mcp_server_denylisted,
            always_ask_setting,
        } = *self;
        command_denylist_hit || protected_write_path || mcp_server_denylisted || always_ask_setting
    }
}

/// Why the policy denied an action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DenyCause {
    /// A hook said deny.
    Hook,
    /// A hook asked to rewrite the input, which cannot be honored yet.
    RewriteUnsupported,
    /// A hook asked for confirmation on a surface with no one to ask.
    ApprovalUnavailable,
    /// A hook asked to defer the action, which Warp cannot resume later.
    DeferralUnsupported,
}

/// What the loop does with an action after hooks and Warp's own check are combined.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionDecision {
    Execute,
    RequireConfirmation {
        reason: Option<HookReason>,
    },
    Deny {
        reason: HookReason,
        cause: DenyCause,
    },
}

impl PermissionDecision {
    /// How far the decision restricts the action, so two decisions can be compared: a denial is
    /// stricter than a confirmation, which is stricter than execution.
    pub fn strictness(&self) -> u8 {
        match self {
            Self::Execute => 0,
            Self::RequireConfirmation { .. } => 1,
            Self::Deny { .. } => 2,
        }
    }
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
        let warp_alone = warp.decision_without_hooks();
        let decision = if merged.replacement_input.is_some() && merged.verdict != HookVerdict::Deny
        {
            PermissionDecision::Deny {
                reason: HookReason {
                    text: REWRITE_UNSUPPORTED_DENY_REASON.to_owned(),
                    binding: None,
                },
                cause: DenyCause::RewriteUnsupported,
            }
        } else {
            match merged.verdict {
                HookVerdict::Deny => PermissionDecision::Deny {
                    reason: deny_reason(merged),
                    cause: DenyCause::Hook,
                },
                HookVerdict::NoOpinion
                | HookVerdict::Allow
                | HookVerdict::Ask
                | HookVerdict::Defer => warp_alone.clone(),
            }
        };
        debug_assert!(
            !floors.any() || decision.strictness() >= warp_alone.strictness(),
            "a hook verdict loosened Warp's decision past a floor"
        );
        decision
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
