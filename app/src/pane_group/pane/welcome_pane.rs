use std::path::PathBuf;

use warpui::{AppContext, ModelHandle, View, ViewContext, ViewHandle};

use super::PaneId;
use crate::app_state::LeafContents;
use crate::pane_group::pane::welcome_view::WelcomeView;
use crate::pane_group::pane::{ShareableLink, ShareableLinkError};
use crate::pane_group::{BackingView, PaneConfiguration, PaneContent, PaneGroup, PaneView};

pub struct WelcomePane {
    view: ViewHandle<PaneView<WelcomeView>>,
    pane_configuration: ModelHandle<PaneConfiguration>,
}

impl WelcomePane {
    pub fn new<V: View>(startup_directory: Option<PathBuf>, ctx: &mut ViewContext<V>) -> Self {
        let welcome_view =
            ctx.add_typed_action_view(|ctx| WelcomeView::new(startup_directory, ctx));
        let pane_configuration = welcome_view.as_ref(ctx).pane_configuration();
        let pane_view = ctx.add_typed_action_view(|ctx| {
            let pane_id = PaneId::from_welcome_pane_ctx(ctx);
            PaneView::new(pane_id, welcome_view, (), pane_configuration.clone(), ctx)
        });
        Self {
            view: pane_view,
            pane_configuration,
        }
    }
}

impl PaneContent for WelcomePane {
    fn id(&self) -> PaneId {
        PaneId::from_welcome_pane_view(&self.view)
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
        let child = self.view.as_ref(ctx).child(ctx);
        ctx.subscribe_to_view(&child, move |pane_group, _, event, ctx| {
            pane_group.handle_pane_event(pane_id, event, ctx);
        });
    }

    fn detach(
        &self,
        _group: &PaneGroup,
        _detach_type: super::DetachType,
        ctx: &mut warpui::ViewContext<PaneGroup>,
    ) {
        let child = self.view.as_ref(ctx).child(ctx);
        ctx.unsubscribe_to_view(&child);
    }

    fn snapshot(&self, ctx: &AppContext) -> LeafContents {
        LeafContents::Welcome {
            startup_directory: self
                .view
                .as_ref(ctx)
                .child(ctx)
                .as_ref(ctx)
                .startup_directory
                .clone(),
        }
    }

    fn has_application_focus(&self, ctx: &mut warpui::ViewContext<PaneGroup>) -> bool {
        self.view.is_self_or_child_focused(ctx)
    }

    fn focus(&self, ctx: &mut warpui::ViewContext<PaneGroup>) {
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
