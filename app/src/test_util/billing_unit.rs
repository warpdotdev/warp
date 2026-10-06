//! Helpers for driving the viewer's billing unit (`Tier.billedInDollars`) in tests.

use warpui::{App, SingletonEntity};

use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::workspaces::workspace::UserTier;

/// Marks the viewer's plan as billed in dollars (or credits) on the current workspace's tier
/// when there is one, else on the user-level tier, the way a workspaces-metadata response
/// would. Does not emit the events a real refresh would; tests whose subscribers must react
/// re-apply the workspaces themselves.
pub fn set_billed_in_dollars(app: &mut App, billed_in_dollars: bool) {
    UserWorkspaces::handle(app).update(app, |workspaces, _| {
        match workspaces.current_workspace_mut() {
            Some(workspace) => {
                workspace.billing_metadata.tier.billed_in_dollars = billed_in_dollars;
            }
            None => workspaces.set_user_tier(UserTier {
                billed_in_dollars,
                ..Default::default()
            }),
        }
    });
}
