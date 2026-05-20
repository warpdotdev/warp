use crate::code::editor::scroll::ScrollPosition;
use crate::code::editor::view::CodeEditorRenderOptions;
use crate::code::editor_management::CodeEditorStatus;
use crate::code::global_buffer_model::GlobalBufferModel;
use crate::code::local_code_editor::ShowFindReferencesCard;
use crate::code::{ImmediateSaveError, SaveOutcome, SaveStatus};
use crate::editor::InteractionState;
use crate::input::Vector2F;
use crate::pane_group::focus_state::PaneFocusHandle;
use crate::pane_group::pane::view::header::components::{
    render_pane_header_buttons, render_pane_header_title_text, render_three_column_header,
    CenteredHeaderEdgeWidth,
};
use crate::pane_group::pane::view::header::render_pane_header_draggable;
use crate::pane_group::{CodePane, PaneConfigurationEvent, PaneDragDropLocation};
use crate::quit_warning::UnsavedStateSummary;
use crate::server::telemetry::CodeContextDestination;
use crate::terminal::cli_agent::{
    build_selection_line_range_prompt, build_selection_substring_prompt,
};
use crate::terminal::view::CliAgentRouting;
use crate::workspace::util::get_context_target_terminal_view;
use crate::workspace::TabBarDropTargetData;
use crate::{code::EditorTabBarDropTargetData, pane_group::pane::ActionOrigin};
use lsp::LspManagerModel;
use pathfinder_color::ColorU;
use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::vec2f;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use warp_core::channel::{Channel, ChannelState};
use warp_core::features::FeatureFlag;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::icons::ICON_DIMENSIONS;
use warp_editor::render::element::VerticalExpansionBehavior;
use warp_util::path::LineAndColumnArg;
use warpui::elements::Rect;
use warpui::fonts::Style;
use warpui::text::point::Point;
use warpui::text_layout::ClipConfig;

#[cfg(feature = "local_fs")]
use warpui::clipboard::ClipboardContent;
use warpui::{
    elements::{
        AcceptedByDropTarget, Align, Border, ChildAnchor, ChildView, Clipped, ConstrainedBox,
        Container, CornerRadius, CrossAxisAlignment, Draggable, DraggableState, DropTarget, Empty,
        Expanded, Flex, Hoverable, MainAxisAlignment, MainAxisSize, MouseStateHandle,
        OffsetPositioning, Padding, ParentAnchor, ParentElement, ParentOffsetBounds, Radius,
        SavePosition, Shrinkable, Stack, Text,
    },
    fonts::{Properties, Weight},
    id,
    keymap::EditableBinding,
    ui_components::{button::ButtonVariant, components::UiComponent},
    AppContext, Element, Entity, ModelHandle, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle, WindowId,
};

use crate::{
    menu::{MenuItem, MenuItemFields},
    notebooks::file::{is_markdown_file, MarkdownDisplayMode},
    search::{files::icon::icon_from_file_path, ItemHighlightState},
    tab::TAB_BAR_BORDER_HEIGHT,
    ui_components::{blended_colors, buttons::icon_button},
    view_components::{DismissibleToast, MarkdownToggleEvent, MarkdownToggleView},
    workspace::{ActiveSession, ToastStack, WorkspaceAction},
};

use crate::pane_group::{
    pane::{view, PaneHeaderAction},
    BackingView, PaneConfiguration, PaneEvent,
};

use super::{
    buffer_location::LocalOrRemotePath,
    diff_viewer::DiffViewer,
    editor::view::{CodeEditorEvent, CodeEditorView},
    editor_management::{CodeManager, CodeSource},
    local_code_editor::{LocalCodeEditorEvent, LocalCodeEditorView},
};

use crate::{send_telemetry_from_ctx, TelemetryEvent};

type SaveCallback =
    Box<dyn FnOnce(SaveOutcome, &mut CodeView, &mut ViewContext<CodeView>) + Send + Sync + 'static>;

const CLOSE_BUTTON_WIDTH: f32 = 24.;
const LANGUAGE_ICON_WIDTH: f32 = 16.;
const TAB_INTERNAL_MARGIN: f32 = 4.;
const TAB_HORIZONTAL_MARGIN: f32 = 8.;
const TAB_PADDING: f32 = 2.;

// Keybinding constants - exported so AI document view can reuse
pub const SAVE_FILE_BINDING_NAME: &str = "code_view:save";
pub const SAVE_FILE_BINDING_DESCRIPTION: &str = "Save file";

pub fn init(app: &mut AppContext) {
    super::editor::view::init(app);
    super::local_code_editor::init(app);

    let text_entry = id!("CodeEditorView") & !id!("IMEOpen");
    app.register_editable_bindings([
        EditableBinding::new(
            SAVE_FILE_BINDING_NAME,
            SAVE_FILE_BINDING_DESCRIPTION,
            CodeViewAction::SaveFile,
        )
        .with_context_predicate(text_entry.clone())
        .with_key_binding("cmdorctrl-s"),
        EditableBinding::new(
            "code_view:save_as",
            "Save file as",
            CodeViewAction::SaveFileAs,
        )
        .with_context_predicate(text_entry.clone())
        .with_key_binding("cmdorctrl-shift-S"),
        EditableBinding::new(
            "code_view:close_all_tabs",
            "Close all tabs",
            CodeViewAction::CloseAll,
        )
        .with_context_predicate(id!("CodeEditorView"))
        .with_key_binding("cmdorctrl-r w"),
        EditableBinding::new(
            "code_view:close_saved_tabs",
            "Close saved tabs",
            CodeViewAction::CloseSaved,
        )
        .with_context_predicate(id!("CodeEditorView"))
        .with_key_binding("cmdorctrl-r u"),
    ]);
}

const PADDING: f32 = 4.;

pub use crate::util::openable_file_type::is_binary_file;
/// Determines the `SavePosition` ID for a draggable tab based on its index.
pub fn tab_position_id(index: usize) -> String {
    format!("file_tab_position_{index}")
}

#[derive(Debug, Clone)]
enum TabBarDragPosition {
    BeforeTab { index: usize },
    AfterTab { index: usize },
}

#[derive(Debug, Clone)]
pub enum CodeViewAction {
    SaveFile,
    SaveFileAs,
    AcceptPendingDiffsAndSave,
    RejectPendingDiffs,
    SetCurrentTabIndex {
        index: usize,
    },
    RemoveTabAtIndex {
        index: usize,
    },
    CloseAll,
    CloseSaved,
    ToggleMaximized,
    #[cfg(feature = "local_fs")]
    CopyFilePath,
    /// Open the active code tab's file in the platform's file manager
    /// (Finder on macOS, Explorer on Windows). No-op when the active tab has
    /// no resolvable local path.
    #[cfg(feature = "local_fs")]
    RevealInFinder,
    #[cfg(feature = "local_fs")]
    RenderMarkdown,
    DragOverIndex {
        target: usize,
        drag_position: RectF,
    },
    DropAtIndex {
        origin: usize,
        target: usize,
        drag_position: RectF,
    },
    ClearEditorTabGroupDragPositions,
    ClearWorkspaceTabGroupDragPositions,
}

#[derive(Debug, Clone)]
pub enum CodeViewEvent {
    Pane(PaneEvent),
    TabChanged {
        location: Option<LocalOrRemotePath>,
        tab_index: usize,
    },
    FileOpened {
        location: LocalOrRemotePath,
        tab_index: usize,
    },
    RunTabConfigSkill {
        path: PathBuf,
    },
    OpenLspLogs {
        log_path: PathBuf,
    },
}

#[derive(Default, Clone)]
struct TabDataMouseStateHandles {
    tab_handle: MouseStateHandle,
    close_handle: MouseStateHandle,
    accept_mouse_state: MouseStateHandle,
    reject_mouse_state: MouseStateHandle,
    tab_draggable_state: DraggableState,
}

#[derive(Clone)]
pub struct TabData {
    location: Option<LocalOrRemotePath>,
    editor_view: ViewHandle<LocalCodeEditorView>,
    mouse_state_handles: TabDataMouseStateHandles,
    preview: bool,
}

#[derive(Debug, Clone)]
pub enum PendingSaveIntent {
    Save,
    Discard,
    Cancel,
}

impl TabData {
    /// Returns the local filesystem path, if this tab is backed by a local file.
    /// Returns `None` for remote files and untitled tabs.
    pub fn local_path(&self) -> Option<PathBuf> {
        self.location
            .as_ref()
            .and_then(|loc| loc.to_local_path().map(Path::to_path_buf))
    }
}

pub struct CodeView {
    tab_group: Vec<TabData>,
    active_tab_index: usize,
    pane_configuration: ModelHandle<PaneConfiguration>,
    focus_handle: Option<PaneFocusHandle>,
    source: CodeSource,
    window_id: WindowId,
    drag_position: Option<TabBarDragPosition>,
    markdown_mode_segmented_control: Option<ViewHandle<MarkdownToggleView>>,
}

impl CodeView {
    fn new_internal(source: CodeSource, ctx: &mut ViewContext<Self>) -> Self {
        let pane_configuration = ctx.add_model(|_ctx| PaneConfiguration::new(""));
        let window_id = ctx.window_id();

        Self {
            tab_group: Default::default(),
            active_tab_index: 0,
            pane_configuration,
            focus_handle: None,
            source,
            window_id,
            drag_position: None,
            markdown_mode_segmented_control: None,
        }
    }

    pub fn new(
        source: CodeSource,
        line_col: Option<LineAndColumnArg>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let location = source.location();
        let mut view = Self::new_internal(source, ctx);
        view.open_or_focus_existing(location, line_col, ctx);
        #[cfg(feature = "local_fs")]
        {
            view.update_markdown_mode_segmented_control(ctx);
        }
        view
    }

