/// This module contains a model that can be used for loading and saving text files
/// and displaying them in a code editor.
/// It also handles applying an optional diff to the file content that will be applied
/// when the file is loaded.
use std::{
    ops::Range,
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

use futures::stream::AbortHandle;
use lsp::{
    types::FileLocation, LanguageId, LanguageServerId, LspEvent, LspManagerModel,
    LspManagerModelEvent, LspServerModel, ReferenceLocation,
};
use lsp_types::FormattingOptions;
use markdown_parser::FormattedText;
use num_traits::SaturatingSub;
use pathfinder_geometry::{rect::RectF, vector::Vector2F};
use string_offset::CharOffset;
use vec1::Vec1;
use warp_core::{features::FeatureFlag, ui::appearance::Appearance};
use warp_editor::{
    content::{buffer::InitialBufferState, text::IndentUnit},
    render::model::{Decoration, LineCount},
};
use warp_util::{
    content_version::ContentVersion,
    file::{FileId, FileLoadError, FileSaveError},
    path::to_relative_path,
    sync::Condition,
};
use warpui::{
    elements::{
        Border, ChildAnchor, ChildView, ClippedScrollStateHandle, ConstrainedBox, Container,
        CornerRadius, CrossAxisAlignment, DropShadow, Flex, Hoverable, MainAxisAlignment,
        MainAxisSize, MouseStateHandle, OffsetPositioning, ParentAnchor, ParentElement,
        ParentOffsetBounds, Radius, Rect, Shrinkable, Stack, Text,
    },
    keymap::{macros::*, FixedBinding},
    text::point::Point,
    ui_components::{
        button::ButtonVariant,
        components::{Coords, UiComponent, UiComponentStyles},
    },
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
    WindowId,
};
use warpui::{platform::SaveFilePickerConfiguration, ModelHandle};

use crate::menu::{Event, Menu, MenuItem, MenuItemFields};

use crate::{
    code::{
        buffer_location::LocalOrRemotePath as BufferFileLocation,
        editor::model::HoverableLink,
        footer::{CodeFooterView, CodeFooterViewEvent},
        global_buffer_model::{BufferState, GlobalBufferModel},
        SaveOutcome, ShowFindReferencesCardProvider,
    },
    debounce::debounce,
    settings::AISettings,
    terminal::TerminalView,
};
use crate::{
    code::{editor::EditorReviewComment, global_buffer_model::GlobalBufferModelEvent},
    code_review::comments::CommentId,
};
use ai::diff_validation::DiffType;
use pathfinder_color::ColorU;
#[cfg(feature = "local_fs")]
use repo_metadata::repositories::DetectedRepositories;
use vim::vim::{MotionType, VimMode};
use warp_core::ui::icons::Icon;
#[cfg(feature = "local_fs")]
use warp_util::local_or_remote_path::LocalOrRemotePath;

use crate::ai::persisted_workspace::{PersistedWorkspace, PersistedWorkspaceEvent};
use crate::workspace::WorkspaceAction;

const DROP_SHADOW_COLOR: ColorU = ColorU {
    r: 0,
    g: 0,
    b: 0,
    a: 48,
};

const HOVER_DEBOUNCE_PERIOD: Duration = Duration::from_millis(500);

use super::diff_viewer::DiffViewer;
use super::editor::{
    scroll::{ScrollPosition, ScrollTrigger},
    view::{CodeEditorEvent, CodeEditorView},
};
use super::find_references_view::{FindReferencesView, FindReferencesViewEvent};
use super::language_server_extension::ProcessedDiagnostic;
use super::lsp_telemetry::LspTelemetryEvent;
use super::ImmediateSaveError;
use warp_core::send_telemetry_from_ctx;

type SaveCallback =
    Box<dyn FnOnce(SaveOutcome, &mut ViewContext<LocalCodeEditorView>) + Send + Sync + 'static>;

pub fn init(app: &mut AppContext) {
    app.register_fixed_bindings([FixedBinding::new(
        "cmdorctrl-l",
        LocalCodeEditorAction::InsertSelectedTextToInput,
        id!("LocalCodeEditorView") & !id!("IMEOpen"),
    )]);
}

pub enum LocalCodeEditorEvent {
    FileLoaded,
    FailedToLoad {
        error: Rc<FileLoadError>,
    },
    FileSaved,
    FailedToSave {
        error: Rc<FileSaveError>,
    },
    DiffAccepted,
    DiffRejected,
    /// Emitted when a user presses Escape in Vim Normal mode inside the embedded editor.
    VimMinimizeRequested,
    /// Emitted when a user edits the file.
    UserEdited,
    /// Emitted when the diff status changes (e.g., line counts update).
    DiffStatusUpdated,
    SelectionAddedAsContext {
        relative_file_path: String,
        /// 1-indexed line range of the selection: `[start, end]` both inclusive.
        line_range: Range<LineCount>,
        /// Literal text content of the selection.
        selected_text: String,
    },
    DiscardUnsavedChanges {
        path: PathBuf,
    },
    GotoDefinition {
        path: PathBuf,
        line: usize,
        column: usize,
        /// The ID of the LSP server that produced this definition.
        /// Used to register external files with the correct server.
        source_server_id: LanguageServerId,
    },
    /// Emitted when a comment is saved. This propagates the comment content
    /// changes to the CodeReviewView, which will update the comment model.
    CommentSaved {
        comment: EditorReviewComment,
    },
    RequestOpenComment(CommentId),
    DeleteComment {
        id: CommentId,
    },
    /// Emitted when the viewport is updated after layout
    ViewportUpdated,
    /// Emitted when the render state layout has been updated.
    LayoutInvalidated,
    /// Request to open LSP logs for the given file path.
    /// The workspace will handle opening a terminal with `tail -f` on the log file.
    OpenLspLogs {
        log_path: PathBuf,
    },
    RunTabConfigSkill {
        path: PathBuf,
    },
    DelayedRenderingFlushed,
}

/// Metadata about a file that is opened in the code view.
#[derive(Debug, Clone)]
struct LoadedFileMetadata {
    id: FileId,
    location: BufferFileLocation,
}

pub use super::diff_viewer::DisplayMode;

type TerminalTargetFn = dyn Fn(WindowId, &AppContext) -> Option<ViewHandle<TerminalView>>;

struct SelectionAsContextTooltip {
    mouse_state: MouseStateHandle,
    terminal_target_fn: Box<TerminalTargetFn>,
}

#[derive(Debug, Clone)]
pub enum LocalCodeEditorAction {
    InsertSelectedTextToInput,
    SaveFile,
    DiscardUnsavedChanges,
    NavigateToTarget(FileLocation),
    GotoDefinition,
    FindReferences,
    OpenContextMenu,
    /// Lazily fetch find-references and show the card (triggered on cmd-click when at definition).
    /// This is the fallback when go-to-definition has no different location to navigate to.
    FetchAndShowFindReferences {
        lsp_position: lsp::types::Location,
        anchor_offset: CharOffset,
    },
}

#[derive(Default)]
struct ConflictResolutionBannerMouseStates {
    discard_mouse_state: MouseStateHandle,
    overwrite_mouse_state: MouseStateHandle,
}

#[derive(Default)]
struct ContextMenuState {
    mouse_state: MouseStateHandle,
    is_open: bool,
}

/// A hover content segment - either plain text/markdown or a code block.
pub(super) enum HoverContentSegment {
    /// Plain text/markdown content (rendered with FormattedTextElement).
    Text(FormattedText),
    /// Code block with syntax highlighting (rendered with CodeEditorView).
    CodeBlock { view: ViewHandle<CodeEditorView> },
}

/// State for the LSP hover tooltip.
pub(super) enum LspHoverState {
    None,
    Loading(Option<AbortHandle>),
    Loaded {
        /// The content segments to display in the hover tooltip (from LSP hover info).
        segments: Vec<HoverContentSegment>,
        /// Diagnostics at the hovered location (displayed first, before hover info).
        diagnostics: Vec<ProcessedDiagnostic>,
        /// The offset range where the hover was triggered (used for positioning).
        hovered_offset_range: Range<CharOffset>,
        /// Scroll state for the tooltip content.
        scroll_state: ClippedScrollStateHandle,
        mouse_state: MouseStateHandle,
    },
}

impl LspHoverState {
    pub(super) fn clear(&mut self) -> bool {
        if matches!(self, LspHoverState::None) {
            return false;
        }

        if let Self::Loading(Some(handle)) = self {
            handle.abort();
        }
        *self = LspHoverState::None;
        true
    }

    pub(super) fn contains_offset(&self, offset: CharOffset) -> bool {
        if let LspHoverState::Loaded {
            hovered_offset_range,
            ..
        } = self
        {
            return hovered_offset_range.contains(&offset);
        }

        false
    }
}

pub(super) const HOVER_TOOLTIP_MAX_WIDTH: f32 = 400.;
pub(super) const HOVER_TOOLTIP_MAX_HEIGHT: f32 = 100.;

pub struct LocalCodeEditorView {
    pub(super) editor: ViewHandle<CodeEditorView>,
    metadata: Option<LoadedFileMetadata>,
    enable_diff_nav_by_default: bool,
    is_new_file: bool,
    diff_type: Option<DiffType>,
    selection_as_context_tooltip: Option<SelectionAsContextTooltip>,
    /// A marker for when the backing file has first been loaded. This is used to prevent applying
    /// a diff before it can be properly calculated.
    file_loaded: Condition,
    /// Whether content was changed from its base.
    was_edited: bool,
    /// Content version of the base file state.
    base_content_version: Option<ContentVersion>,
    /// Set to `true` when a `RemoteBufferConflict` event fires for this
    /// editor's buffer. Cleared when the user discards or overwrites.
    has_remote_conflict: bool,
    conflict_banner_mouse_states: ConflictResolutionBannerMouseStates,
    /// Default directory to use for save dialogs when creating new files
    default_directory: Option<PathBuf>,
    pub(super) lsp_server: Option<ModelHandle<LspServerModel>>,
    /// Footer for displaying LSP status. Only created for normal editing contexts, not for diff/review views.
    footer: Option<ViewHandle<CodeFooterView>>,
    /// Context menu for right-click actions.
    context_menu: ViewHandle<Menu<LocalCodeEditorAction>>,
    context_menu_state: ContextMenuState,
    /// Channel for debouncing hover requests.
    hover_debounce_tx: async_channel::Sender<CharOffset>,
    /// State for the LSP hover tooltip.
    pub(super) lsp_hover_state: LspHoverState,
    /// Pending scroll position to apply after the file is loaded. This is used when
    /// `set_pending_scroll` is called before the file content has finished loading
    /// (e.g., in the GlobalBuffer path where content loads asynchronously).
    pending_scroll_on_load: Option<ScrollPosition>,
    /// Cached processed diagnostics. Updated when diagnostics change.
    /// Used as source of truth for both decorations and hover display.
    pub(super) processed_diagnostics: Vec<ProcessedDiagnostic>,
    /// Decorations for LSP diagnostics (errors and warnings).
    pub(super) diagnostic_decorations: Vec<Decoration>,
    /// View for the find references feature.
    find_references_view: Option<ViewHandle<FindReferencesView>>,
}

impl LocalCodeEditorView {
    pub fn new(
        editor: ViewHandle<CodeEditorView>,
        diff_type: Option<DiffType>,
        enable_diff_nav_by_default: bool,
        display_mode: Option<DisplayMode>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let context_menu = ctx.add_typed_action_view(|_| {
            Menu::new()
                .prevent_interaction_with_other_elements()
                .with_drop_shadow()
        });
        ctx.subscribe_to_view(&context_menu, |me, _, event, ctx| {
            me.handle_menu_event(event, ctx);
        });

        ctx.subscribe_to_view(&editor, |me, _, event, ctx| match event {
            CodeEditorEvent::UnifiedDiffComputed(_) => {
                ctx.emit(LocalCodeEditorEvent::DiffAccepted);
            }
            CodeEditorEvent::ContentChanged { origin, .. } => {
                me.update_diff_hunk_gutter_buttons(ctx);

                // Clear cached diagnostics/decorations on any content change. This is to avoid showing stale diagnostics while we are waiting for new diagnostics.
                if !me.processed_diagnostics.is_empty() || !me.diagnostic_decorations.is_empty() {
                    me.clear_diagnostics(ctx);
                }

                if origin.from_user() {
                    me.was_edited = true;
                    ctx.emit(LocalCodeEditorEvent::UserEdited);
                }
            }
            CodeEditorEvent::VimEscapeInNormalMode => {
                if me.dismiss_lsp_overlays(ctx) {
                    ctx.notify();
                } else {
                    ctx.emit(LocalCodeEditorEvent::VimMinimizeRequested);
                }
            }
            CodeEditorEvent::EscapePressed => {
                if me.dismiss_lsp_overlays(ctx) {
                    ctx.notify();
                }
            }
            CodeEditorEvent::DiffUpdated => {
                ctx.emit(LocalCodeEditorEvent::DiffStatusUpdated);
            }
            CodeEditorEvent::SelectionEnd => {
                ctx.notify();
            }
            CodeEditorEvent::MouseHovered {
                offset,
                cmd,
                clamped,
                is_covered,
            } => {
                // If the mouse location is clamped (meaning it's not hovering on an actual buffer text),
                // or if the event is covered by an element above the editor,
                // we should clear the hovered range and symbol.
                //
                // However, if the mouse is over the LSP hover card itself, we should not clear
                // the hover state - the "covered" event is expected when hovering over the tooltip.
                if *clamped || *is_covered {
                    // Check if the mouse is currently over the hover tooltip.
                    let is_over_hover_card =
                        if let LspHoverState::Loaded { mouse_state, .. } = &me.lsp_hover_state {
                            mouse_state
                                .lock()
                                .ok()
                                .is_some_and(|state| state.is_mouse_over_element())
                        } else {
                            false
                        };

                    // If the mouse is over the hover card, don't clear the hover state.
                    if is_over_hover_card {
                        return;
                    }

                    let mut updated = me
                        .editor
                        .update(ctx, |editor, ctx| editor.clear_hovered_symbol_range(ctx));
                    updated = updated || me.lsp_hover_state.clear();

                    if updated {
                        ctx.notify();
                    }

                    return;
                }

                // If hovering with cmd pressed, trigger goto definition search.
                if *cmd {
                    if me.is_lsp_server_available(ctx) {
                        me.definition_for_hovered_range(*offset, ctx);
                    }
                } else {
                    me.editor.update(ctx, |editor, ctx| {
                        if editor.clear_hovered_symbol_range(ctx) {
                            ctx.notify();
                        }
                    });
                    // Queue hover request for documentation/type info (debounced).
                    if me.is_lsp_server_available(ctx) {
                        // Two conditions where we should early return:
                        // 1. The active lsp hover state already contains the hovered offset.
                        // 2. The lsp hover state is None. Meaning a later movement event cancelled the in progress hover.
                        if me.lsp_hover_state.contains_offset(*offset) {
                            return;
                        }

                        if me.lsp_hover_state.clear() {
                            ctx.notify();
                        }

                        let _ = me.hover_debounce_tx.try_send(*offset);
                        me.lsp_hover_state = LspHoverState::Loading(None);
                    }
                }
            }
            CodeEditorEvent::CommentSaved { comment } => {
                ctx.emit(LocalCodeEditorEvent::CommentSaved {
                    comment: comment.clone(),
                });
            }
            CodeEditorEvent::DeleteComment { id } => {
                ctx.emit(LocalCodeEditorEvent::DeleteComment { id: *id });
            }
            CodeEditorEvent::RequestOpenComment(uuid) => {
                ctx.emit(LocalCodeEditorEvent::RequestOpenComment(*uuid));
            }
            CodeEditorEvent::ViewportUpdated => {
                ctx.emit(LocalCodeEditorEvent::ViewportUpdated);
            }
            CodeEditorEvent::LayoutInvalidated => {
                ctx.emit(LocalCodeEditorEvent::LayoutInvalidated);
            }
            CodeEditorEvent::DelayedRenderingFlushed => {
                ctx.emit(LocalCodeEditorEvent::DelayedRenderingFlushed);
            }
            CodeEditorEvent::VimGotoDefinition
            | CodeEditorEvent::VimFindReferences
            | CodeEditorEvent::VimShowHover => {
                if me.dismiss_lsp_overlays(ctx) {
                    ctx.notify();
                }
                match event {
                    CodeEditorEvent::VimGotoDefinition => me.goto_definition_at_cursor(ctx),
                    CodeEditorEvent::VimFindReferences => me.find_references_at_cursor(ctx),
                    CodeEditorEvent::VimShowHover => me.show_hover_at_cursor(ctx),
                    _ => unreachable!(),
                }
            }
            _ => {}
        });

        let is_new_file = matches!(diff_type, Some(DiffType::Create { .. }));

        // Set up debounce for hover requests
        let (hover_debounce_tx, hover_debounce_rx) = async_channel::unbounded();
        ctx.spawn_stream_local(
            debounce(HOVER_DEBOUNCE_PERIOD, hover_debounce_rx),
            |me, offset, ctx| me.hover_for_offset(offset, ctx),
            |_, _| {},
        );

        let model = Self {
            editor,
            diff_type,
            is_new_file,
            metadata: None,
            enable_diff_nav_by_default,
            file_loaded: Condition::new(),
            selection_as_context_tooltip: None,
            was_edited: false,
            base_content_version: None,
            has_remote_conflict: false,
            conflict_banner_mouse_states: Default::default(),
            default_directory: None,
            lsp_server: None,
            footer: None,
            context_menu,
            context_menu_state: Default::default(),
            hover_debounce_tx,
            lsp_hover_state: LspHoverState::None,
            pending_scroll_on_load: None,
            processed_diagnostics: Vec::new(),
            diagnostic_decorations: Vec::new(),
            find_references_view: None,
        };

        if let Some(display_mode) = display_mode {
            model.set_display_mode(display_mode, ctx);
        }
        model
    }

    /// Calls LSP goto_definition and spawns a callback with the result.
    /// Returns true if the LSP call was initiated, false if prerequisites weren't met.
    fn call_goto_definition<F>(
        &self,
        lsp_position: lsp::types::Location,
        callback: F,
        ctx: &mut ViewContext<Self>,
    ) -> bool
    where
        F: FnOnce(
                &mut Self,
                anyhow::Result<Vec<lsp::types::DefinitionLocation>>,
                &mut ViewContext<Self>,
            ) + Send
            + 'static,
    {
        let Some(file_path) = self.file_path() else {
            return false;
        };

        let Some(lsp_server) = &self.lsp_server else {
            return false;
        };

        let future = match lsp_server
            .as_ref(ctx)
            .goto_definition(file_path.to_path_buf(), lsp_position)
        {
            Ok(future) => future,
            Err(e) => {
                log::error!("Failed to call lsp.goto_definition: {e}");
                return false;
            }
        };

        ctx.spawn(future, callback);
        true
    }

    pub fn definition_for_hovered_range(
        &mut self,
        offset: CharOffset,
        ctx: &mut ViewContext<Self>,
    ) {
        // Early return if user is not moving away from the active hovered range.
        let active_hovered_range = self.editor().as_ref(ctx).hovered_symbol_range(ctx);
        if let Some(range) = active_hovered_range {
            if range.contains(&offset) {
                return;
            }
        }

        let lsp_position = self
            .editor()
            .as_ref(ctx)
            .offset_to_lsp_position(offset, ctx);

        if cfg!(debug_assertions) {
            if let (Some(file_path), Some(lsp_server)) = (self.file_path(), &self.lsp_server) {
                let buffer_version = self.editor().as_ref(ctx).buffer_version(ctx).as_usize();
                lsp_server.as_ref(ctx).log_to_server_log(
                    lsp::LspServerLogLevel::Info,
                    format!(
                        "lsp-sync: gotoDefinition -> server file={} buffer_version={buffer_version} position={}:{}",
                        file_path.display(),
                        lsp_position.line,
                        lsp_position.column,
                    ),
                );
            }
        }

        // Only fetch definition on hover (fast path).
        // Find references is fetched lazily on click as a fallback when at the definition.
        // This follows Zed's approach: cmd-hover shows definition link quickly,
        // and find-references is only fetched when the user clicks and there's no
        // different definition to navigate to.
        self.fetch_definition_for_hover(offset, lsp_position.clone(), ctx);
    }

    /// Fetches goto definition for cmd-hover underline.
    /// On cmd-click: if definition exists and is different from current position, navigate to it.
    /// Otherwise, trigger a lazy find-references fetch.
    fn fetch_definition_for_hover(
        &mut self,
        offset: CharOffset,
        lsp_position: lsp::types::Location,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(file_path) = self.file_path().map(|p| p.to_path_buf()) else {
            return;
        };

        let lsp_position_for_references = lsp_position.clone();
        self.call_goto_definition(
            lsp_position,
            move |me, definition_result, ctx| {
                // Handle definition result - log errors and clear hover on failure
                let definition_locations = match definition_result {
                    Ok(locations) => locations,
                    Err(e) => {
                        log::debug!("Failed to get goto definition: {e}");
                        return;
                    }
                };

                // If no locations returned, clear the hovered symbol range
                if definition_locations.is_empty() {
                    me.editor.update(ctx, |editor, ctx| {
                        if editor.clear_hovered_symbol_range(ctx) {
                            ctx.notify();
                        }
                    });
                    return;
                }

                // Determine highlight range (we know definition_locations is non-empty)
                let editor = me.editor.as_ref(ctx);
                let location = definition_locations.first().unwrap();
                let highlight_range = match &location.origin {
                    Some(range) => {
                        let start = editor.lsp_location_to_offset(&range.start, ctx);
                        let end = editor.lsp_location_to_offset(&range.end, ctx);
                        start.saturating_sub(&CharOffset::from(1))
                            ..end.saturating_sub(&CharOffset::from(1))
                    }
                    None => editor
                        .word_range_at_offset(offset, ctx)
                        .map(|range| {
                            range.start.saturating_sub(&CharOffset::from(1))
                                ..range.end.saturating_sub(&CharOffset::from(1))
                        })
                        .unwrap_or_else(|| offset..offset + 1),
                };

                // Get the LSP position of the hovered offset for comparison
                let hovered_lsp_line = editor.offset_to_lsp_position(offset, ctx).line;

                // Check if definition points to a different location (not the hovered position)
                let has_different_definition = definition_locations.first().is_some_and(|loc| {
                    // Definition is "different" if it's in a different file OR at a different line
                    loc.target.path != file_path || loc.target.location.line != hovered_lsp_line
                });

                let view_id = ctx.view_id();
                let window_id = ctx.window_id();

                // Create the on-click action based on whether we have a definition
                let on_click: Box<dyn Fn(&mut warpui::AppContext)> = if has_different_definition {
                    let target_location = definition_locations.first().unwrap().target.clone();
                    Box::new(move |app| {
                        app.dispatch_typed_action_for_view(
                            window_id,
                            view_id,
                            &LocalCodeEditorAction::NavigateToTarget(target_location.clone()),
                        );
                    })
                } else {
                    // No different definition - on click, lazily fetch find-references as fallback
                    let anchor_offset = highlight_range.start;
                    Box::new(move |app| {
                        app.dispatch_typed_action_for_view(
                            window_id,
                            view_id,
                            &LocalCodeEditorAction::FetchAndShowFindReferences {
                                lsp_position: lsp_position_for_references.clone(),
                                anchor_offset,
                            },
                        );
                    })
                };

                // Set up the hoverable link
                let link = HoverableLink::new(highlight_range).with_on_click(on_click);

                me.editor.update(ctx, |editor, ctx| {
                    if editor.set_hovered_symbol_range(Some(link), ctx) {
                        ctx.notify();
                    }
                });
            },
            ctx,
        );
    }

    /// Lazily fetches find-references and shows the card.
    /// This is called when cmd-clicking on a symbol at its definition
    /// (where go-to-definition has nowhere to navigate to).
    fn fetch_find_references_and_show(
        &mut self,
        lsp_position: lsp::types::Location,
        anchor_offset: CharOffset,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(file_path) = self.file_path().map(|p| p.to_path_buf()) else {
            return;
        };

        let Some(lsp_server) = &self.lsp_server else {
            return;
        };

        let references_future = match lsp_server
            .as_ref(ctx)
            .find_references(file_path, lsp_position)
        {
            Ok(future) => future,
            Err(e) => {
                log::info!("Failed to call lsp.find_references: {e}");
                return;
            }
        };

        ctx.spawn(references_future, move |me, references_result, ctx| {
            let references = references_result.ok().unwrap_or_default();

            if references.is_empty() {
                return; // No references found, nothing to show
            }

            // Store the references (view will render automatically when present)
            me.store_find_references_results(references, anchor_offset, ctx);
            ctx.notify();
        });
    }

    /// Stores find references results and prepares the view for display.
    fn store_find_references_results(
        &mut self,
        references: Vec<ReferenceLocation>,
        request_offset: CharOffset,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(server) = &self.lsp_server {
            send_telemetry_from_ctx!(
                LspTelemetryEvent::FindReferencesShown {
                    server_type: server.as_ref(ctx).server_name(),
                    num_references: references.len(),
                },
                ctx
            );
        }

        // Get workspace root for relative path display from the LSP server
        let workspace_root = self
            .lsp_server
            .as_ref()
            .map(|server| server.as_ref(ctx).initial_workspace().to_path_buf());

        // Create the view with editor views as a TypedActionView
        let view = ctx.add_typed_action_view(|ctx| {
            FindReferencesView::new(references, workspace_root, request_offset, ctx)
        });

        // Focus the view to enable keyboard navigation
        ctx.focus(&view);

        // Subscribe to view events
        ctx.subscribe_to_view(&view, |me, _, event, ctx| match event {
            FindReferencesViewEvent::GotoReference(index) => {
                let Some(source_server_id) = me.lsp_server.as_ref().map(|s| s.as_ref(ctx).id())
                else {
                    log::debug!("No LSP server available for navigate to reference");
                    return;
                };

                if let Some(ref_view) = &me.find_references_view {
                    if let Some(reference) = ref_view.as_ref(ctx).get_reference(*index) {
                        ctx.emit(LocalCodeEditorEvent::GotoDefinition {
                            path: reference.file_path.clone(),
                            line: reference.line_number.saturating_sub(1), // Convert 1-based to 0-based
                            column: reference.column,
                            source_server_id,
                        });
                        // Close the card after navigation
                        me.find_references_view = None;
                        me.editor.update(ctx, |editor, _ctx| {
                            editor.set_find_references_anchor_offset(None);
                        });
                        ctx.notify();
                    }
                }
            }
            FindReferencesViewEvent::CloseRequested => {
                me.close_find_references_card(ctx);
                ctx.notify();
            }
        });

        self.find_references_view = Some(view);
        // Set the anchor offset on the editor so it can cache the gutter position
        self.editor.update(ctx, |editor, _ctx| {
            editor.set_find_references_anchor_offset(Some(request_offset));
        });
        // Don't notify here - the card should only show on cmd-click
    }

    /// Whether the local editor has a corresponding enabled LSP.
    pub fn language_server_enabled(&self) -> bool {
        self.lsp_server.is_some()
    }

    /// Helper function to compute card positioning for overlays at a given offset.
    /// Positions the card above the offset if there's space, otherwise below.
    fn compute_card_positioning(
        &self,
        offset: CharOffset,
        max_card_height: f32,
        app: &AppContext,
    ) -> Option<OffsetPositioning> {
        // Get the bounds of the offset in viewport coordinates.
        let bounds = self
            .editor
            .as_ref(app)
            .character_bounds_in_viewport(offset, app)?;

        // Check if there's enough space above the symbol to render the card.
        // If not, position it below the symbol instead.
        let has_space_above = bounds.origin_y() >= max_card_height;

        if has_space_above {
            // Position the card above the symbol (bottom of card at top of symbol).
            Some(OffsetPositioning::offset_from_parent(
                Vector2F::new(bounds.origin_x(), bounds.origin_y()),
                ParentOffsetBounds::ParentByPosition,
                ParentAnchor::TopLeft,
                ChildAnchor::BottomLeft,
            ))
        } else {
            // Position the card below the symbol (top of card at bottom of symbol).
            Some(OffsetPositioning::offset_from_parent(
                Vector2F::new(bounds.origin_x(), bounds.max_y()),
                ParentOffsetBounds::ParentByPosition,
                ParentAnchor::TopLeft,
                ChildAnchor::TopLeft,
            ))
        }
    }

    /// Get the positioning for the hover tooltip based on the hovered symbol position.
    fn hover_tooltip_positioning(&self, app: &AppContext) -> Option<OffsetPositioning> {
        let offset_start = match &self.lsp_hover_state {
            LspHoverState::Loaded {
                hovered_offset_range,
                ..
            } => hovered_offset_range.start,
            _ => return None,
        };

        self.compute_card_positioning(offset_start, HOVER_TOOLTIP_MAX_HEIGHT, app)
    }

    /// Get the positioning for the find references card based on the request offset.
    /// Anchors horizontally at the gutter start, positioned below the anchor line.
    /// Returns None if the anchor is outside the visible viewport area.
    fn find_references_card_positioning(&self, app: &AppContext) -> Option<OffsetPositioning> {
        let view = self.find_references_view.as_ref()?;
        let offset = view.as_ref(app).request_offset();

        // Get the bounds of the offset in viewport coordinates.
        let bounds = self
            .editor
            .as_ref(app)
            .character_bounds_in_viewport(offset, app)?;

        // Get the viewport height to check if the anchor is within the visible area.
        // The bounds are viewport-relative (scroll offset subtracted), so:
        // - origin_y < 0 means the anchor is above the viewport
        // - origin_y > viewport_height means the anchor is below the viewport
        let viewport_height = self
            .editor
            .as_ref(app)
            .viewport_height(app)
            .unwrap_or(f32::MAX);

        // Check if the anchor is within the visible viewport area.
        // We check if the bottom of the anchor line (max_y) is visible.
        if bounds.origin_y() > viewport_height || bounds.max_y() < 0.0 {
            // Anchor is outside the viewport, don't show the card.
            return None;
        }

        // Position the card at the gutter start (x=0), below the symbol.
        // Use ParentByPosition so the card spans the full width of the editor.
        Some(OffsetPositioning::offset_from_parent(
            Vector2F::new(0., bounds.max_y()),
            ParentOffsetBounds::ParentByPosition,
            ParentAnchor::TopLeft,
            ChildAnchor::TopLeft,
        ))
    }

    fn try_connect_lsp_server(&mut self, ctx: &mut ViewContext<Self>) {
        if self.lsp_server.is_some() {
            return;
        }
        let lsp_manager = LspManagerModel::handle(ctx);
        let Some(path) = self.file_path().map(|p| p.to_path_buf()) else {
            return;
        };

        let Some(lsp_server) = lsp_manager.as_ref(ctx).server_for_path(&path, ctx) else {
            // If the LSP is not registered, try to start it via PersistedWorkspace.
            #[cfg(feature = "local_fs")]
            {
                use crate::ai::persisted_workspace::LspTask;
                PersistedWorkspace::handle(ctx).update(ctx, |workspace, ctx| {
                    workspace.execute_lsp_task(LspTask::Spawn { file_path: path }, ctx);
                });
            }
            return;
        };

        // Connect footer and subscribe to server events BEFORE attempting to open the document.
        // This ensures the footer shows the correct state (including Failed) regardless of
        // whether document opening succeeds.
        if let Some(footer) = &self.footer {
            footer.update(ctx, |footer, ctx| {
                footer.subscribe_to_server_events(&lsp_server, ctx)
            });
        }

        // Subscribe to LSP server events for diagnostics updates.
        ctx.subscribe_to_model(&lsp_server, |me, _, event, ctx| {
            if let LspEvent::DiagnosticsUpdated { path: updated_path } = event {
                if let Some(file_path) = me.file_path() {
                    if file_path == updated_path {
                        me.refresh_diagnostics(ctx);
                    }
                }
            }
        });

        // Store the server reference.
        self.lsp_server = Some(lsp_server.clone());

        // Load initial diagnostics. The document open (didOpen) is handled by
        // GlobalBufferModel, which owns the full LSP document lifecycle.
        self.refresh_diagnostics(ctx);
    }

    /// Subscribe to manager event updates so we could automatically connect when a server is started.
    fn subscribe_to_lsp_manager_updates(&self, ctx: &mut ViewContext<Self>) {
        let lsp_manager = LspManagerModel::handle(ctx);
        ctx.subscribe_to_model(&lsp_manager, |me, _, event, ctx| {
            let Some(file_path) = me.file_path() else {
                return;
            };
            match event {
                LspManagerModelEvent::ServerStarted(path) if file_path.starts_with(path) => {
                    // Make sure we don't connect lsp server when the file content hasn't been loaded yet.
                    if me.lsp_server.is_none() && me.base_content_version.is_some() {
                        me.try_connect_lsp_server(ctx);
                    }
                    ctx.notify();
                }
                LspManagerModelEvent::ServerStopped(path) if file_path.starts_with(path) => {
                    ctx.notify();
                }
                LspManagerModelEvent::ServerRemoved { server_id, .. } => {
                    // Check if the removed server matches our current server by unique ID
                    let matches = me
                        .lsp_server
                        .as_ref()
                        .map(|s| s.as_ref(ctx).id() == *server_id)
                        .unwrap_or(false);
                    if matches {
                        // Clear our reference to the removed server
                        me.lsp_server = None;
                        // Tell footer to clear its server subscription
                        if let Some(footer) = &me.footer {
                            footer.update(ctx, |footer, ctx| {
                                footer.clear_server_subscription(ctx);
                            });
                        }
                        ctx.notify();
                    }
                }
                _ => (),
            }
        });
    }

    fn is_lsp_server_available(&self, ctx: &mut ViewContext<Self>) -> bool {
        self.lsp_server
            .as_ref()
            .map(|server| server.as_ref(ctx).is_ready_for_requests())
            .unwrap_or(false)
    }

    fn format_and_save(&mut self, file_id: FileId, ctx: &mut ViewContext<Self>) {
        let Some(lsp_server) = &self.lsp_server else {
            self.perform_save(file_id, ctx);
            return;
        };

        let Some(file_path) = self.file_path() else {
            self.perform_save(file_id, ctx);
            return;
        };

        if !lsp_server.as_ref(ctx).is_ready_for_requests() {
            self.perform_save(file_id, ctx);
            return;
        };

        let file_path_for_format = file_path.to_path_buf();

        // Now proceed with formatting
        let Some(lsp_server) = &self.lsp_server else {
            self.perform_save(file_id, ctx);
            return;
        };

        let (tab_size, insert_spaces) = match self.editor.as_ref(ctx).indent_unit(ctx) {
            IndentUnit::Tab => (1, false),
            IndentUnit::Space(num) => (num, true),
        };

        // Create default formatting options
        let formatting_options = FormattingOptions {
            tab_size: tab_size as u32,
            insert_spaces,
            properties: Default::default(),
            // TODO: These should eventually come from user settings.
            trim_trailing_whitespace: Some(true),
            insert_final_newline: Some(true),
            trim_final_newlines: Some(true),
        };

        let format_future = match lsp_server
            .as_ref(ctx)
            .format_document(file_path_for_format, formatting_options)
        {
            Ok(future) => future,
            Err(e) => {
                log::warn!("Failed to request document formatting: {e}");
                self.perform_save(file_id, ctx);
                return;
            }
        };

        ctx.spawn(format_future, move |me, result, ctx| {
            match result {
                Ok(Some(text_edits)) => {
                    me.editor.update(ctx, |editor, ctx| {
                        if text_edits.is_empty() {
                            return;
                        }

                        // Convert LSP text edits to editor-friendly format
                        let mut edits: Vec<(String, Range<CharOffset>)> = Vec::new();

                        for text_edit in text_edits {
                            // Convert LSP positions to CharOffsets
                            let start_offset =
                                editor.lsp_location_to_offset(&text_edit.range.start, ctx);
                            let end_offset =
                                editor.lsp_location_to_offset(&text_edit.range.end, ctx);

                            if start_offset > end_offset {
                                log::warn!("Received invalid formatting range from language server {:?}..{:?}", text_edit.range.start, text_edit.range.end);
                                continue;
                            }

                            edits.push((text_edit.text, start_offset..end_offset));
                        }

                        // Sort edits by start position in reverse order to avoid offset shifting issues
                        edits.sort_by(|a, b| b.1.start.cmp(&a.1.start));

                        if let Ok(edits) = Vec1::try_from_vec(edits) {
                            editor.apply_edits(edits, ctx);
                        }
                    });
                }
                Ok(None) => {
                    log::debug!("LSP server doesn't support formatting for this document");
                }
                Err(e) => {
                    log::warn!("Document formatting failed: {e}");
                }
            }
            // After formatting (or if formatting failed), proceed with save
            me.perform_save(file_id, ctx);
        });
    }

    fn perform_save(&mut self, file_id: FileId, ctx: &mut ViewContext<Self>) {
        self.base_content_version = Some(self.editor.as_ref(ctx).version(ctx));

        let result = match self.diff() {
            Some(DiffType::Update {
                rename: Some(new_path),
                ..
            }) => self.editor.update(ctx, |editor, ctx| {
                let content = editor.text(ctx);
                let buffer_version = editor.version(ctx);

                GlobalBufferModel::handle(ctx).update(ctx, move |model, ctx| {
                    model.rename_and_save(
                        file_id,
                        new_path.clone(),
                        content.into_string(),
                        buffer_version,
                        ctx,
                    )
                })
            }),
            Some(DiffType::Delete { .. }) => self.editor.update(ctx, |editor, ctx| {
                let buffer_version = editor.version(ctx);
                GlobalBufferModel::handle(ctx).update(ctx, move |model, ctx| {
                    model.delete(file_id, buffer_version, ctx)
                })
            }),
            _ => self.editor.update(ctx, |editor, ctx| {
                let content = editor.text(ctx);
                let buffer_version = editor.version(ctx);

                GlobalBufferModel::handle(ctx).update(ctx, move |model, ctx| {
                    model.save(file_id, content.into_string(), buffer_version, ctx)
                })
            }),
        };

        if let Err(err) = result {
            log::error!("Failed to save file: {err:?}");
            ctx.emit(LocalCodeEditorEvent::FailedToSave {
                error: Rc::new(err),
            });
        }
    }

    pub fn is_new_file(&self) -> bool {
        self.is_new_file
    }

    // This is a footgun - we should remove this codepath.
    pub fn set_new_file(&mut self, is_new: bool) {
        self.is_new_file = is_new;
    }

    pub fn set_default_directory(&mut self, directory: Option<PathBuf>) {
        self.default_directory = directory;
    }

    pub fn reset_with_state(&mut self, state: InitialBufferState, ctx: &mut ViewContext<Self>) {
        self.base_content_version = Some(state.version);
        self.editor
            .update(ctx, |editor, ctx| editor.reset(state, ctx));
    }

    /// Whether the content of the source file this editor is based on has been loaded into the buffer.
    pub fn file_loaded(&self, ctx: &mut ViewContext<Self>) -> bool {
        // For global buffer, we could have utilized a shared buffer from another open editor. To avoid
        // any race condition, we directly check with the GlobalBufferModel rather than relying on self.base_content_version
        // which is updated via an async event.
        let Some(file_id) = self.file_id() else {
            return false;
        };

        GlobalBufferModel::as_ref(ctx).buffer_loaded(file_id)
    }

    /// Construct a new editor view with a shared buffer backed by the given location.
    ///
    /// For local files, sets the language from the file path and wires up LSP.
    /// For remote files, sets the language from the extension and skips
    /// local-only wiring (LSP, footer).
    pub fn new_with_global_buffer<T>(
        location: BufferFileLocation,
        editor_constructor: T,
        enable_diff_nav_by_default: bool,
        display_mode: Option<DisplayMode>,
        ctx: &mut ViewContext<Self>,
    ) -> Self
    where
        T: FnOnce(BufferState, &mut ViewContext<Self>) -> ViewHandle<CodeEditorView>,
    {
        let buffer_state = GlobalBufferModel::handle(ctx)
            .update(ctx, |model, ctx| model.open(location.clone(), ctx));
        let file_id = buffer_state.file_id;
        let editor = editor_constructor(buffer_state, ctx);

        match &location {
            BufferFileLocation::Local(path) => {
                editor.update(ctx, |editor, ctx| {
                    editor.set_language_with_path(path, ctx);
                    editor.model.update(ctx, |model, ctx| {
                        model.rebuild_layout_with_syntax_highlighting(ctx)
                    });
                });
            }
            BufferFileLocation::Remote(remote_path) => {
                if let Some(ext) = remote_path.path.extension() {
                    editor.update(ctx, |editor, ctx| {
                        editor.set_language_with_name(ext, ctx);
                        editor.model.update(ctx, |model, ctx| {
                            model.rebuild_layout_with_syntax_highlighting(ctx)
                        });
                    });
                }
            }
        }

        let mut local_editor =
            Self::new(editor, None, enable_diff_nav_by_default, display_mode, ctx);

        local_editor.metadata = Some(LoadedFileMetadata {
            id: file_id,
            location,
        });

        Self::subscribe_to_global_buffer_events(file_id, ctx);

        local_editor
    }

    pub fn set_pending_scroll(&mut self, position: ScrollPosition, ctx: &mut ViewContext<Self>) {
        // If the file is already loaded, we can set the scroll trigger immediately with the
        // current buffer version. Otherwise, store it and apply when the file finishes loading.
        // This handles the race condition where set_pending_scroll is called before the file
        // content has been loaded (e.g., in the GlobalBuffer path).
        if self.file_loaded(ctx) {
            self.editor.update(ctx, |editor, ctx| {
                let version = editor.buffer_version(ctx);
                editor.set_pending_scroll(ScrollTrigger::new(position, version));
            });
        } else {
            self.pending_scroll_on_load = Some(position);
        }
    }

    fn on_file_loaded(&mut self, ctx: &mut ViewContext<Self>) {
        self.apply_diffs_if_any(ctx);
        self.file_loaded.set();

        // Apply any pending scroll position that was set before the file finished loading.
        if let Some(position) = self.pending_scroll_on_load.take() {
            self.editor.update(ctx, |editor, ctx| {
                let version = editor.buffer_version(ctx);
                editor.set_pending_scroll(ScrollTrigger::new(position, version));
            });
        }
    }

    /// Updates the enablement state of the visible "add as context" gutter button based on the file state.
    /// If the button is hidden to begin with, this is a no-op.
    pub fn update_diff_hunk_gutter_buttons(&self, ctx: &mut ViewContext<Self>) {
        let has_unsaved_changes = self.has_unsaved_changes(ctx);
        let enabled = !has_unsaved_changes;
        self.editor.update(ctx, |code_editor, ctx| {
            code_editor.set_add_diff_hunk_as_context_button(enabled, ctx);
        });
    }

    pub fn has_unsaved_changes(&self, ctx: &AppContext) -> bool {
        if self.is_new_file {
            let text = self.editor.as_ref(ctx).text(ctx);
            if text.as_str().is_empty() {
                return false;
            }
        }

        self.base_content_version
            .map(|base_version| !self.editor.as_ref(ctx).version_match(&base_version, ctx))
            .unwrap_or(false)
    }

    /// Enables the selection-as-context tooltip. For now, we only want this to be rendered within editors in code panes.
    pub(crate) fn with_selection_as_context(
        mut self,
        terminal_target_fn: Box<TerminalTargetFn>,
    ) -> Self {
        self.selection_as_context_tooltip = Some(SelectionAsContextTooltip {
            mouse_state: Default::default(),
            terminal_target_fn,
        });
        self
    }

    /// Sets the find references card provider on the underlying editor.
    pub(crate) fn with_find_references_provider(
        self,
        provider: impl ShowFindReferencesCardProvider,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        self.editor.update(ctx, |editor, _ctx| {
            editor.set_show_find_references_provider(provider);
        });
        self
    }

    /// Adds the LSP status footer to the editor view.
    pub(crate) fn add_footer(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(path) = self.file_path() {
            let footer =
                ctx.add_typed_action_view(|ctx| CodeFooterView::new(path.to_path_buf(), ctx));
            ctx.subscribe_to_view(&footer, |_, _, event, ctx| match event {
                CodeFooterViewEvent::RunTabConfigSkill { path } => {
                    ctx.emit(LocalCodeEditorEvent::RunTabConfigSkill { path: path.clone() });
                }
                CodeFooterViewEvent::EnableLSP { path, .. } => {
                    Self::enable_lsp_for_path(path, ctx);
                }
                CodeFooterViewEvent::InstallAndEnableLSP { path, .. } => {
                    Self::install_and_enable_lsp_for_path(path, ctx);
                }
                CodeFooterViewEvent::OpenLogs { path } => {
                    Self::open_lsp_logs_for_path(path, ctx);
                }
                CodeFooterViewEvent::RestartServer { server } => {
                    server.update(ctx, |server, ctx| {
                        server.restart(ctx);
                    });
                }
                CodeFooterViewEvent::StopServer { server } => {
                    server.update(ctx, |server, ctx| {
                        let _ = server.stop(true, ctx);
                    });
                }
                CodeFooterViewEvent::StartServer { server } => {
                    server.update(ctx, |server, ctx| {
                        let _ = server.manual_start(ctx);
                    });
                }
                CodeFooterViewEvent::RestartAllServers { .. }
                | CodeFooterViewEvent::StopAllServers { .. }
                | CodeFooterViewEvent::StartAllServers { .. }
                | CodeFooterViewEvent::ManageServers => {}
            });

            // Subscribe to PersistedWorkspace events for LSP installation completion
            #[cfg(feature = "local_fs")]
            {
                ctx.subscribe_to_model(
                    &PersistedWorkspace::handle(ctx),
                    move |me, _, event, ctx| {
                        Self::handle_persisted_workspace_event(me, event, ctx);
                    },
                );
            }

            self.footer = Some(footer);
        }
    }

    /// Handles PersistedWorkspaceEvent for LSP installation completion.
    /// Note: Toast notifications are handled directly by PersistedWorkspace.
    #[cfg(feature = "local_fs")]
    fn handle_persisted_workspace_event(
        me: &mut Self,
        event: &PersistedWorkspaceEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            PersistedWorkspaceEvent::InstallationSucceeded
            | PersistedWorkspaceEvent::InstallationFailed => {
                // PersistedWorkspace handles spawning the server after install;
                // we only need to refresh the footer UI here.
                if let Some(footer) = &me.footer {
                    footer.update(ctx, |_, ctx| {
                        ctx.notify();
                    });
                }
            }
            _ => {}
        }
    }

    /// Enables LSP for the given file path by:
    /// 1. Determining the language from the file extension
    /// 2. Finding the appropriate LSP server type
    /// 3. Getting the repository root from DetectedRepositories
    /// 4. Enabling the LSP server in PersistedWorkspace
    /// 5. Starting the LSP server via PersistedWorkspace
    #[cfg(feature = "local_fs")]
    fn enable_lsp_for_path(path: &Path, ctx: &mut ViewContext<Self>) {
        use crate::ai::persisted_workspace::LspTask;

        // Get the language ID from the file path
        let Some(language_id) = LanguageId::from_path(path) else {
            log::warn!("Enable lsp for path should only work for supported file paths");
            return;
        };

        // Find the appropriate LSP server type for this language
        let lsp_server_type = language_id.server_type();

        // Get the repository root from PersistedWorkspace.
        // If it doesn't exist, try to get it from DetectedRepositories.
        // If it also doesn't exist in DetectedRepositories, use the parent path.
        let repo_root = if let Some(workspace_root) =
            PersistedWorkspace::as_ref(ctx).root_for_workspace(path)
        {
            Some(workspace_root.to_path_buf())
        } else {
            match DetectedRepositories::as_ref(ctx)
                .get_root_for_path(&LocalOrRemotePath::Local(path.to_path_buf()))
                .and_then(|r| PathBuf::try_from(r).ok())
            {
                Some(root) => Some(root),
                None => path.parent().map(|s| s.to_path_buf()), // If we can't find root, treat the parent as the root.
            }
        };

        let Some(repo_root) = repo_root else {
            return;
        };

        // Enable and start the LSP server via PersistedWorkspace
        let path = path.to_path_buf();
        PersistedWorkspace::handle(ctx).update(ctx, |workspace, ctx| {
            workspace.enable_lsp_server_for_path(&repo_root, lsp_server_type);
            workspace.execute_lsp_task(LspTask::Spawn { file_path: path }, ctx);
        });
    }

    /// Installs the LSP server and then enables it for the given file path.
    /// This delegates to PersistedWorkspace which handles the async installation
    /// and emits events that are handled by handle_persisted_workspace_event.
    #[cfg(feature = "local_fs")]
    fn install_and_enable_lsp_for_path(path: &Path, ctx: &mut ViewContext<Self>) {
        use crate::ai::persisted_workspace::LspTask;

        let Some(language_id) = LanguageId::from_path(path) else {
            log::warn!("Install and enable lsp for path should only work for supported file paths");
            return;
        };

        let lsp_server_type = language_id.server_type();
        let path = path.to_path_buf();

        let repo_root = if let Some(workspace_root) =
            PersistedWorkspace::as_ref(ctx).root_for_workspace(&path)
        {
            Some(workspace_root.to_path_buf())
        } else {
            match DetectedRepositories::as_ref(ctx)
                .get_root_for_path(&LocalOrRemotePath::Local(path.to_path_buf()))
                .and_then(|r| PathBuf::try_from(r).ok())
            {
                Some(root) => Some(root),
                None => path.parent().map(|s| s.to_path_buf()),
            }
        };

        let Some(repo_root) = repo_root else {
            return;
        };

        // Delegate to PersistedWorkspace which uses interactive PATH and emits events
        PersistedWorkspace::handle(ctx).update(ctx, |workspace, ctx| {
            workspace.execute_lsp_task(
                LspTask::Install {
                    file_path: path,
                    repo_root,
                    server_type: lsp_server_type,
                },
                ctx,
            );
        });
    }

    /// Opens the LSP log file in a terminal pane using `tail -f`.
    /// Emits an event that bubbles up to Workspace which handles opening the terminal.
    #[cfg(feature = "local_fs")]
    fn open_lsp_logs_for_path(path: &Path, ctx: &mut ViewContext<Self>) {
        // Get the language ID from the file path
        let Some(language_id) = LanguageId::from_path(path) else {
            log::warn!(
                "Could not determine language ID for path: {}",
                path.display()
            );
            return;
        };

        // Get the workspace root from LspManagerModel (canonical source for running servers)
        let lsp_manager = LspManagerModel::handle(ctx);
        let lsp_manager_ref = lsp_manager.as_ref(ctx);

        // Find the LSP server for this path and get its workspace root
        let repo_root = lsp_manager_ref
            .server_for_path(path, ctx)
            .map(|server| server.as_ref(ctx).initial_workspace().to_path_buf());

        let Some(repo_root) = repo_root else {
            log::warn!(
                "Could not determine workspace root for path: {}",
                path.display()
            );
            return;
        };

        // Compute the log file path (the log file is created by LspLogger when the server starts)
        let lsp_server_type = language_id.server_type();
        let log_path = crate::code::lsp_logs::log_file_path(lsp_server_type, &repo_root);

        // Emit event to bubble up to Workspace
        ctx.emit(LocalCodeEditorEvent::OpenLspLogs { log_path });
    }

    /// Unsubscribes from any existing GlobalBufferModel subscription and sets up a
    /// new one for the given `file_id`.  Handles BufferLoaded, FailedToLoad,
    /// BufferUpdatedFromFileEvent, FileSaved, and FailedToSave events.
    fn subscribe_to_global_buffer_events(file_id: FileId, ctx: &mut ViewContext<Self>) {
        ctx.unsubscribe_to_model(&GlobalBufferModel::handle(ctx));
        ctx.subscribe_to_model(&GlobalBufferModel::handle(ctx), move |me, _, event, ctx| {
            if event.file_id() != file_id {
                return;
            }
            me.update_diff_hunk_gutter_buttons(ctx);
            match event {
                GlobalBufferModelEvent::BufferLoaded {
                    content_version, ..
                } => {
                    // For a reopen (discard), base_content_version is already
                    // set from the initial load. Accept the new version and
                    // clear any conflict flag.
                    me.has_remote_conflict = false;
                    if me.base_content_version.is_some() {
                        me.base_content_version = Some(*content_version);
                        ctx.notify();
                        return;
                    }
                    me.base_content_version = Some(*content_version);
                    me.subscribe_to_lsp_manager_updates(ctx);
                    me.try_connect_lsp_server(ctx);
                    me.on_file_loaded(ctx);
                    ctx.emit(LocalCodeEditorEvent::FileLoaded);
                }
                GlobalBufferModelEvent::FailedToLoad { error, .. } => {
                    me.is_new_file = true;
                    me.on_file_loaded(ctx);
                    ctx.emit(LocalCodeEditorEvent::FailedToLoad {
                        error: error.clone(),
                    });
                }
                GlobalBufferModelEvent::BufferUpdatedFromFileEvent {
                    success,
                    content_version,
                    ..
                } => {
                    if !*success {
                        ctx.notify();
                    } else {
                        me.base_content_version = Some(*content_version);
                    }
                }
                GlobalBufferModelEvent::FileSaved { .. } => {
                    me.has_remote_conflict = false;
                    ctx.emit(LocalCodeEditorEvent::FileSaved);
                }
                GlobalBufferModelEvent::FailedToSave { error, .. } => {
                    me.base_content_version = GlobalBufferModel::as_ref(ctx).base_version(file_id);
                    ctx.emit(LocalCodeEditorEvent::FailedToSave {
                        error: error.clone(),
                    });
                }
                GlobalBufferModelEvent::RemoteBufferConflict { .. } => {
                    me.has_remote_conflict = true;
                    ctx.notify();
                }
                GlobalBufferModelEvent::ServerLocalBufferUpdated { .. } => {
                    // Not relevant for local code editors.
                }
            }
        });
    }

    pub fn has_version_conflicts(&self, app: &AppContext) -> bool {
        // Remote buffers use SyncClock for conflict detection.
        // The flag is set by the RemoteBufferConflict event handler.
        if matches!(self.file_location(), Some(BufferFileLocation::Remote(_))) {
            return self.has_remote_conflict;
        }
        let Some(file_id) = self.file_id() else {
            return false;
        };
        self.has_unsaved_changes(app)
            && self.base_content_version != GlobalBufferModel::as_ref(app).base_version(file_id)
    }
    /// Save the file to the local file system (or remotely via the remote server).
    /// This will only return an error immediately if there is a failure in the sync part of the call.
    /// Other errors could be returned asynchronously via the FileModelEvent::FailedToSave event.
    pub fn save_local(&mut self, ctx: &mut ViewContext<Self>) -> Result<(), ImmediateSaveError> {
        let Some(file_id) = self.file_id() else {
            return Err(ImmediateSaveError::NoFileId);
        };

        // Always attempt to format before saving if LSP is available
        self.format_and_save(file_id, ctx);
        Ok(())
    }

    /// Open a save dialog to save the file with a new name, optionally with a completion callback.
    pub fn save_as(&mut self, callback: Option<SaveCallback>, ctx: &mut ViewContext<Self>) {
        ctx.open_save_file_picker(
            move |path_opt, me, ctx| Self::handle_save_as(callback, path_opt, me, ctx),
            if let Some(default_dir) = &self.default_directory {
                SaveFilePickerConfiguration::new().with_default_directory(default_dir.clone())
            } else {
                SaveFilePickerConfiguration::new()
            },
        );
    }

    fn handle_save_as(
        callback: Option<SaveCallback>,
        path_opt: Option<String>,
        me: &mut Self,
        ctx: &mut ViewContext<Self>,
    ) {
        let callback = callback.unwrap_or(Box::new(|_, _| {}));
        let Some(path_str) = path_opt else {
            callback(SaveOutcome::Canceled, ctx);
            return;
        };
        let path = PathBuf::from(path_str);

        // Ensure parent directories exist before registering file watcher / LSP.
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                let _ = std::fs::create_dir_all(parent);
            }
        }

        let buffer = me.editor.as_ref(ctx).model.as_ref(ctx).buffer().clone();
        let buffer_state = GlobalBufferModel::handle(ctx)
            .update(ctx, |model, ctx| model.register(path.clone(), buffer, ctx));

        let file_id = buffer_state.file_id;
        me.metadata = Some(LoadedFileMetadata {
            id: file_id,
            location: BufferFileLocation::Local(path.clone()),
        });

        me.set_new_file(false);

        me.editor.update(ctx, |editor, ctx| {
            editor.set_language_with_path(&path, ctx);
        });

        let content = me.editor.as_ref(ctx).text(ctx).into_string();
        let buffer_version = me.editor.as_ref(ctx).version(ctx);

        me.base_content_version = Some(buffer_version);
        let save_outcome = if let Err(err) = GlobalBufferModel::handle(ctx)
            .update(ctx, move |model, ctx| {
                model.save(file_id, content, buffer_version, ctx)
            }) {
            log::error!("Failed to save file to new path: {err:?}");
            ctx.emit(LocalCodeEditorEvent::FailedToSave {
                error: Rc::new(err),
            });
            SaveOutcome::Failed
        } else {
            Self::subscribe_to_global_buffer_events(file_id, ctx);
            SaveOutcome::Succeeded
        };
        callback(save_outcome, ctx);
    }

    pub fn cursor_at(&self, point: Point, ctx: &mut ViewContext<Self>) {
        self.editor.update(ctx, |editor, ctx| {
            editor.cursor_at(point, ctx);
        });
    }

    /// If there is a pending diff available, apply it on the buffer. This should only be called _after_ the buffer
    /// has been loaded.
    fn apply_diffs_if_any(&mut self, ctx: &mut ViewContext<Self>) -> Option<usize> {
        let diff = self.diff_type.clone()?;
        let deltas = match diff {
            DiffType::Create { delta } => vec![delta],
            DiffType::Update { mut deltas, .. } => {
                deltas.sort_by_key(|delta| delta.replacement_line_range.start);
                deltas
            }
            DiffType::Delete { delta } => vec![delta],
        };

        // Early return if the pending diff itself is empty.
        let first_line_start = deltas
            .first()
            .map(|diff| diff.replacement_line_range.start)?;

        self.editor.update(ctx, |editor, ctx| {
            editor.apply_diffs(deltas, ctx);

            if self.enable_diff_nav_by_default {
                editor.toggle_diff_nav(None, ctx);
            }
        });

        Some(first_line_start)
    }

    pub fn file_id(&self) -> Option<FileId> {
        self.metadata.as_ref().map(|m| m.id)
    }

    /// Returns the unified file location (local or remote).
    pub fn file_location(&self) -> Option<&BufferFileLocation> {
        self.metadata.as_ref().map(|m| &m.location)
    }

    /// Returns the local path if this editor is backed by a local file.
    /// Returns `None` for remote files. Used by LSP and other local-only code paths.
    pub fn file_path(&self) -> Option<&Path> {
        self.file_location().and_then(|loc| loc.to_local_path())
    }

    /// Update this editor's file identity after a `GlobalBufferModel::rename`.
    /// Sets the new file_id and path, re-subscribes to `GlobalBufferModelEvent`,
    /// and updates the language from the new path.
    #[cfg(feature = "local_fs")]
    pub fn apply_rename(
        &mut self,
        buffer_state: BufferState,
        new_path: &Path,
        ctx: &mut ViewContext<Self>,
    ) {
        let file_id = buffer_state.file_id;
        self.metadata = Some(LoadedFileMetadata {
            id: file_id,
            location: BufferFileLocation::Local(new_path.to_path_buf()),
        });

        self.editor.update(ctx, |editor, ctx| {
            editor.set_language_with_path(new_path, ctx);
        });

        // Re-subscribe to GlobalBufferModel events for the new file_id.
        Self::subscribe_to_global_buffer_events(file_id, ctx);
    }

    pub fn editor(&self) -> &ViewHandle<CodeEditorView> {
        &self.editor
    }

    /// Accept the diff that is currently in the editor. For local files, this can only be called after the file contents
    /// have been loaded into the editor.
    /// If it is a local file, the diff content will be retrieved and the pending diff will be marked as completed.
    /// If it is not a local file, the pending diff will be marked as completed with an empty diff.
    pub fn accept_diff(&mut self, ctx: &mut ViewContext<Self>) {
        match self.file_path() {
            Some(file) => {
                // Begin calculating the diff that will be saved.  When the result comes back, the diff will be marked completed.
                self.editor.update(ctx, |view, ctx| {
                    view.retrieve_unified_diff(file.display().to_string(), ctx)
                });
            }
            None => {
                ctx.emit(LocalCodeEditorEvent::DiffAccepted);
            }
        };
    }

    pub fn close_find_bar(&mut self, should_focus_editor: bool, ctx: &mut ViewContext<Self>) {
        self.editor.update(ctx, |editor, ctx| {
            editor.close_find_bar(should_focus_editor, ctx);
        });
    }

    /// If a single terminal view exists in the active window, returns the active file path's relative to to the terminal's session.
    fn file_path_relative_to_terminal_view(&self, app: &AppContext) -> Option<String> {
        if let Some(terminal_target_fn) = self
            .selection_as_context_tooltip
            .as_ref()
            .map(|tooltip| &tooltip.terminal_target_fn)
        {
            app.windows().active_window().and_then(|window_id| {
                terminal_target_fn(window_id, app).and_then(|terminal_view| {
                    terminal_view
                        .as_ref(app)
                        .active_session_path_if_local(app)
                        .and_then(|cwd| {
                            let is_wsl = terminal_view
                                .as_ref(app)
                                .active_session_wsl_distro(app)
                                .is_some();
                            self.file_path()
                                .and_then(|file_path| to_relative_path(is_wsl, file_path, &cwd))
                        })
                })
            })
        } else {
            None
        }
    }

    fn render_selection_tooltip(&self, app: &AppContext) -> Option<Box<dyn Element>> {
        // If there's a single selection and an active terminal view, we want to give the user an option to add the selection as context.
        self.selection_as_context_tooltip
            .as_ref()
            .and_then(|selection_as_context_tooltip| {
                if self.editor.as_ref(app).selected_lines(app).is_some()
                    && self.file_path_relative_to_terminal_view(app).is_some()
                {
                    let appearance = Appearance::as_ref(app);
                    let theme = appearance.theme();
                    let modifier_keys = if cfg!(target_os = "macos") {
                        "⌘L"
                    } else {
                        "Ctrl-L"
                    };

                    let mut row = Flex::row()
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_main_axis_alignment(MainAxisAlignment::Center)
                        .with_main_axis_size(MainAxisSize::Min);
                    row.add_child(
                        Shrinkable::new(
                            1.,
                            Text::new_inline(
                                "Add as context",
                                appearance.ui_font_family(),
                                appearance.ui_font_size(),
                            )
                            .with_color(theme.active_ui_text_color().into())
                            .finish(),
                        )
                        .finish(),
                    );
                    row.add_child(
                        Container::new(
                            Text::new_inline(
                                modifier_keys,
                                appearance.ui_font_family(),
                                appearance.ui_font_size() * 0.75,
                            )
                            .with_color(theme.disabled_ui_text_color().into())
                            .finish(),
                        )
                        .with_margin_left(8.)
                        .finish(),
                    );

                    Some(
                        Hoverable::new(selection_as_context_tooltip.mouse_state.clone(), |state| {
                            let background_color = if state.is_hovered() {
                                theme.surface_2()
                            } else {
                                theme.surface_1()
                            };
                            let internal_container = Container::new(row.finish())
                                .with_padding_left(12.)
                                .with_padding_right(12.)
                                .with_padding_top(4.)
                                .with_padding_bottom(4.)
                                .finish();
                            Container::new(internal_container)
                                .with_background(background_color)
                                .with_padding_top(4.)
                                .with_padding_bottom(4.)
                                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                                .with_border(Border::all(1.5).with_border_fill(theme.surface_2()))
                                .with_drop_shadow(DropShadow::new_with_standard_offset_and_spread(
                                    DROP_SHADOW_COLOR,
                                ))
                                .finish()
                        })
                        .on_click(move |ctx, _app, _pos| {
                            ctx.dispatch_typed_action(
                                LocalCodeEditorAction::InsertSelectedTextToInput,
                            );
                        })
                        .finish(),
                    )
                } else {
                    None
                }
            })
    }

    fn insert_selected_text_to_input(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(relative_file_path) = self.file_path_relative_to_terminal_view(ctx) else {
            return;
        };

        let mut line_range: Option<Range<LineCount>> = None;
        let mut selected_text: Option<String> = None;
        self.editor.update(ctx, |editor, ctx| {
            // If we have a vim visual selection, update the editor model to use that as a selection range
            let has_vim_visual = matches!(editor.vim_mode(ctx), Some(VimMode::Visual(_)));
            if has_vim_visual {
                editor.model.update(ctx, |model, ctx| {
                    model.vim_visual_selection_range(MotionType::Linewise, false, ctx);
                });
            }

            if let Some((start, end)) = editor.selected_lines(ctx) {
                // selected_lines() returns 1-indexed row numbers.
                line_range = Some(LineCount::from(start as usize)..LineCount::from(end as usize));
                selected_text = Some(editor.selected_text(ctx).unwrap_or_default());
            }

            // Enter normal mode
            if has_vim_visual {
                editor.enter_vim_normal_mode(ctx);
            }
        });

        let (Some(line_range), Some(selected_text)) = (line_range, selected_text) else {
            return;
        };

        ctx.emit(LocalCodeEditorEvent::SelectionAddedAsContext {
            relative_file_path,
            line_range,
            selected_text,
        });
        self.editor.update(ctx, |editor, ctx| {
            editor.clear_selection(ctx);
        });
    }

    pub fn diff(&self) -> Option<&DiffType> {
        self.diff_type.as_ref()
    }

    /// Handles context menu events (like menu closing)
    fn handle_menu_event(&mut self, event: &Event, ctx: &mut ViewContext<Self>) {
        if let Event::Close { .. } = event {
            self.context_menu_state.is_open = false;
        }
        ctx.notify();
    }

    /// Creates menu items for the context menu
    fn context_menu_items(&self) -> Vec<MenuItem<LocalCodeEditorAction>> {
        vec![
            MenuItemFields::new("Go to definition")
                .with_on_select_action(LocalCodeEditorAction::GotoDefinition)
                .into_item(),
            MenuItemFields::new("Find references")
                .with_on_select_action(LocalCodeEditorAction::FindReferences)
                .into_item(),
        ]
    }

    /// Perform find references at the cursor position and show the references card.
    fn find_references_at_cursor(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.is_lsp_server_available(ctx) {
            return;
        }
        let editor = self.editor().as_ref(ctx);
        let lsp_position = editor.cursor_lsp_position(ctx);
        let anchor_offset = editor.lsp_location_to_offset(&lsp_position, ctx);
        self.fetch_find_references_and_show(lsp_position, anchor_offset, ctx);
    }

    /// Dismiss any open LSP overlays (hover tooltip and find references card).
    fn dismiss_lsp_overlays(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        let had_refs = self.close_find_references_card(ctx);
        let had_hover = self.lsp_hover_state.clear();
        had_refs || had_hover
    }

    /// Perform goto definition at the cursor position and navigate directly.
    fn goto_definition_at_cursor(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.is_lsp_server_available(ctx) {
            return;
        }
        let lsp_position = self.editor().as_ref(ctx).cursor_lsp_position(ctx);
        let Some(source_server_id) = self.lsp_server.as_ref().map(|s| s.as_ref(ctx).id()) else {
            log::debug!("No LSP server available for goto definition");
            return;
        };

        let server_type_name = self
            .lsp_server
            .as_ref()
            .map(|s| s.as_ref(ctx).server_name());

        self.call_goto_definition(
            lsp_position,
            move |_me, result, ctx| {
                let had_result = matches!(&result, Ok(locations) if !locations.is_empty());

                if let Some(server_type) = server_type_name {
                    send_telemetry_from_ctx!(
                        LspTelemetryEvent::GotoDefinition {
                            server_type,
                            had_result,
                        },
                        ctx
                    );
                }

                match result {
                    Ok(locations) => {
                        if let Some(location) = locations.first() {
                            ctx.emit(LocalCodeEditorEvent::GotoDefinition {
                                path: location.target.path.clone(),
                                line: location.target.location.line,
                                column: location.target.location.column,
                                source_server_id,
                            });
                        }
                    }
                    Err(e) => {
                        log::debug!("Failed to get goto definition: {e}");
                    }
                }
            },
            ctx,
        );
    }

    /// Show hover (documentation/type info) at the current cursor position.
    fn show_hover_at_cursor(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.is_lsp_server_available(ctx) {
            return;
        }
        let cursor_offset = self.editor().as_ref(ctx).cursor_head_offset(ctx);
        self.lsp_hover_state = LspHoverState::Loading(None);
        self.hover_for_offset(cursor_offset, ctx);
    }

    /// Close the find references card if it is open and refocus the editor.
    /// Returns true if a card was closed.
    fn close_find_references_card(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        if self.find_references_view.is_some() {
            self.find_references_view = None;
            self.editor.update(ctx, |editor, _ctx| {
                editor.set_find_references_anchor_offset(None);
            });
            ctx.focus(&self.editor);
            true
        } else {
            false
        }
    }
}

