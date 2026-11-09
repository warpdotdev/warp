use std::collections::HashMap;

use futures::future::{AbortHandle, AbortRegistration, Abortable};
use warp_graphql::queries::get_included_usage_info::WorkspaceIncludedUsageInfo;
use warpui::{AppContext, ModelContext, SingletonEntity};

use super::{AIRequestUsageModel, AIRequestUsageModelEvent};
use crate::auth::{AuthManager, AuthStateProvider, UserUid};
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::workspaces::workspace::{CustomerType, WorkspaceUid};

#[derive(Default)]
pub(crate) enum WorkspaceIncludedUsageState {
    #[default]
    Idle,
    Loading,
    Ready(WorkspaceIncludedUsageInfo),
    Unavailable,
}

#[derive(Default)]
pub(super) struct WorkspaceIncludedUsageRequest {
    user_uid: Option<UserUid>,
    allowances: Option<HashMap<WorkspaceUid, WorkspaceIncludedUsageInfo>>,
    abort_handle: Option<AbortHandle>,
}

impl WorkspaceIncludedUsageRequest {
    fn begin(&mut self, user_uid: Option<UserUid>) -> Option<(AbortHandle, AbortRegistration)> {
        if let Some(handle) = self.abort_handle.take() {
            handle.abort();
        }
        self.user_uid = user_uid;
        self.allowances = None;
        user_uid?;
        let (handle, registration) = AbortHandle::new_pair();
        self.abort_handle = Some(handle.clone());
        Some((handle, registration))
    }

    fn complete(
        &mut self,
        handle: &AbortHandle,
        result: anyhow::Result<HashMap<WorkspaceUid, WorkspaceIncludedUsageInfo>>,
    ) {
        // Cancellation must also reject successful results already queued for the UI thread.
        if !handle.is_aborted() {
            self.allowances = result.ok();
            self.abort_handle = None;
        }
    }

    fn for_workspace(&self, workspace_uid: WorkspaceUid) -> WorkspaceIncludedUsageState {
        if self.abort_handle.is_some() {
            WorkspaceIncludedUsageState::Loading
        } else {
            self.allowances
                .as_ref()
                .and_then(|allowances| allowances.get(&workspace_uid))
                .cloned()
                .map_or(
                    WorkspaceIncludedUsageState::Unavailable,
                    WorkspaceIncludedUsageState::Ready,
                )
        }
    }
}

impl AIRequestUsageModel {
    fn included_usage_user(ctx: &AppContext) -> Option<UserUid> {
        let auth = AuthStateProvider::as_ref(ctx).get();
        if !auth.is_logged_in() {
            return None;
        }
        auth.user_id()
    }

    pub(crate) fn workspace_included_usage(&self, ctx: &AppContext) -> WorkspaceIncludedUsageState {
        let Some(user_uid) = Self::included_usage_user(ctx) else {
            return WorkspaceIncludedUsageState::Idle;
        };
        let Some(workspace) = UserWorkspaces::as_ref(ctx).current_workspace() else {
            return WorkspaceIncludedUsageState::Idle;
        };
        if workspace.billing_metadata.customer_type != CustomerType::Scale {
            return WorkspaceIncludedUsageState::Idle;
        }
        match &self.workspace_included_usage {
            Some(request) if request.user_uid == Some(user_uid) => {
                request.for_workspace(workspace.uid)
            }
            _ => WorkspaceIncludedUsageState::Idle,
        }
    }

    fn refresh_included_usage_if_user_changed(&mut self, ctx: &mut ModelContext<Self>) {
        if self
            .workspace_included_usage
            .as_ref()
            .is_some_and(|request| request.user_uid != Self::included_usage_user(ctx))
        {
            self.refresh_workspace_included_usage(ctx);
        }
    }

    pub(crate) fn refresh_workspace_included_usage(&mut self, ctx: &mut ModelContext<Self>) {
        if self.workspace_included_usage.is_none() {
            // Auth is initialized after the usage model.
            ctx.subscribe_to_model(&AuthManager::handle(ctx), |model, _, _, ctx| {
                model.refresh_included_usage_if_user_changed(ctx);
            });
        }
        let user_uid = Self::included_usage_user(ctx);
        let request = self.workspace_included_usage.get_or_insert_default();
        if request.user_uid == user_uid && request.abort_handle.is_some() {
            return;
        }
        let pending = request.begin(user_uid);
        ctx.emit(AIRequestUsageModelEvent::WorkspaceIncludedUsageUpdated);
        ctx.notify();
        let Some((handle, registration)) = pending else {
            return;
        };
        let client = self.workspace_client.clone();
        // Keep unsupported allowance fields from breaking the existing usage/grants query.
        ctx.spawn(
            async move { Abortable::new(client.included_usage_by_workspace(), registration).await },
            move |model, result, ctx| {
                if user_uid != Self::included_usage_user(ctx) || handle.is_aborted() {
                    return;
                }
                if let Ok(result) = result
                    && let Some(request) = &mut model.workspace_included_usage
                {
                    request.complete(&handle, result);
                    ctx.emit(AIRequestUsageModelEvent::WorkspaceIncludedUsageUpdated);
                    ctx.notify();
                }
            },
        );
    }
}

#[cfg(test)]
#[path = "workspace_included_usage_tests.rs"]
mod tests;