    #[cfg(feature = "local_fs")]
    fn update_markdown_mode_segmented_control(&mut self, ctx: &mut ViewContext<Self>) {
        let path = self
            .local_path(ctx)
            .or_else(|| {
                self.tab_at(self.active_tab_index)
                    .and_then(|t| t.local_path())
            })
            .or_else(|| self.source.path());

        let is_markdown = path.as_ref().map(is_markdown_file).unwrap_or(false);

        if !is_markdown {
            self.markdown_mode_segmented_control = None;
            ctx.notify();
            return;
        }

        if self.markdown_mode_segmented_control.is_none() {
            let handle = ctx.add_typed_action_view(|ctx| {
                MarkdownToggleView::new(MarkdownDisplayMode::Raw, ctx)
            });

            ctx.subscribe_to_view(&handle, |view, _, event, ctx| {
                let MarkdownToggleEvent::ModeSelected(mode) = event;
                match mode {
                    MarkdownDisplayMode::Rendered => {
                        view.handle_action(&CodeViewAction::RenderMarkdown, ctx);
                    }
                    MarkdownDisplayMode::Raw => {}
                }
            });

            self.markdown_mode_segmented_control = Some(handle);
        }

        ctx.notify();
    }

    /// Restore a code view from a persisted multi-tab snapshot.
    pub fn restore(
        tabs: &[crate::app_state::CodePaneTabSnapshot],
        active_tab_index: usize,
        source: CodeSource,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let mut view = Self::new_internal(source, ctx);
        for tab_snapshot in tabs {
            let location = tab_snapshot.path.clone().map(LocalOrRemotePath::Local);
            let tab_data = view.build_tab_data(location, false, ctx);
            view.tab_group.push(tab_data);
        }
        let clamped_index = if view.tab_group.is_empty() {
            0
        } else {
            active_tab_index.min(view.tab_group.len() - 1)
        };
        view.active_tab_index = clamped_index;
        view
    }

    /// Create a new "preview" code view for when a user is exploring the file tree.
    /// There is only one preview active at a time
    pub fn new_preview(source: CodeSource, ctx: &mut ViewContext<Self>) -> Self {
        let path = source.path();
        let mut view = Self::new_internal(source, ctx);

        if let Some(path) = path {
            view.open_in_preview_or_promote(path, ctx);
            #[cfg(feature = "local_fs")]
            {
                view.update_markdown_mode_segmented_control(ctx);
            }
        } else {
            log::warn!("Preview CodeView constructed with no path");
        }
        view
    }

    /// If a tab is a preview, promote it and emit "FileOpened"
    fn promote_if_preview(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(tab) = self.tab_group.get_mut(self.active_tab_index) {
            if tab.preview {
                tab.preview = false;
                self.set_title_after_content_update(ctx);
                self.update_tab_bar_state(ctx);
                self.focus_contents(ctx);
                send_telemetry_from_ctx!(TelemetryEvent::PreviewPanePromoted, ctx);
                ctx.notify();
            }
        }
    }

    /// Construct an editor backed by the global shared buffer for the given location.
    ///
    /// For local files, additional features are wired up (selection-as-context,
    /// find-references, footer). Remote files skip these because LSP and
    /// related tooling run on the local machine.
    fn construct_editor_for_location(
        &mut self,
        location: LocalOrRemotePath,
        ctx: &mut ViewContext<Self>,
    ) -> ViewHandle<LocalCodeEditorView> {
        let is_local = matches!(location, LocalOrRemotePath::Local(_));
        ctx.add_typed_action_view(|ctx| {
            let mut editor = LocalCodeEditorView::new_with_global_buffer(
                location,
                |buffer_state, ctx| {
                    ctx.add_typed_action_view(|ctx| {
                        CodeEditorView::new(
                            None,
                            Some(buffer_state.buffer),
                            CodeEditorRenderOptions::new(VerticalExpansionBehavior::FillMaxHeight),
                            ctx,
                        )
                        .with_horizontal_scrollbar_appearance(
                            warpui::elements::new_scrollable::ScrollableAppearance::new(
                                warpui::elements::ScrollbarWidth::Auto,
                                true,
                            ),
                        )
                    })
                },
                false,
                None,
                ctx,
            );
            if is_local {
                if FeatureFlag::HoaCodeReview.is_enabled() {
                    editor = editor
                        .with_selection_as_context(Box::new(get_context_target_terminal_view));
                }
                let mut editor = editor.with_find_references_provider(
                    ShowFindReferencesCard {
                        editor_window_id: ctx.window_id(),
                        parent_scrollable_position_id: None,
                    },
                    ctx,
                );
                editor.add_footer(ctx);
                editor
            } else {
                editor
            }
        })
    }

    /// Construct an editor for a new (unsaved) file with no file backing.
    fn construct_new_file_editor(
        &mut self,
        ctx: &mut ViewContext<Self>,
    ) -> ViewHandle<LocalCodeEditorView> {
        let editor = ctx.add_typed_action_view(|ctx| {
            CodeEditorView::new(
                None,
                None,
                CodeEditorRenderOptions::new(VerticalExpansionBehavior::FillMaxHeight),
                ctx,
            )
            .with_horizontal_scrollbar_appearance(
                warpui::elements::new_scrollable::ScrollableAppearance::new(
                    warpui::elements::ScrollbarWidth::Auto,
                    true,
                ),
            )
        });

        ctx.add_typed_action_view(|ctx| {
            let mut local_editor = LocalCodeEditorView::new(editor, None, false, None, ctx);
            if FeatureFlag::HoaCodeReview.is_enabled() {
                local_editor = local_editor
                    .with_selection_as_context(Box::new(get_context_target_terminal_view));
            }
            local_editor.with_find_references_provider(
                ShowFindReferencesCard {
                    editor_window_id: ctx.window_id(),
                    parent_scrollable_position_id: None,
                },
                ctx,
            )
        })
    }

    fn build_tab_data(
        &mut self,
        location: Option<LocalOrRemotePath>,
        preview: bool,
        ctx: &mut ViewContext<Self>,
    ) -> TabData {
        let (code_editor, tab_location) = match location {
            Some(loc) => {
                let editor = self.construct_editor_for_location(loc.clone(), ctx);
                (editor, Some(loc))
            }
            None => (self.construct_new_file_editor(ctx), None),
        };

        let editor = code_editor.as_ref(ctx).editor().clone();

        ctx.subscribe_to_view(&editor, |me, _, event, ctx| match event {
            CodeEditorEvent::Focused => {
                me.promote_if_preview(ctx);
                ctx.emit(CodeViewEvent::Pane(PaneEvent::FocusSelf));
            }
            CodeEditorEvent::ContentChanged { .. } => {
                me.set_title_after_content_update(ctx);
            }
            _ => {}
        });

        // For new files (CodeSource::New), mark the editor as a new file and set default directory
        if tab_location.is_none() && matches!(self.source, CodeSource::New { .. }) {
            let default_directory = self.source.default_directory().cloned();
            code_editor.update(ctx, |local_editor, _ctx| {
                local_editor.set_new_file(true);
                local_editor.set_default_directory(default_directory);
            });
        }

        // Bundled skills cannot be edited.
        if self.source.is_bundled_skill() {
            editor.update(ctx, |editor, ctx| {
                editor.set_interaction_state(InteractionState::Selectable, ctx);
            });
        }
        ctx.subscribe_to_view(&code_editor, |me, _, event, ctx| match event {
            LocalCodeEditorEvent::FileLoaded => {
                me.pane_configuration.update(ctx, |pane_config, ctx| {
                    pane_config.refresh_pane_header_overflow_menu_items(ctx);
                });
                ctx.emit(CodeViewEvent::Pane(PaneEvent::AppStateChanged));
            }
            LocalCodeEditorEvent::FailedToLoad { error: err } => {
                // When code source is New, AIAction, or ProjectRules, it is possible that the
                // passed in file path might not exist currently if the intention is to create a
                // new file or if the project rules file doesn't exist yet.
                if let CodeSource::AIAction { .. }
                | CodeSource::New { .. }
                | CodeSource::ProjectRules { .. } = me.source
                {
                    return;
                }
                log::warn!("Failed to load file. {err:?}");
                CodeView::display_load_failure(ctx.window_id(), ctx);
            }
            LocalCodeEditorEvent::SelectionAddedAsContext {
                relative_file_path,
                line_range,
                selected_text,
            } => {
                me.insert_selection_as_context(
                    relative_file_path.clone(),
                    line_range.start.as_usize(),
                    line_range.end.as_usize(),
                    selected_text.clone(),
                    ctx,
                );
            }
            LocalCodeEditorEvent::FileSaved => {
                me.sync_active_tab_path(ctx);
                me.set_title_after_content_update(ctx);
                CodeView::display_save_success(ctx.window_id(), ctx);
                ctx.notify();
            }
            LocalCodeEditorEvent::FailedToSave { error: err } => {
                log::warn!("Failed to load file. {err:?}");
                CodeView::display_save_failure(ctx.window_id(), ctx);
            }
            LocalCodeEditorEvent::DiffAccepted => {
                CodeManager::handle(ctx).update(ctx, |code_manager, ctx| {
                    code_manager.complete_pending_diffs(me.source.clone(), ctx);
                });
            }
            LocalCodeEditorEvent::DiffRejected => {
                CodeManager::handle(ctx).update(ctx, |code_manager, ctx| {
                    code_manager.complete_pending_diffs(me.source.clone(), ctx);
                });
            }
            LocalCodeEditorEvent::DiffStatusUpdated => (),
            LocalCodeEditorEvent::UserEdited => (),
            LocalCodeEditorEvent::VimMinimizeRequested => (),
            LocalCodeEditorEvent::ViewportUpdated => (),
            LocalCodeEditorEvent::LayoutInvalidated => (),
            LocalCodeEditorEvent::DiscardUnsavedChanges { path } => {
                #[cfg(feature = "local_fs")]
                GlobalBufferModel::handle(ctx).update(ctx, |global_buffer, ctx| {
                    global_buffer.discard_unsaved_changes(path, ctx);
                });
            }
            LocalCodeEditorEvent::GotoDefinition {
                path,
                line,
                column,
                source_server_id,
            } => {
                // Register the external file so it can use LSP features.
                // The manager will skip registration if the path is under an existing workspace.
                let lsp_manager = LspManagerModel::handle(ctx);
                lsp_manager.update(ctx, |mgr, _| {
                    mgr.maybe_register_external_file(path, *source_server_id);
                });

                // LSP uses 0-based line numbers, convert to 1-based for LineAndColumnArg
                let line_1based = *line + 1;
                let line_col = LineAndColumnArg {
                    line_num: line_1based,
                    column_num: Some(*column),
                };

                me.open_or_focus_existing(
                    Some(LocalOrRemotePath::Local(path.to_path_buf())),
                    Some(line_col),
                    ctx,
                );
                if let Some(editor) = me.tab_at(me.active_tab_index()).map(|tab| &tab.editor_view) {
                    editor.update(ctx, |editor, ctx| {
                        editor.cursor_at(Point::new(line_1based as u32, *column as u32), ctx);
                    });
                }
                me.focus_contents(ctx);
            }
            LocalCodeEditorEvent::CommentSaved { .. }
            | LocalCodeEditorEvent::RequestOpenComment(_)
            | LocalCodeEditorEvent::DeleteComment { .. } => {
                // Comment events are handled by CodeReviewView, not CodeView
            }
            LocalCodeEditorEvent::RunTabConfigSkill { path } => {
                ctx.emit(CodeViewEvent::RunTabConfigSkill { path: path.clone() });
            }
            LocalCodeEditorEvent::OpenLspLogs { log_path } => {
                ctx.emit(CodeViewEvent::OpenLspLogs {
                    log_path: log_path.clone(),
                });
            }
            LocalCodeEditorEvent::DelayedRenderingFlushed => (),
        });

        TabData {
            location: tab_location,
            editor_view: code_editor,
            mouse_state_handles: Default::default(),
            preview,
        }
    }