impl DiffViewer for LocalCodeEditorView {
    fn editor(&self) -> &ViewHandle<CodeEditorView> {
        &self.editor
    }

    fn diff(&self) -> Option<&DiffType> {
        self.diff_type.as_ref()
    }

    fn was_edited(&self) -> bool {
        self.was_edited
    }

    /// Automatically accept and save this diff. Unlike [`Self::accept_diff`] and [`Self::save_local`], this
    /// waits for the initial file contents to be loaded.
    fn accept_and_save_diff(&self, ctx: &mut ViewContext<Self>) {
        ctx.spawn(self.file_loaded.wait(), move |me, _, ctx| {
            me.accept_diff(ctx);
            if let Err(err) = me.save_local(ctx) {
                log::error!("{err:?}");
                if let ImmediateSaveError::FailedToSave(err) = err {
                    ctx.emit(LocalCodeEditorEvent::FailedToSave {
                        error: Rc::new(err),
                    });
                }
            }
        });
    }

    fn reject_diff(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(LocalCodeEditorEvent::DiffRejected);
    }

    fn restore_diff_base(&mut self, ctx: &mut ViewContext<Self>) -> Result<(), String> {
        if self.is_new_file {
            if let Some(file_id) = self.file_id() {
                GlobalBufferModel::handle(ctx).update(ctx, |model, ctx| {
                    model.remove(file_id, ctx);
                });
            }
            if let Some(path) = self.file_path().map(|p| p.to_path_buf()) {
                if let Err(e) = std::fs::remove_file(&path) {
                    log::error!("Failed to delete file after save: {e}");
                } else {
                    // This will close tabs with the file open
                    ctx.dispatch_typed_action(&WorkspaceAction::FileDeleted { path });
                }
            }

            return Ok(());
        }

        let base_content = self
            .editor
            .as_ref(ctx)
            .model
            .as_ref(ctx)
            .diff()
            .as_ref(ctx)
            .base()
            .ok_or_else(|| "Missing base content".to_string())?
            .to_string();

        let file_id = self
            .file_id()
            .ok_or_else(|| "Missing file_id".to_string())?;

        let buffer_version = self.editor.as_ref(ctx).version(ctx);

        GlobalBufferModel::handle(ctx)
            .update(ctx, |model, ctx| {
                model.save(file_id, base_content, buffer_version, ctx)
            })
            .map_err(|e| format!("Failed to save file: {e:?}"))
    }
}

