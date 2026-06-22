use warpui::{AppContext, ModelHandle, SingletonEntity, View, ViewContext, ViewHandle};

use super::view::PaneView;
use super::{
    DetachType, PaneConfiguration, PaneContent, PaneGroup, PaneId, ShareableLink,
    ShareableLinkError,
};
use crate::ai::execution_profiles::editor::{
    ExecutionProfileEditorManager, ExecutionProfileEditorView, ExecutionProfileEditorViewEvent,
};
use crate::ai::execution_profiles::profiles::ClientProfileId;
use crate::app_state::LeafContents;

pub struct ExecutionProfileEditorPane {
    view: ViewHandle<PaneView<ExecutionProfileEditorView>>,
    pane_configuration: ModelHandle<PaneConfiguration>,
}

impl ExecutionProfileEditorPane {
    pub fn from_view(
        execution_profile_editor_view: ViewHandle<ExecutionProfileEditorView>,
        ctx: &mut AppContext,
    ) -> Self {
        let pane_configuration = execution_profile_editor_view
            .as_ref(ctx)
            .pane_configuration();

        let view = ctx.add_typed_action_view(execution_profile_editor_view.window_id(ctx), |ctx| {
            let pane_id = PaneId::from_execution_profile_editor_pane_ctx(ctx);
            PaneView::new(
                pane_id,
                execution_profile_editor_view,
                (),
                pane_configuration.clone(),
                ctx,
            )
        });

        Self {
            view,
            pane_configuration,
        }
    }

    pub fn new<V: View>(profile_id: ClientProfileId, ctx: &mut ViewContext<V>) -> Self {
        let view =
            ctx.add_typed_action_view(|ctx| ExecutionProfileEditorView::new(profile_id, ctx));
        Self::from_view(view, ctx)
    }

    pub fn execution_profile_editor_view(
        &self,
        ctx: &AppContext,
    ) -> ViewHandle<ExecutionProfileEditorView> {
        self.view.as_ref(ctx).child(ctx)
    }
}

impl PaneContent for ExecutionProfileEditorPane {
    fn id(&self) -> PaneId {
        PaneId::from_execution_profile_editor_pane_view(&self.view)
    }

    fn attach(
        &self,
        _group: &PaneGroup,
        focus_handle: crate::pane_group::focus_state::PaneFocusHandle,
        ctx: &mut ViewContext<PaneGroup>,
    ) {
        self.view
            .update(ctx, |view, ctx| view.set_focus_handle(focus_handle, ctx));

        let exec_view_handle = self.execution_profile_editor_view(ctx);
        let pane_id = self.id();
        let pane_group_id = ctx.view_id();
        let window_id = ctx.window_id();
        let profile_id = exec_view_handle.as_ref(ctx).profile_id();

        ctx.subscribe_to_view(&exec_view_handle, move |pane_group, _, event, ctx| {
            let ExecutionProfileEditorViewEvent::Pane(pane_event) = event;
            pane_group.handle_pane_event(pane_id, pane_event, ctx)
        });
        ctx.subscribe_to_view(&self.view, move |group, _, event, ctx| {
            group.handle_pane_view_event(pane_id, event, ctx);
        });

        ExecutionProfileEditorManager::handle(ctx).update(ctx, |manager, ctx| {
            manager.register_pane(self, pane_group_id, window_id, profile_id, ctx);
        });
    }

    fn detach(
        &self,
        _group: &PaneGroup,
        _detach_type: DetachType,
        ctx: &mut ViewContext<PaneGroup>,
    ) {
        // Always unsubscribe from views
        let execution_profile_editor_view = self.execution_profile_editor_view(ctx);
        let profile_id = execution_profile_editor_view.as_ref(ctx).profile_id();
        ctx.unsubscribe_to_view(&execution_profile_editor_view);
        ctx.unsubscribe_to_view(&self.view);

        // Always deregister from ExecutionProfileEditorManager - it will be re-registered on attach if restored
        let window_id = ctx.window_id();
        ExecutionProfileEditorManager::handle(ctx).update(ctx, |manager, _| {
            manager.deregister_pane(&window_id, &profile_id);
        });
    }

    fn snapshot(&self, _app: &AppContext) -> LeafContents {
        LeafContents::ExecutionProfileEditor
    }

    fn has_application_focus(&self, ctx: &mut ViewContext<PaneGroup>) -> bool {
        self.view.is_self_or_child_focused(ctx)
    }

    fn focus(&self, ctx: &mut ViewContext<PaneGroup>) {
        self.execution_profile_editor_view(ctx)
            .update(ctx, |view, ctx| view.focus(ctx));
    }

    fn shareable_link(
        &self,
        _ctx: &mut ViewContext<PaneGroup>,
    ) -> Result<ShareableLink, ShareableLinkError> {
        Ok(ShareableLink::Base)
    }

    fn pane_configuration(&self) -> ModelHandle<PaneConfiguration> {
        self.pane_configuration.clone()
    }

    fn is_pane_being_dragged(&self, ctx: &AppContext) -> bool {
        self.view.as_ref(ctx).is_being_dragged()
    }
}