    fn clear_drag_position(&mut self) {
        self.drag_position = None;
    }

    pub fn tab_at(&self, index: usize) -> Option<&TabData> {
        self.tab_group.get(index)
    }

    pub fn active_tab_index(&self) -> usize {
        self.active_tab_index
    }

    pub fn source(&self) -> &CodeSource {
        &self.source
    }

    /// Gets the selected text from the active tab's editor, if any.
    pub fn selected_text(&self, ctx: &AppContext) -> Option<String> {
        self.tab_at(self.active_tab_index).and_then(|tab| {
            let editor = tab.editor_view.as_ref(ctx).editor();
            editor.as_ref(ctx).selected_text(ctx)
        })
    }

    pub fn local_path(&self, ctx: &AppContext) -> Option<PathBuf> {
        self.tab_at(self.active_tab_index).and_then(|t| {
            t.editor_view.as_ref(ctx).file_id().and_then(|file_id| {
                GlobalBufferModel::as_ref(ctx)
                    .file_path(file_id)
                    .map(|p| p.to_path_buf())
            })
        })
    }

    pub fn pane_configuration(&self) -> ModelHandle<PaneConfiguration> {
        self.pane_configuration.clone()
    }

    pub fn focus(&self, ctx: &mut ViewContext<Self>) {
        if let Some(tab) = self.tab_at(self.active_tab_index) {
            ctx.focus(&tab.editor_view);
        }
    }

    fn preview_tab(&mut self) -> Option<(usize, &mut TabData)> {
        self.tab_group
            .iter_mut()
            .enumerate()
            .find(|(_, tab)| tab.preview)
    }

    /// Open a local file as a "preview" or if it's already being previewed, promote it to "open", making it
    /// active and editable.
    pub fn open_in_preview_or_promote(&mut self, path: PathBuf, ctx: &mut ViewContext<Self>) {
        // If the file already is open, set the active tab to the existing tab and return.
        if let Some(existing_index) = self
            .tab_group
            .iter()
            .position(|tab| tab.location.as_ref() == Some(&LocalOrRemotePath::Local(path.clone())))
        {
            self.set_active_tab_index(existing_index, ctx);
            self.promote_if_preview(ctx);
            return;
        }

        // Find the existing preview tab (if any) and replace it with a new GlobalBuffer-backed editor
        if let Some((preview_index, _)) = self.preview_tab() {
            let new_tab =
                self.build_tab_data(Some(LocalOrRemotePath::Local(path.clone())), true, ctx);
            self.tab_group[preview_index] = new_tab;

            GlobalBufferModel::handle(ctx).update(ctx, |model, ctx| {
                model.remove_deallocated_buffers(ctx);
            });

            self.set_active_tab_index(preview_index, ctx);
            return;
        }

        // Create a new preview tab
        let new_tab = self.build_tab_data(Some(LocalOrRemotePath::Local(path.clone())), true, ctx);

        self.tab_group.push(new_tab);
        let active_tab_index = self.tab_group.len() - 1;
        self.set_active_tab_index(active_tab_index, ctx);

        ctx.emit(CodeViewEvent::FileOpened {
            location: LocalOrRemotePath::Local(path),
            tab_index: self.active_tab_index,
        });
    }

    pub fn open_in_preview_or_promote_and_jump(
        &mut self,
        path: PathBuf,
        line_col: Option<LineAndColumnArg>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.open_in_preview_or_promote(path, ctx);
        if let Some(line_col) = line_col {
            self.jump_to_line_col_in_active_tab(line_col, ctx);
        }
    }

    pub fn open_or_focus_existing(
        &mut self,
        location: Option<LocalOrRemotePath>,
        line_col: Option<LineAndColumnArg>,
        ctx: &mut ViewContext<Self>,
    ) {
        // If the tab already exists, focus it (and optionally jump) without re-opening from disk.
        if let Some(existing_index) = self.focus_existing_tab_if_present(location.as_ref(), ctx) {
            if let Some(line_col) = line_col {
                self.jump_to_line_col_in_tab(existing_index, line_col, ctx);
            }
            return;
        }

        self.open_new_tab(location, line_col, ctx);
    }

    fn focus_existing_tab_if_present(
        &mut self,
        location: Option<&LocalOrRemotePath>,
        ctx: &mut ViewContext<Self>,
    ) -> Option<usize> {
        let location = location?;
        let existing_index = self
            .tab_group
            .iter()
            .position(|tab| tab.location.as_ref() == Some(location))?;
        self.set_active_tab_index(existing_index, ctx);
        Some(existing_index)
    }

    fn jump_to_line_col_in_active_tab(
        &self,
        line_col: LineAndColumnArg,
        ctx: &mut ViewContext<Self>,
    ) {
        self.jump_to_line_col_in_tab(self.active_tab_index, line_col, ctx);
    }

    fn jump_to_line_col_in_tab(
        &self,
        tab_index: usize,
        line_col: LineAndColumnArg,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(tab) = self.tab_group.get(tab_index) else {
            return;
        };

        let position = ScrollPosition::LineAndColumn(line_col);
        tab.editor_view.update(ctx, |editor, ctx| {
            editor.set_pending_scroll(position, ctx);
        });
    }

    fn open_new_tab(
        &mut self,
        location: Option<LocalOrRemotePath>,
        line_col: Option<LineAndColumnArg>,
        ctx: &mut ViewContext<Self>,
    ) {
        let new_tab = self.build_tab_data(location.clone(), false, ctx);
        self.tab_group.push(new_tab);
        let active_tab_index = self.tab_group.len() - 1;

        if let (Some(loc), Some(tab)) = (&location, self.tab_group.get(active_tab_index)) {
            ctx.emit(CodeViewEvent::FileOpened {
                location: loc.clone(),
                tab_index: active_tab_index,
            });

            let scroll_position = match line_col {
                Some(line_col) => ScrollPosition::LineAndColumn(line_col),
                // By default scroll to the first line.
                None => ScrollPosition::LineAndColumn(LineAndColumnArg {
                    line_num: 1,
                    column_num: None,
                }),
            };

            // For GlobalBuffer path, set_pending_scroll handles the case where the file
            // hasn't finished loading yet by deferring the scroll until FileLoaded.
            tab.editor_view.update(ctx, |editor, ctx| {
                editor.set_pending_scroll(scroll_position, ctx);
            });
        }

        self.set_active_tab_index(active_tab_index, ctx);
    }

    /// Set the title of the pane, which is the file path.
    fn set_title(&self, _unsaved_changes: bool, ctx: &mut ViewContext<Self>) {
        let file_location = self
            .tab_at(self.active_tab_index)
            .and_then(|t| t.editor_view.as_ref(ctx).file_location().cloned());
        let is_new = self
            .tab_at(self.active_tab_index)
            .is_some_and(|t| t.editor_view.as_ref(ctx).is_new_file());

        let title = match &file_location {
            Some(LocalOrRemotePath::Local(path)) => path.display().to_string(),
            Some(LocalOrRemotePath::Remote(remote_path)) => remote_path.path.as_str().to_string(),
            None => "Untitled".to_string(),
        };

        self.pane_configuration.update(ctx, |pane_config, ctx| {
            let mut secondary = String::new();
            if self.tab_group.len() > 1 {
                secondary.push_str(&format!(" (+{})", self.tab_group.len() - 1));
            } else if is_new {
                secondary.push_str(" (new)");
            }

            pane_config.set_title(title, ctx);
            pane_config.set_title_secondary(secondary, ctx);
            ctx.emit(PaneConfigurationEvent::TitleUpdated);
            ctx.emit(PaneConfigurationEvent::HeaderContentChanged);
        });
    }

