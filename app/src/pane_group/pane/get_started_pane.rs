use warpui::{AppContext, ModelHandle, View, ViewContext, ViewHandle};

use super::PaneId;
use crate::app_state::LeafContents;
use crate::pane_group::pane::get_started_view::GetStartedView;
use crate::pane_group::pane::{ShareableLink, ShareableLinkError};
use crate::pane_group::{BackingView, PaneConfiguration, PaneContent, PaneGroup, PaneView};

pub struct GetStartedPane {
    view: ViewHandle<PaneView<GetStartedView>>,
    pane_configuration: ModelHandle<PaneConfiguration>,
}

impl GetStartedPane {
    pub fn new<V: View>(ctx: &mut ViewContext<V>) -> Self {
        let get_started_view = ctx.add_typed_action_view(GetStartedView::new);
        let pane_configuration = get_started_view.as_ref(ctx).pane_configuration();
        let pane_view = ctx.add_typed_action_view(|ctx| {
            let pane_id = PaneId::from_get_started_pane_ctx(ctx);
            PaneView::new(
                pane_id,
                get_started_view,
                (),
                pane_configuration.clone(),
                ctx,
            )
        });
        Self {
            view: pane_view,
            pane_configuration,
        }
    }
}

impl PaneContent for GetStartedPane {
    fn id(&self) -> PaneId {
        PaneId::from_get_started_pane_view(&self.view)
    }

    fn attach(
        &self,
        _group: &PaneGroup,
        focus_handle: crate::pane_group::focus_state::PaneFocusHandle,
        ctx: &mut ViewContext<PaneGroup>,
    ) {
        self.view
            .update(ctx, |view, ctx| view.set_focus_handle(focus_handle, ctx));
        let child = self.view.as_ref(ctx).child(ctx);

        let pane_id = self.id();
        ctx.subscribe_to_view(&child, move |pane_group, _, event, ctx| {
            pane_group.handle_pane_event(pane_id, event, ctx);
        });
    }

    fn detach(
        &self,
        _group: &PaneGroup,
        _detach_type: super::DetachType,
        ctx: &mut ViewContext<PaneGroup>,
    ) {
        let child = self.view.as_ref(ctx).child(ctx);
        ctx.unsubscribe_to_view(&child);
    }

    fn snapshot(&self, _ctx: &AppContext) -> LeafContents {
        LeafContents::GetStarted
    }

    fn has_application_focus(&self, ctx: &mut ViewContext<PaneGroup>) -> bool {
        self.view.is_self_or_child_focused(ctx)
    }

    fn focus(&self, ctx: &mut ViewContext<PaneGroup>) {
        self.view
            .as_ref(ctx)
            .child(ctx)
            .update(ctx, BackingView::focus_contents)
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
