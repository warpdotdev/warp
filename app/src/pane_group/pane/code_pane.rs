use warp_util::path::LineAndColumnArg;
use warpui::{AppContext, ModelHandle, SingletonEntity, View, ViewContext, ViewHandle};

#[cfg(feature = "local_fs")]
use crate::code::buffer_location::LocalOrRemotePath;
use crate::{
    app_state::{CodePaneSnapShot, CodePaneTabSnapshot, LeafContents},
    code::{
        editor_management::{CodeEditorStatus, CodeManager, CodeSource},
        view::{CodeView, CodeViewEvent},
    },
    pane_group::PaneGroup,
};
#[cfg(feature = "local_fs")]
use std::path::PathBuf;

use super::{
    DetachType, PaneConfiguration, PaneContent, PaneId, PaneView, ShareableLink, ShareableLinkError,
};

pub struct CodePane {
    view: ViewHandle<PaneView<CodeView>>,
    pane_configuration: ModelHandle<PaneConfiguration>,
}

impl CodePane {
    pub fn from_view(file_view: ViewHandle<CodeView>, ctx: &mut AppContext) -> Self {
        let pane_configuration = file_view.as_ref(ctx).pane_configuration();

        let view = ctx.add_typed_action_view(file_view.window_id(ctx), |ctx| {
            let pane_id = PaneId::from_code_pane_ctx(ctx);
            PaneView::new(pane_id, file_view, (), pane_configuration.clone(), ctx)
        });

        Self {
            view,
            pane_configuration,
        }
    }

    /// For now, make the pane-opening behavior consistent with markdown viewer.
    pub fn new<V: View>(
        source: CodeSource,
        line_col: Option<LineAndColumnArg>,
        ctx: &mut ViewContext<V>,
    ) -> Self {
        let view = ctx.add_typed_action_view(move |ctx| CodeView::new(source, line_col, ctx));
        Self::from_view(view, ctx)
    }

    #[cfg(feature = "local_fs")]
    pub fn new_preview<V: View>(source: CodeSource, ctx: &mut ViewContext<V>) -> Self {
        let view = ctx.add_typed_action_view(move |ctx| CodeView::new_preview(source, ctx));
        Self::from_view(view, ctx)
    }

    pub fn file_view(&self, ctx: &AppContext) -> ViewHandle<CodeView> {
        self.view.as_ref(ctx).child(ctx)
    }

    pub fn editor_status(&self, app: &AppContext) -> CodeEditorStatus {
        CodeEditorStatus::editor_status(&self.file_view(app), app)
    }
}

impl PaneContent for CodePane {
    fn id(&self) -> PaneId {
        PaneId::from_code_pane_view(&self.view)
    }

