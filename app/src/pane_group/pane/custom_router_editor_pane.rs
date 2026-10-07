use warpui::{AppContext, ModelHandle, View, ViewContext, ViewHandle};

use super::view::PaneView;
use super::{
    DetachType, PaneConfiguration, PaneContent, PaneGroup, PaneId, ShareableLink,
    ShareableLinkError,
};
use crate::ai::custom_model_router_editor::{CustomRouterEditorEvent, CustomRouterEditorView};
use crate::ai::custom_model_routers::CustomModelRouter;
use crate::app_state::LeafContents;
use crate::pane_group::focus_state::PaneFocusHandle;

pub struct CustomRouterEditorPane {
    view: ViewHandle<PaneView<CustomRouterEditorView>>,
    pane_configuration: ModelHandle<PaneConfiguration>,
}

impl CustomRouterEditorPane {
    /// Create a new router editor pane.
    ///
    /// Pass `existing = None` for creating a new router, or
    /// `existing = Some(router)` for editing an existing one.
    pub fn new<V: View>(existing: Option<CustomModelRouter>, ctx: &mut ViewContext<V>) -> Self {
        let editor_view =
            ctx.add_typed_action_view(|ctx| CustomRouterEditorView::new(existing, ctx));
        Self::from_view(editor_view, ctx)
    }

    pub fn from_view(
        editor_view: ViewHandle<CustomRouterEditorView>,
        ctx: &mut AppContext,
    ) -> Self {
        let pane_configuration = editor_view.as_ref(ctx).pane_configuration();

        let view = ctx.add_typed_action_view(editor_view.window_id(ctx), |ctx| {
            let pane_id = PaneId::from_custom_router_editor_pane_ctx(ctx);
            PaneView::new(pane_id, editor_view, (), pane_configuration.clone(), ctx)
        });

        Self {
            view,
            pane_configuration,
        }
    }

    pub fn custom_router_editor_view(
        &self,
        ctx: &AppContext,
    ) -> ViewHandle<CustomRouterEditorView> {
        self.view.as_ref(ctx).child(ctx)
    }
}

impl PaneContent for CustomRouterEditorPane {
    fn id(&self) -> PaneId {
        PaneId::from_custom_router_editor_pane_view(&self.view)
    }

    fn attach(
        &self,
        _group: &PaneGroup,
        focus_handle: PaneFocusHandle,
        ctx: &mut ViewContext<PaneGroup>,
    ) {
        self.view
            .update(ctx, |view, ctx| view.set_focus_handle(focus_handle, ctx));

        let editor_view = self.custom_router_editor_view(ctx);
        let pane_id = self.id();

        ctx.subscribe_to_view(&editor_view, move |pane_group, _, event, ctx| {
            let CustomRouterEditorEvent::Pane(pane_event) = event;
            pane_group.handle_pane_event(pane_id, pane_event, ctx);
        });
        ctx.subscribe_to_view(&self.view, move |group, _, event, ctx| {
            group.handle_pane_view_event(pane_id, event, ctx);
        });
    }

    fn detach(
        &self,
        _group: &PaneGroup,
        _detach_type: DetachType,
        ctx: &mut ViewContext<PaneGroup>,
    ) {
        let editor_view = self.custom_router_editor_view(ctx);
        ctx.unsubscribe_to_view(&editor_view);
        ctx.unsubscribe_to_view(&self.view);
    }

    fn snapshot(&self, _app: &AppContext) -> LeafContents {
        LeafContents::CustomRouterEditor
    }

    fn has_application_focus(&self, ctx: &mut ViewContext<PaneGroup>) -> bool {
        self.view.is_self_or_child_focused(ctx)
    }

    fn focus(&self, ctx: &mut ViewContext<PaneGroup>) {
        self.custom_router_editor_view(ctx)
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