impl Entity for LocalCodeEditorView {
    type Event = LocalCodeEditorEvent;
}

impl View for LocalCodeEditorView {
    fn ui_name() -> &'static str {
        "LocalCodeEditorView"
    }

    fn on_focus(&mut self, focus_ctx: &warpui::FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            self.editor.update(ctx, |editor, ctx| editor.focus(ctx));
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn warpui::Element> {
        // Rendering the version conflict banner.
        let base: Box<dyn Element> = if self.has_version_conflicts(app) {
            let appearance = Appearance::as_ref(app);
            let banner = render_unsaved_changes_banner(
                appearance,
                self.conflict_banner_mouse_states
                    .discard_mouse_state
                    .clone(),
                self.conflict_banner_mouse_states
                    .overwrite_mouse_state
                    .clone(),
            );
            let mut col = Flex::column().with_child(banner);

            let editor_view = ChildView::new(&self.editor).finish();
            if self.editor.as_ref(app).needs_vertical_constraint() {
                col.add_child(Shrinkable::new(1., editor_view).finish());
            } else {
                col.add_child(editor_view);
            }
            col.finish()
        } else {
            ChildView::new(&self.editor).finish()
        };

        let base_with_handler =
            Hoverable::new(self.context_menu_state.mouse_state.clone(), |_| base)
                .on_right_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(LocalCodeEditorAction::OpenContextMenu);
                })
                .finish();

        let mut stack = Stack::new()
            .with_constrain_absolute_children()
            .with_child(base_with_handler);

        let editor = self.editor().as_ref(app);
        if self.selection_as_context_tooltip.is_some() {
            // When a single terminal exists in the window and the user has made a selection (but isn't currently selecting),
            // we render a tooltip that allows them to add the selected text to the terminal context.
            let is_ai_enabled = AISettings::as_ref(app).is_any_ai_enabled(app);
            if is_ai_enabled
                && FeatureFlag::SelectionAsContext.is_enabled()
                && !editor.is_selecting()
            {
                let tooltip = self.render_selection_tooltip(app);
                if let Some(tooltip) = tooltip {
                    stack.add_positioned_child(tooltip, editor.selection_position_anchor(app))
                }
            }
        }

        // Render context menu if open
        if self.context_menu_state.is_open {
            stack.add_positioned_child(
                ChildView::new(&self.context_menu).finish(),
                editor.selection_position_anchor(app),
            )
        }
        // Render find references card if loaded (render before hover tooltip so hover appears on top)
        if let Some(references_view) = &self.find_references_view {
            let provider = editor.show_find_references_provider();

            // Get the cached gutter position from the last paint (in screen coordinates).
            // Same pattern as comment editor in CodeEditorView::render.
            let line_location = app.element_position_by_id_at_last_frame(
                editor.window_id(),
                editor.find_references_save_position_id(),
            );

            // Determine if we should show the card.
            // When line_location is None (first frame or anchor not rendered), default to true.
            // This matches the comment editor pattern and avoids a one-frame delay on first click.
            // The scroll-bounding check only runs when we have a cached position.
            let should_show = match line_location {
                Some(line_location) => {
                    provider.should_show_find_references_card(line_location, app)
                }
                None => true,
            };

            if should_show {
                // Compute positioning fresh from the stable CharOffset each frame.
                // Same pattern as comment editor which uses vertical_offset_at_render_location.
                if let Some(positioning) = self.find_references_card_positioning(app) {
                    stack.add_positioned_overlay_child(
                        ChildView::new(references_view).finish(),
                        positioning,
                    );
                }
            }
        }

        // Render LSP hover tooltip if available (render last so it appears on top)
        if let (Some(hover_tooltip), Some(positioning)) = (
            self.render_hover_tooltip(app),
            self.hover_tooltip_positioning(app),
        ) {
            stack.add_positioned_overlay_child(hover_tooltip, positioning);
        }

        if let Some(footer) = &self.footer {
            let mut col = Flex::column();

            if self.editor.as_ref(app).needs_vertical_constraint() {
                col.add_child(Shrinkable::new(1., stack.finish()).finish());
            } else {
                col.add_child(stack.finish());
            }
            col.with_child(ChildView::new(footer).finish()).finish()
        } else {
            stack.finish()
        }
    }
}

