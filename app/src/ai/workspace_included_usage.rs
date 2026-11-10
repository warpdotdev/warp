use warp_graphql::queries::get_request_limit_info::WorkspaceIncludedUsageInfo;
use warpui::{AppContext, SingletonEntity};

use super::AIRequestUsageModel;
use crate::auth::AuthStateProvider;
use crate::workspaces::user_workspaces::UserWorkspaces;

impl AIRequestUsageModel {
    pub(crate) fn workspace_included_usage(
        &self,
        ctx: &AppContext,
    ) -> Option<&WorkspaceIncludedUsageInfo> {
        if !AuthStateProvider::as_ref(ctx).get().is_logged_in() {
            return None;
        }
        let workspace = UserWorkspaces::as_ref(ctx).current_workspace()?;
        self.workspace_allowances
            .iter()
            .find(|allowance| allowance.workspace_uid == workspace.uid)
            .map(|allowance| &allowance.info)
    }
}

#[cfg(test)]
#[path = "workspace_included_usage_tests.rs"]
mod tests;
