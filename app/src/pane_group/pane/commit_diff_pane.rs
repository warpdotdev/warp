use warpui::{AppContext, ModelHandle, ViewContext, ViewHandle};

use super::{
    DetachType, PaneConfiguration, PaneContent, PaneId, PaneView, ShareableLink, ShareableLinkError,
};
use crate::app_state::{CodePaneSnapShot, CodePaneTabSnapshot, LeafContents};
use crate::code::commit_diff_view::CommitDiffView;
use crate::pane_group::{BackingView, PaneGroup};

/// Main-area pane hosting a [`CommitDiffView`]: read-only view of what a commit changed
/// in a single file.
///
/// Created by clicking a changed file in the Git Graph commit detail; it's for temporary
/// viewing and is not persisted across restores: [`Self::snapshot`] reuses
/// [`LeafContents::Code`] but with `source: None`, so the restore path skips it as a
/// non-restorable source (the same approach as `CodeDiffPane`), avoiding the need for a
/// new `LeafContents` variant.
pub struct CommitDiffPane {
    view: ViewHandle<PaneView<CommitDiffView>>,
    pane_configuration: ModelHandle<PaneConfiguration>,
}

impl CommitDiffPane {
    pub fn from_view(diff_view: ViewHandle<CommitDiffView>, ctx: &mut AppContext) -> Self {
        let window_id = diff_view.window_id(ctx);
        let pane_configuration = diff_view.as_ref(ctx).pane_configuration();

        let view = ctx.add_typed_action_view(window_id, |ctx| {
            let pane_id = PaneId::from_commit_diff_pane_ctx(ctx);
            PaneView::new(pane_id, diff_view, (), pane_configuration.clone(), ctx)
        });

        Self {
            view,
            pane_configuration,
        }
    }

    /// The hosted [`CommitDiffView`], used to update its displayed file diff in place on reuse.
    pub fn diff_view(&self, ctx: &AppContext) -> ViewHandle<CommitDiffView> {
        self.view.as_ref(ctx).child(ctx)
    }
}

impl PaneContent for CommitDiffPane {
    fn id(&self) -> PaneId {
        PaneId::from_commit_diff_pane_view(&self.view)
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
        let child = self.view.as_ref(ctx).child(ctx);
        ctx.unsubscribe_to_view(&child);
        ctx.unsubscribe_to_view(&self.view);
    }

    fn snapshot(&self, _app: &AppContext) -> LeafContents {
        // Temporary diff pane, not restorable: reuse the Code snapshot but with source = None,
        // so it's gracefully skipped on restore.
        LeafContents::Code(CodePaneSnapShot::Local {
            tabs: vec![CodePaneTabSnapshot { path: None }],
            active_tab_index: 0,
            source: None,
        })
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