impl TypedActionView for LocalCodeEditorView {
    type Action = LocalCodeEditorAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            LocalCodeEditorAction::InsertSelectedTextToInput => {
                self.insert_selected_text_to_input(ctx);
            }
            LocalCodeEditorAction::SaveFile => {
                if let Err(ImmediateSaveError::FailedToSave(err)) = self.save_local(ctx) {
                    log::error!("Failed to save file {err:?}");
                    ctx.emit(LocalCodeEditorEvent::FailedToSave {
                        error: Rc::new(err),
                    });
                };
            }
            LocalCodeEditorAction::DiscardUnsavedChanges => {
                if let Some(path) = self.file_path().map(Path::to_path_buf) {
                    self.base_content_version = Some(self.editor().as_ref(ctx).version(ctx));
                    ctx.emit(LocalCodeEditorEvent::DiscardUnsavedChanges { path });
                } else if self.has_remote_conflict {
                    // Remote file: re-open the buffer from the server to get
                    // the latest on-disk content. The BufferLoaded event will
                    // clear has_remote_conflict and update base_content_version.
                    // If the re-open fails, has_remote_conflict stays true and
                    // the banner remains visible so the user can retry.
                    if let Some(file_id) = self.file_id() {
                        GlobalBufferModel::handle(ctx).update(ctx, |model, ctx| {
                            model.reopen_remote_buffer(file_id, ctx);
                        });
                    }
                }
            }
            LocalCodeEditorAction::NavigateToTarget(location) => {
                let Some(source_server_id) = self.lsp_server.as_ref().map(|s| s.as_ref(ctx).id())
                else {
                    log::debug!("No LSP server available for navigate to target");
                    return;
                };
                ctx.emit(LocalCodeEditorEvent::GotoDefinition {
                    path: location.path.clone(),
                    line: location.location.line,
                    column: location.location.column,
                    source_server_id,
                });
            }
            LocalCodeEditorAction::GotoDefinition => {
                self.context_menu_state.is_open = false;
                // Cursor was already set to right-click position, use it
                self.goto_definition_at_cursor(ctx);
            }
            LocalCodeEditorAction::FindReferences => {
                self.context_menu_state.is_open = false;
                self.find_references_at_cursor(ctx);
            }
            LocalCodeEditorAction::OpenContextMenu => {
                // Only show context menu if LSP is available
                if self.is_lsp_server_available(ctx) {
                    self.context_menu_state.is_open = true;
                    let menu_items = self.context_menu_items();
                    self.context_menu.update(ctx, move |menu, ctx| {
                        menu.set_items(menu_items, ctx);
                        ctx.notify();
                    });
                    ctx.notify();
                }
            }
            LocalCodeEditorAction::FetchAndShowFindReferences {
                lsp_position,
                anchor_offset,
            } => {
                // Lazily fetch find-references as fallback when at the definition.
                // This is triggered on cmd-click when go-to-definition has no different location.
                self.fetch_find_references_and_show(lsp_position.clone(), *anchor_offset, ctx);
            }
        }
    }
}