    fn save_local(
        &mut self,
        index: usize,
        callback: Option<SaveCallback>,
        ctx: &mut ViewContext<Self>,
    ) -> SaveStatus {
        // This will only return an error immediately if there is a failure in the sync part of the call.
        // Other errors could be returned asynchronously via the FileModelEvent::FailedToSave event.
        let result = self
            .tab_at(index)
            .map(|tab| {
                tab.editor_view
                    .update(ctx, |code_diff, ctx| code_diff.save_local(ctx))
            })
            .unwrap_or_else(|| Err(ImmediateSaveError::NoActiveFileTab));

        // This will only return an error immediately if there is a failure in the sync part of the call.
        // Other errors could be returned asynchronously via the FileModelEvent::FailedToSave event.
        match result {
            Err(ImmediateSaveError::NoFileId) => {
                // If there's no file ID, this is a new file - trigger Save As
                self.save_as(index, callback, ctx)
            }
            Err(err) => {
                log::warn!("Failed to save file. {err:?}");
                CodeView::display_save_failure(ctx.window_id(), ctx);
                if let Some(callback) = callback {
                    callback(SaveOutcome::Failed, self, ctx);
                }
                SaveStatus::Failed(err)
            }
            Ok(()) => {
                if let Some(callback) = callback {
                    callback(SaveOutcome::Succeeded, self, ctx);
                }
                SaveStatus::SavedImmediately
            }
        }
    }

    fn save_as(
        &mut self,
        index: usize,
        callback: Option<SaveCallback>,
        ctx: &mut ViewContext<Self>,
    ) -> SaveStatus {
        if let Some(tab) = self.tab_at(index) {
            let view_handle = ctx.handle().clone();
            tab.editor_view.update(ctx, |editor, ctx| match callback {
                Some(cb) => {
                    editor.save_as(
                        Some(Box::new(move |outcome, ctx| {
                            if let Some(view) = view_handle.upgrade(ctx) {
                                view.update(ctx, |me, ctx| {
                                    cb(outcome, me, ctx);
                                });
                            }
                        })),
                        ctx,
                    );
                }
                None => {
                    editor.save_as(None, ctx);
                }
            });
            SaveStatus::AsyncSaveInProgress
        } else {
            SaveStatus::Failed(ImmediateSaveError::NoActiveFileTab)
        }
    }

