//! Renderer-neutral guidance for Agent Mode requests denied for billing reasons.
//!
//! The GUI's prompt-alert chip and the failed-output block rendered by both the
//! GUI and the TUI all answer the same question — which billing situation
//! blocked this request, and what unblocks *this* user given their team role.
//! Deriving that answer here is what keeps those surfaces from drifting apart.

use warpui::{AppContext, SingletonEntity};

use crate::auth::AuthStateProvider;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::workspaces::workspace::Workspace;

const ADMIN_RESOLVE_PAYMENT_ISSUE_NEXT_STEP: &str =
    "Resolve the payment issue in Warp billing to restore AI access.";
const MEMBER_RESOLVE_PAYMENT_ISSUE_NEXT_STEP: &str =
    "Contact a team admin to resolve the payment issue.";
const ADMIN_ENABLE_OVERAGES_NEXT_STEP: &str =
    "Enable premium overages or add credits under Settings > Billing & Usage.";
const MEMBER_ENABLE_OVERAGES_NEXT_STEP: &str =
    "Ask a team admin to enable premium overages or add more credits.";
const ADMIN_RAISE_SPEND_LIMIT_NEXT_STEP: &str =
    "Increase the monthly spend limit or add credits under Settings > Billing & Usage.";
const MEMBER_RAISE_SPEND_LIMIT_NEXT_STEP: &str =
    "Ask a team admin to increase the monthly spend limit.";
const MEMBER_UPGRADE_NEXT_STEP: &str = "Ask a team admin to upgrade the plan or add more credits.";
const CONTACT_SUPPORT_NEXT_STEP: &str = "Contact support@warp.dev to restore AI access.";

/// The billing situation behind an Agent Mode denial.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BillingDenialKind {
    /// Billing is past due or unpaid.
    DelinquentDueToPaymentIssue,
    /// The plan supports premium overages, but they are switched off.
    OveragesToggleableButNotEnabled,
    /// Overages are on and the monthly spend limit is exhausted.
    MonthlyOveragesSpendLimitReached,
    /// Included usage is exhausted and no overage policy applies.
    RequestLimitReached,
}

/// What a specific user should do about a billing denial.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BillingDenialGuidance {
    pub kind: BillingDenialKind,
    /// Whether this user administers the billing account behind the denial. A
    /// user with no team administers their own billing.
    pub is_admin: bool,
    /// One sentence naming this user's next step, or `None` when the surface's
    /// own subscribe / upgrade call to action already covers it.
    pub next_step: Option<&'static str>,
}

/// Picks the most actionable denial for a workspace whose credits are
/// exhausted, based on its overage policy.
pub fn out_of_credits_denial_kind(app: &AppContext) -> BillingDenialKind {
    let Some(workspace) = UserWorkspaces::as_ref(app).current_workspace() else {
        return BillingDenialKind::RequestLimitReached;
    };

    if workspace.are_overages_toggleable() {
        if workspace.are_overages_enabled() {
            return BillingDenialKind::MonthlyOveragesSpendLimitReached;
        }
        return BillingDenialKind::OveragesToggleableButNotEnabled;
    }

    BillingDenialKind::RequestLimitReached
}

/// The guidance to show alongside an Agent Mode request that a billing check
/// rejected.
pub fn billing_denial_guidance(app: &AppContext) -> BillingDenialGuidance {
    let workspace = UserWorkspaces::as_ref(app).current_workspace();
    let kind = if workspace.is_some_and(|workspace| {
        workspace
            .billing_metadata
            .is_delinquent_due_to_payment_issue()
    }) {
        BillingDenialKind::DelinquentDueToPaymentIssue
    } else {
        out_of_credits_denial_kind(app)
    };
    let is_admin = administers_billing(workspace, app);

    BillingDenialGuidance {
        kind,
        is_admin,
        next_step: next_step(kind, is_admin, workspace),
    }
}

/// The failed-output block has no window to resolve a team from, so billing
/// role is read from the current workspace's first team; a user with no team
/// administers their own billing.
fn administers_billing(workspace: Option<&Workspace>, app: &AppContext) -> bool {
    let Some(team) = workspace.and_then(|workspace| workspace.teams.first()) else {
        return true;
    };
    AuthStateProvider::as_ref(app)
        .get()
        .user_email()
        .is_some_and(|email| team.has_admin_permissions(&email))
}

fn next_step(
    kind: BillingDenialKind,
    is_admin: bool,
    workspace: Option<&Workspace>,
) -> Option<&'static str> {
    match kind {
        BillingDenialKind::DelinquentDueToPaymentIssue => Some(if is_admin {
            ADMIN_RESOLVE_PAYMENT_ISSUE_NEXT_STEP
        } else {
            MEMBER_RESOLVE_PAYMENT_ISSUE_NEXT_STEP
        }),
        BillingDenialKind::OveragesToggleableButNotEnabled => Some(if is_admin {
            ADMIN_ENABLE_OVERAGES_NEXT_STEP
        } else {
            MEMBER_ENABLE_OVERAGES_NEXT_STEP
        }),
        BillingDenialKind::MonthlyOveragesSpendLimitReached => Some(if is_admin {
            ADMIN_RAISE_SPEND_LIMIT_NEXT_STEP
        } else {
            MEMBER_RAISE_SPEND_LIMIT_NEXT_STEP
        }),
        BillingDenialKind::RequestLimitReached => {
            let can_upgrade = workspace.is_none_or(|workspace| {
                workspace.billing_metadata.can_upgrade_to_higher_tier_plan()
            });
            match (can_upgrade, is_admin) {
                (false, _) => Some(CONTACT_SUPPORT_NEXT_STEP),
                (true, true) => None,
                (true, false) => Some(MEMBER_UPGRADE_NEXT_STEP),
            }
        }
    }
}

#[cfg(test)]
#[path = "billing_denial_tests.rs"]
mod tests;