/// Renders a banner warning that the file has saved changes not reflected in the diff
pub fn render_unsaved_changes_banner(
    appearance: &Appearance,
    discard_mouse_state: MouseStateHandle,
    overwrite_mouse_state: MouseStateHandle,
) -> Box<dyn Element> {
    let left = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(
            Container::new(
                ConstrainedBox::new(
                    Icon::Warning
                        .to_warpui_icon(appearance.theme().active_ui_text_color())
                        .finish(),
                )
                .with_height(16.)
                .with_width(16.)
                .finish(),
            )
            .with_margin_right(8.)
            .finish(),
        )
        .with_child(
            Shrinkable::new(
                1.,
                Text::new(
                    "This file has saved changes that are not reflected here.",
                    appearance.ui_font_family(),
                    appearance.ui_font_size(),
                )
                .with_color(appearance.theme().active_ui_text_color().into())
                .soft_wrap(true)
                .finish(),
            )
            .finish(),
        )
        .finish();

    let right = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(
            appearance
                .ui_builder()
                .button(ButtonVariant::Text, discard_mouse_state)
                .with_text_label("Discard this version".into())
                .with_style(UiComponentStyles {
                    height: Some(24.),
                    padding: Some(Coords {
                        left: 8.,
                        right: 8.,
                        ..Default::default()
                    }),
                    font_color: Some(appearance.theme().active_ui_text_color().into()),
                    ..Default::default()
                })
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(LocalCodeEditorAction::DiscardUnsavedChanges)
                })
                .finish(),
        )
        .with_child(
            Container::new(
                appearance
                    .ui_builder()
                    .button(ButtonVariant::Outlined, overwrite_mouse_state)
                    .with_text_label("Overwrite".into())
                    .with_style(UiComponentStyles {
                        font_color: Some(appearance.theme().active_ui_text_color().into()),
                        ..Default::default()
                    })
                    .build()
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(LocalCodeEditorAction::SaveFile)
                    })
                    .finish(),
            )
            .with_margin_left(4.)
            .finish(),
        )
        .finish();

    Container::new(
        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(Shrinkable::new(1., left).finish())
            .with_child(right)
            .finish(),
    )
    .with_background(appearance.theme().text_selection_as_context_color())
    .with_padding_top(4.)
    .with_padding_bottom(4.)
    .with_padding_left(12.)
    .with_padding_right(12.)
    .finish()
}

