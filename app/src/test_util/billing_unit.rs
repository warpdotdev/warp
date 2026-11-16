//! Helpers for driving the viewer's charge unit (`Tier.chargeUnit`) and usage display preference
//! in tests.

use settings::Setting as _;
use warpui::{App, SingletonEntity};

use crate::settings::{AISettings, UsageDisplayUnit};
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

/// Sets the `usage_display_unit` preference. Requires the test settings to be initialized.
pub fn set_usage_display_unit(app: &mut App, unit: UsageDisplayUnit) {
    AISettings::handle(app).update(app, |settings, ctx| {
        settings
            .usage_display_unit
            .set_value(unit, ctx)
            .expect("usage display unit should be settable in tests");
    });
}
