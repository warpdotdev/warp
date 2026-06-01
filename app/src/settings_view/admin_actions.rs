use warpui::AppContext;

use crate::channel::ChannelState;
use crate::server::ids::ServerId;

/// Shared admin panel actions and utilities for settings views
pub struct AdminActions;

impl AdminActions {
    /// Generate the admin panel URL for a given team
    pub fn admin_panel_link_for_team(team_uid: ServerId) -> String {
        format!("{}/admin/{}", ChannelState::server_root_url(), team_uid)
    }

    /// Open the admin panel for a specific team
    pub fn open_admin_panel(team_uid: ServerId, ctx: &mut AppContext) {
        let url = Self::admin_panel_link_for_team(team_uid);
        ctx.open_url(&url);
    }

    /// Open the support email link
    pub fn contact_support(ctx: &mut AppContext) {
        ctx.open_url("mailto:support@warp.dev");
    }

    /// Open the contact sales page
    pub fn contact_sales(ctx: &mut AppContext) {
        ctx.open_url("https://warp.dev/contact-sales");
    }
}

#[cfg(test)]
#[path = "admin_actions_tests.rs"]
mod tests;
