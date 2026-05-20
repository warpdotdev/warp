use std::{
    mem,
    path::{Path, PathBuf},
    sync::Arc,
};

use pathfinder_geometry::vector::vec2f;
use warp_util::path::user_friendly_path;
#[cfg(feature = "local_fs")]
use warpui::clipboard::ClipboardContent;
use warpui::{
    accessibility::{AccessibilityContent, WarpA11yRole},
    elements::{
        Align, Container, CrossAxisAlignment, DispatchEventResult, Empty, EventHandler, Flex,
        MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement, SavePosition, Shrinkable,
        Stack, Text,
    },
    keymap::EditableBinding,
    presenter::ChildView,
    ui_components::{
        button::{ButtonVariant, TextAndIcon, TextAndIconAlignment},
        components::{UiComponent, UiComponentStyles},
    },
    AppContext, Element, Entity, ModelHandle, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};

#[cfg(feature = "local_fs")]
use crate::notebooks::post_process_notebook;
use crate::{
    appearance::Appearance,
    cmd_or_ctrl_shift,
    editor::InteractionState,
    menu::{MenuItem, MenuItemFields},
    notebooks::editor::{model::NotebooksEditorModel, rich_text_styles},
    pane_group::{
        focus_state::PaneFocusHandle,
        pane::view,
        pane::view::header::components::{
            render_pane_header_buttons, render_pane_header_title_text, render_three_column_header,
            CenteredHeaderEdgeWidth,
        },
        BackingView, PaneConfiguration, PaneEvent,
    },
    safe_warn, send_telemetry_from_ctx,
    server::telemetry::{NotebookActionEvent, NotebookTelemetryMetadata, TelemetryEvent},
    settings::FontSettings,
    terminal::model::session::Session,
    ui_components::icons::Icon,
    view_components::{MarkdownToggleEvent, MarkdownToggleView},
    workflows::{WorkflowSource, WorkflowType},
    workspace::ActiveSession,
};

use super::{
    context_menu::{show_rich_editor_context_menu, ContextMenuAction, ContextMenuState},
    editor::view::{EditorViewEvent, RichTextEditorConfig, RichTextEditorView},
    link::{NotebookLinks, SessionSource},
    styles,
    telemetry::NotebookTelemetryAction,
    NotebookLocation,
};
#[cfg(feature = "local_fs")]
use crate::code::editor_management::CodeSource;
#[cfg(feature = "local_fs")]
use crate::util::openable_file_type::FileTarget;
use warp_core::ui::icons::ICON_DIMENSIONS;
use warp_editor::model::CoreEditorModel;
#[cfg(feature = "local_fs")]
use warp_files::{FileModel, FileModelEvent};
#[cfg(feature = "local_fs")]
use warp_util::file::FileId;

pub use crate::util::openable_file_type::is_markdown_file;

/// Display mode for markdown files shown via the header segmented control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkdownDisplayMode {
    Rendered,
    Raw,
}

/// View for a read-only notebook backed by a file, rather than Warp Drive.
pub struct FileNotebookView {
    /// The location of the open file. This is cached for displaying the title and breadcrumbs.
    location: Option<FileLocation>,
    /// Read-only rich text editor used to show the notebook contents.
    editor: ViewHandle<RichTextEditorView>,
    retry_button_mouse_state: MouseStateHandle,
    /// Tracks the state for loading the backing Markdown file.
    file_state: FileState,
    /// File watcher id for the currently opened file (if any). Only needed when we have local fs
    /// access.
    #[cfg(feature = "local_fs")]
    file_id: Option<FileId>,
    pane_configuration: ModelHandle<PaneConfiguration>,
    focus_handle: Option<PaneFocusHandle>,
    links: ModelHandle<NotebookLinks>,
    context_menu: ContextMenuState<Self>,
    view_position_id: String,
    /// Display mode for markdown content (rendered vs editable)
    markdown_display_mode: MarkdownDisplayMode,
    display_mode_segmented_control: ViewHandle<MarkdownToggleView>,
    /// The CodeSource that was used to open this file, if it came from a CodePane.
    /// This is preserved so we can restore it when toggling between raw and rendered Markdown.
    #[cfg(feature = "local_fs")]
    code_source: Option<CodeSource>,
}

#[derive(Debug, Clone)]
pub enum FileNotebookEvent {
    RunWorkflow {
        workflow: Arc<WorkflowType>,
        source: WorkflowSource,
    },
    TitleUpdated,
    FileLoaded,
    Pane(PaneEvent),
    #[cfg(feature = "local_fs")]
    OpenFileWithTarget {
        path: PathBuf,
        target: FileTarget,
        line_col: Option<warp_util::path::LineAndColumnArg>,
    },
}

