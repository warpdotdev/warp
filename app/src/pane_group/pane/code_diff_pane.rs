use warpui::{AppContext, ModelHandle, SingletonEntity, ViewContext, ViewHandle};

use super::code_diff_pane_model::CodeDiffPaneModel;
use super::{
    DetachType, PaneConfiguration, PaneContent, PaneEvent, PaneId, PaneView, ShareableLink,
    ShareableLinkError,
};
use crate::ai::blocklist::inline_action::code_diff_view::{CodeDiffView, CodeDiffViewEvent};
use crate::app_state::{CodePaneSnapShot, CodePaneTabSnapshot, LeafContents};
use crate::code::editor_management::{CodeManager, CodeSource};
use crate::pane_group::PaneGroup;

pub struct CodeDiffPane {
    view: ViewHandle<PaneView<CodeDiffView>>,
    pane_configuration: ModelHandle<PaneConfiguration>,
    model: ModelHandle<CodeDiffPaneModel>,
}

impl CodeDiffPane {
    pub fn from_view(diff_view: ViewHandle<CodeDiffView>, ctx: &mut AppContext) -> Self {
        let window_id = diff_view.window_id(ctx);
        let pane_configuration = ctx.add_model(|_ctx| {
            let mut config = PaneConfiguration::new("");
            // This title must be set with .set_title and not just ::new() to ensure that the tab renders immediately.
            config.set_title("Requested Edit", _ctx);
            config
        });

        let diff_view_clone = diff_view.clone();

        let view = ctx.add_typed_action_view(window_id, |ctx| {
            let pane_id = PaneId::from_code_diff_pane_ctx(ctx);
            PaneView::new(pane_id, diff_view, (), pane_configuration.clone(), ctx)
        });

        let model = ctx.add_model(|ctx| CodeDiffPaneModel::new(diff_view_clone, ctx));

        Self {
            view,
            pane_configuration,
            model,
        }
    }

    pub fn diff_view(&self, ctx: &AppContext) -> ViewHandle<CodeDiffView> {
        self.view.as_ref(ctx).child(ctx)
    }
}

impl PaneContent for CodeDiffPane {
    fn id(&self) -> PaneId {
        PaneId::from_code_diff_pane_view(&self.view)
    }

    fn attach(
        &self,
        _group: &PaneGroup,
        focus_handle: crate::pane_group::focus_state::PaneFocusHandle,
        ctx: &mut ViewContext<PaneGroup>,
    ) {
        self.view
            .update(ctx, |view, ctx| view.set_focus_handle(focus_handle, ctx));

        // Skip if the diff view is already in full-pane mode to avoid an
        // unnecessary re-render.
        let diff_view = self.diff_view(ctx);
        let is_already_full_pane = diff_view.as_ref(ctx).display_mode().is_full_pane();
        if !is_already_full_pane {
            diff_view.update(ctx, |view, ctx| {
                view.set_embedded_display_mode(false, ctx);
            });
        }

        let pane_id = self.id();

        ctx.subscribe_to_model(&self.model, move |pane_group, _, event, ctx| match event {
            CodeDiffViewEvent::Pane(pane_event) => {
                pane_group.handle_pane_event(pane_id, pane_event, ctx)
            }
            CodeDiffViewEvent::EditorFocused => {
                pane_group.handle_pane_event(pane_id, &PaneEvent::FocusSelf, ctx)
            }
            _ => (),
        });

        ctx.subscribe_to_view(&self.view, move |group, _, event, ctx| {
            group.handle_pane_view_event(pane_id, event, ctx);
        });

        let action_id = self.diff_view(ctx).as_ref(ctx).action_id().clone();
        let pane_group_id = ctx.view_id();
        let window_id = ctx.window_id();
        CodeManager::handle(ctx).update(ctx, |manager, _ctx| {
            manager.register_pane(
                pane_group_id,
                window_id,
                pane_id,
                CodeSource::AIAction { id: action_id },
            );
        });
    }

    fn detach(
        &self,
        _group: &PaneGroup,
        detach_type: DetachType,
        ctx: &mut ViewContext<PaneGroup>,
    ) {
        let diff_view = self.diff_view(ctx);

        // When a pane is moved to another window (tab transfer), the diff view
        // should keep its full-pane display mode because it will be re-attached
        // in the target window. Only reset to embedded mode on close/hide.
        if !matches!(detach_type, DetachType::Moved) {
            diff_view.update(ctx, |view, ctx| {
                view.set_embedded_display_mode(true, ctx);
            });
        }

        // Always unsubscribe from models and views
        ctx.unsubscribe_to_model(&self.model);
        ctx.unsubscribe_to_view(&self.view);

        if matches!(detach_type, DetachType::Closed) {
            // Only deregister from CodeManager when permanently closed
            let action_id = self.diff_view(ctx).as_ref(ctx).action_id().clone();
            CodeManager::handle(ctx).update(ctx, |manager, _ctx| {
                manager.deregister_pane(&CodeSource::AIAction { id: action_id });
            });
        }
    }

    fn snapshot(&self, _app: &AppContext) -> LeafContents {
        // Todo (kc) Implement snapshots.
        LeafContents::Code(CodePaneSnapShot::Local {
            tabs: vec![CodePaneTabSnapshot { path: None }],
            active_tab_index: 0,
            source: None,
        })
    }

    fn focus(&self, ctx: &mut ViewContext<PaneGroup>) {
        ctx.focus(&self.diff_view(ctx));
    }

    fn has_application_focus(&self, ctx: &mut ViewContext<PaneGroup>) -> bool {
        self.view.is_self_or_child_focused(ctx)
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
