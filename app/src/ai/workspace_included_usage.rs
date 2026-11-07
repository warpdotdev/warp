use futures::future::{AbortHandle, AbortRegistration, Abortable};
use warp_graphql::queries::get_included_usage_info::WorkspaceIncludedUsageInfo;
use warpui::{AppContext, ModelContext, SingletonEntity};

use super::{AIRequestUsageModel, AIRequestUsageModelEvent};
use crate::auth::{AuthManager, AuthStateProvider, UserUid};
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::workspaces::workspace::{CustomerType, WorkspaceUid};

type IncludedUsageContext = (UserUid, WorkspaceUid);

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
    context: Option<IncludedUsageContext>,
    state: WorkspaceIncludedUsageState,
    abort_handle: Option<AbortHandle>,
}

impl WorkspaceIncludedUsageRequest {
    fn begin(
        &mut self,
        context: Option<IncludedUsageContext>,
    ) -> Option<(AbortHandle, AbortRegistration)> {
        if let Some(handle) = self.abort_handle.take() {
            handle.abort();
        }
        self.context = context;
        self.state = WorkspaceIncludedUsageState::Idle;
        context?;
        self.state = WorkspaceIncludedUsageState::Loading;
        let (handle, registration) = AbortHandle::new_pair();
        self.abort_handle = Some(handle.clone());
        Some((handle, registration))
    }

    fn complete(
        &mut self,
        handle: &AbortHandle,
        result: anyhow::Result<Option<WorkspaceIncludedUsageInfo>>,
    ) {
        // Cancellation must also reject successful results already queued for the UI thread.
        if !handle.is_aborted() {
            self.state = result.ok().flatten().map_or(
                WorkspaceIncludedUsageState::Unavailable,
                WorkspaceIncludedUsageState::Ready,
            );
        }
    }
}

impl AIRequestUsageModel {
    fn included_usage_context(ctx: &AppContext) -> Option<IncludedUsageContext> {
        let auth = AuthStateProvider::as_ref(ctx).get();
        if !auth.is_logged_in() {
            return None;
        }
        let workspace = UserWorkspaces::as_ref(ctx).current_workspace()?;
        (workspace.billing_metadata.customer_type == CustomerType::Scale)
            .then_some((auth.user_id()?, workspace.uid))
    }

    pub(crate) fn workspace_included_usage(
        &self,
        ctx: &AppContext,
    ) -> &WorkspaceIncludedUsageState {
        match &self.workspace_included_usage {
            Some(request) if request.context == Self::included_usage_context(ctx) => &request.state,
            _ => &WorkspaceIncludedUsageState::Idle,
        }
    }

    fn refresh_included_usage_if_context_changed(&mut self, ctx: &mut ModelContext<Self>) {
        if self
            .workspace_included_usage
            .as_ref()
            .is_some_and(|request| request.context != Self::included_usage_context(ctx))
        {
            self.refresh_workspace_included_usage(ctx);
        }
    }

    pub(crate) fn refresh_workspace_included_usage(&mut self, ctx: &mut ModelContext<Self>) {
        if self.workspace_included_usage.is_none() {
            // Workspace and auth models are initialized after the usage model.
            ctx.subscribe_to_model(&UserWorkspaces::handle(ctx), |model, _, _, ctx| {
                model.refresh_included_usage_if_context_changed(ctx);
            });
            ctx.subscribe_to_model(&AuthManager::handle(ctx), |model, _, _, ctx| {
                model.refresh_included_usage_if_context_changed(ctx);
            });
        }
        let context = Self::included_usage_context(ctx);
        let request = self.workspace_included_usage.get_or_insert_default();
        if request.context == context
            && matches!(request.state, WorkspaceIncludedUsageState::Loading)
        {
            return;
        }
        let pending = request.begin(context);
        ctx.emit(AIRequestUsageModelEvent::WorkspaceIncludedUsageUpdated);
        ctx.notify();
        let Some((handle, registration)) = pending else {
            return;
        };
        let Some((_, workspace_uid)) = context else {
            return;
        };
        let client = self.workspace_client.clone();
        // Older servers must not break personal allowance and purchased-credit refreshes.
        ctx.spawn(
            async move {
                Abortable::new(client.included_usage_info(workspace_uid), registration).await
            },
            move |model, result, ctx| {
                if context != Self::included_usage_context(ctx) || handle.is_aborted() {
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