impl From<PaneEvent> for FileNotebookEvent {
    fn from(event: PaneEvent) -> Self {
        FileNotebookEvent::Pane(event)
    }
}

#[derive(Debug, Clone)]
pub enum FileNotebookAction {
    Focus,
    Close,
    FocusTerminalInput,
    ReloadFile,
    #[cfg(feature = "local_fs")]
    CopyFilePath,
    #[cfg(feature = "local_fs")]
    OpenInEditor,
    #[cfg(feature = "local_fs")]
    OpenAsCode,
    ContextMenu(ContextMenuAction),
    ToggleMarkdownDisplayMode(MarkdownDisplayMode),
}

impl From<ContextMenuAction> for FileNotebookAction {
    fn from(action: ContextMenuAction) -> Self {
        FileNotebookAction::ContextMenu(action)
    }
}

/// Information about the notebook's backing file.
// TODO: This should probably build on the `warp_files` abstractions.
#[derive(Debug, Clone)]
enum SourceFile {
    Local {
        /// The full path to the open file - for now, _only_ local files are supported.
        ///
        /// See [this comment](https://docs.google.com/document/d/18h7VzSAl6r5a94CovShlpPSahYqECX9WZliLjUqUsko/edit?disco=AAAA5Y1THuk);
        /// we cannot use [`PathBuf`] to represent non-local paths.
        local_path: PathBuf,
        session: Option<Arc<Session>>,
    },
    Static {
        title: String,
    },
}

impl SourceFile {
    fn local_path(&self) -> Option<&Path> {
        match self {
            SourceFile::Local { local_path, .. } => Some(local_path.as_path()),
            SourceFile::Static { .. } => None,
        }
    }

    fn display_name(&self) -> String {
        match self {
            SourceFile::Local { local_path, .. } => local_path.display().to_string(),
            SourceFile::Static { title } => title.clone(),
        }
    }
}

#[derive(Debug)]
enum FileState {
    NoFile,
    Loading(SourceFile),
    Error(SourceFile),
    Loaded(SourceFile),
}

impl FileState {
    /// The path to the open file, if it exists and is local.
    fn local_path(&self) -> Option<&Path> {
        self.source().and_then(|src| src.local_path())
    }

    fn source(&self) -> Option<&SourceFile> {
        match self {
            FileState::NoFile => None,
            FileState::Loading(source) | FileState::Error(source) | FileState::Loaded(source) => {
                Some(source)
            }
        }
    }

    fn display_name(&self) -> Option<String> {
        self.source().map(|src| src.display_name())
    }
}

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_editable_bindings([
        EditableBinding::new(
            "notebookview:focus_terminal_input",
            "Focus Terminal Input from File",
            FileNotebookAction::FocusTerminalInput,
        )
        .with_context_predicate(id!("FileNotebookView"))
        .with_key_binding(cmd_or_ctrl_shift("l")),
        EditableBinding::new(
            "notebookview:reload_file",
            "Reload file",
            FileNotebookAction::ReloadFile,
        )
        .with_context_predicate(id!("FileNotebookView")),
    ])
}

impl FileNotebookView {
    /// Create a new file notebook view, with no open file.
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let window_id = ctx.window_id();
        // Use the active session for links until we have something more specific.
        let links = ctx.add_model(|ctx| NotebookLinks::new(SessionSource::Active(window_id), ctx));

        let view_position_id = format!("file_notebook_view_{}", ctx.view_id());

        let editor_model = ctx.add_model(|ctx| {
            let styles = rich_text_styles(Appearance::as_ref(ctx), FontSettings::as_ref(ctx));
            let mut model = NotebooksEditorModel::new(styles, window_id, ctx);
            model.set_default_mermaid_display_mode(MarkdownDisplayMode::Rendered, ctx);
            model
        });
        let editor = ctx.add_typed_action_view(|ctx| {
            let mut view = RichTextEditorView::new(
                view_position_id.clone(),
                editor_model,
                links.clone(),
                RichTextEditorConfig::default(),
                ctx,
            );
            view.set_interaction_state(InteractionState::Selectable, ctx);
            view
        });

        ctx.subscribe_to_view(&editor, Self::handle_editor_event);

        let pane_configuration = ctx.add_model(|_ctx| PaneConfiguration::new(""));