/// Renders a small yellow circle with tooltip indicating unsaved changes
pub fn render_unsaved_circle_with_tooltip(
    mouse_state: MouseStateHandle,
    tooltip_text: String,
    size: f32,
    right_margin: f32,
    appearance: &Appearance,
) -> Box<dyn Element> {
    Hoverable::new(mouse_state, |state| {
        let rect = Container::new(
            ConstrainedBox::new(
                Rect::new()
                    .with_background_color(appearance.theme().active_ui_text_color().into())
                    .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
                    .finish(),
            )
            .with_width(size)
            .with_height(size)
            .finish(),
        )
        .with_margin_right(right_margin)
        .finish();

        if state.is_hovered() {
            let mut stack = Stack::new().with_child(rect);

            let tooltip = appearance
                .ui_builder()
                .tool_tip(tooltip_text)
                .build()
                .finish();

            stack.add_positioned_overlay_child(
                tooltip,
                OffsetPositioning::offset_from_parent(
                    Vector2F::new(0., 4.),
                    ParentOffsetBounds::Unbounded,
                    ParentAnchor::BottomMiddle,
                    ChildAnchor::TopMiddle,
                ),
            );
            stack.finish()
        } else {
            rect
        }
    })
    .finish()
}

/// Provider for determining find references card visibility based on scroll state.
#[derive(Debug)]
pub struct ShowFindReferencesCard {
    pub editor_window_id: WindowId,
    /// Optional: position ID of parent scrollable container (e.g., code review list).
    /// If None, card is in standalone LocalCodeEditorView - visibility is determined
    /// by whether the anchor gutter element is rendered (cached position exists).
    pub parent_scrollable_position_id: Option<String>,
}

impl ShowFindReferencesCardProvider for ShowFindReferencesCard {
    fn should_show_find_references_card(
        &self,
        card_anchor_location: RectF,
        app: &AppContext,
    ) -> bool {
        // For standalone editors (no parent scrollable), we don't need to check scroll bounds.
        let Some(parent_position_id) = &self.parent_scrollable_position_id else {
            return true;
        };

        // For editors within a parent scrollable (e.g., code review), check if anchor
        // is within the parent's visible bounds.
        let Some(parent_bounds) =
            app.element_position_by_id_at_last_frame(self.editor_window_id, parent_position_id)
        else {
            return false;
        };

        // Check if anchor point is within parent scrollable bounds
        // Use same logic as ShowCommentEditor: check upper-right and lower-left corners
        let upper_right_in = parent_bounds.contains_point(card_anchor_location.upper_right());
        let lower_left_in = parent_bounds.contains_point(card_anchor_location.lower_left());
        upper_right_in || lower_left_in
    }
}