    fn display_load_failure(window_id: WindowId, ctx: &mut ViewContext<Self>) {
        ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
            let toast = DismissibleToast::error(String::from("Failed to load file."))
                .with_object_id("failed_to_load_file".to_string());
            toast_stack.add_ephemeral_toast(toast, window_id, ctx);
        });
    }

    fn display_save_failure(window_id: WindowId, ctx: &mut ViewContext<Self>) {
        ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
            let toast = DismissibleToast::error(String::from("Failed to save file."))
                .with_object_id("failed_to_save_file".to_string());
            toast_stack.add_ephemeral_toast(toast, window_id, ctx);
        });
    }

    fn display_save_success(window_id: WindowId, ctx: &mut ViewContext<Self>) {
        ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
            let toast = DismissibleToast::success(String::from("File saved."))
                .with_object_id("file_saved".to_string());
            toast_stack.add_ephemeral_toast(toast, window_id, ctx);
        });
    }

    pub fn tab_count(&self) -> usize {
        self.tab_group.len()
    }

    // Check if there are unsaved changes in the buffer.
    // Implemented by comparing the ContentVersion in the FileModel with the ContentVersion in the Buffer.
    pub fn contains_unsaved_changes(&self, ctx: &AppContext) -> bool {
        self.tab_group
            .iter()
            .any(|tab| Self::has_unsaved_changes(tab, ctx))
    }

    // Returns the indices of tabs with unsaved changes as a vector.
    fn unsaved_indices(&self, ctx: &AppContext) -> Vec<usize> {
        self.tab_group
            .iter()
            .enumerate()
            .filter_map(|(index, tab)| Self::has_unsaved_changes(tab, ctx).then_some(index))
            .collect()
    }

    pub fn active_tab_has_unsaved_changes(&self, ctx: &AppContext) -> bool {
        let Some(tab) = self.tab_at(self.active_tab_index) else {
            return false;
        };
        Self::has_unsaved_changes(tab, ctx)
    }

    fn has_unsaved_changes(tab: &TabData, ctx: &AppContext) -> bool {
        let local_editor = tab.editor_view.as_ref(ctx);
        local_editor.has_unsaved_changes(ctx)
    }

    /// Check whether there are unsaved changes and reset the pane title accordingly.
    fn set_title_after_content_update(&self, ctx: &mut ViewContext<Self>) {
        self.set_title(self.contains_unsaved_changes(ctx), ctx);
    }

    /// Update the TabData path for the active tab to match the LocalCodeEditor metadata.
    /// This is needed after save_as operations to keep the paths in sync.
    fn sync_active_tab_path(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(tab) = self.tab_group.get_mut(self.active_tab_index) {
            let new_path = tab
                .editor_view
                .as_ref(ctx)
                .file_path()
                .map(|p| LocalOrRemotePath::Local(p.to_path_buf()));
            tab.location = new_path;
        }
    }

    pub fn cleanup_all_tabs(&mut self, ctx: &mut ViewContext<Self>) {
        self.tab_group.clear();
        GlobalBufferModel::handle(ctx).update(ctx, |model, ctx| {
            model.remove_deallocated_buffers(ctx);
        });
    }

    fn insert_selection_as_context(
        &mut self,
        file_path: String,
        start_line: usize,
        end_line: usize,
        selected_text: String,
        ctx: &mut ViewContext<Self>,
    ) {
        // If a CLI agent is active, send appropriate content to the PTY (or rich input if open).
        let window_id = ctx.window_id();
        if let Some(terminal_view) = get_context_target_terminal_view(window_id, ctx) {
            let prompt = if start_line == end_line {
                // Single-line: send the literal text with file/line context.
                build_selection_substring_prompt(&file_path, start_line, &selected_text)
            } else {
                // Multi-line: send a line-range reference with format note.
                build_selection_line_range_prompt(&file_path, start_line, end_line)
            };
            if let Some(routing) = terminal_view.update(ctx, |tv, ctx| {
                tv.try_send_text_to_cli_agent_or_rich_input(prompt, ctx)
            }) {
                let destination = match routing {
                    CliAgentRouting::RichInput => CodeContextDestination::RichInput,
                    CliAgentRouting::Pty => CodeContextDestination::Pty,
                };
                send_telemetry_from_ctx!(
                    TelemetryEvent::CodeSelectionAddedAsContext { destination },
                    ctx
                );
                return;
            }
        }

        // Otherwise insert the location snippet into the input buffer (original behavior).
        send_telemetry_from_ctx!(
            TelemetryEvent::CodeSelectionAddedAsContext {
                destination: CodeContextDestination::AgentInput,
            },
            ctx
        );
        ctx.dispatch_typed_action(&WorkspaceAction::InsertInInput {
            content: format!("{file_path}:{start_line}-{end_line} "),
            replace_buffer: false,
            ensure_agent_mode: true,
        });
    }

    fn render_request_edit_action_header(
        &self,
        tab: &TabData,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        ConstrainedBox::new(
            Align::new(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Min)
                    .with_child(
                        Container::new(
                            appearance
                                .ui_builder()
                                .button(
                                    ButtonVariant::Outlined,
                                    tab.mouse_state_handles.reject_mouse_state.clone(),
                                )
                                .with_text_label("Reject".to_string())
                                .build()
                                .on_click(|ctx, _, _| {
                                    ctx.dispatch_typed_action(CodeViewAction::RejectPendingDiffs)
                                })
                                .finish(),
                        )
                        .with_padding_right(16.)
                        .finish(),
                    )
                    .with_child(
                        Container::new(
                            appearance
                                .ui_builder()
                                .button(
                                    ButtonVariant::Outlined,
                                    tab.mouse_state_handles.accept_mouse_state.clone(),
                                )
                                .with_text_label("Accept and save".to_string())
                                .build()
                                .on_click(|ctx, _, _| {
                                    ctx.dispatch_typed_action(
                                        CodeViewAction::AcceptPendingDiffsAndSave,
                                    )
                                })
                                .finish(),
                        )
                        .with_padding_right(16.)
                        .finish(),
                    )
                    .finish(),
            )
            .right()
            .finish(),
        )
        .with_height(40.)
        .finish()
    }

    pub fn close_overlays(&mut self, ctx: &mut ViewContext<Self>) {
        for tab in self.tab_group.iter() {
            tab.editor_view.update(ctx, |editor, ctx| {
                editor.close_find_bar(false, ctx);
            })
        }
    }

    fn set_active_tab_index_after_remove(
        &mut self,
        remove_index: usize,
        ctx: &mut ViewContext<Self>,
    ) {
        if remove_index <= self.active_tab_index {
            self.set_active_tab_index(self.active_tab_index.saturating_sub(1), ctx);
        }
        self.update_tab_bar_state(ctx);
        ctx.notify();
    }

    fn remove_tab_with_confirmation(
        &mut self,
        index: usize,
        is_clearing_group: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(tab) = self.tab_at(index) {
            let file_name = tab
                .location
                .as_ref()
                .map(|loc| loc.display_name().to_string())
                .filter(|n| !n.is_empty());
            let summary = UnsavedStateSummary::for_editor_tab(
                file_name,
                vec![CodeEditorStatus::new(Self::has_unsaved_changes(tab, ctx))],
                ctx,
            );

            // If the tab being removed has unsaved changes, we attempt to show a modal before closing it.
            if summary.should_display_warning(ctx)
                && ChannelState::channel() != Channel::Integration
            {
                let handle_save_intent = |intent: PendingSaveIntent| {
                    let handle = ctx.handle().clone();
                    move |ctx: &mut AppContext| {
                        if let Some(view) = handle.upgrade(ctx) {
                            view.update(ctx, |view, ctx| {
                                if is_clearing_group {
                                    let unsaved_indices = view.unsaved_indices(ctx);
                                    view.clear_tab_group_with_intent(
                                        unsaved_indices,
                                        0,
                                        Some(intent),
                                        ctx,
                                    );
                                } else {
                                    view.remove_tab_with_intent(index, Some(intent), ctx);
                                }
                            });
                        }
                    }
                };
                summary
                    .dialog()
                    .on_save_changes(handle_save_intent(PendingSaveIntent::Save))
                    .on_discard_changes(handle_save_intent(PendingSaveIntent::Discard))
                    .on_cancel(handle_save_intent(PendingSaveIntent::Cancel))
                    .show(ctx);
            } else {
                self.remove_tab_data_index(index, ctx);
            }
        }
    }

    fn remove_tab_data_index(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        self.tab_group.remove(index);
        GlobalBufferModel::handle(ctx).update(ctx, |model, ctx| {
            model.remove_deallocated_buffers(ctx);
        });
        self.set_active_tab_index_after_remove(index, ctx);
    }

    pub fn remove_tab_for_move(
        &mut self,
        index: usize,
        ctx: &mut ViewContext<Self>,
    ) -> Option<CodePane> {
        self.tab_at(index).and_then(|t| t.local_path()).map(|path| {
            let source = CodeSource::Link {
                path,
                range_start: None,
                range_end: None,
            };
            self.remove_tab_data_index(index, ctx);
            CodePane::new(source, None, ctx)
        })
    }

    fn remove_tab_with_intent(
        &mut self,
        index: usize,
        intent: Option<PendingSaveIntent>,
        ctx: &mut ViewContext<Self>,
    ) {
        match intent {
            Some(PendingSaveIntent::Save) => {
                self.save_local(
                    index,
                    Some(Box::new(move |outcome, me, ctx| {
                        if outcome != SaveOutcome::Canceled {
                            me.remove_tab_data_index(index, ctx);
                        }
                    })),
                    ctx,
                );
            }
            Some(PendingSaveIntent::Discard) => {
                self.remove_tab_data_index(index, ctx);
            }
            _ => (),
        }
    }

    fn clear_tab_group_with_intent(
        &mut self,
        unsaved_indices: Vec<usize>,
        current_index: usize,
        intent: Option<PendingSaveIntent>,
        ctx: &mut ViewContext<Self>,
    ) {
        match intent {
            Some(PendingSaveIntent::Save) => {
                self.save_local(
                    unsaved_indices[current_index],
                    Some(Box::new(move |outcome, me, ctx| {
                        if outcome != SaveOutcome::Canceled {
                            me.process_next_tab_for_clear(unsaved_indices, current_index + 1, ctx);
                        }
                    })),
                    ctx,
                );
            }
            Some(PendingSaveIntent::Discard) => {
                self.process_next_tab_for_clear(unsaved_indices, current_index + 1, ctx);
            }
            _ => (),
        }
    }

    fn process_next_tab_for_clear(
        &mut self,
        unsaved_indices: Vec<usize>,
        current_index: usize,
        ctx: &mut ViewContext<Self>,
    ) {
        // If we've processed all tabs with unsaved changes, we can clear the tab group immediately.
        if current_index >= unsaved_indices.len() {
            self.cleanup_all_tabs(ctx);
            self.set_active_tab_index(0, ctx);
            return;
        }

        // Otherwise, we need to prompt the user for a decision on the current tab with unsaved changes.
        let tab_index = unsaved_indices[current_index];
        self.remove_tab_with_confirmation(tab_index, true, ctx);
    }

    fn close_saved_tabs(&mut self, ctx: &mut ViewContext<Self>) {
        self.tab_group
            .retain(|tab| Self::has_unsaved_changes(tab, ctx));
        GlobalBufferModel::handle(ctx).update(ctx, |model, ctx| {
            model.remove_deallocated_buffers(ctx);
        });
        self.set_active_tab_index(0, ctx);
    }

    pub fn set_active_tab_index(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        self.active_tab_index = index;
        self.update_tab_bar_state(ctx);

        let location = self.tab_at(index).and_then(|tab| tab.location.clone());
        ctx.emit(CodeViewEvent::TabChanged {
            location,
            tab_index: index,
        });

        #[cfg(feature = "local_fs")]
        {
            self.update_markdown_mode_segmented_control(ctx);
        }

        ctx.notify();
    }

    /// Close all tabs with the specified file path.
    /// Used when a file is renamed to prevent saving to the old path.
    pub fn close_tabs_with_path(&mut self, file_path: &Path, ctx: &mut ViewContext<Self>) {
        let mut indices_to_remove = Vec::new();
        for (tab_idx, tab) in self.tab_group.iter().enumerate() {
            if tab.local_path().is_some_and(|path| path == file_path) {
                indices_to_remove.push(tab_idx);
            }
        }
        // Remove tabs in reverse order to maintain indices
        for &tab_idx in indices_to_remove.iter().rev() {
            self.remove_tab_data_index(tab_idx, ctx);
        }
    }

    /// Update any tabs opened to `old_path` so they now point to `new_path`,
    /// preserving any unsaved edits.
    pub fn rename_tabs_with_path(
        &mut self,
        old_path: &Path,
        new_path: &Path,
        ctx: &mut ViewContext<Self>,
    ) {
        for tab in self.tab_group.iter_mut() {
            if tab.local_path().is_some_and(|path| path == old_path) {
                tab.location = Some(LocalOrRemotePath::Local(new_path.to_path_buf()));
                tab.editor_view.update(ctx, |editor, ctx| {
                    let was_unsaved = editor.has_unsaved_changes(ctx);

                    // Remap the buffer from old_path to new_path via GlobalBufferModel,
                    // preserving buffer content and unsaved edits.
                    if let Some(old_file_id) = editor.file_id() {
                        let buffer_state = GlobalBufferModel::handle(ctx).update(
                            ctx,
                            |model, ctx| {
                                model.rename(old_file_id, new_path.to_path_buf(), ctx)
                            },
                        );
                        if let Some(buffer_state) = buffer_state {
                            editor.apply_rename(buffer_state, new_path, ctx);
                        }
                    }

                    if was_unsaved {
                        let summary = UnsavedStateSummary::for_editor_tab(
                            Some(new_path.file_name().unwrap().to_string_lossy().to_string()),
                            vec![CodeEditorStatus::new(true)], /* editor_status(unsaved_changes=true) */
                            ctx,
                        );

                        let on_save = {
                            let handle = ctx.handle().clone();
                            move |ctx: &mut AppContext| {
                                if let Some(view) = handle.upgrade(ctx) {
                                    view.update(ctx, |editor, ctx| {
                                        let _ = editor.save_local(ctx);
                                    });
                                }
                            }
                        };

                        summary
                            .dialog()
                            .on_save_changes(on_save)
                            .on_discard_changes(|_| {})
                            .show(ctx);
                    }
                });
            }
        }

        self.update_tab_bar_state(ctx);
        self.set_title_after_content_update(ctx);
        ctx.notify();
    }

    fn update_tab_bar_state(&mut self, ctx: &mut ViewContext<Self>) {
        if self.tab_group.is_empty() {
            ctx.emit(CodeViewEvent::Pane(PaneEvent::Close));
        } else {
            self.set_title_after_content_update(ctx);
            ctx.notify();
        }
    }

    fn relative_path(path: PathBuf, window_id: WindowId, app: &AppContext) -> String {
        let maybe_relative_path = ActiveSession::as_ref(app)
            .path_if_local(window_id)
            .and_then(|cwd| {
                path.strip_prefix(cwd)
                    .ok()
                    .map(|p| p.to_string_lossy().to_string())
            });

        maybe_relative_path.unwrap_or(path.to_string_lossy().to_string())
    }

    fn render_close_button(
        appearance: &Appearance,
        close_handle: MouseStateHandle,
        index: usize,
    ) -> Box<dyn Element> {
        icon_button(
            appearance,
            crate::ui_components::icons::Icon::X,
            false,
            close_handle,
        )
        .build()
        .on_click(move |ctx, _app, _pos| {
            ctx.dispatch_typed_action(
                PaneHeaderAction::<CodeViewAction, CodeViewAction>::CustomAction(
                    CodeViewAction::RemoveTabAtIndex { index },
                ),
            );
        })
        .finish()
    }

    fn calculate_tab_bar_dragged_position(
        drag_position: &RectF,
        index: usize,
        ctx: &ViewContext<Self>,
    ) -> TabBarDragPosition {
        if let Some(tab_rect) = ctx.element_position_by_id(tab_position_id(index)) {
            let tab_center_x = tab_rect.center().x();
            if drag_position.center().x() < tab_center_x {
                TabBarDragPosition::BeforeTab { index }
            } else {
                TabBarDragPosition::AfterTab { index }
            }
        } else {
            // If for some reason we can't retrieve the drag position, assume that the drag occurred before the tab.
            TabBarDragPosition::BeforeTab { index }
        }
    }

    fn render_tab_drag_element(file_name: String, appearance: &Appearance) -> Box<dyn Element> {
        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween);

        let language_icon =
            icon_from_file_path(&file_name, appearance, ItemHighlightState::Default);
        row.add_child(
            Container::new(
                ConstrainedBox::new(language_icon)
                    .with_width(LANGUAGE_ICON_WIDTH)
                    .with_height(LANGUAGE_ICON_WIDTH)
                    .finish(),
            )
            .with_margin_right(TAB_INTERNAL_MARGIN)
            .finish(),
        );

        let file_name_text = Text::new_inline(
            file_name,
            appearance.ui_font_family(),
            appearance.ui_font_size(),
        )
        .finish();
        row.add_child(Shrinkable::new(1., Container::new(file_name_text).finish()).finish());

        Container::new(
            ConstrainedBox::new(row.finish())
                .with_height(34.)
                .with_max_width(200.)
                .finish(),
        )
        .with_vertical_padding(TAB_PADDING)
        .with_horizontal_padding(TAB_PADDING + TAB_HORIZONTAL_MARGIN)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
        .with_background_color(appearance.theme().surface_1().with_opacity(70).into())
        .finish()
    }

    fn render_tab_internal(
        tab_data: &TabData,
        index: usize,
        is_active: bool,
        is_hovered: bool,
        has_unsaved_changes: bool,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let text_color = if is_active {
            blended_colors::text_main(theme, theme.surface_1())
        } else {
            blended_colors::text_sub(theme, theme.surface_1())
        };

        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween);

        let file_name = tab_data
            .location
            .as_ref()
            .map(|loc| loc.display_name().to_string())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| "Untitled".to_string());
        let language_icon =
            icon_from_file_path(&file_name, appearance, ItemHighlightState::Default);
        row.add_child(
            Container::new(
                ConstrainedBox::new(language_icon)
                    .with_width(LANGUAGE_ICON_WIDTH)
                    .with_height(LANGUAGE_ICON_WIDTH)
                    .finish(),
            )
            .with_margin_right(TAB_INTERNAL_MARGIN)
            .finish(),
        );

        if has_unsaved_changes {
            row.add_child(
                Container::new(render_unsaved_changes_icon(text_color))
                    .with_margin_right(TAB_INTERNAL_MARGIN)
                    .finish(),
            )
        }

        let style = if tab_data.preview {
            Properties::default().style(Style::Italic)
        } else {
            Properties::default().weight(Weight::Semibold)
        };
        let file_name_text = Text::new_inline(
            file_name.clone(),
            appearance.ui_font_family(),
            appearance.ui_font_size(),
        )
        .with_color(text_color)
        .with_style(style)
        .finish();
        row.add_child(
            Shrinkable::new(
                1.,
                Container::new(file_name_text)
                    .with_margin_right(TAB_INTERNAL_MARGIN)
                    .finish(),
            )
            .finish(),
        );

        let show_close = is_active || is_hovered;
        row.add_child(
            Shrinkable::new(
                1.,
                if show_close {
                    Self::render_close_button(
                        appearance,
                        tab_data.mouse_state_handles.close_handle.clone(),
                        index,
                    )
                } else {
                    Container::new(
                        ConstrainedBox::new(Empty::new().finish())
                            .with_width(CLOSE_BUTTON_WIDTH)
                            .with_height(CLOSE_BUTTON_WIDTH)
                            .finish(),
                    )
                    .finish()
                },
            )
            .finish(),
        );

        let draggable = Draggable::new(
            tab_data.mouse_state_handles.tab_draggable_state.clone(),
            row.finish(),
        )
        .with_drag_bounds_callback(|_, window_size| Some(RectF::new(Vector2F::zero(), window_size)))
        .with_accepted_by_drop_target_fn(move |_, _| AcceptedByDropTarget::Yes)
        .with_keep_original_visible(true)
        .on_drag(move |ctx, _, drag_position, data| {
            if let Some(tab_group_index) =
                data.and_then(|data| data.as_any().downcast_ref::<EditorTabBarDropTargetData>())
            {
                // If an editor tab is dragged over the editor tab bar, we should clear all drag indicators on the workspace tab group.
                ctx.dispatch_typed_action(
                    PaneHeaderAction::<CodeViewAction, CodeViewAction>::CustomAction(
                        CodeViewAction::ClearWorkspaceTabGroupDragPositions,
                    ),
                );

                ctx.dispatch_typed_action(
                    PaneHeaderAction::<CodeViewAction, CodeViewAction>::CustomAction(
                        CodeViewAction::DragOverIndex {
                            target: tab_group_index.index,
                            drag_position,
                        },
                    ),
                );
            } else if let Some(data) =
                data.and_then(|data| data.as_any().downcast_ref::<TabBarDropTargetData>())
            {
                // If an editor tab is dragged over the workspace tab bar, we should clear all drag indicators on the editor tab group.
                ctx.dispatch_typed_action(
                    PaneHeaderAction::<CodeViewAction, CodeViewAction>::CustomAction(
                        CodeViewAction::ClearEditorTabGroupDragPositions,
                    ),
                );

                ctx.dispatch_typed_action(
                    PaneHeaderAction::<CodeViewAction, CodeViewAction>::PaneHeaderDragged {
                        origin: ActionOrigin::EditorTab(index),
                        drag_location: PaneDragDropLocation::TabBar(data.tab_bar_location),
                        drag_position,
                        precomputed_tab_hover_index: None,
                    },
                );
            } else {
                // If an editor tab is dragged anywhere else, we should clear all drag indicators on the editor and workspace tab groups.
                ctx.dispatch_typed_action(
                    PaneHeaderAction::<CodeViewAction, CodeViewAction>::CustomAction(
                        CodeViewAction::ClearWorkspaceTabGroupDragPositions,
                    ),
                );
                ctx.dispatch_typed_action(
                    PaneHeaderAction::<CodeViewAction, CodeViewAction>::CustomAction(
                        CodeViewAction::ClearEditorTabGroupDragPositions,
                    ),
                );
            }
        })
        .on_drop(move |ctx, _, drag_position, data| {
            if let Some(tab_group_index) =
                data.and_then(|data| data.as_any().downcast_ref::<EditorTabBarDropTargetData>())
            {
                ctx.dispatch_typed_action(
                    PaneHeaderAction::<CodeViewAction, CodeViewAction>::CustomAction(
                        CodeViewAction::DropAtIndex {
                            origin: index,
                            target: tab_group_index.index,
                            drag_position,
                        },
                    ),
                );
            } else if let Some(data) =
                data.and_then(|data| data.as_any().downcast_ref::<TabBarDropTargetData>())
            {
                ctx.dispatch_typed_action(
                    PaneHeaderAction::<CodeViewAction, CodeViewAction>::PaneHeaderDropped {
                        origin: ActionOrigin::EditorTab(index),
                        drop_location: PaneDragDropLocation::TabBar(data.tab_bar_location),
                    },
                );
            }
        })
        .with_alternate_drag_element(Self::render_tab_drag_element(file_name, appearance));

        SavePosition::new(
            if !tab_data
                .mouse_state_handles
                .tab_draggable_state
                .is_dragging()
            {
                DropTarget::new(draggable.finish(), EditorTabBarDropTargetData { index }).finish()
            } else {
                draggable.finish()
            },
            &tab_position_id(index),
        )
        .finish()
    }

    /// Renders the tab bar with explicit draggable handling for multi-tab case.
    fn render_tab_bar_with_draggable(
        &self,
        header_ctx: &view::HeaderRenderContext<'_>,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let is_pane_dragging = header_ctx.draggable_state.is_dragging();

        let mut header_row = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::Start)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min);

        if header_ctx.header_left_inset > 0. {
            header_row.add_child(
                Container::new(Empty::new().finish())
                    .with_padding_left(header_ctx.header_left_inset)
                    .finish(),
            );
        }

        let mut tabs_row = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::Start)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(if is_pane_dragging {
                MainAxisSize::Min
            } else {
                MainAxisSize::Max
            });

        for (index, tab_data) in self.tab_group.iter().enumerate() {
            let is_active = index == self.active_tab_index;
            let close_handle = tab_data.mouse_state_handles.close_handle.clone();
            let tab = Hoverable::new(
                tab_data.mouse_state_handles.tab_handle.clone(),
                |tab_handle| {
                    let mut stack = Stack::new();
                    let container = Container::new(
                        Container::new(Self::render_tab_internal(
                            tab_data,
                            index,
                            is_active,
                            tab_handle.is_hovered(),
                            Self::has_unsaved_changes(tab_data, app),
                            appearance,
                        ))
                        .with_horizontal_margin(TAB_HORIZONTAL_MARGIN)
                        .with_padding(Padding::uniform(TAB_PADDING))
                        .finish(),
                    )
                    .with_border(
                        Border::new(TAB_BAR_BORDER_HEIGHT)
                            .with_border_fill(theme.outline())
                            .with_sides(false, false, !is_active, true),
                    );

                    // Renders a border to the left/right of a tab being dragged over to display the intended drop position.
                    let border = match &self.drag_position {
                        Some(TabBarDragPosition::BeforeTab { index: drag_index })
                            if *drag_index == index =>
                        {
                            Some(Border::left(2.).with_border_fill(theme.foreground()))
                        }
                        Some(TabBarDragPosition::AfterTab { index: drag_index })
                            if *drag_index == index =>
                        {
                            Some(Border::right(2.).with_border_fill(theme.foreground()))
                        }
                        _ => None,
                    };

                    if let Some(border) = border {
                        stack.add_child(
                            Container::new(container.finish())
                                .with_border(border)
                                .finish(),
                        );
                    } else {
                        stack.add_child(container.finish());
                    }

                    if tab_handle.is_hovered()
                        && !tab_data
                            .mouse_state_handles
                            .tab_draggable_state
                            .is_dragging()
                    {
                        if let Some(path) = tab_data.local_path() {
                            let tooltip = appearance
                                .ui_builder()
                                .tool_tip(Self::relative_path(path, self.window_id, app))
                                .build()
                                .finish();
                            stack.add_positioned_overlay_child(
                                tooltip,
                                OffsetPositioning::offset_from_parent(
                                    vec2f(10., -1.),
                                    ParentOffsetBounds::Unbounded,
                                    ParentAnchor::BottomLeft,
                                    ChildAnchor::TopLeft,
                                ),
                            );
                        }
                    }

                    stack.finish()
                },
            )
            .on_click(move |ctx, _app, _pos| {
                let is_close_button_hovered = close_handle
                    .lock()
                    .map(|handle| handle.is_hovered())
                    .unwrap_or(false);

                if !is_close_button_hovered {
                    ctx.dispatch_typed_action(
                        PaneHeaderAction::<CodeViewAction, CodeViewAction>::CustomAction(
                            CodeViewAction::SetCurrentTabIndex { index },
                        ),
                    );
                }
            })
            .on_middle_click(move |ctx, _app, _pos| {
                ctx.dispatch_typed_action(
                    PaneHeaderAction::<CodeViewAction, CodeViewAction>::CustomAction(
                        CodeViewAction::RemoveTabAtIndex { index },
                    ),
                );
            });

            let tab_element = tab.finish();
            if is_pane_dragging {
                // ConstrainedBox gives the tab finite horizontal constraints so
                // its internal Shrinkable children don't hit the infinite-flex
                // assertion.
                tabs_row.add_child(
                    ConstrainedBox::new(tab_element)
                        .with_max_width(300.)
                        .finish(),
                );
            } else {
                tabs_row.add_child(Shrinkable::new(1., tab_element).finish());
            }
        }

        // Draggable spacer fills the gap between tabs and buttons and supports header dragging.
        let spacer = Container::new(Empty::new().finish())
            .with_border(Border::bottom(TAB_BAR_BORDER_HEIGHT).with_border_fill(theme.outline()))
            .finish();
        let draggable_spacer = render_pane_header_draggable::<CodeView>(
            self.pane_configuration.clone(),
            spacer,
            header_ctx.draggable_state.clone(),
            app,
        );
        tabs_row.add_child(if is_pane_dragging {
            draggable_spacer
        } else {
            Expanded::new(1., draggable_spacer).finish()
        });

        // Clip tabs so overflow doesn't push the buttons off-screen.
        let clipped_tabs = Clipped::new(tabs_row.finish()).finish();
        header_row.add_child(if is_pane_dragging {
            clipped_tabs
        } else {
            Expanded::new(1., clipped_tabs).finish()
        });

        let show_close_button = self
            .focus_handle
            .as_ref()
            .is_some_and(|h| h.is_in_split_pane(app));

        let buttons = render_pane_header_buttons::<CodeViewAction, CodeViewAction>(
            header_ctx,
            appearance,
            show_close_button,
            None,
            None,
        );

        header_row.add_child(
            Container::new(Align::new(buttons).finish())
                .with_padding_right(4.)
                .with_border(
                    Border::bottom(TAB_BAR_BORDER_HEIGHT).with_border_fill(theme.outline()),
                )
                .finish(),
        );

        header_row.finish()
    }

    /// Renders the header for the single-tab (or empty) case with a centered title.
    fn render_single_tab_header(
        &self,
        header_ctx: &view::HeaderRenderContext<'_>,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let title = self
            .tab_group
            .first()
            .and_then(|tab| {
                tab.location
                    .as_ref()
                    .map(|loc| loc.display_name().to_string())
                    .filter(|n| !n.is_empty())
            })
            .unwrap_or_else(|| "Untitled".to_string());

        let appearance = Appearance::as_ref(app);
        let is_pane_dragging = header_ctx.draggable_state.is_dragging();
        let mut right_row = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::End)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Min);

        if let Some(segmented) = &self.markdown_mode_segmented_control {
            right_row.add_child(ChildView::new(segmented).finish());
        }

        let show_close_button = self
            .focus_handle
            .as_ref()
            .is_some_and(|h| h.is_in_split_pane(app));

        right_row.add_child(
            render_pane_header_buttons::<CodeViewAction, CodeViewAction>(
                header_ctx,
                appearance,
                show_close_button,
                None,
                None,
            ),
        );

        let button_count = show_close_button as u32 + header_ctx.has_overflow_items as u32;
        let buttons_width = button_count as f32 * ICON_DIMENSIONS;
        let edge_width = if self.markdown_mode_segmented_control.is_some() {
            220.0
        } else {
            view::StandardHeaderOptions::DEFAULT_CONTROL_CONTAINER_WIDTH
        };

        // Get tooltip path and handle from the first tab (if any).
        let tab = self.tab_group.first();
        let tab_handle = tab.map(|tab| tab.mouse_state_handles.tab_handle.clone());

        // Check unsaved changes for the active tab.
        let has_unsaved = tab.is_some_and(|tab| Self::has_unsaved_changes(tab, app));

        // Build the center title element, with a hover tooltip showing the full path.
        let title_element: Box<dyn Element> = match tab_handle {
            Some(handle) => Hoverable::new(handle, |hover_state| {
                let title_text =
                    render_pane_header_title_text(title.clone(), appearance, ClipConfig::start());

                let mut title_row = Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_main_axis_size(MainAxisSize::Min);
                if has_unsaved {
                    let dot_color = appearance
                        .theme()
                        .sub_text_color(appearance.theme().background());
                    title_row.add_child(
                        Container::new(render_unsaved_changes_icon(dot_color.into()))
                            .with_margin_right(4.)
                            .finish(),
                    );
                }
                title_row.add_child(title_text);

                let mut stack = Stack::new();
                stack.add_child(title_row.finish());
                if hover_state.is_hovered() {
                    let tooltip_relative_path = tab
                        .and_then(|tab| tab.local_path())
                        .map(|p| Self::relative_path(p, self.window_id, app));
                    if let Some(ref path) = tooltip_relative_path {
                        let tooltip = appearance
                            .ui_builder()
                            .tool_tip(path.clone())
                            .build()
                            .finish();
                        stack.add_positioned_overlay_child(
                            tooltip,
                            OffsetPositioning::offset_from_parent(
                                vec2f(0., 4.),
                                ParentOffsetBounds::Unbounded,
                                ParentAnchor::BottomMiddle,
                                ChildAnchor::TopMiddle,
                            ),
                        );
                    }
                }
                stack.finish()
            })
            .finish(),
            None => {
                let title_text =
                    render_pane_header_title_text(title, appearance, ClipConfig::start());
                if has_unsaved {
                    let dot_color = appearance
                        .theme()
                        .sub_text_color(appearance.theme().background());
                    let mut row = Flex::row()
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_main_axis_size(MainAxisSize::Min);
                    row.add_child(
                        Container::new(render_unsaved_changes_icon(dot_color.into()))
                            .with_margin_right(4.)
                            .finish(),
                    );
                    row.add_child(title_text);
                    row.finish()
                } else {
                    title_text
                }
            }
        };

        render_three_column_header(
            Empty::new().finish(),
            title_element,
            right_row.finish(),
            CenteredHeaderEdgeWidth {
                min: buttons_width,
                max: edge_width,
            },
            header_ctx.header_left_inset,
            is_pane_dragging,
        )
    }

    fn overflow_menu_items(&self, ctx: &AppContext) -> Vec<MenuItem<CodeViewAction>> {
        let is_maximized = self
            .focus_handle
            .as_ref()
            .is_some_and(|h| h.is_maximized(ctx));
        let modifier_keys = if cfg!(target_os = "macos") {
            "⌘R"
        } else {
            "Ctrl-R"
        };

        let mut items = vec![
            MenuItemFields::new_with_label("Close saved", &format!("{modifier_keys} U"))
                .with_on_select_action(CodeViewAction::CloseSaved)
                .into_item(),
            MenuItemFields::toggle_pane_action(is_maximized)
                .with_on_select_action(CodeViewAction::ToggleMaximized)
                .into_item(),
        ];

        #[cfg(feature = "local_fs")]
        if let Some(path) = self.local_path(ctx) {
            let reveal_label = if cfg!(target_os = "macos") {
                "Reveal in Finder"
            } else if cfg!(target_os = "windows") {
                "Reveal in Explorer"
            } else {
                "Reveal in file manager"
            };
            items.extend([
                MenuItem::Separator,
                MenuItemFields::new("Copy file path")
                    .with_on_select_action(CodeViewAction::CopyFilePath)
                    .into_item(),
                MenuItemFields::new(reveal_label)
                    .with_on_select_action(CodeViewAction::RevealInFinder)
                    .into_item(),
            ]);

            if is_markdown_file(&path) {
                items.push(
                    MenuItemFields::new("View Markdown preview")
                        .with_on_select_action(CodeViewAction::RenderMarkdown)
                        .into_item(),
                );
            }
        }

        items
    }

    /// Merges tabs from another `CodeView`, avoiding duplicates and updating the active tab index.
    pub fn merge_tabs(&mut self, source_code_view: &CodeView, ctx: &mut ViewContext<Self>) {
        let existing_locations_to_idx: HashMap<&LocalOrRemotePath, usize> = self
            .tab_group
            .iter()
            .enumerate()
            .filter_map(|(idx, tab)| tab.location.as_ref().map(|loc| (loc, idx)))
            .collect();
        let mut active_tab_index = self.active_tab_index();
        let mut to_extend: Vec<TabData> = Vec::new();

        for (i, tab_data) in source_code_view.tab_group.iter().enumerate() {
            if let Some(loc) = tab_data.location.as_ref() {
                if let Some(&index) = existing_locations_to_idx.get(loc) {
                    // If the tab already exists in the tab group and is the active tab in the source CodeView,
                    // update the active tab index to point to it.
                    if i == source_code_view.active_tab_index() {
                        active_tab_index = index;
                    }
                } else {
                    // Unset preview on merged tabs
                    let mut new_data = tab_data.clone();
                    new_data.preview = false;

                    to_extend.push(new_data);
                    // If the newly added tab is the active tab in the source CodeView, update the active tab index to point to it.
                    if i == source_code_view.active_tab_index() {
                        active_tab_index = self.tab_group.len() + to_extend.len() - 1;
                    }
                }
            }
        }

        self.tab_group.extend(to_extend);
        self.set_active_tab_index(active_tab_index, ctx);
    }
}