        ctx.observe(
            &ActiveSession::handle(ctx),
            Self::handle_active_session_change,
        );

        let context_menu = ContextMenuState::new(ctx);

        let display_mode_segmented_control = ctx.add_typed_action_view(|ctx| {
            MarkdownToggleView::new(MarkdownDisplayMode::Rendered, ctx)
        });

        ctx.subscribe_to_view(&display_mode_segmented_control, |view, _, event, ctx| {
            let MarkdownToggleEvent::ModeSelected(mode) = event;
            view.handle_action(&FileNotebookAction::ToggleMarkdownDisplayMode(*mode), ctx);
        });

        Self {
            location: None,
            editor,
            file_state: FileState::NoFile,
            retry_button_mouse_state: Default::default(),
            #[cfg(feature = "local_fs")]
            file_id: None,
            pane_configuration,
            focus_handle: None,
            links,
            context_menu,
            view_position_id,
            markdown_display_mode: MarkdownDisplayMode::Rendered,
            display_mode_segmented_control,
            #[cfg(feature = "local_fs")]
            code_source: None,
        }
    }

    /// Set the CodeSource that was used to open this file.
    /// This is preserved so we can restore it when toggling between raw and rendered Markdown.
    #[cfg(feature = "local_fs")]
    pub fn set_code_source(&mut self, source: Option<CodeSource>) {
        self.code_source = source;
    }

    pub fn title(&self) -> String {
        // Prefer the location name that's been resolved against a Session, but if that's not
        // available yet, fall back to the raw file path.
        self.location
            .as_ref()
            .map(|location| location.name.clone())
            .or_else(|| self.file_state.display_name())
            .unwrap_or_else(|| "Untitled".to_string())
    }

    pub fn focus(&self, ctx: &mut ViewContext<Self>) {
        // Emit accessibility content for the notebook, rather than the generic text input.
        if let Some(a11y_content) = self.accessibility_contents(ctx) {
            ctx.emit_a11y_content(a11y_content);
        }
        ctx.focus(&self.editor);
    }

    /// Reset the rich text contents based on the given Markdown content.
    pub fn set_content(&mut self, content: &str, ctx: &mut ViewContext<Self>) {
        let doc_path = self.file_state.local_path().map(|p| p.to_path_buf());
        self.editor.update(ctx, |editor, ctx| {
            editor.reset_with_markdown(content, ctx);
            // Set the document path for resolving relative image paths
            editor.model().update(ctx, |model, ctx| {
                model.set_document_path(doc_path, ctx);
            });
        });
    }

    #[cfg(feature = "local_fs")]
    fn open_telemetry_metadata(&self, ctx: &ViewContext<Self>) -> NotebookTelemetryMetadata {
        NotebookTelemetryMetadata::new(None, None, NotebookLocation::LocalFile, None)
            .with_markdown_table_count(
                self.editor
                    .as_ref(ctx)
                    .model()
                    .as_ref(ctx)
                    .markdown_table_count(ctx),
            )
    }

    /// Set the notebook's location context.
    fn set_context(&mut self, path: &Path, session: Arc<Session>, ctx: &mut ViewContext<Self>) {
        self.location = Some(FileLocation::new(path, session.home_dir()));
        let title = self.title();
        self.pane_configuration.update(ctx, |pane_config, ctx| {
            pane_config.set_title(title, ctx);
        });
        if let Some(parent) = path.parent() {
            self.links.update(ctx, |links, ctx| {
                links.set_session_source(
                    SessionSource::Target {
                        session,
                        base_directory: parent.to_path_buf(),
                    },
                    ctx,
                )
            })
        }

        ctx.notify();
    }

    /// Asynchronously open a local file, watching for local file changes.
    pub fn open_local(
        &mut self,
        path: impl Into<PathBuf>,
        session: Option<Arc<Session>>,
        ctx: &mut ViewContext<Self>,
    ) {
        let local_path = path.into();

        // If a session is available, initialize the location and link context now. Otherwise,
        // we'll wait until one is available.
        if let Some(session) = &session {
            self.set_context(&local_path, session.clone(), ctx);
        } else {
            // Set a temporary title until the context is available.
            self.pane_configuration.update(ctx, |pane_config, ctx| {
                pane_config.set_title(local_path.display().to_string(), ctx);
            });
        }

        self.file_state = FileState::Loading(SourceFile::Local {
            local_path: local_path.clone(),
            session: session.clone(),
        });

        #[cfg(feature = "local_fs")]
        {
            // Cancel in-flight loads and unsubscribe from any previously opened file.
            if let Some(prev_id) = self.file_id.take() {
                FileModel::handle(ctx).update(ctx, |m, ctx| {
                    m.cancel(prev_id);
                    m.unsubscribe(prev_id, ctx)
                });
            }

            let file_model = FileModel::handle(ctx);
            let file_id = file_model.update(ctx, |m, ctx| m.open(&local_path, true, ctx));
            self.file_id = Some(file_id);

            ctx.subscribe_to_model(
                &file_model,
                move |me, file_model: ModelHandle<FileModel>, event: &FileModelEvent, ctx| {
                    if event.file_id() != file_id {
                        return;
                    }
                    match event {
                        FileModelEvent::FileLoaded { content, .. } => {
                            let cleaned = post_process_notebook(content);
                            me.set_content(&cleaned, ctx);
                            send_telemetry_from_ctx!(
                                TelemetryEvent::OpenNotebook(me.open_telemetry_metadata(ctx)),
                                ctx
                            );

                            // Record the canonical path instead of the input path when available.
                            if let Some(canonical_path) = file_model.as_ref(ctx).file_path(file_id)
                            {
                                me.file_state = FileState::Loaded(SourceFile::Local {
                                    local_path: canonical_path,
                                    session: session.clone(),
                                });
                            }

                            me.pane_configuration.update(ctx, |pane_config, ctx| {
                                pane_config.refresh_pane_header_overflow_menu_items(ctx);
                            });

                            // Notify the view to re-render now that file_state
                            // has transitioned from Loading to Loaded.
                            ctx.notify();

                            // Trigger to save the open file path for session restoration.
                            ctx.emit(FileNotebookEvent::FileLoaded);
                        }
                        FileModelEvent::FailedToLoad { error, .. } => {
                            safe_warn!(
                                safe: ("Unable to read local notebook file"),
                                full: ("Unable to read local notebook file: {error}")
                            );
                            me.file_state =
                                match mem::replace(&mut me.file_state, FileState::NoFile) {
                                    FileState::NoFile => FileState::NoFile,
                                    FileState::Loading(source)
                                    | FileState::Loaded(source)
                                    | FileState::Error(source) => FileState::Error(source),
                                };
                            ctx.notify();
                        }
                        FileModelEvent::FileUpdated { content, .. } => {
                            let cleaned = post_process_notebook(content);
                            me.set_content(&cleaned, ctx);
                        }
                        FileModelEvent::FileSaved { .. } | FileModelEvent::FailedToSave { .. } => {}
                    }
                },
            );
        }

        #[cfg(not(feature = "local_fs"))]
        {
            // WASM builds should never call `open_local`, so we should never get here!
            safe_warn!(
                safe: ("Local filesystem access is not available in this build"),
                full: ("Local filesystem access is not available in this build (feature \"local_fs\" disabled)")
            );
            self.file_state = FileState::Error(SourceFile::Local {
                local_path,
                session,
            });
            ctx.notify();
        }
    }

    /// Open static Markdown as a file pane.
    pub fn open_static(
        &mut self,
        title: impl Into<String>,
        content: &str,
        ctx: &mut ViewContext<Self>,
    ) {
        #[cfg(feature = "local_fs")]
        {
            // Unsubscribe from any previously opened file.
            if let Some(prev_id) = self.file_id.take() {
                FileModel::handle(ctx).update(ctx, |m, ctx| m.unsubscribe(prev_id, ctx));
            }
        }
        self.set_content(content, ctx);
        let title = title.into();
        self.pane_configuration.update(ctx, |pane_config, ctx| {
            pane_config.set_title(title.clone(), ctx);
            pane_config.refresh_pane_header_overflow_menu_items(ctx);
        });
        self.file_state = FileState::Loaded(SourceFile::Static { title });
    }

    /// Send a [`NotebookTelemetryAction`] telemetry event.
    fn send_telemetry_action(&self, action: NotebookTelemetryAction, ctx: &mut ViewContext<Self>) {
        send_telemetry_from_ctx!(
            TelemetryEvent::NotebookAction(NotebookActionEvent {
                action,
                metadata: NotebookTelemetryMetadata::new(
                    None,
                    None,
                    NotebookLocation::LocalFile,
                    None
                )
            }),
            ctx
        );
    }

    /// Reload the file that was most recently opened (or attempted to open).
    fn reload_file(&mut self, ctx: &mut ViewContext<Self>) {
        // We can take the file state here because either it's (a) already NoFile or (b) about to
        // be replaced with a loading state.
        let (local_path, session) = match mem::replace(&mut self.file_state, FileState::NoFile) {
            FileState::NoFile => return,
            FileState::Loading(source) | FileState::Error(source) | FileState::Loaded(source) => {
                match source {
                    SourceFile::Local {
                        local_path,
                        session,
                    } => (local_path, session),
                    SourceFile::Static { .. } => return,
                }
            }
        };
        self.open_local(local_path, session, ctx);
    }

    #[cfg(feature = "local_fs")]
    fn open_as_code(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(path) = self.local_path() {
            // Emit an event to the pane group to handle the replacement
            ctx.emit(FileNotebookEvent::Pane(PaneEvent::ReplaceWithCodePane {
                path: path.clone(),
                source: self.code_source.clone(),
            }));
        }
    }

    /// The path to the currently-open file, if it is local.
    pub fn local_path(&self) -> Option<PathBuf> {
        self.file_state.local_path().map(Path::to_path_buf)
    }

    pub fn pane_configuration(&self) -> ModelHandle<PaneConfiguration> {
        self.pane_configuration.clone()
    }

    /// Model for resolving and opening links relative to this notebook.
    pub fn links(&self) -> ModelHandle<NotebookLinks> {
        self.links.clone()
    }

    #[cfg(feature = "local_fs")]
    fn is_markdown_file(&self) -> bool {
        self.file_state
            .local_path()
            .map(is_markdown_file)
            .unwrap_or(false)
    }

    #[cfg(not(feature = "local_fs"))]
    fn is_markdown_file(&self) -> bool {
        false
    }

    fn update_editor_display_mode(&mut self, ctx: &mut ViewContext<Self>) {
        match self.markdown_display_mode {
            MarkdownDisplayMode::Rendered => {
                self.editor.update(ctx, |editor, ctx| {
                    editor.set_interaction_state(InteractionState::Selectable, ctx);
                });
            }
            MarkdownDisplayMode::Raw => {
                // For Raw we switch panes entirely (to CodePane). Interaction state here remains
                // in the rendered notebook mode.
            }
        }
    }

    fn handle_editor_event(
        &mut self,
        _handle: ViewHandle<RichTextEditorView>,
        event: &EditorViewEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            EditorViewEvent::Focused => ctx.emit(FileNotebookEvent::Pane(PaneEvent::FocusSelf)),
            EditorViewEvent::RunWorkflow(workflow) => {
                let workflow_type = workflow.named_workflow(|| {
                    self.location
                        .as_ref()
                        .map(|location| format!("Command from {}", location.name))
                });
                let source = workflow.source.unwrap_or(WorkflowSource::Notebook {
                    notebook_id: None,
                    team_uid: None,
                    location: NotebookLocation::LocalFile,
                });
                ctx.emit(FileNotebookEvent::RunWorkflow {
                    workflow: workflow_type,
                    source,
                });
            }
            EditorViewEvent::OpenedBlockInsertionMenu(source) => self.send_telemetry_action(
                NotebookTelemetryAction::OpenBlockInsertionMenu { source: *source },
                ctx,
            ),
            EditorViewEvent::OpenedEmbeddedObjectSearch => {
                self.send_telemetry_action(NotebookTelemetryAction::OpenEmbeddedObjectSearch, ctx)
            }
            EditorViewEvent::OpenedFindBar => {
                self.send_telemetry_action(NotebookTelemetryAction::OpenFindBar, ctx)
            }
            EditorViewEvent::InsertedEmbeddedObject(info) => self
                .send_telemetry_action(NotebookTelemetryAction::InsertEmbeddedObject(*info), ctx),
            EditorViewEvent::CopiedBlock { block, entrypoint } => self.send_telemetry_action(
                NotebookTelemetryAction::CopyBlock {
                    block: *block,
                    entrypoint: *entrypoint,
                },
                ctx,
            ),
            EditorViewEvent::NavigatedCommands => {
                self.send_telemetry_action(NotebookTelemetryAction::CommandKeyboardNavigation, ctx)
            }
            EditorViewEvent::ChangedSelectionMode(mode) => self.send_telemetry_action(
                NotebookTelemetryAction::ChangeSelectionMode { mode: *mode },
                ctx,
            ),
            EditorViewEvent::Navigate(_)
            | EditorViewEvent::Edited
            | EditorViewEvent::EditWorkflow(_)
            | EditorViewEvent::CmdEnter
            | EditorViewEvent::EscapePressed
            | EditorViewEvent::TextSelectionChanged => (),
            EditorViewEvent::OpenFile { .. } => {
                // We don't support opening files from the notebook view.
                // File paths rely on a Session to be present, and this is only set from the AI document view today.
            }
        }
    }

    fn handle_active_session_change(
        &mut self,
        handle: ModelHandle<ActiveSession>,
        ctx: &mut ViewContext<Self>,
    ) {
        // If this file notebook is opened without a target session, we wait for one to start and
        // use that instead.
        if self.location.is_none() {
            let Some(path) = self.local_path() else {
                return;
            };
            if let Some(active_session) = handle.as_ref(ctx).session(ctx.window_id()) {
                if active_session.is_local() {
                    self.set_context(&path, active_session, ctx);
                    ctx.unsubscribe_to_model(&handle);
                }
            }
        }
    }

    fn render_title(
        &self,
        appearance: &Appearance,
        font_settings: &FontSettings,
    ) -> Box<dyn Element> {
        let title = Text::new_inline(
            self.title(),
            appearance.ui_font_family(),
            styles::title_font_size(font_settings),
        )
        .with_color(styles::title_text_fill(appearance).into())
        .with_style(styles::TITLE_FONT_PROPERTIES)
        .finish();

        let details = self.location.as_ref().map(|location| {
            appearance
                .ui_builder()
                .span(location.breadcrumbs.clone())
                .with_style(UiComponentStyles {
                    font_color: Some(styles::title_text_fill(appearance).into_solid()),
                    ..Default::default()
                })
                .build()
                .finish()
        });

        styles::wrap_title(title, details)
    }

    /// Style for loading/error states.
    fn state_style(&self, appearance: &Appearance) -> UiComponentStyles {
        UiComponentStyles {
            font_color: Some(
                appearance
                    .theme()
                    .sub_text_color(appearance.theme().background())
                    .into_solid(),
            ),
            ..Default::default()
        }
    }

    /// Render an error state for when loading the source file failed.
    fn render_error(&self, source: &SourceFile, appearance: &Appearance) -> Box<dyn Element> {
        let error_text_color = appearance
            .theme()
            .sub_text_color(appearance.theme().background());
        let error = Flex::column()
            .with_main_axis_alignment(MainAxisAlignment::Center)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                appearance
                    .ui_builder()
                    .paragraph(format!("Could not read {}", source.display_name()))
                    .with_style(self.state_style(appearance))
                    .build()
                    .finish(),
            )
            .with_child(
                Container::new(
                    appearance
                        .ui_builder()
                        .button(ButtonVariant::Basic, self.retry_button_mouse_state.clone())
                        .with_text_and_icon_label(
                            TextAndIcon::new(
                                TextAndIconAlignment::TextFirst,
                                "Try again".to_string(),
                                Icon::Refresh.to_warpui_icon(error_text_color),
                                MainAxisSize::Min,
                                MainAxisAlignment::Center,
                                vec2f(16., 16.),
                            )
                            .with_inner_padding(4.),
                        )
                        .build()
                        .on_click(|ctx, _, _| {
                            ctx.dispatch_typed_action(FileNotebookAction::ReloadFile)
                        })
                        .finish(),
                )
                .with_margin_top(8.)
                .finish(),
            );

        Align::new(error.finish()).finish()
    }

    /// Render the loading state while the source file is still being read.
    fn render_loading(&self, source: &SourceFile, appearance: &Appearance) -> Box<dyn Element> {
        Align::new(
            appearance
                .ui_builder()
                .paragraph(format!("Loading {}...", source.display_name()))
                .with_style(self.state_style(appearance))
                .build()
                .finish(),
        )
        .finish()
    }

    /// Renders a placeholder for when no file has been specified.
    fn render_no_file(&self, appearance: &Appearance) -> Box<dyn Element> {
        Align::new(
            appearance
                .ui_builder()
                .paragraph("Missing source file".to_string())
                .with_style(self.state_style(appearance))
                .build()
                .finish(),
        )
        .finish()
    }

    fn render_body(&self, appearance: &Appearance) -> Box<dyn Element> {
        let body = match &self.file_state {
            FileState::NoFile => self.render_no_file(appearance),
            FileState::Loading(source) => self.render_loading(source, appearance),
            FileState::Error(source) => self.render_error(source, appearance),
            FileState::Loaded(_) => ChildView::new(&self.editor).finish(),
        };
        styles::wrap_body(body)
    }
}

