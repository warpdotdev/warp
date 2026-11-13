use crate::hooks::model::{HookReason, HookVerdict, MergedEffects};

/// Text used when a hook denies without saying why.
pub const DEFAULT_DENY_REASON: &str = "An agent hook denied this action.";

/// Text used when a hook asks for confirmation without saying why.
pub const DEFAULT_ASK_REASON: &str = "An agent hook asked for confirmation before this action.";

/// Text used when a hook asks for a rewrite, which cannot be honored yet. Running the original
/// input instead would silently drop a control effect, so the action is denied.
pub const REWRITE_UNSUPPORTED_DENY_REASON: &str = "An agent hook asked to rewrite this action. \
     Rewrites are not supported yet, so the action was not run.";

/// Start of the reason given when a hook asked for confirmation on a surface with no one to ask.
/// The hook's own reason, when it gave one, follows in parentheses.
pub const APPROVAL_UNAVAILABLE_PREFIX: &str =
    "Approval unavailable: an agent hook asked for confirmation";

/// Text used when a hook asks to defer an action and nothing could resume it later.
pub const DEFERRAL_UNSUPPORTED_DENY_REASON: &str = "An agent hook asked to defer this action for later. \
     Resumable deferral is not supported, so the action was not run.";

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
    pub is_headless: bool,
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

/// Composes a merged hook verdict with Warp's own permission result so that hooks can only
/// tighten what Warp decided.
pub struct HookPermissionPolicy;

impl HookPermissionPolicy {
    pub fn compose(
        merged: &MergedEffects,
        warp: WarpPermissionResult,
        floors: &PermissionFloors,
        run: RunFacts,
    ) -> PermissionDecision {
        let warp_alone = warp.decision_without_hooks();
        // Before an operation the dispatcher already turns a rewrite request into a denial that
        // names the binding, so effects that came through it never reach this branch. It is a
        // backstop for effects that did not, because a rewrite must never be run as if it had
        // been honored.
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
                    reason: verdict_reason(merged)
                        .unwrap_or_else(|| default_reason(merged, DEFAULT_DENY_REASON)),
                    cause: DenyCause::Hook,
                },
                HookVerdict::Ask => {
                    Self::compose_ask(merged, run, warp.is_user_initiated, warp_alone.clone())
                }
                HookVerdict::Defer => {
                    // Interactive surfaces follow the vendor's "warn and ignore" rule for a
                    // deferral, unless the strict policy turns every unsupported control output
                    // into a denial.
                    if run.is_headless || run.is_strict {
                        PermissionDecision::Deny {
                            reason: default_reason(merged, DEFERRAL_UNSUPPORTED_DENY_REASON),
                            cause: DenyCause::DeferralUnsupported,
                        }
                    } else {
                        warp_alone.clone()
                    }
                }
                HookVerdict::NoOpinion | HookVerdict::Allow => warp_alone.clone(),
            }
        };
        // Nothing above loosens, so no floor can be crossed. Bounded loosening, when it comes,
        // must consult `floors` before weakening `warp_alone` and keep this true whenever one
        // is set.
        debug_assert!(
            decision.strictness() >= warp_alone.strictness(),
            "a hook verdict loosened Warp's decision (floors: {floors:?})"
        );
        decision
    }

    /// Resolves an `Ask` verdict. Headless surfaces deny even when `is_user_initiated` is set,
    /// because no person can have approved the action on a surface with no one to ask.
    fn compose_ask(
        merged: &MergedEffects,
        run: RunFacts,
        is_user_initiated: bool,
        warp_alone: PermissionDecision,
    ) -> PermissionDecision {
        if run.is_headless {
            let text = match verdict_reason(merged) {
                Some(reason) => format!(
                    "{APPROVAL_UNAVAILABLE_PREFIX} ({}) and this run has no one to ask.",
                    reason.text
                ),
                None => format!("{APPROVAL_UNAVAILABLE_PREFIX} and this run has no one to ask."),
            };
            return PermissionDecision::Deny {
                reason: HookReason {
                    text,
                    binding: merged.verdict_binding.clone(),
                },
                cause: DenyCause::ApprovalUnavailable,
            };
        }
        if is_user_initiated {
            return warp_alone;
        }
        PermissionDecision::RequireConfirmation {
            reason: Some(
                verdict_reason(merged)
                    .unwrap_or_else(|| default_reason(merged, DEFAULT_ASK_REASON)),
            ),
        }
    }
}

/// The reason the winning binding gave for its verdict, or failing that the first reason any
/// binding gave.
fn verdict_reason(merged: &MergedEffects) -> Option<HookReason> {
    merged
        .reasons
        .iter()
        .find(|reason| reason.binding.is_some() && reason.binding == merged.verdict_binding)
        .or_else(|| merged.reasons.first())
        .cloned()
}

/// A Warp-written reason, attributed to the binding whose verdict won.
fn default_reason(merged: &MergedEffects, text: &str) -> HookReason {
    HookReason {
        text: text.to_owned(),
        binding: merged.verdict_binding.clone(),
    }
}

#[cfg(test)]
#[path = "permission_tests.rs"]
mod tests;