impl Entity for CodeView {
    type Event = CodeViewEvent;
}

impl View for CodeView {
    fn ui_name() -> &'static str {
        "CodeView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let tab = self.tab_at(self.active_tab_index);
        let body = if let Some(tab) = tab {
            match self.source {
                CodeSource::AIAction { .. } => Flex::column()
                    .with_child(self.render_request_edit_action_header(tab, app))
                    .with_child(
                        Shrinkable::new(1., ChildView::new(&tab.editor_view).finish()).finish(),
                    )
                    .finish(),
                _ => ChildView::new(&tab.editor_view).finish(),
            }
        } else {
            Empty::new().finish()
        };

        Container::new(body).with_padding_top(PADDING).finish()
    }
}

impl TypedActionView for CodeView {
    type Action = CodeViewAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            CodeViewAction::SaveFile => {
                self.save_local(self.active_tab_index, None, ctx);
            }
            CodeViewAction::SaveFileAs => {
                self.save_as(self.active_tab_index, None, ctx);
            }
            CodeViewAction::AcceptPendingDiffsAndSave => {
                if !matches!(self.source, CodeSource::AIAction { .. }) {
                    log::warn!("Received Accept and save in code without the AIAction source");
                    return;
                }

                // Accepts the diff and marks it complete.
                if let Some(tab) = self.tab_at(self.active_tab_index) {
                    tab.editor_view.update(ctx, |code_diff, ctx| {
                        code_diff.accept_diff(ctx);
                    });
                }

                self.save_local(
                    self.active_tab_index,
                    Some(Box::new(|outcome, me, ctx| {
                        if outcome != SaveOutcome::Canceled {
                            me.close(ctx);
                        }
                    })),
                    ctx,
                );
            }
            CodeViewAction::RejectPendingDiffs => {
                if !matches!(self.source, CodeSource::AIAction { .. }) {
                    log::warn!("Received Reject in code without the AIAction source");
                    return;
                }

                if let Some(tab) = self.tab_at(self.active_tab_index) {
                    tab.editor_view.update(ctx, |code_diff, ctx| {
                        code_diff.reject_diff(ctx);
                    });
                }

                self.close(ctx);
            }
            CodeViewAction::SetCurrentTabIndex { index } => {
                self.set_active_tab_index(*index, ctx);
            }