impl Entity for FileNotebookView {
    type Event = FileNotebookEvent;
}

impl View for FileNotebookView {
    fn ui_name() -> &'static str {
        "FileNotebookView"
    }

    fn accessibility_contents(&self, _ctx: &AppContext) -> Option<AccessibilityContent> {
        Some(AccessibilityContent::new_without_help(
            format!("{} notebook", self.title()),
            WarpA11yRole::TextRole,
        ))
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let font_settings = FontSettings::as_ref(app);

        let column = Flex::column().with_children([
            self.render_title(appearance, font_settings),
            Shrinkable::new(1., self.render_body(appearance)).finish(),
        ]);

        let mut stack = Stack::new().with_child(column.finish());
        self.context_menu.render(&mut stack);

        let parent_position_id = self.view_position_id.clone();
        let editor = self.editor.clone();

        SavePosition::new(
            EventHandler::new(Align::new(stack.finish()).top_left().finish())
                .on_left_mouse_down(|ctx, _, _| {
                    ctx.dispatch_typed_action(FileNotebookAction::Focus);
                    DispatchEventResult::StopPropagation
                })
                .on_right_mouse_down(move |ctx, _, position| {
                    show_rich_editor_context_menu::<FileNotebookAction>(
                        ctx,
                        position,
                        &parent_position_id,
                        &editor,
                    );
                    DispatchEventResult::StopPropagation
                })
                .finish(),
            &self.view_position_id,
        )
        .finish()
    }
}

