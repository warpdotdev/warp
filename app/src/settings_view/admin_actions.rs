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

    pub fn admin_panel_link_for_workspace() -> String {
        format!("{}/admin", ChannelState::server_root_url())
    }

    /// Open the admin panel for a specific team
    pub fn open_admin_panel(team_uid: ServerId, ctx: &mut AppContext) {
        let url = Self::admin_panel_link_for_team(team_uid);
        ctx.open_url(&url);
    }

    pub fn open_workspace_admin_panel(ctx: &mut AppContext) {
        let url = Self::admin_panel_link_for_workspace();
        ctx.open_url(&url);
    }

    /// Picks the admin panel URL for the current context: native workspaces
    /// administer settings and spend limits at the workspace level, so they
    /// get the workspace-scoped page; everyone else administers per team.
    /// Returns `None` when a team-scoped panel is called for but no team is
    /// known.
    pub fn admin_panel_link(
        native_workspaces_enabled: bool,
        team_uid: Option<ServerId>,
    ) -> Option<String> {
        if native_workspaces_enabled {
            Some(Self::admin_panel_link_for_workspace())
        } else {
            team_uid.map(Self::admin_panel_link_for_team)
        }
    }

    /// Open the admin panel scoped to the current workspace/team context.
    pub fn open_resolved_admin_panel(
        native_workspaces_enabled: bool,
        team_uid: Option<ServerId>,
        ctx: &mut AppContext,
    ) {
        if let Some(url) = Self::admin_panel_link(native_workspaces_enabled, team_uid) {
            ctx.open_url(&url);
        }
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
