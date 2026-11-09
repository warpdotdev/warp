use warp_graphql::queries::get_request_limit_info::WorkspaceIncludedUsageInfo;
use warpui::{AppContext, SingletonEntity};

use super::AIRequestUsageModel;
use crate::auth::AuthStateProvider;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::workspaces::workspace::CustomerType;

#[derive(Default)]
pub(crate) enum WorkspaceIncludedUsageState {
    #[default]
    Idle,
    Ready(WorkspaceIncludedUsageInfo),
    Unavailable,
}

impl AIRequestUsageModel {
    pub(crate) fn workspace_included_usage(&self, ctx: &AppContext) -> WorkspaceIncludedUsageState {
        if !AuthStateProvider::as_ref(ctx).get().is_logged_in() {
            return WorkspaceIncludedUsageState::Idle;
        }
        let Some(workspace) = UserWorkspaces::as_ref(ctx).current_workspace() else {
            return WorkspaceIncludedUsageState::Idle;
        };
        if workspace.billing_metadata.customer_type != CustomerType::Scale {
            return WorkspaceIncludedUsageState::Idle;
        }
        self.workspace_allowances
            .get(&workspace.uid)
            .cloned()
            .map_or(
                WorkspaceIncludedUsageState::Unavailable,
                WorkspaceIncludedUsageState::Ready,
            )
    }
}

#[cfg(test)]
#[path = "workspace_included_usage_tests.rs"]
mod tests;