impl TypedActionView for FileNotebookView {
    type Action = FileNotebookAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            FileNotebookAction::Focus => ctx.focus_self(),
            FileNotebookAction::Close => ctx.emit(FileNotebookEvent::Pane(PaneEvent::Close)),
            FileNotebookAction::FocusTerminalInput => {
                ctx.emit(FileNotebookEvent::Pane(PaneEvent::FocusActiveSession))
            }
            FileNotebookAction::ReloadFile => self.reload_file(ctx),
            #[cfg(feature = "local_fs")]
            FileNotebookAction::CopyFilePath => {
                if let Some(path) = self.local_path() {
                    ctx.clipboard()
                        .write(ClipboardContent::plain_text(path.display().to_string()));
                }
            }
            #[cfg(feature = "local_fs")]
            FileNotebookAction::OpenInEditor => {
                if let Some(path) = self.local_path() {
                    use crate::util::file::external_editor::EditorSettings;
                    use crate::util::openable_file_type::resolve_file_target;
                    // Resolve target and emit event - workspace will handle all cases
                    let settings = EditorSettings::as_ref(ctx);
                    let target = resolve_file_target(&path, settings, None);
                    ctx.emit(FileNotebookEvent::OpenFileWithTarget {
                        path,
                        target,
                        line_col: None,
                    });
                }
            }
            #[cfg(feature = "local_fs")]
            FileNotebookAction::OpenAsCode => self.open_as_code(ctx),
            FileNotebookAction::ContextMenu(action) => {
                if matches!(action, ContextMenuAction::Open(_)) {
                    self.send_telemetry_action(NotebookTelemetryAction::OpenContextMenu, ctx);
                }
                self.context_menu.handle_action(action, ctx);
            }
            FileNotebookAction::ToggleMarkdownDisplayMode(mode) => {
                self.markdown_display_mode = *mode;
                self.display_mode_segmented_control
                    .update(ctx, |control, ctx| {
                        control.set_selected_mode(*mode, ctx);
                    });

                match mode {
                    MarkdownDisplayMode::Rendered => {
                        // Already in FileNotebookView with rendered content; nothing else to do.
                        self.update_editor_display_mode(ctx);
                    }
                    MarkdownDisplayMode::Raw => {
                        #[cfg(feature = "local_fs")]
                        {
                            if let Some(path) = self.local_path() {
                                ctx.emit(FileNotebookEvent::Pane(PaneEvent::ReplaceWithCodePane {
                                    path,
                                    source: self.code_source.clone(),
                                }));
                            }
                        }
                    }
                }
            }
        }
    }
}