    fn pre_attach(&self, group: &PaneGroup, ctx: &mut ViewContext<PaneGroup>) -> bool {
        let source = self.file_view(ctx).as_ref(ctx).source().clone();
        let Some(location) = source.location() else {
            return true;
        };
        let pane_group_id = ctx.view_id();

        let existing_locator = CodeManager::handle(ctx).read(ctx, |manager, _ctx| {
            manager.get_locator_for_location_in_tab(pane_group_id, &location)
        });

        // If the file is already open in the same tab, don't restore it, just focus it (and jump).
        if let Some(existing_locator) = existing_locator {
            if let Some(code_pane) = group.code_pane_by_id(existing_locator.pane_id) {
                let line_col = match &source {
                    CodeSource::Link { range_start, .. } => *range_start,
                    _ => None,
                };
                code_pane.file_view(ctx).update(ctx, |code_view, ctx| {
                    code_view.open_or_focus_existing(Some(location.clone()), line_col, ctx);
                });
            }

            ctx.emit(crate::pane_group::Event::FocusPaneInWorkspace {
                locator: existing_locator,
            });
            return false;
        }

        #[cfg(feature = "local_fs")]
        self.file_view(ctx).update(ctx, |code_view, ctx| {
            let line_col = match &source {
                CodeSource::Link { range_start, .. } => *range_start,
                _ => None,
            };
            code_view.open_or_focus_existing(Some(location.clone()), line_col, ctx);
        });

        true
    }
    fn attach(
        &self,
        group: &PaneGroup,
        focus_handle: crate::pane_group::focus_state::PaneFocusHandle,
        ctx: &mut ViewContext<PaneGroup>,
    ) {
        let pane_id = self.id();
        let _code_model = group.active_file_model().clone();

        self.view
            .update(ctx, |view, ctx| view.set_focus_handle(focus_handle, ctx));

        ctx.subscribe_to_view(
            &self.file_view(ctx),
            move |pane_group, _, event, ctx| match event {
                CodeViewEvent::Pane(pane_event) => {
                    pane_group.handle_pane_event(pane_id, pane_event, ctx)
                }
                CodeViewEvent::TabChanged { location, .. } => {
                    if let Some(loc) = location {
                        pane_group.active_file_model().update(ctx, |model, ctx| {
                            model.active_file_changed(loc.clone(), ctx);
                        });
                    }
                }
                CodeViewEvent::FileOpened { location, .. } => {
                    pane_group.active_file_model().update(ctx, |model, ctx| {
                        model.active_file_changed(location.clone(), ctx);
                    });

                    // Track the opened file in the OpenedFilesModel (local files only)
                    #[cfg(feature = "local_fs")]
                    if let LocalOrRemotePath::Local(file_path) = location {
                        use crate::code::opened_files::OpenedFilesModel;
                        use repo_metadata::repositories::DetectedRepositories;

                        if let Some(repo_path) = DetectedRepositories::as_ref(ctx)
                            .get_root_for_path(&LocalOrRemotePath::Local(file_path.to_path_buf()))
                            .and_then(|r| PathBuf::try_from(r).ok())
                        {
                            OpenedFilesModel::handle(ctx).update(ctx, |opened_files, ctx| {
                                opened_files.file_opened(repo_path, file_path.clone(), ctx);
                            });
                        }
                    }
                }
                CodeViewEvent::RunTabConfigSkill { path } => {
                    ctx.emit(crate::pane_group::Event::RunTabConfigSkill { path: path.clone() });
                }
                #[cfg(not(target_family = "wasm"))]
                CodeViewEvent::OpenLspLogs { log_path } => {
                    ctx.emit(crate::pane_group::Event::OpenLspLogs {
                        log_path: log_path.clone(),
                    });
                }
                #[cfg(target_family = "wasm")]
                CodeViewEvent::OpenLspLogs { .. } => {}
            },
        );

        ctx.subscribe_to_view(&self.view, move |group, _, event, ctx| {
            group.handle_pane_view_event(pane_id, event, ctx);
        });

        let source = self.file_view(ctx).as_ref(ctx).source().clone();
        let pane_group_id = ctx.view_id();
        let window_id = ctx.window_id();
        CodeManager::handle(ctx).update(ctx, |manager, _ctx| {
            manager.register_pane(pane_group_id, window_id, pane_id, source);
        });
    }

    fn detach(
        &self,
        _group: &PaneGroup,
        detach_type: DetachType,
        ctx: &mut ViewContext<PaneGroup>,
    ) {
        // Always unsubscribe from views
        let file_view = self.file_view(ctx);
        ctx.unsubscribe_to_view(&file_view);
        ctx.unsubscribe_to_view(&self.view);

        // Deregister from CodeManager for both HiddenForClose and Closed cases
        // This ensures files can be opened elsewhere even during the undo grace period
        if matches!(detach_type, DetachType::HiddenForClose | DetachType::Closed) {
            let source = self.file_view(ctx).as_ref(ctx).source().clone();
            CodeManager::handle(ctx).update(ctx, |manager, _ctx| {
                manager.deregister_pane(&source);
            });
        }

        // Only cleanup tabs when the pane is actually being destroyed (not during undo grace period)
        // This preserves the tab state so it can be properly restored via undo-close
        #[cfg(feature = "local_fs")]
        if matches!(detach_type, DetachType::Closed) {
            file_view.update(ctx, |code_view, ctx| {
                code_view.cleanup_all_tabs(ctx);
            });
        }
    }

    fn snapshot(&self, app: &AppContext) -> LeafContents {
        let code_view_ref = self.file_view(app).as_ref(app);

        let tabs: Vec<CodePaneTabSnapshot> = (0..code_view_ref.tab_count())
            .filter_map(|i| code_view_ref.tab_at(i))
            .map(|tab| CodePaneTabSnapshot {
                path: tab.local_path(),
            })
            .collect();

        let active_tab_index = code_view_ref.active_tab_index();
        let source = code_view_ref.source().clone();

        LeafContents::Code(CodePaneSnapShot::Local {
            tabs,
            active_tab_index,
            source: Some(source),
        })
    }

    fn focus(&self, ctx: &mut ViewContext<PaneGroup>) {
        self.file_view(ctx).update(ctx, |view, ctx| view.focus(ctx));
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
