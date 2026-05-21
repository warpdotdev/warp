use warpui::{AppContext, ModelHandle, SingletonEntity, View, ViewContext, ViewHandle};

use super::view::PaneView;
use super::{
    DetachType, PaneConfiguration, PaneContent, PaneGroup, PaneId, ShareableLink,
    ShareableLinkError,
};
use crate::ai::facts::{AIFactManager, AIFactView, AIFactViewEvent};
use crate::app_state::{AIFactPaneSnapshot, LeafContents};

pub struct AIFactPane {
    view: ViewHandle<PaneView<AIFactView>>,
    pane_configuration: ModelHandle<PaneConfiguration>,
}

impl AIFactPane {
    pub fn from_view(ai_fact_view: ViewHandle<AIFactView>, ctx: &mut AppContext) -> Self {
        let pane_configuration = ai_fact_view.as_ref(ctx).pane_configuration();

        let view = ctx.add_typed_action_view(ai_fact_view.window_id(ctx), |ctx| {
            let pane_id = PaneId::from_ai_fact_pane_ctx(ctx);
            PaneView::new(pane_id, ai_fact_view, (), pane_configuration.clone(), ctx)
        });

        Self {
            view,
            pane_configuration,
        }
    }

    pub fn new<V: View>(ctx: &mut ViewContext<V>) -> Self {
        let window_id = ctx.window_id();
        let view =
            AIFactManager::handle(ctx).read(ctx, |manager, _ctx| manager.ai_fact_view(window_id));
        Self::from_view(view, ctx)
    }

    pub fn ai_fact_view(&self, ctx: &AppContext) -> ViewHandle<AIFactView> {
        self.view.as_ref(ctx).child(ctx)
    }
}

impl PaneContent for AIFactPane {
    fn id(&self) -> PaneId {
        PaneId::from_ai_fact_pane_view(&self.view)
    }

    fn attach(
        &self,
        _group: &PaneGroup,
        focus_handle: crate::pane_group::focus_state::PaneFocusHandle,
        ctx: &mut ViewContext<PaneGroup>,
    ) {
        self.view
            .update(ctx, |view, ctx| view.set_focus_handle(focus_handle, ctx));

        let pane_id = self.id();
        let pane_group_id = ctx.view_id();
        let window_id = ctx.window_id();

        ctx.subscribe_to_view(&self.ai_fact_view(ctx), move |pane_group, _, event, ctx| {
            if let AIFactViewEvent::Pane(pane_event) = event {
                pane_group.handle_pane_event(pane_id, pane_event, ctx)
            }
        });
        ctx.subscribe_to_view(&self.view, move |group, _, event, ctx| {
            group.handle_pane_view_event(pane_id, event, ctx);
        });

        AIFactManager::handle(ctx).update(ctx, |manager, ctx| {
            manager.register_pane(self, pane_group_id, window_id, ctx);
        });
    }

    fn detach(
        &self,
        _group: &PaneGroup,
        _detach_type: DetachType,
        ctx: &mut ViewContext<PaneGroup>,
    ) {
        // Always unsubscribe from views
        let ai_fact_view = self.ai_fact_view(ctx);
        ctx.unsubscribe_to_view(&ai_fact_view);
        ctx.unsubscribe_to_view(&self.view);

        // Always deregister from AIFactManager - it will be re-registered on attach if restored
        let window_id = ctx.window_id();
        AIFactManager::handle(ctx).update(ctx, |manager, ctx| {
            manager.deregister_pane(&window_id, ctx);
        });
    }

    fn snapshot(&self, _app: &AppContext) -> LeafContents {
        LeafContents::AIFact(AIFactPaneSnapshot::Personal)
    }

    fn has_application_focus(&self, ctx: &mut ViewContext<PaneGroup>) -> bool {
        self.view.is_self_or_child_focused(ctx)
    }

    fn focus(&self, ctx: &mut ViewContext<PaneGroup>) {
        self.ai_fact_view(ctx)
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