impl BackingView for FileNotebookView {
    type PaneHeaderOverflowMenuAction = FileNotebookAction;
    type CustomAction = ();
    type AssociatedData = ();

    fn handle_pane_header_overflow_menu_action(
        &mut self,
        action: &Self::PaneHeaderOverflowMenuAction,
        ctx: &mut ViewContext<Self>,
    ) {
        self.handle_action(action, ctx);
    }

    fn pane_header_overflow_menu_items(
        &self,
        _ctx: &AppContext,
    ) -> Vec<MenuItem<FileNotebookAction>> {
        let mut actions = vec![];
        if let Some(SourceFile::Local {
            local_path: _local_path,
            ..
        }) = self.file_state.source()
        {
            actions.push(
                MenuItemFields::new("Refresh file")
                    .with_on_select_action(FileNotebookAction::ReloadFile)
                    .into_item(),
            );

            #[cfg(feature = "local_fs")]
            {
                // The markdown rendered/raw toggle is always visible in the pane header, so we don't
                // duplicate it in the overflow menu. Keep "Open in editor" available for local files.
                actions.push(
                    MenuItemFields::new("Open in editor")
                        .with_on_select_action(FileNotebookAction::OpenInEditor)
                        .into_item(),
                );
                actions.extend([
                    MenuItem::Separator,
                    MenuItemFields::new("Copy file path")
                        .with_on_select_action(FileNotebookAction::CopyFilePath)
                        .into_item(),
                ]);
            }
        }
        actions
    }

