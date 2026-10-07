//! Helpers for driving the viewer's charge unit (`Tier.chargeUnit`) in tests.

use warpui::{App, SingletonEntity};

use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::workspaces::workspace::{ChargeUnit, UserTier};

/// Sets the unit the viewer's plan charges usage in, on the current workspace's tier when there
/// is one, else on the user-level tier, the way a workspaces-metadata response would. Does not
/// emit the events a real refresh would; tests whose subscribers must react re-apply the
/// workspaces themselves.
pub fn set_charge_unit(app: &mut App, charge_unit: ChargeUnit) {
    UserWorkspaces::handle(app).update(app, |workspaces, _| {
        match workspaces.current_workspace_mut() {
            Some(workspace) => workspace.billing_metadata.tier.charge_unit = charge_unit,
            None => workspaces.set_user_tier(UserTier {
                charge_unit,
                ..Default::default()
            }),
        }
    });
}