            CodeViewAction::RemoveTabAtIndex { index } => {
                self.remove_tab_with_confirmation(*index, false, ctx);
            }

            CodeViewAction::CloseAll => {
                let unsaved_indices = self.unsaved_indices(ctx);
                self.process_next_tab_for_clear(unsaved_indices, 0, ctx);
            }

            CodeViewAction::CloseSaved => {
                self.close_saved_tabs(ctx);
            }

            CodeViewAction::ToggleMaximized => {
                ctx.emit(CodeViewEvent::Pane(PaneEvent::ToggleMaximized));
                self.pane_configuration.update(ctx, |pane_config, ctx| {
                    pane_config.refresh_pane_header_overflow_menu_items(ctx);
                });
            }

            #[cfg(feature = "local_fs")]
            CodeViewAction::CopyFilePath => {
                if let Some(path) = self.local_path(ctx) {
                    ctx.clipboard()
                        .write(ClipboardContent::plain_text(path.display().to_string()));
                }
            }
            #[cfg(feature = "local_fs")]
            CodeViewAction::RevealInFinder => {
                if let Some(path) = self.local_path(ctx) {
                    ctx.open_file_path_in_explorer(&path);
                } else {
                    log::warn!(
                        "Reveal in Finder requested, but the active code tab has no local file path"
                    );
                }
            }
            #[cfg(feature = "local_fs")]
            CodeViewAction::RenderMarkdown => {
                let path = self.local_path(ctx).or_else(|| {
                    self.tab_at(self.active_tab_index)
                        .and_then(|t| t.local_path())
                });

                if let Some(path) = path {
                    let source = self.source.clone();
                    if self.active_tab_has_unsaved_changes(ctx) {
                        self.save_local(
                            self.active_tab_index,
                            Some(Box::new(move |outcome, _me, ctx| {
                                if outcome != SaveOutcome::Canceled {
                                    ctx.emit(CodeViewEvent::Pane(PaneEvent::ReplaceWithFilePane {
                                        path: path.clone(),
                                        source: Some(source.clone()),
                                    }));
                                }
                            })),
                            ctx,
                        );
                    } else {
                        ctx.emit(CodeViewEvent::Pane(PaneEvent::ReplaceWithFilePane {
                            path,
                            source: Some(source),
                        }));
                    }
                }
            }

