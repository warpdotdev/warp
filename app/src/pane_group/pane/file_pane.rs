use std::{path::PathBuf, sync::Arc};

use warpui::{AppContext, ModelHandle, SingletonEntity, View, ViewContext, ViewHandle};

#[cfg(feature = "local_fs")]
use crate::code::editor_management::CodeSource;
use crate::{
    app_state::{LeafContents, NotebookPaneSnapshot},
    notebooks::{
        file::{FileNotebookEvent, FileNotebookView},
        link::{LinkEvent, NotebookLinks},
    },
    terminal::model::session::Session,
    workflows::WorkflowSelectionSource,
    workspace::ActiveSession,
};

use super::{
    super::{DefaultSessionModeBehavior, Direction},
    view::PaneView,
    DetachType, PaneConfiguration, PaneContent, PaneGroup, PaneId, ShareableLink,
    ShareableLinkError,
};

pub struct FilePane {
    view: ViewHandle<PaneView<FileNotebookView>>,
    pane_configuration: ModelHandle<PaneConfiguration>,
}

impl FilePane {
    fn from_view(file_view: ViewHandle<FileNotebookView>, ctx: &mut AppContext) -> Self {
        let pane_configuration = file_view.as_ref(ctx).pane_configuration();

        let view = ctx.add_typed_action_view(file_view.window_id(ctx), |ctx| {
            let pane_id = PaneId::from_file_pane_ctx(ctx);
            PaneView::new(pane_id, file_view, (), pane_configuration.clone(), ctx)
        });

        Self {
            view,
            pane_configuration,
        }
    }

    /// Create a new file notebook pane for the given path and optional target session. If `path`
    /// is `None` or the target session is remote, the pane is created but left empty. If `path` is
    /// `Some`, but there's no target session, the pane is created using the next focused local
    /// session.
    pub fn new<V: View>(
        path: Option<PathBuf>,
        target_session: Option<Arc<Session>>,
        #[cfg(feature = "local_fs")] code_source: Option<CodeSource>,
        ctx: &mut ViewContext<V>,
    ) -> Self {
        let view = ctx.add_typed_action_view(move |ctx| {
            let mut view = FileNotebookView::new(ctx);
            #[cfg(feature = "local_fs")]
            view.set_code_source(code_source);

            if let Some(path) = path {
                if let Some(target_session) = target_session {
                    // If the target session is Some, but non-local, do not fall back - the path is
                    // remote, so we can't reliably use the fallback behavior.
                    if target_session.is_local() {
                        view.open_local(path, Some(target_session), ctx);
                    }
                } else {
                    // If the active session is None or remote, the pane will wait for a local
                    // session to be activated.
                    let session = ActiveSession::as_ref(ctx)
                        .session(ctx.window_id())
                        .filter(|session| session.is_local());
                    view.open_local(path, session, ctx);
                }
            }

            view
        });
        Self::from_view(view, ctx)
    }
    pub fn file_view(&self, ctx: &AppContext) -> ViewHandle<FileNotebookView> {
        self.view.as_ref(ctx).child(ctx)
    }
}

fn subscribe_to_link_model(
    pane_id: PaneId,
    handle: &ModelHandle<NotebookLinks>,
    ctx: &mut ViewContext<PaneGroup>,
) {
    ctx.subscribe_to_model(handle, move |pane_group, _, event, ctx| match event {
        LinkEvent::OpenFileNotebook { path, session } => {
            ctx.emit(crate::pane_group::Event::OpenFileInWarp {
                path: path.clone(),
                session: session.clone(),
            })
        }
        LinkEvent::StartLocalSession { path } => {
            pane_group.add_session_in_directory(
                Direction::Right,
                Some(pane_id),
                None,
                Some(path.clone()),
                None,
                DefaultSessionModeBehavior::Apply,
                ctx,
            );
        }
        #[cfg(feature = "local_fs")]
        LinkEvent::OpenFileWithTarget {
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
        LinkEvent::RefreshLinks => {}
    });
}

impl PaneContent for FilePane {
    fn id(&self) -> PaneId {
        PaneId::from_file_pane_view(&self.view)
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
        let file_view = self.file_view(ctx);

        ctx.subscribe_to_view(
            &self.file_view(ctx),
            move |pane_group, _, event, ctx| match event {
                FileNotebookEvent::RunWorkflow { workflow, source } => {
                    ctx.emit(crate::pane_group::Event::RunWorkflow {
                        workflow: workflow.clone(),
                        workflow_source: source.clone(),
                        workflow_selection_source: WorkflowSelectionSource::Notebook,
                        argument_override: None,
                    });
                }
                FileNotebookEvent::TitleUpdated => {
                    ctx.emit(crate::pane_group::Event::PaneTitleUpdated)
                }
                FileNotebookEvent::FileLoaded => {
                    ctx.emit(crate::pane_group::Event::AppStateChanged)
                }
                #[cfg(feature = "local_fs")]
                FileNotebookEvent::OpenFileWithTarget {
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
                FileNotebookEvent::Pane(pane_event) => {
                    pane_group.handle_pane_event(pane_id, pane_event, ctx)
                }
            },
        );
        subscribe_to_link_model(pane_id, &file_view.as_ref(ctx).links(), ctx);

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
        // Always unsubscribe from views and models
        let file_view = self.file_view(ctx);
        ctx.unsubscribe_to_view(&file_view);
        ctx.unsubscribe_to_model(&file_view.as_ref(ctx).links());
        ctx.unsubscribe_to_view(&self.view);
    }

    fn snapshot(&self, app: &AppContext) -> LeafContents {
        let path = self.file_view(app).as_ref(app).local_path();
        LeafContents::Notebook(NotebookPaneSnapshot::LocalFileNotebook { path })
    }

    fn has_application_focus(&self, ctx: &mut ViewContext<PaneGroup>) -> bool {
        self.view.is_self_or_child_focused(ctx)
    }

    fn focus(&self, ctx: &mut ViewContext<PaneGroup>) {
        self.file_view(ctx).update(ctx, |view, ctx| view.focus(ctx));
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
