use warpui::{AppContext, ModelHandle, SingletonEntity, ViewContext, ViewHandle};

use super::view::PaneView;
use super::{
    DetachType, PaneConfiguration, PaneContent, PaneGroup, PaneId, ShareableLink,
    ShareableLinkError,
};
use crate::ai::ai_document_view::{AIDocumentEvent, AIDocumentView};
use crate::ai::document::ai_document_model::AIDocumentModel;
use crate::app_state::{AIDocumentPaneSnapshot, LeafContents};

pub struct AIDocumentPane {
    view: ViewHandle<PaneView<AIDocumentView>>,
    pane_configuration: ModelHandle<PaneConfiguration>,
}

impl AIDocumentPane {
    pub fn new(document_view: ViewHandle<AIDocumentView>, ctx: &mut AppContext) -> Self {
        let pane_configuration = document_view.as_ref(ctx).pane_configuration().to_owned();
        let view = ctx.add_typed_action_view(document_view.window_id(ctx), |ctx| {
            let pane_id = PaneId::from_ai_document_pane_ctx(ctx);
            PaneView::new(pane_id, document_view, (), pane_configuration.clone(), ctx)
        });

        Self {
            view,
            pane_configuration,
        }
    }

    pub fn document_view(&self, ctx: &AppContext) -> ViewHandle<AIDocumentView> {
        self.view.as_ref(ctx).child(ctx)
    }
}

impl PaneContent for AIDocumentPane {
    fn id(&self) -> PaneId {
        PaneId::from_ai_document_pane_view(&self.view)
    }

    fn snapshot(&self, app: &AppContext) -> LeafContents {
        let document_view = self.document_view(app).as_ref(app);
        let document_id = *document_view.document_id();
        let ai_document_model = AIDocumentModel::as_ref(app);
        let content = ai_document_model.get_document_content(&document_id, app);
        let title = ai_document_model
            .get_current_document(&document_id)
            .map(|doc| doc.title.clone());
        if content.is_none() {
            log::warn!(
                "AI document snapshot: no content for {document_id} (document not in model)"
            );
        }
        LeafContents::AIDocument(AIDocumentPaneSnapshot::Local {
            document_id: document_id.to_string(),
            version: document_view.document_version().0 as i32,
            content,
            title,
        })
    }

    fn attach(
        &self,
        _group: &PaneGroup,
        focus_handle: crate::pane_group::focus_state::PaneFocusHandle,
        ctx: &mut ViewContext<PaneGroup>,
    ) {
        let pane_id = self.id();
        self.view
            .update(ctx, |view, ctx| view.set_focus_handle(focus_handle, ctx));

        // Bind the editor model to this window now that we know it
        let window_id = ctx.window_id();
        let doc_view = self.document_view(ctx);
        doc_view.update(ctx, move |view, ctx| {
            view.bind_window(window_id, ctx);
        });

        // Update visibility state when pane is attached/opened
        let document_id = *doc_view.as_ref(ctx).document_id();
        let pane_group_id = ctx.view_id();
        AIDocumentModel::handle(ctx).update(ctx, |model, ctx| {
            model.set_document_visible(&document_id, pane_group_id, true, ctx);
        });

        ctx.subscribe_to_view(
            &self.document_view(ctx),
            move |group, _, event, ctx| match event {
                AIDocumentEvent::Pane(pane_event) => {
                    group.handle_pane_event(pane_id, pane_event, ctx);
                }
                AIDocumentEvent::CloseRequested => {
                    group.close_pane_with_confirmation(pane_id, ctx);
                }
                AIDocumentEvent::ViewInWarpDrive(id) => {
                    ctx.emit(crate::pane_group::Event::ViewInWarpDrive(*id));
                }
                #[cfg(feature = "local_fs")]
                AIDocumentEvent::OpenCodeInWarp {
                    source,
                    layout,
                    line_col,
                } => {
                    ctx.emit(crate::pane_group::Event::OpenCodeInWarp {
                        source: source.clone(),
                        layout: *layout,
                        line_col: *line_col,
                    });
                }
                #[cfg(feature = "local_fs")]
                AIDocumentEvent::OpenFileWithTarget {
                    path,
                    target,
                    line_col,
                } => {
                    ctx.emit(crate::pane_group::Event::OpenFileWithTarget {
                        path: path.clone(),
                        target: target.clone(),
                        line_col: *line_col,
                    });
                }
                AIDocumentEvent::AttachPlanAsContext(ai_document_id) => {
                    ctx.emit(crate::pane_group::Event::AttachPlanAsContext {
                        ai_document_id: *ai_document_id,
                    });
                }
            },
        );

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
        let document_view = self.document_view(ctx);
        ctx.unsubscribe_to_view(&document_view);
        ctx.unsubscribe_to_view(&self.view);

        // Clear visibility for this pane group on close, hide, or move.
        // On move, attach() in the destination pane group will re-add the new ID.
        let document_id = *document_view.as_ref(ctx).document_id();
        let pane_group_id = ctx.view_id();
        AIDocumentModel::handle(ctx).update(ctx, |model, ctx| {
            model.set_document_visible(&document_id, pane_group_id, false, ctx);
        });
    }

    fn has_application_focus(&self, ctx: &mut ViewContext<PaneGroup>) -> bool {
        self.view.is_self_or_child_focused(ctx)
    }

    fn focus(&self, ctx: &mut ViewContext<PaneGroup>) {
        self.document_view(ctx)
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