            CodeViewAction::DragOverIndex {
                target,
                drag_position,
            } => {
                self.drag_position = Some(Self::calculate_tab_bar_dragged_position(
                    drag_position,
                    *target,
                    ctx,
                ));
                self.update_tab_bar_state(ctx);
            }

            CodeViewAction::DropAtIndex {
                origin,
                target,
                drag_position,
            } => {
                self.clear_drag_position();

                let calculated_drag_position =
                    Self::calculate_tab_bar_dragged_position(drag_position, *target, ctx);
                let mut target_index = *target;
                if *origin <= *target
                    && matches!(
                        calculated_drag_position,
                        TabBarDragPosition::BeforeTab { .. }
                    )
                {
                    target_index = target.saturating_sub(1);
                } else if *origin > *target
                    && matches!(
                        calculated_drag_position,
                        TabBarDragPosition::AfterTab { .. }
                    )
                {
                    target_index = (*target + 1).min(self.tab_group.len().saturating_sub(1));
                }

                if *origin != target_index {
                    let tab = self.tab_group.remove(*origin);
                    self.tab_group.insert(target_index, tab);
                    self.active_tab_index = target_index;
                }
                self.update_tab_bar_state(ctx);
            }

            CodeViewAction::ClearEditorTabGroupDragPositions => {
                self.clear_drag_position();
                self.update_tab_bar_state(ctx);
            }

            CodeViewAction::ClearWorkspaceTabGroupDragPositions => {
                ctx.emit(CodeViewEvent::Pane(PaneEvent::ClearHoveredTabIndex));
            }
        }
    }
}

impl BackingView for CodeView {
    type PaneHeaderOverflowMenuAction = CodeViewAction;
    type CustomAction = CodeViewAction;
    type AssociatedData = ();

    fn pane_header_overflow_menu_items(&self, ctx: &AppContext) -> Vec<MenuItem<CodeViewAction>> {
        self.overflow_menu_items(ctx)
    }

    fn handle_pane_header_overflow_menu_action(
        &mut self,
        action: &Self::PaneHeaderOverflowMenuAction,
        ctx: &mut ViewContext<Self>,
    ) {
        self.handle_action(action, ctx);
    }

    fn close(&mut self, ctx: &mut ViewContext<Self>) {
        self.handle_action(&CodeViewAction::CloseAll, ctx);
    }

    fn focus_contents(&mut self, ctx: &mut ViewContext<Self>) {
        self.focus(ctx);
    }

    fn handle_custom_action(
        &mut self,
        custom_action: &Self::CustomAction,
        ctx: &mut ViewContext<Self>,
    ) {
        self.handle_action(custom_action, ctx);
    }

    fn render_header_content(
        &self,
        ctx: &view::HeaderRenderContext<'_>,
        app: &AppContext,
    ) -> view::HeaderContent {
        if self.tab_group.len() >= 2 {
            // Multi-tab case: render custom tab bar with explicit draggable handling
            view::HeaderContent::Custom {
                element: self.render_tab_bar_with_draggable(ctx, app),
                has_custom_draggable_behavior: true,
            }
        } else {
            view::HeaderContent::Custom {
                element: self.render_single_tab_header(ctx, app),
                has_custom_draggable_behavior: false,
            }
        }
    }

    fn set_focus_handle(&mut self, focus_handle: PaneFocusHandle, _ctx: &mut ViewContext<Self>) {
        self.focus_handle = Some(focus_handle);
    }
}

fn render_unsaved_changes_icon(color: ColorU) -> Box<dyn Element> {
    ConstrainedBox::new(
        Rect::new()
            .with_background_color(color)
            .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
            .finish(),
    )
    .with_width(8.)
    .with_height(8.)
    .finish()
}