    fn close(&mut self, ctx: &mut ViewContext<Self>) {
        #[cfg(feature = "local_fs")]
        {
            // Unsubscribe from the file watcher before closing.
            if let Some(prev_id) = self.file_id.take() {
                FileModel::handle(ctx).update(ctx, |m, ctx| m.unsubscribe(prev_id, ctx));
            }
        }
        ctx.emit(FileNotebookEvent::Pane(PaneEvent::Close));
    }

    fn focus_contents(&mut self, ctx: &mut ViewContext<Self>) {
        self.focus(ctx);
    }

    fn render_header_content(
        &self,
        ctx: &view::HeaderRenderContext<'_>,
        app: &AppContext,
    ) -> view::HeaderContent {
        let title = self.pane_configuration.as_ref(app).title().to_owned();

        if self.is_markdown_file() {
            // For markdown files we use a custom header
            // so that the title stays centered identically in both rendered and raw (CodeView) modes.
            let appearance = Appearance::as_ref(app);
            let is_pane_dragging = ctx.draggable_state.is_dragging();

            let mut right_row = Flex::row()
                .with_main_axis_alignment(MainAxisAlignment::End)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_main_axis_size(MainAxisSize::Min);

            right_row.add_child(ChildView::new(&self.display_mode_segmented_control).finish());

            let show_close_button = self
                .focus_handle
                .as_ref()
                .is_some_and(|h| h.is_in_split_pane(app));

            right_row.add_child(render_pane_header_buttons::<FileNotebookAction, ()>(
                ctx,
                appearance,
                show_close_button,
                None,
                None,
            ));

            let button_count = show_close_button as u32 + ctx.has_overflow_items as u32;
            let buttons_width = button_count as f32 * ICON_DIMENSIONS;

            let title_text = render_pane_header_title_text(
                title,
                appearance,
                warpui::text_layout::ClipConfig::start(),
            );

            view::HeaderContent::Custom {
                element: render_three_column_header(
                    Empty::new().finish(),
                    title_text,
                    right_row.finish(),
                    CenteredHeaderEdgeWidth {
                        min: buttons_width,
                        max: 220.0,
                    },
                    ctx.header_left_inset,
                    is_pane_dragging,
                ),
                has_custom_draggable_behavior: false,
            }
        } else {
            // Non-markdown files: use the standard header.
            view::HeaderContent::Standard(view::StandardHeader {
                title,
                title_secondary: None,
                title_style: None,
                title_clip_config: warpui::text_layout::ClipConfig::start(),
                title_max_width: None,
                left_of_title: None,
                right_of_title: None,
                left_of_overflow: None,
                options: Default::default(),
            })
        }
    }

    fn set_focus_handle(&mut self, focus_handle: PaneFocusHandle, _ctx: &mut ViewContext<Self>) {
        self.focus_handle = Some(focus_handle.clone());
        self.context_menu.set_focus_handle(focus_handle);
    }
}

/// Location information for a file, used to show its title and context.
struct FileLocation {
    /// Breadcrumb path to the file.
    breadcrumbs: String,
    /// The file's name.
    name: String,
}

impl FileLocation {
    fn new(path: &Path, home_directory: Option<&str>) -> Self {
        let breadcrumbs = match path.parent() {
            Some(directory) => {
                user_friendly_path(directory.to_string_lossy().as_ref(), home_directory)
                    .into_owned()
            }
            None => String::new(),
        };
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Unnamed".to_string());

        Self { breadcrumbs, name }
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
