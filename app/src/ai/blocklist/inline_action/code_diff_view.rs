use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use ai::diff_validation::{
    fuzzy_match_diffs, fuzzy_match_v4a_diffs, parse_line_numbers, DiffDelta, DiffType, ParsedDiff,
    SearchAndReplace, V4AHunk,
};
use anyhow::Result;
use lazy_static::lazy_static;
use markdown_parser::{FormattedText, FormattedTextFragment, FormattedTextLine};
use pathfinder_geometry::vector::vec2f;
use rand::distributions::Alphanumeric;
use rand::{thread_rng, Rng as _};
use warp_core::features::FeatureFlag;
use warp_core::platform::SessionPlatform;
use warp_core::settings::ToggleableSetting;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::color::CLAUDE_ORANGE;
use warp_core::ui::theme::color::internal_colors::{fg_overlay_6, neutral_1, neutral_4};
use warp_core::ui::theme::Fill;
use warp_core::HostId;
use warp_editor::content::buffer::InitialBufferState;
use warp_editor::render::element::VerticalExpansionBehavior;
use warp_util::file::FileSaveError;
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warp_util::remote_path::RemotePath;
use warp_util::standardized_path::StandardizedPath;
use warpui::elements::new_scrollable::{ScrollableAppearance, SingleAxisConfig};
use warpui::elements::{
    Align, Border, ChildAnchor, ChildView, Clipped, ClippedScrollStateHandle, ConstrainedBox,
    Container, CornerRadius, CrossAxisAlignment, DispatchEventResult, Empty, EventHandler, Flex,
    FormattedTextElement, HighlightedHyperlink, Hoverable, MainAxisAlignment, MainAxisSize,
    MouseStateHandle, NewScrollable, OffsetPositioning, ParentAnchor, ParentElement,
    ParentOffsetBounds, PositionedElementAnchor, PositionedElementOffsetBounds, Radius,
    SavePosition, ScrollTarget, ScrollToPositionMode, ScrollbarWidth, Shrinkable,
    SizeConstraintCondition, SizeConstraintSwitch, Stack, Text,
};
use warpui::keymap::{EditableBinding, FixedBinding, Keystroke};
use warpui::platform::{Cursor, OperatingSystem};
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{
    AppContext, Element, Entity, FocusContext, ModelHandle, SingletonEntity, TypedActionView, View,
    ViewContext, ViewHandle,
};

use super::malformed_line_heuristics::has_malformed_terminal_correction_signal;
use crate::ai::agent::icons::{self, yellow_stop_icon};
use crate::ai::agent::{AIAgentActionId, AIIdentifiers, FileEdit, FileLocations, ServerOutputId};
use crate::ai::blocklist::action_model::{
    AIActionStatus, BlocklistAIActionEvent, BlocklistAIActionModel,
    EditAcceptAndContinueClickedEvent, EditAcceptClickedEvent, EditResolvedEvent, EditStats,
    MalformedFinalLineProxyEvent, RequestFileEditsFormatKind, RequestFileEditsTelemetryEvent,
};
use crate::ai::blocklist::history_model::BlocklistAIHistoryModel;
use crate::ai::blocklist::inline_action::inline_action_header::INLINE_ACTION_HORIZONTAL_PADDING;
use crate::ai::blocklist::inline_action::inline_action_icons::{
    cancelled_icon, green_check_icon, icon_size, reverted_icon,
};
use crate::ai::blocklist::model::{AIBlockModel, AIBlockModelHelper};
use crate::ai::blocklist::view_util::render_provider_icon_button;
use crate::ai::blocklist::RequestedEditResolution;
use crate::ai::mcp::{mcp_provider_from_file_path, MCPProvider};
use crate::ai::paths::host_native_absolute_path;
use crate::ai::predict::prompt_suggestions::ACCEPT_PROMPT_SUGGESTION_KEYBINDING;
use crate::ai::skills::{
    icon_override_for_skill_name, render_skill_button, skill_path_from_location, SkillManager,
    SkillOpenOrigin, SkillReference, SkillTelemetryEvent,
};
use crate::code::diff_viewer::{DiffViewer, DisplayMode};
use crate::code::editor::view::{CodeEditorEvent, CodeEditorRenderOptions, CodeEditorView};
use crate::code::editor::{add_color, remove_color};
use crate::code::inline_diff::{InlineDiffView, InlineDiffViewEvent};
use crate::code::DiffResult;
use crate::code_review::telemetry_event::CodeReviewPaneEntrypoint;
use crate::menu::{Event as MenuEvent, Menu, MenuItemFields, MenuVariant};
use crate::pane_group::focus_state::PaneFocusHandle;
use crate::pane_group::pane::{view, PaneId};
use crate::pane_group::{BackingView, PaneEvent};
use crate::server::telemetry::{
    AgentModeCodeFileNavigationSource, ToggleCodeSuggestionsSettingSource,
};
use crate::settings::AISettings;
use crate::terminal::input::SET_INPUT_MODE_AGENT_ACTION_NAME;
use crate::terminal::ShellLaunchData;
use crate::ui_components::blended_colors;
use crate::ui_components::icons::Icon;
use crate::util::bindings::keybinding_name_to_keystroke;
use crate::view_components::action_button::{
    ActionButton, ButtonSize, KeystrokeSource, NakedTheme,
};
use crate::view_components::compactible_action_button::{
    render_compact_and_regular_button_rows, CompactibleActionButton, RenderCompactibleActionButton,
    MEDIUM_SIZE_SWITCH_THRESHOLD, XLARGE_SIZE_SWITCH_THRESHOLD,
};
use crate::view_components::compactible_split_action_button::CompactibleSplitActionButton;
use crate::view_components::DismissibleToast;
use crate::workspace::ToastStack;
use crate::{cmd_or_ctrl_shift, send_telemetry_from_ctx, TelemetryEvent};

const REQUESTED_EDIT_CANCEL_LABEL: &str = "Cancel";
const REQUESTED_EDIT_REFINE_LABEL: &str = "Refine";
const REQUESTED_EDIT_ACCEPT_LABEL: &str = "Accept";
const REQUESTED_EDIT_ACCEPT_AND_AUTOEXECUTE_LABEL: &str = "Auto-approve";
const REQUESTED_EDIT_EDIT_LABEL: &str = "Edit";
const REQUESTED_EDIT_MINIMIZE_LABEL: &str = "Done";
const SUGGESTED_EDIT_ACCEPT_LABEL: &str = "Accept";
const SUGGESTED_EDIT_ACCEPT_AND_CONTINUE_LABEL: &str = "Accept and continue with agent";
const SUGGESTED_EDIT_ITERATE_WITH_AGENT_LABEL: &str = "Iterate with agent";
const SUGGESTED_EDIT_DISMISS_LABEL: &str = "Dismiss";
const MAX_EDITOR_HEIGHT: f32 = 500.;
const INLINE_EDITOR_HEIGHT: f32 = 94.;
const INLINE_EDITOR_HEIGHT_EXPANDED: f32 = 400.;
const FILE_TAB_FONT_SIZE: f32 = 12.;
const FILE_TAB_HEIGHT: f32 = 32.;
const FILE_TAB_WIDTH: f32 = 120.;
const FILE_TAB_HORIZONTAL_PADDING: f32 = 8.;
const HEADER_MARGIN: f32 = 8.;

const DISPATCHED_REQUESTED_EDIT_EXPANDED: &str = "DispatchedRequestedEditExpanded";
const SUGGESTED_EDIT_INLINE_BANNER: &str = "SuggestedEditInlineBanner";
/// Slightly smaller than other action header vertical padding to account for the 1px border on the code diff line count.
const HEADER_VERTICAL_PADDING: f32 = 9.;

const ACCEPT_KEY: &str = "enter";
const EDIT_OR_EXPAND_KEY: &str = "e";

const EDIT_REQUESTED_EDIT_NAME: &str = "code_diff_view:edit_requested_edit";

lazy_static! {
    static ref CANCEL_REQUESTED_EDIT_KEYSTROKE: Keystroke = Keystroke {
        ctrl: true,
        key: "c".to_owned(),
        ..Default::default()
    };
    static ref MINIMIZE_REQUESTED_EDIT_KEYSTROKE: Keystroke = Keystroke {
        key: "escape".to_owned(),
        ..Default::default()
    };
}

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings([
        FixedBinding::new(
            "ctrl-c",
            CodeDiffViewAction::Reject,
            id!(CodeDiffView::ui_name()),
        ),
        FixedBinding::new(
            "escape",
            CodeDiffViewAction::Minimize,
            id!(CodeDiffView::ui_name()) & id!(DISPATCHED_REQUESTED_EDIT_EXPANDED),
        ),
        FixedBinding::new(
            ACCEPT_KEY,
            CodeDiffViewAction::TryAccept,
            id!(CodeDiffView::ui_name()) & !id!(DISPATCHED_REQUESTED_EDIT_EXPANDED),
        ),
        FixedBinding::new(
            "numpadenter",
            CodeDiffViewAction::TryAccept,
            id!(CodeDiffView::ui_name()) & !id!(DISPATCHED_REQUESTED_EDIT_EXPANDED),
        ),
        FixedBinding::new(
            "up",
            CodeDiffViewAction::NavigateToDiffHunk(Direction::Previous),
            id!(CodeDiffView::ui_name()) & !id!(DISPATCHED_REQUESTED_EDIT_EXPANDED),
        ),
        FixedBinding::new(
            "down",
            CodeDiffViewAction::NavigateToDiffHunk(Direction::Next),
            id!(CodeDiffView::ui_name()) & !id!(DISPATCHED_REQUESTED_EDIT_EXPANDED),
        ),
        FixedBinding::new(
            "left",
            CodeDiffViewAction::SelectFile(Direction::Previous),
            id!(CodeDiffView::ui_name()) & !id!(DISPATCHED_REQUESTED_EDIT_EXPANDED),
        ),
        FixedBinding::new(
            "right",
            CodeDiffViewAction::SelectFile(Direction::Next),
            id!(CodeDiffView::ui_name()) & !id!(DISPATCHED_REQUESTED_EDIT_EXPANDED),
        ),
    ]);

    app.register_editable_bindings([EditableBinding::new(
        EDIT_REQUESTED_EDIT_NAME,
        "Edit Code Diff",
        CodeDiffViewAction::Edit,
    )
    .with_context_predicate(id!(CodeDiffView::ui_name()) & !id!(DISPATCHED_REQUESTED_EDIT_EXPANDED))
    .with_key_binding(cmd_or_ctrl_shift("e"))]);
}

#[derive(Default, Clone)]
struct CodeDiffViewMouseStates {
    show_hide_button: MouseStateHandle,
    scroll_icon_button: MouseStateHandle,
    passive_code_suggestion_checkbox: MouseStateHandle,
    ai_settings_link_highlight_index: HighlightedHyperlink,
    skill_button_handle: MouseStateHandle,
    stats_badge_button: MouseStateHandle,
    mcp_config_button_handle: MouseStateHandle,
}

#[derive(Debug, Clone)]
pub enum CodeDiffViewEvent {
    TryAccept,
    EnableAutoexecuteMode,
    SavedAcceptedDiffs {
        diff: DiffResult,
        updated_files: Vec<(FileLocations, bool)>,
        /// The accepted file contents keyed by file path, extracted from the
        /// editor buffers at the time of acceptance. This avoids re-reading
        /// files from disk (or the remote server) when building context for
        /// the LLM.
        file_contents: Vec<(String, String)>,
        deleted_files: Vec<String>,
        save_errors: Vec<Rc<FileSaveError>>,
    },
    Rejected,
    Pane(PaneEvent),
    EditModeChanged {
        enabled: bool,
    },
    ToggledEditVisibility,
    TextSelected,
    CopiedEmptyText,
    EditorFocused,
    Blur,
    DisplayModeChanged,
    OpenSettings,
    CancelPassive,
    ViewDetails,
    ContinuePassiveCodeDiffWithAgent {
        accepted: bool,
    },
    ToggleCodeReviewPane {
        entrypoint: CodeReviewPaneEntrypoint,
    },
    /// Emitted when candidate diffs are loaded and ready to display.
    /// Used to trigger AIBlock height recalculation for passive code diffs.
    LoadedDiffs,
    /// Emitted when the user opens a skill file from a code diff
    OpenSkill {
        reference: SkillReference,
        path: LocalOrRemotePath,
    },
    /// Emitted when the user opens an MCP config file from a code diff
    OpenMCPConfig {
        provider: MCPProvider,
        path: PathBuf,
    },
}

/// The base content and file path for a diff.
#[derive(Clone)]
#[cfg_attr(debug_assertions, derive(Debug))]
pub struct DiffBase {
    /// The original file content before the diff is applied.
    /// Empty for new file creation.
    pub content: String,
    /// The absolute file path.
    pub file_path: String,
}

/// User-visible file diff with the original contents of the file
/// and the changes to those contents.
#[derive(Clone)]
#[cfg_attr(debug_assertions, derive(Debug))]
pub struct FileDiff {
    pub base: DiffBase,
    pub diff_type: DiffType,
}

impl FileDiff {
    pub fn new(content: String, file_path: String, diff_type: DiffType) -> FileDiff {
        FileDiff {
            base: DiffBase { content, file_path },
            diff_type,
        }
    }

    pub fn file_path(&self) -> String {
        self.base.file_path.clone()
    }
}

/// The state of the saving diffs for a list of files.
///
/// When the requested edit is accepted, we have to 1) wait for the diff to be computed
/// by the CodeDiffModel for each file 2) wait for the changes to be saved locally.
///
/// After the above two conditions are all met, we can emit the Accepted event with all of the diffs.
/// This tracks which diffs have been computed.
#[derive(Clone, Debug)]
#[cfg_attr(target_family = "wasm", allow(dead_code))]
pub struct SavingDiffs {
    pending_diffs: Vec<DiffApplicationState>,
}

impl SavingDiffs {
    /// Initialize an accepted diff state with N files.
    fn new(length: usize) -> Self {
        Self {
            pending_diffs: vec![DiffApplicationState::default(); length],
        }
    }

    /// Returns true if all of the diffs are saved and computed.
    fn pending_diff_is_complete(&self) -> bool {
        self.pending_diffs
            .iter()
            .all(|diff| diff.computed_diff.is_some() && diff.save_status.is_complete())
    }

    /// Update the diff application save state at the given idx.
    fn mark_diff_saved(&mut self, idx: usize, save_error: Option<Rc<FileSaveError>>) {
        if let Some(state) = self.pending_diffs.get_mut(idx) {
            state.save_status = match save_error {
                None => SaveStatus::Success,
                Some(error) => SaveStatus::Failed(error),
            };
        }
    }

    /// Update the diff application compute state at the given idx.
    fn mark_diff_computed(&mut self, idx: usize, diff: Rc<DiffResult>) {
        if let Some(state) = self.pending_diffs.get_mut(idx) {
            state.computed_diff = Some(diff);
        }
    }
}

/// The status of saving a file.
#[derive(Clone, Debug, Default)]
#[cfg_attr(target_family = "wasm", allow(dead_code))]
enum SaveStatus {
    #[default]
    Pending,
    Success,
    Failed(Rc<FileSaveError>),
}

impl SaveStatus {
    fn is_complete(&self) -> bool {
        !matches!(self, SaveStatus::Pending)
    }
}

/// The diff application state for a single file.
#[derive(Clone, Debug, Default)]
#[cfg_attr(target_family = "wasm", allow(dead_code))]
struct DiffApplicationState {
    computed_diff: Option<Rc<DiffResult>>,
    save_status: SaveStatus,
}

#[derive(Clone, Debug)]
pub enum CodeDiffState {
    /// The diff is received, but is queued for interaction behind another action.
    Queued,
    /// The user is reviewing (and possibly editing) the code diff.
    /// Unlike requested commands, a [`CodeDiffView`] is only created upon stream completion.
    WaitingForUser,
    /// If the payload is some, the code diff was accepted but the individual file changes have not
    /// been fully computed and saved yet. We cache the accepted diff state to collect unified diffs
    /// from each file source.
    Accepted(Option<SavingDiffs>),
    /// The user rejected this code diff.
    Rejected,
    /// The changes were reverted after acceptance.
    Reverted,
    /// The diff is being viewed in a shared session (read-only mode).
    /// is_complete indicates whether the diff has been accepted or is still pending.
    ViewOnly { is_complete: bool },
}

impl CodeDiffState {
    fn is_complete(&self) -> bool {
        matches!(
            self,
            CodeDiffState::Accepted(_)
                | CodeDiffState::Rejected
                | CodeDiffState::Reverted
                | CodeDiffState::ViewOnly { is_complete: true }
        )
    }

    fn is_waiting_for_user(&self) -> bool {
        matches!(self, CodeDiffState::WaitingForUser)
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Direction {
    Next,
    Previous,
}

#[derive(Clone, Debug)]
pub enum CodeDiffViewAction {
    TryAccept,
    AcceptAndAutoExecute,
    AcceptPassiveDiffAndContinueWithAgent,
    IterateOnPassiveDiffWithAgent,
    Reject,
    ToggleRequestedEditVisibility,
    TabSelected(usize),
    Edit,
    Minimize,
    NavigateToDiffHunk(Direction),
    SelectFile(Direction),
    ScrollToExpand,
    ToggleCodeSuggestions,
    OpenSettings,
    ToggleAcceptMenu,
    OpenCodeReviewPane,
    RevertChanges,
    OpenSkill {
        reference: SkillReference,
        path: LocalOrRemotePath,
        mouse_state: MouseStateHandle,
    },
    OpenMCPConfig {
        provider: MCPProvider,
        path: PathBuf,
        mouse_state: MouseStateHandle,
    },
}

#[derive(Clone, Copy, Debug)]
enum AcceptSelection {
    Only,
    AndAutoExecute,
    AndContinueWithAgent,
}

/// Whether a code diff targets the local filesystem or a remote host.
#[derive(Clone, Debug)]
pub enum DiffSessionType {
    Local,
    Remote(HostId),
}

#[derive(Clone)]
struct PendingDiff {
    diff_view: ViewHandle<InlineDiffView>,
    tab_handle: MouseStateHandle,
}

#[derive(Clone)]
pub struct CodeDiffView {
    action_id: AIAgentActionId,

    pending_diffs: Vec<PendingDiff>,
    button_mouse_states: CodeDiffViewMouseStates,
    scrollable_state: ClippedScrollStateHandle,
    cancel_button: CompactibleActionButton,
    edit_button: CompactibleActionButton,
    minimize_button: CompactibleActionButton,
    iterate_with_agent_button: CompactibleActionButton,
    accept_and_autoexecute_split_button: CompactibleSplitActionButton,
    is_accept_split_button_menu_open: bool,
    accept_split_button_menu: ViewHandle<Menu<CodeDiffViewAction>>,
    code_review_button: ViewHandle<ActionButton>,
    expansion_button_collapsed: ViewHandle<ActionButton>,
    expansion_button_expanded: ViewHandle<ActionButton>,
    state: CodeDiffState,
    should_expand_when_complete: bool,
    selected_tab: usize,
    display_mode: DisplayMode,
    title: Option<String>,
    focus_handle: Option<PaneFocusHandle>,
    /// Client and server identifiers for the AI output associated with the code diffs.
    identifiers: AIIdentifiers,
    edit_format_kind: RequestFileEditsFormatKind,
    /// `False` until a user makes the first edit to one of the diffs in the view.
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    user_edited_file_contents: bool,
    /// The ID of the pane that opened this code diff view.
    /// Used to return to the original pane after editing.
    original_pane_id: Option<PaneId>,
    /// A randomly-generated string prefix to ensure the [`SavePosition`]s in this view are unique.
    position_id_prefix: String,
    /// Whether this code diff is a passive code suggestion.
    is_passive: bool,
    should_show_speedbump: bool,
    session_platform: Option<SessionPlatform>,
    /// Whether diffs target local disk or a remote host.
    diff_session_type: DiffSessionType,
}

impl CodeDiffView {
    fn open_accept_split_button_menu(&mut self, ctx: &mut ViewContext<Self>) {
        // Don't allow menu toggling in view-only mode.
        if matches!(self.state, CodeDiffState::ViewOnly { .. }) {
            log::error!("Attempted to toggle accept menu in view-only mode");
            return;
        }
        self.is_accept_split_button_menu_open = true;
        let is_passive = self.is_passive();
        if is_passive {
            self.accept_split_button_menu.update(ctx, |menu, ctx| {
                menu.set_items(
                    vec![MenuItemFields::new_multiline(
                        SUGGESTED_EDIT_ACCEPT_AND_CONTINUE_LABEL,
                        2,
                    )
                    .with_on_select_action(
                        CodeDiffViewAction::AcceptPassiveDiffAndContinueWithAgent,
                    )
                    .into_item()],
                    ctx,
                );
            });
        } else {
            let accept_keystroke = accept_keystroke_source(is_passive)
                .displayed(ctx)
                .unwrap_or_default();
            let auto_keystroke = keybinding_name_to_keystroke(
                crate::terminal::TOGGLE_AUTOEXECUTE_MODE_KEYBINDING,
                ctx,
            )
            .map(|k| k.displayed())
            .unwrap_or_default();

            let accept_item = MenuItemFields::new_with_label(
                REQUESTED_EDIT_ACCEPT_LABEL,
                accept_keystroke.as_str(),
            )
            .with_on_select_action(CodeDiffViewAction::TryAccept)
            .into_item();

            let auto_item = MenuItemFields::new_with_label(
                REQUESTED_EDIT_ACCEPT_AND_AUTOEXECUTE_LABEL,
                auto_keystroke.as_str(),
            )
            .with_on_select_action(CodeDiffViewAction::AcceptAndAutoExecute)
            .into_item();

            self.accept_split_button_menu.update(ctx, |menu, ctx| {
                menu.set_items(vec![accept_item, auto_item], ctx);
            });
        }
        self.accept_split_button_menu
            .update(ctx, |menu, ctx| menu.set_selected_by_index(0, ctx));
        ctx.focus(&self.accept_split_button_menu);
        ctx.notify();
    }

    /// Creates a CodeEditorView and sets up its event subscriptions.
    /// This is common logic shared by set_candidate_diffs.
    fn create_editor_with_subscriptions(
        &self,
        ctx: &mut ViewContext<Self>,
    ) -> ViewHandle<CodeEditorView> {
        let session_platform = self.session_platform.clone();
        let editor = ctx.add_typed_action_view(move |ctx| {
            CodeEditorView::new(
                session_platform,
                None,
                CodeEditorRenderOptions::new(VerticalExpansionBehavior::GrowToMaxHeight)
                    .lazy_layout(),
                ctx,
            )
            .with_horizontal_scrollbar_appearance(
                warpui::elements::new_scrollable::ScrollableAppearance::new(
                    warpui::elements::ScrollbarWidth::Auto,
                    true,
                ),
            )
        });

        #[cfg_attr(not(windows), allow(unused_variables))]
        ctx.subscribe_to_view(&editor, |me, view, event, ctx| match event {
            CodeEditorEvent::Focused => ctx.emit(CodeDiffViewEvent::EditorFocused),
            CodeEditorEvent::SelectionChanged => {
                // The `is_some` check is necessary because `CodeEditorEvent::SelectionChanged` is
                // also emitted when the editor's selection is cleared via external means
                // (i.e. when a text selection is made outside the `CodeEditorView`).
                if view.as_ref(ctx).selected_text(ctx).is_some() {
                    ctx.emit(CodeDiffViewEvent::TextSelected);
                }
            }
            CodeEditorEvent::CopiedEmptyText => {
                ctx.emit(CodeDiffViewEvent::CopiedEmptyText);
            }
            #[cfg(windows)]
            CodeEditorEvent::WindowsCtrlC { copied_selection } if !copied_selection => {
                me.reject(ctx);
            }
            _ => {}
        });

        editor
    }

    /// Sets up event subscriptions for an `InlineDiffView` at the given index.
    fn setup_diff_view_subscriptions(
        &self,
        diff_view: &ViewHandle<InlineDiffView>,
        idx: usize,
        file_path_for_error: String,
        ctx: &mut ViewContext<Self>,
    ) {
        #[cfg(not(target_family = "wasm"))]
        let file_path_clone = file_path_for_error;
        #[cfg(target_family = "wasm")]
        let _ = file_path_for_error;
        #[cfg(not(target_family = "wasm"))]
        let window_id = ctx.window_id();

        ctx.subscribe_to_view(diff_view, move |me, _, event, ctx| match event {
            InlineDiffViewEvent::DiffStatusUpdated => {
                ctx.notify();
            }
            #[cfg(not(target_family = "wasm"))]
            InlineDiffViewEvent::FileLoaded => {
                ctx.notify();
            }
            #[cfg(not(target_family = "wasm"))]
            InlineDiffViewEvent::FileSaved => {
                me.handle_save_completed(idx, None, ctx);
            }
            #[cfg(not(target_family = "wasm"))]
            InlineDiffViewEvent::FailedToSave { error } => {
                crate::safe_error!(
                    safe: ("Failed to save file for accepted AgentMode diffs"),
                    full: ("Failed to save file for accepted AgentMode diffs for {}: {}", file_path_clone, error)
                );
                let toast = DismissibleToast::error(format!(
                    "Failed to save file {file_path_clone}"
                ));
                ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                    toast_stack.add_ephemeral_toast(toast, window_id, ctx);
                });
                me.handle_save_completed(idx, Some(error.clone()), ctx);
            }
            InlineDiffViewEvent::DiffAccepted { diff } => {
                me.accepted_file_diff_computed(idx, diff.clone(), ctx);
            }
            InlineDiffViewEvent::UserEdited => {
                if me.user_edited_file_contents {
                    return;
                }
                me.user_edited_file_contents = true;

                let Some(output_id) = me.server_output_id() else {
                    return;
                };

                send_telemetry_from_ctx!(
                    TelemetryEvent::AgentModeCodeSuggestionEditedByUser { output_id },
                    ctx
                );
            }
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        action_id: &AIAgentActionId,
        model: &dyn AIBlockModel<View = crate::ai::blocklist::AIBlock>,
        title: Option<String>,
        identifiers: AIIdentifiers,
        edit_format_kind: RequestFileEditsFormatKind,
        should_show_speedbump: bool,
        action_model: ModelHandle<BlocklistAIActionModel>,
        session_platform: Option<SessionPlatform>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let is_passive = model.request_type(ctx).is_passive_code_diff();
        let initial_state = if action_model.as_ref(ctx).is_view_only() {
            CodeDiffState::ViewOnly { is_complete: false }
        } else if model.is_first_action_in_output(action_id, ctx) {
            CodeDiffState::WaitingForUser
        } else {
            CodeDiffState::Queued
        };

        let view = Self::build(
            action_id,
            is_passive,
            initial_state,
            title,
            identifiers,
            edit_format_kind,
            should_show_speedbump,
            session_platform,
            ctx,
        );

        ctx.subscribe_to_model(
            &action_model,
            move |me, action_model, event, ctx| match event {
                BlocklistAIActionEvent::FinishedAction { action_id, .. } if !me.is_complete() => {
                    match action_model.as_ref(ctx).get_action_status(&me.action_id) {
                        Some(AIActionStatus::Blocked) => {
                            me.state = CodeDiffState::WaitingForUser;
                            ctx.notify();
                        }
                        Some(status) => {
                            if matches!(me.state, CodeDiffState::ViewOnly { .. })
                                && status.is_success()
                            {
                                me.state = CodeDiffState::ViewOnly { is_complete: true };
                                me.should_expand_when_complete = false;
                            } else if status.is_cancelled() {
                                me.state = CodeDiffState::Rejected;
                                me.should_expand_when_complete = false;
                            }
                            ctx.notify();
                        }
                        None => {
                            log::error!(
                                "Action {action_id} finished but status not found in action model",
                            );
                        }
                    }
                }
                _ => (),
            },
        );

        view
    }

    /// Creates a passive `CodeDiffView` for out-of-band code diff suggestions.
    ///
    /// Unlike [`Self::new`], this does not require an `AIBlockModel` or
    /// `BlocklistAIActionModel` — the view is standalone and not tied to the
    /// action executor pipeline.
    #[allow(clippy::too_many_arguments)]
    pub fn new_passive(
        action_id: &AIAgentActionId,
        title: Option<String>,
        identifiers: AIIdentifiers,
        edit_format_kind: RequestFileEditsFormatKind,
        should_show_speedbump: bool,
        session_platform: Option<SessionPlatform>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        Self::build(
            action_id,
            true,
            CodeDiffState::WaitingForUser,
            title,
            identifiers,
            edit_format_kind,
            should_show_speedbump,
            session_platform,
            ctx,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn build(
        action_id: &AIAgentActionId,
        is_passive: bool,
        initial_state: CodeDiffState,
        title: Option<String>,
        identifiers: AIIdentifiers,
        edit_format_kind: RequestFileEditsFormatKind,
        should_show_speedbump: bool,
        session_platform: Option<SessionPlatform>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let display_mode = if is_passive {
            DisplayMode::with_inline_banner(INLINE_EDITOR_HEIGHT)
        } else {
            DisplayMode::with_embedded(MAX_EDITOR_HEIGHT)
        };

        let position_id_prefix: String = thread_rng()
            .sample_iter(&Alphanumeric)
            .take(8)
            .map(char::from)
            .collect();

        let cancel_button_label = if is_passive {
            SUGGESTED_EDIT_DISMISS_LABEL
        } else {
            REQUESTED_EDIT_REFINE_LABEL
        };
        let cancel_button = CompactibleActionButton::new(
            cancel_button_label.to_string(),
            Some(KeystrokeSource::Fixed(
                CANCEL_REQUESTED_EDIT_KEYSTROKE.clone(),
            )),
            ButtonSize::Small,
            CodeDiffViewAction::Reject,
            Icon::X,
            Arc::new(NakedTheme),
            ctx,
        );

        let edit_button = CompactibleActionButton::new(
            REQUESTED_EDIT_EDIT_LABEL.to_string(),
            Some(KeystrokeSource::Binding(EDIT_REQUESTED_EDIT_NAME)),
            ButtonSize::Small,
            CodeDiffViewAction::Edit,
            Icon::Pencil,
            Arc::new(NakedTheme),
            ctx,
        );

        let minimize_button = CompactibleActionButton::new(
            REQUESTED_EDIT_MINIMIZE_LABEL.to_string(),
            Some(KeystrokeSource::Fixed(
                MINIMIZE_REQUESTED_EDIT_KEYSTROKE.clone(),
            )),
            ButtonSize::Small,
            CodeDiffViewAction::Minimize,
            Icon::ArrowBlockLeft,
            Arc::new(NakedTheme),
            ctx,
        );

        let iterate_with_agent_button = CompactibleActionButton::new(
            SUGGESTED_EDIT_ITERATE_WITH_AGENT_LABEL.to_string(),
            Some(KeystrokeSource::Binding(SET_INPUT_MODE_AGENT_ACTION_NAME)),
            ButtonSize::Small,
            CodeDiffViewAction::IterateOnPassiveDiffWithAgent,
            Icon::ChatDashed,
            Arc::new(NakedTheme),
            ctx,
        );

        let accept_and_autoexecute_split_button = CompactibleSplitActionButton::new(
            if is_passive {
                SUGGESTED_EDIT_ACCEPT_LABEL.to_string()
            } else {
                REQUESTED_EDIT_ACCEPT_LABEL.to_string()
            },
            Some(accept_keystroke_source(is_passive)),
            ButtonSize::Small,
            CodeDiffViewAction::TryAccept,
            CodeDiffViewAction::ToggleAcceptMenu,
            Icon::Check,
            true,
            Some(Self::get_position_id_for_accept_split_button(
                &position_id_prefix,
            )),
            ctx,
        );

        let accept_menu = ctx.add_typed_action_view(|ctx| {
            let theme = Appearance::as_ref(ctx).theme();
            Menu::new()
                .with_menu_variant(MenuVariant::Fixed)
                .with_border(Border::all(1.).with_border_fill(theme.outline()))
                .prevent_interaction_with_other_elements()
        });
        ctx.subscribe_to_view(&accept_menu, |me, _menu, event, ctx| match event {
            MenuEvent::Close { .. } => {
                me.is_accept_split_button_menu_open = false;
                ctx.notify();
            }
            MenuEvent::ItemSelected | MenuEvent::ItemHovered => {}
        });

        let code_review_button = ctx.add_typed_action_view(|ctx| {
            ActionButton::new("", NakedTheme)
                .with_icon(Icon::Diff)
                .with_tooltip("Review changes")
                .with_width(icon_size(ctx))
                .with_height(icon_size(ctx))
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(CodeDiffViewAction::OpenCodeReviewPane);
                    ctx.notify();
                })
        });

        let expansion_button_collapsed = ctx.add_typed_action_view(|ctx| {
            ActionButton::new("", NakedTheme)
                .with_icon(Icon::ChevronRight)
                .with_tooltip("Expand")
                .with_width(icon_size(ctx))
                .with_height(icon_size(ctx))
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(CodeDiffViewAction::ToggleRequestedEditVisibility);
                    ctx.notify();
                })
        });
        let expansion_button_expanded = ctx.add_typed_action_view(|ctx| {
            ActionButton::new("", NakedTheme)
                .with_icon(Icon::ChevronDown)
                .with_tooltip("Collapse")
                .with_width(icon_size(ctx))
                .with_height(icon_size(ctx))
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(CodeDiffViewAction::ToggleRequestedEditVisibility);
                    ctx.notify();
                })
        });

        Self {
            action_id: action_id.clone(),
            pending_diffs: Vec::new(),
            button_mouse_states: Default::default(),
            cancel_button,
            edit_button,
            minimize_button,
            iterate_with_agent_button,
            accept_and_autoexecute_split_button,
            is_accept_split_button_menu_open: false,
            accept_split_button_menu: accept_menu,
            code_review_button,
            expansion_button_collapsed,
            expansion_button_expanded,
            state: initial_state,
            should_expand_when_complete: false,
            selected_tab: 0,
            display_mode,
            title,
            focus_handle: None,
            identifiers,
            edit_format_kind,
            user_edited_file_contents: false,
            original_pane_id: None,
            scrollable_state: Default::default(),
            position_id_prefix,
            is_passive,
            should_show_speedbump,
            session_platform,
            diff_session_type: DiffSessionType::Local,
        }
    }

    /// Set the session type for this diff view.
    ///
    /// When `Remote`, `set_candidate_diffs` registers files with the
    /// remote backend instead of the local filesystem.
    pub fn set_diff_session_type(&mut self, session_type: DiffSessionType) {
        self.diff_session_type = session_type;
    }

    pub fn is_pending_diffs_empty(&self) -> bool {
        self.pending_diffs.is_empty()
    }

    /// Returns the number of lines added and removed across all files.
    fn pending_diffs_line_counts(&self, app: &AppContext) -> (usize, usize) {
        let mut total_added = 0;
        let mut total_removed = 0;
        for pending_diff in &self.pending_diffs {
            let editor = pending_diff.diff_view.as_ref(app).editor().as_ref(app);
            let (lines_added, lines_removed) = editor.diff_hunks_changed_lines(app);
            total_added += lines_added;
            total_removed += lines_removed;
        }
        (total_added, total_removed)
    }

    pub fn is_passive(&self) -> bool {
        self.is_passive
    }

    /// Sets the set of candidate diffs to be displayed to the user to accept.
    pub fn set_candidate_diffs(&mut self, diffs: Vec<FileDiff>, ctx: &mut ViewContext<Self>) {
        debug_assert!(
            self.is_pending_diffs_empty(),
            "set_candidate_diffs should only be called once"
        );
        let display_mode = self.display_mode;
        let pending_diffs = diffs
            .into_iter()
            .enumerate()
            .map(|(idx, diff)| {
                #[cfg(debug_assertions)]
                log::debug!("Create CodeEditorView with diff: {diff:#?}");
                let editor = self.create_editor_with_subscriptions(ctx);
                let file_path = diff.base.file_path.clone();

                // Set up the editor buffer with the pre-loaded content.
                let standardized_path = StandardizedPath::try_new(&file_path).ok();
                editor.update(ctx, |editor_view, ctx| {
                    editor_view.set_language_with_local_path(Path::new(&file_path), ctx);
                    let state = InitialBufferState::plain_text(&diff.base.content);
                    editor_view.reset(state, ctx);
                });

                // Create the InlineDiffView which applies diffs to the editor buffer.
                let diff_viewer = ctx.add_typed_action_view(|ctx| {
                    InlineDiffView::new(
                        editor.clone(),
                        Some(diff.diff_type),
                        Some(display_mode),
                        standardized_path,
                        ctx,
                    )
                });

                // On non-WASM, register the file with FileModel for save support.
                #[cfg(not(target_family = "wasm"))]
                {
                    let session_type = &self.diff_session_type;
                    diff_viewer.update(ctx, |view, ctx| view.register_file(session_type, ctx));
                }

                self.setup_diff_view_subscriptions(&diff_viewer, idx, file_path, ctx);

                PendingDiff {
                    diff_view: diff_viewer,
                    tab_handle: Default::default(),
                }
            })
            .collect();

        self.pending_diffs = pending_diffs;
        ctx.emit(CodeDiffViewEvent::LoadedDiffs);
        ctx.notify();
    }

    /// Save the file and mark the diff as accepted.
    pub fn accept_and_save(&mut self, ctx: &mut ViewContext<Self>) {
        // Don't let users save an old diff while in view-only mode.
        if matches!(self.state, CodeDiffState::ViewOnly { .. }) {
            return;
        }

        // Todo:   kc INT-328 Handle error in save
        for diff in &self.pending_diffs {
            diff.diff_view
                .update(ctx, |v, ctx| v.accept_and_save_diff(ctx));
        }
        self.state = CodeDiffState::Accepted(Some(SavingDiffs::new(self.pending_diffs.len())));
        ctx.notify();

        self.minimize(ctx);
    }

    pub fn try_accept_action(&mut self, ctx: &mut ViewContext<Self>) {
        let _ = self.try_accept_action_with_selection(AcceptSelection::Only, ctx);
    }

    /// Attempts to accept the diff and returns Ok(()) if the accept flow was initiated.
    /// Returns Err when acceptance is disallowed (e.g. view-only mode).
    fn try_accept_action_with_selection(
        &mut self,
        selection: AcceptSelection,
        ctx: &mut ViewContext<Self>,
    ) -> Result<()> {
        if matches!(self.state, CodeDiffState::ViewOnly { .. }) {
            log::error!("Attempted to accept diff in view-only mode");
            return Err(anyhow::anyhow!(
                "Attempted to accept diff in view-only mode"
            ));
        }

        match selection {
            AcceptSelection::Only => {
                send_telemetry_from_ctx!(
                    RequestFileEditsTelemetryEvent::EditAcceptClicked(EditAcceptClickedEvent {
                        identifiers: self.identifiers.clone(),
                        passive_diff: self.is_passive,
                    }),
                    ctx
                );
            }
            AcceptSelection::AndContinueWithAgent => {
                send_telemetry_from_ctx!(
                    RequestFileEditsTelemetryEvent::EditAcceptAndContinueClicked(
                        EditAcceptAndContinueClickedEvent {
                            identifiers: self.identifiers.clone(),
                        }
                    ),
                    ctx
                );
            }
            AcceptSelection::AndAutoExecute => {}
        }

        if self.display_mode().is_inline_banner() {
            self.set_embedded_display_mode(true, ctx);
        }

        ctx.emit(CodeDiffViewEvent::TryAccept);
        Ok(())
    }

    /// Mark the diff as rejected.
    pub fn reject(&mut self, ctx: &mut ViewContext<Self>) {
        // Don't let users reject an old diff while in view-only mode.
        if matches!(self.state, CodeDiffState::ViewOnly { .. }) {
            log::error!("Attempted to reject diff in view-only mode");
            return;
        }

        for diff in &self.pending_diffs {
            diff.diff_view.update(ctx, |v, ctx| v.reject_diff(ctx));
        }
        self.state = CodeDiffState::Rejected;
        self.should_expand_when_complete = true;

        if self.is_passive {
            ctx.emit(CodeDiffViewEvent::CancelPassive);
        } else {
            ctx.emit(CodeDiffViewEvent::Rejected);
        }
        ctx.notify();

        self.minimize(ctx);

        // Handled in `CodeDiffView` instead of `CodeDiffModel` so we emit one event for all files.
        // This isn't emitted in the executor because rejected diffs aren't executed.
        self.send_telemetry_for_edit_resolution(RequestedEditResolution::Reject, ctx);
    }

    /// Revert all changes by replacing file contents with the base version.
    /// For newly created files, this deletes them instead.
    fn revert_changes(&mut self, ctx: &mut ViewContext<Self>) {
        if !matches!(self.state, CodeDiffState::Accepted(None)) {
            log::warn!(
                "Attempted to revert changes when not in Accepted(None) state - actual state: {:?}",
                self.state
            );
            return;
        }

        let window_id = ctx.window_id();
        for diff in &self.pending_diffs {
            if let Err(err) = diff
                .diff_view
                .update(ctx, |v, ctx| v.restore_diff_base(ctx))
            {
                log::error!("Failed to restore diff base: {err:?}");
                let file_name = diff
                    .diff_view
                    .as_ref(ctx)
                    .file_name()
                    .unwrap_or_else(|| "file".to_string());
                ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                    toast_stack.add_ephemeral_toast(
                        DismissibleToast::error(format!("Failed to revert changes to {file_name}")),
                        window_id,
                        ctx,
                    );
                });
            }
        }

        self.state = CodeDiffState::Reverted;
        self.mark_action_as_reverted(ctx);
        ctx.notify();
    }

    fn mark_action_as_reverted(&self, ctx: &mut ViewContext<Self>) {
        if let Some(conversation_id) = self.identifiers.client_conversation_id {
            let action_id = self.action_id.clone();
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
                if let Some(conversation) = history_model.conversation_mut(&conversation_id) {
                    conversation.mark_action_as_reverted(action_id, ctx);
                }
            });
        }
    }

    /// Set the view state to the specified state.
    /// This is used when restoring conversations to set the appropriate state based on action status.
    /// Restored views should start unexpanded by default.
    pub fn set_state(&mut self, state: CodeDiffState, ctx: &mut ViewContext<Self>) {
        self.should_expand_when_complete = false;
        self.state = state;
        ctx.notify();
    }

    /// If this view is in full pane mode, close the pane and adjust the settings
    /// so this view can be displayed in the blocklist.
    fn minimize(&mut self, ctx: &mut ViewContext<Self>) {
        if self.display_mode().is_full_pane() {
            self.set_embedded_display_mode(true, ctx);
            if let Some(original_pane_id) = self.original_pane_id {
                self.close_and_focus(original_pane_id, ctx);
            } else {
                self.close(ctx);
            }
        } else if self.display_mode().is_inline_banner() {
            self.set_embedded_display_mode(true, ctx);
        }
    }

    pub fn expand_and_edit(&mut self, ctx: &mut ViewContext<Self>) {
        // Don't let users edit an old diff while in view-only mode.
        if matches!(self.state, CodeDiffState::ViewOnly { .. }) {
            log::error!("Attempted to edit/expand diff in view-only mode");
            return;
        }

        if self.display_mode().is_inline_banner() {
            self.set_embedded_display_mode(true, ctx);
        }

        ctx.emit(CodeDiffViewEvent::ViewDetails);
        self.set_edit_mode(true, ctx);

        // After opening for edit, focus the embedded editor so it is active by default.
        if let Some(current) = self.pending_diffs.get(self.selected_tab) {
            current.diff_view.update(ctx, |v, ctx| {
                v.editor().update(ctx, |editor, ctx| editor.focus(ctx));
            });
        }
    }

    pub fn set_edit_mode(&mut self, enabled: bool, ctx: &mut ViewContext<Self>) {
        ctx.emit(CodeDiffViewEvent::EditModeChanged { enabled });
        ctx.focus_self();
        ctx.notify();
    }

    pub fn is_expanded(&self) -> bool {
        self.state.is_waiting_for_user()
            || self.should_expand_when_complete
            || self.display_mode().is_full_pane()
    }

    fn is_inline_banner_expanded(&self) -> bool {
        matches!(
            self.display_mode(),
            DisplayMode::InlineBanner {
                is_expanded: true,
                ..
            }
        )
    }

    pub fn is_inline_banner_dismissed(&self) -> bool {
        matches!(
            self.display_mode(),
            DisplayMode::InlineBanner {
                is_dismissed: true,
                ..
            }
        )
    }

    fn render_scroll_icon_for_inline_banner(&self, appearance: &Appearance) -> Box<dyn Element> {
        let background = appearance.theme().foreground();
        let icon = warpui::elements::Icon::new(
            Icon::ArrowDown.into(),
            appearance.theme().main_text_color(background).into_solid(),
        )
        .finish();

        Hoverable::new(
            self.button_mouse_states.scroll_icon_button.clone(),
            move |mouse_state| {
                let background = if mouse_state.is_clicked() {
                    blended_colors::fg_overlay_3(appearance.theme())
                } else if mouse_state.is_mouse_over_element() {
                    blended_colors::fg_overlay_2(appearance.theme())
                } else {
                    background
                };

                Container::new(
                    ConstrainedBox::new(icon)
                        .with_width(16.)
                        .with_height(16.)
                        .finish(),
                )
                .with_background(background)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                .with_uniform_padding(2.)
                .finish()
            },
        )
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(CodeDiffViewAction::ScrollToExpand);
        })
        .with_cursor(Cursor::PointingHand)
        .finish()
    }

    /// Returns the regular row, the compact row,
    /// and the size switch threshold that tells us when to switch between the two.
    fn render_compactible_edits_buttons(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> (Box<dyn Element>, Box<dyn Element>, f32) {
        match self.display_mode() {
            DisplayMode::FullPane => {
                self.render_compactible_requested_edits_buttons_in_full_pane(appearance, app)
            }
            DisplayMode::Embedded { .. } => {
                self.render_compactible_requested_edits_buttons_in_blocklist(appearance, app)
            }
            DisplayMode::InlineBanner { .. } => {
                self.render_compactible_buttons_in_inline_banner(appearance, app)
            }
        }
    }

    fn render_compactible_buttons_in_inline_banner(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> (Box<dyn Element>, Box<dyn Element>, f32) {
        let mut buttons: Vec<&dyn RenderCompactibleActionButton> =
            vec![&self.cancel_button, &self.edit_button];
        buttons.extend(self.compactible_accept_buttons());

        let button_rows = render_compact_and_regular_button_rows(buttons, None, appearance, app);

        (button_rows.0, button_rows.1, MEDIUM_SIZE_SWITCH_THRESHOLD)
    }

    fn render_compactible_requested_edits_buttons_in_blocklist(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> (Box<dyn Element>, Box<dyn Element>, f32) {
        let mut buttons: Vec<&dyn RenderCompactibleActionButton> =
            vec![&self.cancel_button, &self.edit_button];
        buttons.extend(self.compactible_accept_buttons());

        let button_rows = render_compact_and_regular_button_rows(
            buttons,
            if self.is_complete() {
                Some(self.should_expand_when_complete)
            } else {
                None
            },
            appearance,
            app,
        );

        (button_rows.0, button_rows.1, XLARGE_SIZE_SWITCH_THRESHOLD)
    }

    fn compactible_accept_buttons(&self) -> Vec<&dyn RenderCompactibleActionButton> {
        let mut buttons: Vec<&dyn RenderCompactibleActionButton> = Vec::new();
        if self.is_passive() {
            buttons.push(&self.iterate_with_agent_button);
        }
        buttons.push(&self.accept_and_autoexecute_split_button);
        buttons
    }

    fn render_compactible_requested_edits_buttons_in_full_pane(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> (Box<dyn Element>, Box<dyn Element>, f32) {
        let mut buttons: Vec<&dyn RenderCompactibleActionButton> =
            vec![&self.cancel_button, &self.minimize_button];
        buttons.extend(self.compactible_accept_buttons());
        let button_rows = render_compact_and_regular_button_rows(buttons, None, appearance, app);

        (button_rows.0, button_rows.1, MEDIUM_SIZE_SWITCH_THRESHOLD)
    }

    fn render_code_header_line_stats(
        &self,
        lines_added: usize,
        lines_removed: usize,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let font_family = appearance.ui_font_family();
        let font_size = appearance.monospace_font_size() - 1.0;
        let theme = appearance.theme();

        let mut stats = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);

        let mut stats_children = vec![];

        if lines_added > 0 {
            // Create the +# text in green
            let added_text = Text::new_inline(format!("+{lines_added}"), font_family, font_size)
                .with_line_height_ratio(1.)
                .with_color(add_color(appearance))
                .with_selectable(false)
                .finish();
            stats_children.push(added_text);
        }

        if lines_removed > 0 {
            // Create the -# text in red
            let removed_text =
                Text::new_inline(format!("-{lines_removed}"), font_family, font_size)
                    .with_line_height_ratio(1.)
                    .with_color(remove_color(appearance))
                    .with_selectable(false)
                    .finish();
            stats_children.push(removed_text);
        }

        // This should never happen, but just in case, return an empty transparent element
        // instead of a colored dot.
        if stats_children.is_empty() {
            return Empty::new().finish();
        }

        let separator = Container::new(
            ConstrainedBox::new(Empty::new().finish())
                .with_height(4.)
                .with_width(4.)
                .finish(),
        )
        .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
        .with_background(neutral_4(theme))
        .with_horizontal_margin(8.)
        .finish();

        if stats_children.len() > 1 {
            stats_children.insert(1, separator);
        }

        stats.add_children(stats_children);

        Container::new(stats.finish())
            .with_background(neutral_1(theme))
            .with_border(Border::all(1.).with_border_fill(neutral_4(theme)))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
            .with_vertical_padding(4.)
            .with_horizontal_padding(8.)
            .with_horizontal_margin(8.)
            .finish()
    }

    fn render_icon(
        &self,
        header_background: Fill,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Option<Box<dyn Element>> {
        let icon_size = if self.display_mode().is_inline_banner() {
            appearance.monospace_font_size()
        } else {
            icon_size(app)
        };

        let icon = match self.state {
            CodeDiffState::Accepted(_) | CodeDiffState::ViewOnly { is_complete: true } => {
                green_check_icon(appearance).finish()
            }
            CodeDiffState::Rejected => cancelled_icon(appearance).finish(),
            CodeDiffState::Reverted => reverted_icon(appearance).finish(),
            CodeDiffState::Queued | CodeDiffState::ViewOnly { is_complete: false } => {
                icons::gray_stop_icon(appearance).finish()
            }
            CodeDiffState::WaitingForUser => {
                if self.display_mode().is_inline_banner() {
                    warpui::elements::Icon::new(
                        Icon::Code2.into(),
                        appearance
                            .theme()
                            .main_text_color(header_background)
                            .into_solid(),
                    )
                    .finish()
                } else {
                    yellow_stop_icon(appearance).finish()
                }
            }
        };

        Some(
            ConstrainedBox::new(icon)
                .with_width(icon_size)
                .with_height(icon_size)
                .finish(),
        )
    }

    fn render_header(
        &self,
        is_expanded: bool,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let (regular_row, compact_row, size_switch_threshold) =
            self.render_compactible_edits_buttons(appearance, app);

        let regular_header = self.render_header_contents(is_expanded, regular_row, appearance, app);
        let compact_header = self.render_header_contents(is_expanded, compact_row, appearance, app);

        let size_switch_threshold = size_switch_threshold * appearance.monospace_ui_scalar();
        SizeConstraintSwitch::new(
            regular_header,
            vec![(
                SizeConstraintCondition::WidthLessThan(size_switch_threshold),
                compact_header,
            )],
        )
        .finish()
    }

    fn render_header_contents(
        &self,
        is_expanded: bool,
        action_buttons: Box<dyn Element>,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        if self.display_mode().is_inline_banner() {
            return self.render_header_in_inline_banner(action_buttons, appearance, app);
        }

        let header_background = appearance.theme().surface_2();
        let mut header_row = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center);

        let mut left_content_row = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::Start)
            .with_cross_axis_alignment(CrossAxisAlignment::Center);

        if let Some(title) = &self.title {
            let title = Text::new_inline(
                title.to_string(),
                appearance.ui_font_family(),
                appearance.monospace_font_size(),
            )
            .with_color(
                if matches!(
                    self.state,
                    CodeDiffState::Queued | CodeDiffState::ViewOnly { is_complete: false }
                ) {
                    appearance
                        .theme()
                        .disabled_text_color(header_background)
                        .into_solid()
                } else {
                    appearance
                        .theme()
                        .main_text_color(header_background)
                        .into_solid()
                },
            )
            .with_selectable(false)
            .finish();
            if let Some(icon) = self.render_icon(header_background, appearance, app) {
                left_content_row.add_child(Container::new(icon).with_margin_right(12.).finish());
            }
            left_content_row.add_child(
                Shrinkable::new(1., Container::new(title).with_margin_right(12.).finish()).finish(),
            );
        }

        let (total_added, total_removed) = self.pending_diffs_line_counts(app);
        if total_added > 0 || total_removed > 0 {
            let stats_badge =
                self.render_code_header_line_stats(total_added, total_removed, appearance);
            // Wrap the stats badge in a clickable element that opens code review.
            let clickable_stats =
                Hoverable::new(self.button_mouse_states.stats_badge_button.clone(), |_| {
                    stats_badge
                })
                .on_click(|ctx, _, _| {
                    ctx.dispatch_typed_action(CodeDiffViewAction::OpenCodeReviewPane);
                })
                .with_cursor(Cursor::PointingHand)
                .with_defer_events_to_children()
                .finish();
            left_content_row.add_child(clickable_stats);
        }

        header_row.add_child(
            Clipped::new(Shrinkable::new(1.0, left_content_row.finish()).finish()).finish(),
        );

        let mut right_side_row = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::End)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Min);

        let file_locations: Vec<LocalOrRemotePath> = self
            .pending_diffs
            .iter()
            .filter_map(|diff| {
                self.location_for_standardized_path(diff.diff_view.as_ref(app).file_path()?)
            })
            .collect();

        // Renders the 'open skill' button only if every edited file lives in the same skill directory.
        let skill_paths = file_locations
            .iter()
            .map(skill_path_from_location)
            .collect::<Option<Vec<_>>>();
        let skill = skill_paths.and_then(|skill_paths| {
            let first_path = skill_paths.first()?;
            skill_paths
                .iter()
                .all(|path| path == first_path)
                .then(|| SkillManager::as_ref(app).skill_by_path(first_path))
                .flatten()
        });
        if let Some(skill) = skill {
            let skill_path = skill.path.clone();
            let skill_reference = SkillManager::handle(app)
                .as_ref(app)
                .reference_for_skill_path(&skill_path);
            let skill_button_handle = self.button_mouse_states.skill_button_handle.clone();

            let skill_icon_override = icon_override_for_skill_name(&skill.name);
            let skill_button = render_skill_button(
                format!("/{}", skill.name).as_str(),
                skill_button_handle.clone(),
                appearance,
                skill.provider,
                skill_icon_override,
                move |ctx| {
                    ctx.dispatch_typed_action(CodeDiffViewAction::OpenSkill {
                        reference: skill_reference.clone(),
                        path: skill_path.clone(),
                        mouse_state: skill_button_handle.clone(),
                    });
                },
            );
            right_side_row.add_child(
                Container::new(skill_button)
                    .with_margin_right(HEADER_MARGIN)
                    .finish(),
            );
        }

        // Renders the 'open config' button only when every MCP config file in this diff
        // belongs to the same provider. Mixed-provider diffs (e.g. editing both a Claude
        // config and a Warp config at once) show no badge to avoid misleading attribution.
        // MCP config actions currently operate on local paths only.
        let local_file_paths: Vec<PathBuf> = file_locations
            .iter()
            .filter_map(|path| path.to_local_path().map(Path::to_path_buf))
            .collect();
        let mcp_configs: Vec<_> = local_file_paths
            .iter()
            .filter_map(|path| {
                mcp_provider_from_file_path(path).map(|provider| (provider, path.to_path_buf()))
            })
            .collect();
        let mcp_config = mcp_configs
            .first()
            .and_then(|(first_provider, first_path)| {
                mcp_configs
                    .iter()
                    .all(|(p, _)| p == first_provider)
                    .then(|| (*first_provider, first_path.clone()))
            });
        if let Some((provider, config_path)) = mcp_config {
            let mcp_button_handle = self.button_mouse_states.mcp_config_button_handle.clone();
            let icon = provider.icon();
            let color = if provider == MCPProvider::Claude {
                Fill::Solid(CLAUDE_ORANGE)
            } else {
                fg_overlay_6(appearance.theme())
            };
            let mcp_config_button = render_provider_icon_button(
                "Open config",
                mcp_button_handle.clone(),
                appearance,
                icon,
                color,
                move |ctx| {
                    ctx.dispatch_typed_action(CodeDiffViewAction::OpenMCPConfig {
                        provider,
                        path: config_path.clone(),
                        mouse_state: mcp_button_handle.clone(),
                    });
                },
            );
            right_side_row.add_child(
                Container::new(mcp_config_button)
                    .with_margin_right(HEADER_MARGIN)
                    .finish(),
            );
        }

        if matches!(self.state, CodeDiffState::WaitingForUser) {
            right_side_row.add_child(action_buttons);
        } else {
            // Don't show the code review button for viewers of shared sessions
            if !matches!(self.state, CodeDiffState::ViewOnly { .. }) {
                right_side_row.add_child(
                    Container::new(ChildView::new(&self.code_review_button).finish())
                        .with_margin_right(HEADER_MARGIN)
                        .finish(),
                );
            }

            let expansion_button = if is_expanded {
                &self.expansion_button_expanded
            } else {
                &self.expansion_button_collapsed
            };
            right_side_row.add_child(ChildView::new(expansion_button).finish());
        }

        header_row.add_child(right_side_row.finish());

        // The border has a radius of 8px, so we need to shrink this by 1 to
        // have it actually match the curvature of the 1px border.
        let radius = Radius::Pixels(7.);
        let corner_radius = if is_expanded {
            CornerRadius::with_top(radius)
        } else {
            CornerRadius::with_all(radius)
        };
        let container = Container::new(header_row.finish())
            .with_horizontal_padding(INLINE_ACTION_HORIZONTAL_PADDING)
            .with_vertical_padding(HEADER_VERTICAL_PADDING)
            .with_background(header_background)
            .with_corner_radius(corner_radius)
            .finish();

        // Create base header with defer_events_to_children to allow button clicks
        if self.is_complete() && self.display_mode().is_embedded() {
            return Hoverable::new(self.button_mouse_states.show_hide_button.clone(), |_| {
                container
            })
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(CodeDiffViewAction::ToggleRequestedEditVisibility);
            })
            .with_cursor(Cursor::PointingHand)
            .with_defer_events_to_children()
            .finish();
        }

        container
    }

    fn render_header_in_inline_banner(
        &self,
        action_buttons: Box<dyn Element>,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let header_background = appearance.theme().background();
        let mut header_row = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Start);

        let mut left_content_container = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::Start)
            .with_cross_axis_alignment(CrossAxisAlignment::Start);

        if let Some(icon) = self.render_icon(header_background, appearance, app) {
            left_content_container.add_child(
                Container::new(icon)
                    .with_margin_right(HEADER_MARGIN)
                    .finish(),
            );
        }

        let mut col = Flex::column();
        if let Some(title) = &self.title {
            let title = Text::new_inline(
                title.to_string(),
                appearance.ui_font_family(),
                appearance.monospace_font_size() + 2.,
            )
            .with_color(
                appearance
                    .theme()
                    .main_text_color(header_background)
                    .into_solid(),
            )
            .with_selectable(false)
            .finish();
            col.add_child(title);
        }
        if let Some(subtitle) = self.display_mode().title() {
            let subtitle = Text::new_inline(
                subtitle.to_string(),
                appearance.ui_font_family(),
                appearance.monospace_font_size(),
            )
            .with_color(
                appearance
                    .theme()
                    .sub_text_color(header_background)
                    .into_solid(),
            )
            .with_selectable(false)
            .finish();
            col.add_child(subtitle);
        }

        left_content_container.add_child(
            Shrinkable::new(
                1.,
                Container::new(col.finish())
                    .with_margin_right(HEADER_MARGIN)
                    .finish(),
            )
            .finish(),
        );

        header_row.add_child(Shrinkable::new(1., left_content_container.finish()).finish());

        if !self.is_complete() {
            // If this step is completed, we don't need to render the action buttons.
            header_row.add_child(action_buttons)
        };

        Container::new(header_row.finish())
            .with_padding_top(16.)
            .with_padding_bottom(HEADER_VERTICAL_PADDING)
            .finish()
    }

    /// Returns the rename target path if this diff is a rename, None otherwise.
    fn get_rename_target(diff_type: Option<&DiffType>) -> Option<&Path> {
        match diff_type {
            Some(DiffType::Update {
                rename: Some(rename_to),
                ..
            }) => Some(rename_to.as_path()),
            _ => None,
        }
    }

    /// Returns true if this diff is a rename without any content changes.
    fn is_rename_without_changes(diff_type: Option<&DiffType>) -> bool {
        match diff_type {
            Some(DiffType::Update {
                rename: Some(_),
                deltas,
            }) => deltas.is_empty(),
            _ => false,
        }
    }

    fn render_file_selection(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let theme = appearance.theme();

        let (
            file_tab_height,
            file_tab_width,
            file_tab_vertical_padding,
            file_tab_horizontal_padding,
        ) = if self.display_mode().is_inline_banner() {
            (FILE_TAB_HEIGHT - 10., FILE_TAB_WIDTH - 10., 2., 12.)
        } else {
            (FILE_TAB_HEIGHT, FILE_TAB_WIDTH, 8., 8.)
        };

        let mut row = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::Start)
            .with_cross_axis_alignment(CrossAxisAlignment::Center);

        for (idx, diff) in self.pending_diffs.iter().enumerate() {
            let diff_type = diff.diff_view.as_ref(app).diff();
            let file_name = match diff.diff_view.as_ref(app).file_name() {
                Some(file_name) if matches!(diff_type, Some(DiffType::Create { .. })) => {
                    format!("{file_name} (new)")
                }
                Some(file_name) if matches!(diff_type, Some(DiffType::Delete { .. })) => {
                    format!("{file_name} (deleted)")
                }
                Some(file_name) => {
                    // Check if this is a rename
                    if let Some(rename_to) = Self::get_rename_target(diff_type) {
                        // Extract just the filename from the rename target path
                        let rename_file_name = rename_to
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or_default();
                        format!("{file_name} → {rename_file_name}")
                    } else {
                        file_name
                    }
                }
                None => "No file name".to_string(),
            };

            // Get the full path for the tooltip
            let tooltip_text = diff
                .diff_view
                .as_ref(app)
                .file_path()
                .map(|p| p.to_string())
                .unwrap_or_else(|| file_name.clone());

            let button = Hoverable::new(diff.tab_handle.clone(), |tab_handle| {
                let background = if idx == self.selected_tab {
                    theme.surface_2()
                } else if tab_handle.is_mouse_over_element() {
                    theme.surface_3()
                } else {
                    theme.surface_1()
                };

                let mut stack = Stack::new();
                let mut container = Container::new(
                    ConstrainedBox::new(
                        Align::new(
                            Container::new(
                                Text::new_inline(
                                    file_name.clone(),
                                    appearance.ui_font_family(),
                                    FILE_TAB_FONT_SIZE,
                                )
                                .with_color(blended_colors::text_sub(theme, background))
                                .with_selectable(false)
                                .finish(),
                            )
                            .with_horizontal_padding(file_tab_horizontal_padding)
                            .with_vertical_padding(file_tab_vertical_padding)
                            .finish(),
                        )
                        .finish(),
                    )
                    .with_width(file_tab_width)
                    .with_height(file_tab_height)
                    .finish(),
                )
                .with_background(background)
                .with_horizontal_padding(FILE_TAB_HORIZONTAL_PADDING);

                if idx == 0 && self.display_mode().is_inline_banner() {
                    container =
                        container.with_corner_radius(CornerRadius::with_top(Radius::Pixels(8.)));
                }
                stack.add_child(container.finish());

                if tab_handle.is_hovered() {
                    let tooltip = appearance
                        .ui_builder()
                        .tool_tip(tooltip_text)
                        .build()
                        .finish();

                    stack.add_positioned_overlay_child(
                        tooltip,
                        OffsetPositioning::offset_from_parent(
                            vec2f(10., 1.),
                            ParentOffsetBounds::Unbounded,
                            ParentAnchor::TopLeft,
                            ChildAnchor::BottomLeft,
                        ),
                    );
                }
                stack.finish()
            })
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(CodeDiffViewAction::TabSelected(idx))
            })
            .with_hover_in_delay(Duration::from_millis(300))
            .finish();

            row.add_child(SavePosition::new(button, &self.position_id_for_file(idx)).finish());
        }

        let container = Container::new(
            NewScrollable::horizontal(
                SingleAxisConfig::Clipped {
                    handle: self.scrollable_state.clone(),
                    child: row.finish(),
                },
                theme.nonactive_ui_detail().into(),
                theme.active_ui_detail().into(),
                warpui::elements::Fill::None,
            )
            .with_horizontal_scrollbar(ScrollableAppearance::new(ScrollbarWidth::Custom(4.), true))
            .with_propagate_mousewheel_if_not_handled(true)
            .finish(),
        )
        .with_background(theme.surface_1());

        if self.display_mode().is_inline_banner() {
            container
                .with_corner_radius(CornerRadius::with_top(Radius::Pixels(8.)))
                .with_border(
                    Border::new(1.)
                        .with_sides(true, true, false, true)
                        .with_border_fill(blended_colors::neutral_4(theme)),
                )
                .finish()
        } else {
            container.finish()
        }
    }

    fn render_editor(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let theme = appearance.theme();
        let diff_view = &self.pending_diffs[self.selected_tab].diff_view;
        let diff_type = diff_view.as_ref(app).diff();

        // Check if this is a rename without changes - show placeholder instead of editor
        if Self::is_rename_without_changes(diff_type) {
            let placeholder = Container::new(
                Text::new(
                    "File renamed without changes",
                    appearance.monospace_font_family(),
                    appearance.monospace_font_size(),
                )
                .with_color(theme.main_text_color(theme.background()).into())
                .finish(),
            )
            .with_background(theme.background())
            .with_vertical_padding(8.)
            .with_horizontal_padding(16.);

            return match self.display_mode() {
                DisplayMode::Embedded { .. } => placeholder
                    .with_corner_radius(CornerRadius::with_bottom(Radius::Pixels(8.)))
                    .finish(),
                DisplayMode::InlineBanner { .. } => placeholder
                    .with_border(
                        Border::new(1.)
                            .with_sides(false, true, true, true)
                            .with_border_fill(blended_colors::neutral_4(theme)),
                    )
                    .finish(),
                DisplayMode::FullPane => {
                    Shrinkable::new(1.0, placeholder.with_padding_bottom(12.).finish()).finish()
                }
            };
        }

        let inner_editor = Container::new(ChildView::new(diff_view.as_ref(app).editor()).finish())
            .with_background(theme.background());

        match self.display_mode() {
            DisplayMode::Embedded { max_height } => Container::new(
                ConstrainedBox::new(
                    inner_editor
                        .with_corner_radius(CornerRadius::with_bottom(Radius::Pixels(8.)))
                        .finish(),
                )
                .with_max_height(*max_height)
                .finish(),
            )
            .finish(),
            DisplayMode::InlineBanner { max_height, .. } => {
                let container = Container::new(
                    ConstrainedBox::new(inner_editor.finish())
                        .with_max_height(*max_height)
                        .finish(),
                )
                .with_border(
                    Border::new(1.)
                        .with_sides(false, true, true, true)
                        .with_border_fill(blended_colors::neutral_4(theme)),
                )
                .finish();

                if self.should_show_speedbump {
                    let checkbox = self.render_code_suggestions_toggle(appearance, app);
                    return Flex::column().with_children([container, checkbox]).finish();
                }
                container
            }
            DisplayMode::FullPane => Shrinkable::new(
                1.0,
                Container::new(inner_editor.finish())
                    .with_padding_bottom(12.)
                    .finish(),
            )
            .finish(),
        }
    }

    pub fn state(&self) -> &CodeDiffState {
        &self.state
    }

    pub fn is_complete(&self) -> bool {
        self.state.is_complete()
    }

    fn position_id_for_file(&self, i: usize) -> String {
        format!("CodeDiffView-{}-tab-{i}", &self.position_id_prefix)
    }

    fn position_id_for_inline_editor(&self) -> String {
        format!("CodeDiffView-inline-editor-{}", &self.position_id_prefix)
    }

    fn position_id_for_inline_speedbump(&self) -> String {
        format!("CodeDiffView-inline-speedbump-{}", &self.position_id_prefix)
    }

    fn position_id_for_accept_split_button(&self) -> String {
        Self::get_position_id_for_accept_split_button(&self.position_id_prefix)
    }

    fn get_position_id_for_accept_split_button(position_id_prefix: &str) -> String {
        format!("CodeDiffView-{position_id_prefix}-accept-split")
    }

    fn navigate_to_diff_hunk(&mut self, direction: Direction, ctx: &mut ViewContext<Self>) {
        let Some(editor) = self.pending_diffs.get(self.selected_tab) else {
            return;
        };

        match direction {
            Direction::Next => editor
                .diff_view
                .update(ctx, |v, ctx| v.navigate_next_diff_hunk(ctx)),
            Direction::Previous => editor
                .diff_view
                .update(ctx, |v, ctx| v.navigate_previous_diff_hunk(ctx)),
        };

        if let Some(output_id) = self.server_output_id() {
            send_telemetry_from_ctx!(
                TelemetryEvent::AgentModeCodeDiffHunksNavigated { output_id },
                ctx
            );
        }
    }

    fn select_file(&mut self, direction: Direction, ctx: &mut ViewContext<Self>) {
        let total = self.pending_diffs.len();
        // Cycle up to the start if navigating down at the last index.
        self.selected_tab = match direction {
            Direction::Next => {
                if self.selected_tab == total.saturating_sub(1) {
                    0
                } else {
                    self.selected_tab + 1
                }
            }
            Direction::Previous => {
                if self.selected_tab == 0 {
                    total.saturating_sub(1)
                } else {
                    self.selected_tab - 1
                }
            }
        };
        self.scrollable_state.scroll_to_position(ScrollTarget {
            position_id: self.position_id_for_file(self.selected_tab),
            mode: ScrollToPositionMode::FullyIntoView,
        });
        ctx.notify();

        if let Some(output_id) = self.server_output_id() {
            send_telemetry_from_ctx!(
                TelemetryEvent::AgentModeCodeFilesNavigated {
                    output_id,
                    source: AgentModeCodeFileNavigationSource::NavigationCommand
                },
                ctx
            );
        }
    }

    fn set_display_mode(&mut self, display_mode: DisplayMode, ctx: &mut ViewContext<Self>) {
        self.display_mode = display_mode;
        let is_passive = self.is_passive();

        self.accept_and_autoexecute_split_button.set_keybinding(
            match self.display_mode {
                DisplayMode::Embedded { .. } | DisplayMode::InlineBanner { .. } => {
                    Some(accept_keystroke_source(is_passive))
                }
                DisplayMode::FullPane => None,
            },
            ctx,
        );

        self.edit_button.set_keybinding(
            Some(KeystrokeSource::Fixed(keystroke_for_mode(
                EDIT_OR_EXPAND_KEY,
                is_passive,
            ))),
            ctx,
        );

        if self.display_mode.is_embedded() {
            let label = if self.is_passive {
                SUGGESTED_EDIT_DISMISS_LABEL
            } else {
                REQUESTED_EDIT_CANCEL_LABEL
            };
            self.cancel_button.set_label(label.to_string(), ctx);
        }

        for diff in &self.pending_diffs {
            diff.diff_view
                .update(ctx, |v, ctx| v.set_display_mode(self.display_mode, ctx));
        }
        ctx.emit(CodeDiffViewEvent::DisplayModeChanged);
        ctx.notify();
    }

    /// Set whether this view is being displayed in a full pane or not.
    pub fn set_embedded_display_mode(&mut self, embedded: bool, ctx: &mut ViewContext<Self>) {
        self.set_display_mode(
            if embedded {
                DisplayMode::with_embedded(MAX_EDITOR_HEIGHT)
            } else {
                DisplayMode::FullPane
            },
            ctx,
        );
    }

    pub fn expand_inline_banner(&mut self, ctx: &mut ViewContext<Self>) {
        if let DisplayMode::InlineBanner { is_dismissed, .. } = self.display_mode {
            let display_mode = DisplayMode::InlineBanner {
                max_height: INLINE_EDITOR_HEIGHT_EXPANDED,
                is_expanded: true,
                is_dismissed,
            };
            self.set_display_mode(display_mode, ctx);
        }
    }

    pub fn display_mode(&self) -> &DisplayMode {
        &self.display_mode
    }

    pub fn action_id(&self) -> &AIAgentActionId {
        &self.action_id
    }

    /// Returns the currently selected text within the entire `CodeDiffView` view sub-hierarchy.
    /// There **shouldn't** be more than one instance of selected text at any given time across
    /// any **visible** view within the same `CodeDiffView` view sub-hierarchy.
    ///
    /// Only text selections in the selected diff tab are considered.
    pub fn selected_text(&self, ctx: &AppContext) -> Option<String> {
        let diff = self.pending_diffs.get(self.selected_tab)?;
        diff.diff_view
            .as_ref(ctx)
            .editor()
            .as_ref(ctx)
            .selected_text(ctx)
    }

    /// Clears all text selections in all components within this `CodeDiffView`'s view sub-hierarchy.
    /// Clears all selected text in each code diff.
    pub fn clear_all_selections(&mut self, ctx: &mut ViewContext<Self>) {
        for diff in self.pending_diffs.iter() {
            diff.diff_view.update(ctx, |v, ctx| {
                v.editor()
                    .update(ctx, |editor, ctx| editor.clear_selection(ctx));
            });
        }
    }

    fn server_output_id(&self) -> Option<ServerOutputId> {
        self.identifiers.server_output_id.clone()
    }

    /// Helper function to send telemetry for edit resolution.
    /// Consolidates the common telemetry logic for reject operations.
    fn send_telemetry_for_edit_resolution(
        &self,
        response: RequestedEditResolution,
        ctx: &mut ViewContext<Self>,
    ) {
        let (lines_added, lines_removed) = self.pending_diffs_line_counts(ctx);
        send_telemetry_from_ctx!(
            RequestFileEditsTelemetryEvent::EditResolved(EditResolvedEvent {
                identifiers: self.identifiers.clone(),
                response,
                stats: EditStats {
                    files_edited: self.pending_diffs.len(),
                    lines_added,
                    lines_removed,
                },
                passive_diff: self.is_passive,
            }),
            ctx
        );
    }

    /// We are processing unified diff and saving files concurrently. That's why
    /// we need to have separate handlers for diff calculation and save completed.
    ///
    /// The diffs applied for each CodeEditorView are received individually.
    /// Store this diff, and try to emit the diffs saved event.
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    fn accepted_file_diff_computed(
        &mut self,
        file_idx: usize,
        diff: Rc<DiffResult>,
        ctx: &mut ViewContext<Self>,
    ) {
        if let CodeDiffState::Accepted(Some(state)) = &mut self.state {
            state.mark_diff_computed(file_idx, diff);
            self.try_emit_diffs_saved(ctx);
        } else {
            log::warn!("Received computed diff when not in accepted state");
        }
    }

    /// We are processing unified diff and saving files concurrently. That's why
    /// we need to have separate handlers for diff calculation and save completed.
    ///
    /// The save state for each CodeEditorView are received individually.
    /// Update the accepted diff state, and try to emit the diffs saved event.
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    fn handle_save_completed(
        &mut self,
        file_idx: usize,
        save_error: Option<Rc<FileSaveError>>,
        ctx: &mut ViewContext<Self>,
    ) {
        if let CodeDiffState::Accepted(Some(state)) = &mut self.state {
            state.mark_diff_saved(file_idx, save_error);
            self.try_emit_diffs_saved(ctx);
        } else if !matches!(self.state, CodeDiffState::Reverted) {
            log::warn!("Received saved diff when not in accepted or reverted state");
        }
    }

    /// Check if we have all pending diffs computed and saved.
    /// Emit the SavedAcceptedDiffs event if so.
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    fn try_emit_diffs_saved(&mut self, ctx: &mut ViewContext<Self>) {
        if let CodeDiffState::Accepted(state) = &mut self.state {
            let is_complete = state
                .as_ref()
                .map(|diff_state| diff_state.pending_diff_is_complete())
                .unwrap_or(false);

            if is_complete {
                let replaced_state = state.take().expect("Checked above");

                let mut combined_diff = DiffResult::default();
                let mut save_errors = Vec::new();
                for diff_state in replaced_state.pending_diffs {
                    if let Some(diff) = diff_state.computed_diff {
                        combined_diff += diff.as_ref();
                    }
                    if let SaveStatus::Failed(error) = diff_state.save_status {
                        save_errors.push(error);
                    }
                }

                let mut updated_files = Vec::new();
                let mut deleted_files = Vec::new();
                let mut edited_file_count = 0;
                let mut correction_count = 0;
                let mut edited_correction_count = 0;
                let mut unedited_correction_count = 0;

                for diff in self.pending_diffs.iter() {
                    let Some(path) = diff.diff_view.as_ref(ctx).file_path() else {
                        continue;
                    };

                    let mut file_path_str = path.to_string();
                    if matches!(
                        diff.diff_view.as_ref(ctx).diff(),
                        Some(DiffType::Delete { .. })
                    ) {
                        deleted_files.push(file_path_str);
                    } else {
                        // If this was a rename, the file being renamed should be considered "deleted".
                        if let Some(DiffType::Update {
                            rename: Some(rename),
                            ..
                        }) = diff.diff_view.as_ref(ctx).diff()
                        {
                            deleted_files.push(file_path_str);
                            file_path_str = rename.to_string_lossy().to_string();
                        }
                        let was_edited = diff.diff_view.as_ref(ctx).was_edited();
                        let changed_lines = diff.diff_view.as_ref(ctx).changed_lines(ctx);
                        let has_malformed_terminal_signal = diff
                            .diff_view
                            .as_ref(ctx)
                            .diff()
                            .is_some_and(|editor_diff| {
                                has_malformed_terminal_correction_signal(
                                    editor_diff,
                                    &changed_lines,
                                )
                            });

                        if was_edited {
                            edited_file_count += 1;
                        }
                        if has_malformed_terminal_signal {
                            correction_count += 1;
                            if was_edited {
                                edited_correction_count += 1;
                            } else {
                                unedited_correction_count += 1;
                            }
                        }
                        updated_files.push((
                            FileLocations {
                                name: file_path_str,
                                lines: if FeatureFlag::ChangedLinesOnlyApplyDiffResult.is_enabled()
                                {
                                    changed_lines
                                } else {
                                    vec![]
                                },
                            },
                            was_edited,
                        ));
                    }
                }
                if correction_count > 0 {
                    send_telemetry_from_ctx!(
                        RequestFileEditsTelemetryEvent::MalformedFinalLineProxy(
                            MalformedFinalLineProxyEvent {
                                identifiers: self.identifiers.clone(),
                                file_count: self.pending_diffs.len(),
                                edited_file_count,
                                correction_count,
                                edited_correction_count,
                                unedited_correction_count,
                                format_kind: self.edit_format_kind,
                                passive_diff: self.is_passive,
                            }
                        ),
                        ctx
                    );
                }

                // Extract accepted file contents from editor buffers so the
                // executor doesn't need to re-read from disk or the network.
                let file_contents: Vec<(String, String)> = self
                    .pending_diffs
                    .iter()
                    .filter_map(|diff| {
                        let path = diff.diff_view.as_ref(ctx).file_path()?.to_string();
                        // Skip deleted files — they have no meaningful content.
                        if matches!(
                            diff.diff_view.as_ref(ctx).diff(),
                            Some(DiffType::Delete { .. })
                        ) {
                            return None;
                        }
                        let content = diff
                            .diff_view
                            .as_ref(ctx)
                            .editor()
                            .as_ref(ctx)
                            .text(ctx)
                            .into_string();
                        Some((path, content))
                    })
                    .collect();

                ctx.emit(CodeDiffViewEvent::SavedAcceptedDiffs {
                    diff: combined_diff,
                    updated_files,
                    file_contents,
                    deleted_files,
                    save_errors,
                });
            }
        }
    }

    pub fn set_original_pane_id(&mut self, original_pane_id: Option<PaneId>) {
        self.original_pane_id = original_pane_id;
    }

    fn close_and_focus(&self, pane_to_focus: PaneId, ctx: &mut ViewContext<Self>) {
        ctx.emit(CodeDiffViewEvent::Pane(PaneEvent::CloseAndFocus {
            pane_to_focus,
        }));
    }

    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    pub fn dismiss(&mut self, ctx: &mut ViewContext<Self>) {
        // Only inline banners can be dismissed.
        let DisplayMode::InlineBanner {
            max_height,
            is_expanded,
            ..
        } = self.display_mode
        else {
            return;
        };
        self.set_display_mode(
            DisplayMode::InlineBanner {
                max_height,
                is_expanded,
                is_dismissed: true,
            },
            ctx,
        );

        // Keybindings should not be available when the banner is dismissed.
        self.accept_and_autoexecute_split_button
            .set_keybinding(None, ctx);
        self.edit_button.set_keybinding(None, ctx);
        self.cancel_button.set_keybinding(None, ctx);

        ctx.notify();
    }

    /// Render the code suggestions toggle checkbox for passive code diffs.
    fn render_code_suggestions_toggle(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let font_color = theme.sub_text_color(theme.background()).into_solid();
        let font_family = appearance.ui_font_family();
        let font_size = 12.;

        let checked = AISettings::as_ref(app).is_code_suggestions_enabled(app);
        let checkbox = appearance
            .ui_builder()
            .checkbox(
                self.button_mouse_states
                    .passive_code_suggestion_checkbox
                    .clone(),
                Some(font_size),
            )
            .check(!checked)
            .with_style(UiComponentStyles {
                font_color: Some(font_color),
                font_size: Some(font_size),
                ..Default::default()
            })
            .build()
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(CodeDiffViewAction::ToggleCodeSuggestions);
            })
            .with_cursor(Cursor::PointingHand)
            .finish();

        let checkbox_text = appearance
            .ui_builder()
            .span("Don't show me suggested code banners again")
            .with_style(UiComponentStyles {
                font_color: Some(font_color),
                font_size: Some(font_size),
                padding: Some(Coords::default().left(4.)),
                ..Default::default()
            })
            .build()
            .finish();

        let formatted_text = FormattedTextElement::new(
            FormattedText::new([FormattedTextLine::Line(vec![
                FormattedTextFragment::hyperlink(
                    "Manage suggested code banner settings",
                    "Settings > AI",
                ),
            ])]),
            font_size,
            font_family,
            font_family,
            font_color,
            self.button_mouse_states
                .ai_settings_link_highlight_index
                .clone(),
        )
        .with_hyperlink_font_color(blended_colors::accent_fg_strong(theme).into())
        .register_default_click_handlers(|_, ctx, _| {
            ctx.dispatch_typed_action(CodeDiffViewAction::OpenSettings);
        })
        .finish();

        let mut container = Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(
                    Flex::row()
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_children([checkbox, checkbox_text])
                        .finish(),
                )
                .with_child(formatted_text)
                .finish(),
        )
        .with_horizontal_padding(INLINE_ACTION_HORIZONTAL_PADDING)
        .with_background(theme.surface_1())
        .with_border(
            Border::new(1.)
                .with_sides(false, true, true, true)
                .with_border_fill(blended_colors::neutral_4(theme)),
        );

        if self.is_inline_banner_expanded() {
            container = container.with_corner_radius(CornerRadius::with_bottom(Radius::Pixels(8.)))
        }

        SavePosition::new(container.finish(), &self.position_id_for_inline_speedbump()).finish()
    }

    pub fn primary_file_path(&self, app: &AppContext) -> Option<String> {
        let first = self.pending_diffs.first()?;
        first
            .diff_view
            .as_ref(app)
            .file_path()
            .map(|p| p.to_string())
    }

    /// Returns the primary file location as a `LocalOrRemotePath`,
    /// using `diff_session_type` to correctly identify remote files.
    pub fn primary_file_location(&self, app: &AppContext) -> Option<LocalOrRemotePath> {
        self.pending_diffs
            .first()?
            .diff_view
            .as_ref(app)
            .file_path()
            .and_then(|path| self.location_for_standardized_path(path))
    }

    fn location_for_standardized_path(&self, path: &StandardizedPath) -> Option<LocalOrRemotePath> {
        match &self.diff_session_type {
            DiffSessionType::Local => path.to_local_path().map(LocalOrRemotePath::Local),
            DiffSessionType::Remote(host_id) => Some(LocalOrRemotePath::Remote(RemotePath {
                host_id: host_id.clone(),
                path: path.clone(),
            })),
        }
    }
}

impl Entity for CodeDiffView {
    type Event = CodeDiffViewEvent;
}

impl View for CodeDiffView {
    fn ui_name() -> &'static str {
        "CodeDiffView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let is_expanded = self.is_expanded();

        let header = self.render_header(is_expanded, appearance, app);
        let mut flex = Flex::column().with_child(header);

        if self.pending_diffs.is_empty() {
            return flex.finish();
        }

        if is_expanded {
            let file_selection = self.render_file_selection(appearance, app);
            let editor = self.render_editor(appearance, app);
            flex.add_children([file_selection, editor]);

            if self.display_mode().is_inline_banner()
                && !self.is_inline_banner_expanded()
                && !self.is_inline_banner_dismissed()
            {
                // Scroll icon is anchored to either the editor or the speedbump, depending on which is visible.
                let (saved_position_id, saved_position_anchor) = if self.should_show_speedbump {
                    (
                        self.position_id_for_inline_speedbump(),
                        PositionedElementAnchor::TopMiddle,
                    )
                } else {
                    (
                        self.position_id_for_inline_editor(),
                        PositionedElementAnchor::BottomMiddle,
                    )
                };

                let mut stack = Stack::new();
                stack.add_child(
                    SavePosition::new(flex.finish(), &self.position_id_for_inline_editor())
                        .finish(),
                );
                // Add the inline banner scroll icon overlay
                stack.add_positioned_child(
                    self.render_scroll_icon_for_inline_banner(appearance),
                    OffsetPositioning::offset_from_save_position_element(
                        saved_position_id,
                        vec2f(0., -8.),
                        PositionedElementOffsetBounds::ParentByPosition,
                        saved_position_anchor,
                        ChildAnchor::BottomMiddle,
                    ),
                );

                if self.is_accept_split_button_menu_open {
                    stack.add_positioned_child(
                        ChildView::new(&self.accept_split_button_menu).finish(),
                        OffsetPositioning::offset_from_save_position_element(
                            self.position_id_for_accept_split_button(),
                            vec2f(0., 8.),
                            PositionedElementOffsetBounds::WindowByPosition,
                            PositionedElementAnchor::BottomRight,
                            ChildAnchor::TopRight,
                        ),
                    );
                }

                return EventHandler::new(stack.finish())
                    .on_scroll_wheel(move |ctx, _, delta, _| {
                        if delta.y() < 0. {
                            ctx.dispatch_typed_action(CodeDiffViewAction::ScrollToExpand);
                            DispatchEventResult::StopPropagation
                        } else {
                            DispatchEventResult::PropagateToParent
                        }
                    })
                    .finish();
            }
        }

        let mut root_stack = Stack::new();
        root_stack.add_child(flex.finish());

        if self.is_accept_split_button_menu_open {
            root_stack.add_positioned_child(
                ChildView::new(&self.accept_split_button_menu).finish(),
                OffsetPositioning::offset_from_save_position_element(
                    self.position_id_for_accept_split_button(),
                    vec2f(0., 8.),
                    PositionedElementOffsetBounds::WindowByPosition,
                    PositionedElementAnchor::BottomRight,
                    ChildAnchor::TopRight,
                ),
            );
        }

        root_stack.finish()
    }

    fn keymap_context(&self, _app: &AppContext) -> warpui::keymap::Context {
        let mut context = Self::default_keymap_context();

        if self.display_mode().is_full_pane() {
            context.set.insert(DISPATCHED_REQUESTED_EDIT_EXPANDED);
        } else if self.is_inline_banner_dismissed() {
            context.set.remove(SUGGESTED_EDIT_INLINE_BANNER);
        } else if self.display_mode().is_inline_banner() {
            context.set.insert(SUGGESTED_EDIT_INLINE_BANNER);
        }

        context
    }

    fn on_focus(&mut self, _focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        // If this is a passive code diff, focus the terminal/input editor on render instead.
        if self.is_passive() && self.display_mode().is_inline_banner() {
            ctx.emit(CodeDiffViewEvent::Blur);
        }
    }
}

impl TypedActionView for CodeDiffView {
    type Action = CodeDiffViewAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            CodeDiffViewAction::TryAccept => {
                self.try_accept_action(ctx);
            }
            CodeDiffViewAction::Reject => {
                self.reject(ctx);
            }
            CodeDiffViewAction::ToggleAcceptMenu => {
                self.open_accept_split_button_menu(ctx);
            }
            CodeDiffViewAction::AcceptAndAutoExecute => {
                // First, accept (same flow as TryAccept)
                if self
                    .try_accept_action_with_selection(AcceptSelection::AndAutoExecute, ctx)
                    .is_ok()
                {
                    // Then, request enabling auto-execute mode
                    ctx.emit(CodeDiffViewEvent::EnableAutoexecuteMode);
                }
            }
            CodeDiffViewAction::AcceptPassiveDiffAndContinueWithAgent => {
                if self
                    .try_accept_action_with_selection(AcceptSelection::AndContinueWithAgent, ctx)
                    .is_ok()
                {
                    ctx.emit(CodeDiffViewEvent::ContinuePassiveCodeDiffWithAgent {
                        accepted: true,
                    });
                }
            }
            CodeDiffViewAction::IterateOnPassiveDiffWithAgent => {
                // Rejects the diff but enters an agent context so that
                // the user can iterate on the diff with the agent.
                self.is_passive = false;
                self.reject(ctx);
                ctx.emit(CodeDiffViewEvent::ContinuePassiveCodeDiffWithAgent { accepted: false });
            }
            CodeDiffViewAction::ToggleRequestedEditVisibility => {
                self.should_expand_when_complete = !self.should_expand_when_complete;
                ctx.emit(CodeDiffViewEvent::ToggledEditVisibility);
                ctx.notify();
            }
            CodeDiffViewAction::TabSelected(idx) => {
                if *idx < self.pending_diffs.len() {
                    self.selected_tab = *idx;
                    ctx.notify();

                    if let Some(output_id) = self.server_output_id() {
                        send_telemetry_from_ctx!(
                            TelemetryEvent::AgentModeCodeFilesNavigated {
                                output_id,
                                source: AgentModeCodeFileNavigationSource::SelectedFileTab
                            },
                            ctx
                        );
                    }
                }
            }
            CodeDiffViewAction::Edit => {
                self.expand_and_edit(ctx);
            }
            CodeDiffViewAction::Minimize => {
                self.minimize(ctx);
                self.set_edit_mode(false, ctx);
                ctx.notify();
            }
            CodeDiffViewAction::NavigateToDiffHunk(direction) => {
                self.navigate_to_diff_hunk(*direction, ctx);
            }
            CodeDiffViewAction::SelectFile(direction) => {
                self.select_file(*direction, ctx);
            }
            CodeDiffViewAction::ScrollToExpand => {
                self.expand_inline_banner(ctx);
                send_telemetry_from_ctx!(
                    TelemetryEvent::ExpandedCodeSuggestions {
                        identifiers: self.identifiers.clone(),
                    },
                    ctx
                );
            }
            CodeDiffViewAction::ToggleCodeSuggestions => {
                let checked = AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    settings
                        .code_suggestions_enabled_internal
                        .toggle_and_save_value(ctx)
                });
                ctx.notify();

                if let Ok(checked) = checked {
                    send_telemetry_from_ctx!(
                        TelemetryEvent::ToggleCodeSuggestionsSetting {
                            source: ToggleCodeSuggestionsSettingSource::Speedbump,
                            is_code_suggestions_enabled: checked,
                        },
                        ctx
                    );
                }
            }
            CodeDiffViewAction::OpenSettings => {
                ctx.emit(CodeDiffViewEvent::OpenSettings);
            }
            CodeDiffViewAction::OpenCodeReviewPane => {
                self.code_review_button.update(ctx, |_, ctx| {
                    ctx.notify();
                });
                ctx.emit(CodeDiffViewEvent::ToggleCodeReviewPane {
                    entrypoint: CodeReviewPaneEntrypoint::CodeDiffHeader,
                });
                ctx.notify();
            }
            CodeDiffViewAction::RevertChanges => {
                self.revert_changes(ctx);
            }
            CodeDiffViewAction::OpenSkill {
                reference,
                path,
                mouse_state,
            } => {
                // Sends a telemetry event when a skill is opened from a code diff view
                send_telemetry_from_ctx!(
                    SkillTelemetryEvent::Opened {
                        reference: reference.clone(),
                        name: SkillManager::as_ref(ctx)
                            .skill_by_reference(reference)
                            .map(|skill| skill.name.clone()),
                        origin: SkillOpenOrigin::EditFiles,
                    },
                    ctx
                );

                // Resets the interaction state of the skill button to avoid an immediate re-hover
                if let Ok(mut state) = mouse_state.lock() {
                    state.reset_interaction_state();
                }

                ctx.emit(CodeDiffViewEvent::OpenSkill {
                    reference: reference.clone(),
                    path: path.clone(),
                });
            }
            CodeDiffViewAction::OpenMCPConfig {
                provider,
                path,
                mouse_state,
            } => {
                // Resets the interaction state of the button to avoid an immediate re-hover
                if let Ok(mut state) = mouse_state.lock() {
                    state.reset_interaction_state();
                }

                ctx.emit(CodeDiffViewEvent::OpenMCPConfig {
                    provider: *provider,
                    path: path.clone(),
                });
            }
        }
    }
}

/// Converts a list of FileEdits to a list of FileDiffs with mocked content.
/// For restored FileEdits we don't have access to the actual file content, so we construct
/// a lossy version of the FileDiff.
pub fn convert_file_edits_to_file_diffs(
    file_edits: Vec<FileEdit>,
    shell_launch_data: &Option<ShellLaunchData>,
    current_working_directory: &Option<String>,
) -> Vec<FileDiff> {
    // Group file edits by file path
    let mut edits_by_file: HashMap<String, Vec<FileEdit>> = HashMap::new();
    for edit in file_edits {
        if let Some(file_path) = edit.file().map(|f| f.to_string()) {
            edits_by_file.entry(file_path).or_default().push(edit);
        }
    }

    edits_by_file
        .into_iter()
        .filter(|(_, edits)| {
            // Filter out files that have no valid edits
            edits.iter().any(|edit| {
                matches!(
                    edit,
                    FileEdit::Edit(_)
                        | FileEdit::Create {
                            content: Some(_),
                            ..
                        }
                        | FileEdit::Delete { .. }
                )
            })
        })
        .map(|(file_path, edits)| {
            let path =
                host_native_absolute_path(&file_path, shell_launch_data, current_working_directory);

            // Extract search content from file edits to create dummy content
            let mut search_and_replace_diffs = Vec::new();
            let mut v4a_hunks: Vec<V4AHunk> = Vec::new();
            let mut v4a_move_to: Option<String> = None;
            let mut create_diffs = Vec::new();
            let mut max_line_number = 0;
            let mut search_blocks_with_ranges = Vec::new();

            // Track if the file should be shown as deleted (explicit delete or move to another file)
            let mut show_as_deleted = false;

            for edit in &edits {
                match edit {
                    FileEdit::Edit(parsed_diff) => match parsed_diff {
                        ParsedDiff::StrReplaceEdit { .. } => {
                            if let Ok(search_replace) =
                                SearchAndReplace::try_from(parsed_diff.clone())
                            {
                                // Parse line numbers from the search content
                                let (line_range, search_content) =
                                    parse_line_numbers(&search_replace.search);

                                // Track the maximum line number we've seen
                                if let Some(range) = &line_range {
                                    max_line_number = max_line_number.max(range.end);
                                }

                                // Store the search content with its line range for later use
                                search_blocks_with_ranges.push((line_range, search_content));

                                search_and_replace_diffs.push(search_replace);
                            }
                        }
                        ParsedDiff::V4AEdit { hunks, move_to, .. } => {
                            // For V4A edits, collect hunks and track move_to
                            v4a_hunks.extend(hunks.clone());
                            if move_to.is_some() {
                                v4a_move_to = move_to.clone();
                                // If this file is being moved/renamed, show source as deleted
                                show_as_deleted = true;
                            }

                            // Build dummy content from V4A hunks using pre_context + old + post_context
                            for hunk in hunks {
                                let mut hunk_content = String::new();
                                if !hunk.pre_context.is_empty() {
                                    hunk_content.push_str(&hunk.pre_context);
                                    if !hunk_content.ends_with('\n') {
                                        hunk_content.push('\n');
                                    }
                                }
                                if !hunk.old.is_empty() {
                                    hunk_content.push_str(&hunk.old);
                                    if !hunk_content.ends_with('\n') {
                                        hunk_content.push('\n');
                                    }
                                }
                                if !hunk.post_context.is_empty() {
                                    hunk_content.push_str(&hunk.post_context);
                                }
                                // We don't have line numbers for V4A hunks in restored state,
                                // so use None for the range
                                if !hunk_content.is_empty() {
                                    search_blocks_with_ranges.push((None, hunk_content));
                                }
                            }
                        }
                    },
                    FileEdit::Create {
                        content: Some(content),
                        ..
                    } => {
                        // For file creation, create a DiffDelta that inserts the content at the beginning
                        create_diffs.push(DiffDelta {
                            replacement_line_range: 0..0,
                            insertion: content.clone(),
                        });
                    }
                    FileEdit::Delete { .. } => {
                        show_as_deleted = true;
                    }
                    _ => {}
                }
            }

            // Create dummy file content with proper line numbering
            let mut dummy_content = if max_line_number > 0 || !search_blocks_with_ranges.is_empty()
            {
                search_blocks_with_ranges.sort_unstable_by_key(|(range, _)| {
                    range.as_ref().map(|r| r.start).unwrap_or(0)
                });

                let mut dummy_content = String::new();

                // If the first change is not at the start of the file, prepend an ellipsis
                let first_start = search_blocks_with_ranges
                    .iter()
                    .filter_map(|(r, _)| r.as_ref().map(|r| r.start))
                    .min();
                if first_start.is_some_and(|s| s > 1) {
                    dummy_content.push_str("...\n");
                }

                for (index, (_, search_content)) in search_blocks_with_ranges.iter().enumerate() {
                    if index > 0 {
                        // add "..." between each edit to indicate the separation
                        dummy_content.push_str("...\n");
                    }
                    for search_line in search_content.lines() {
                        dummy_content.push_str(search_line);
                        dummy_content.push('\n');
                    }
                }

                dummy_content
            } else {
                // No line numbers found, create minimal dummy content
                String::new()
            };

            // For file deletions/moves we may not have any other context to show. Provide a minimal stub.
            if show_as_deleted && dummy_content.is_empty() {
                dummy_content = if v4a_move_to.is_some() {
                    "(renamed)".to_string()
                } else {
                    "(deleted file)".to_string()
                };
            }

            // Create diff deltas using fuzzy matching for search-and-replace diffs
            let mut applied_diffs = Vec::new();

            if !search_and_replace_diffs.is_empty() {
                let fuzzy_match_result =
                    fuzzy_match_diffs(&path, &search_and_replace_diffs, &dummy_content);
                if let ai::diff_validation::DiffType::Update { deltas, .. } =
                    fuzzy_match_result.diff_type
                {
                    applied_diffs.extend(deltas);
                }
            }

            // Handle V4A hunks using fuzzy_match_v4a_diffs
            if !v4a_hunks.is_empty() && !show_as_deleted {
                let v4a_match_result =
                    fuzzy_match_v4a_diffs(&path, &v4a_hunks, v4a_move_to.clone(), &dummy_content);
                if let ai::diff_validation::DiffType::Update { deltas, .. } =
                    v4a_match_result.diff_type
                {
                    applied_diffs.extend(deltas);
                }
            }

            // Add create diffs directly
            applied_diffs.extend(create_diffs);

            // For deletions/moves, prefer showing a whole-file deletion.
            if show_as_deleted {
                let num_lines = dummy_content.lines().count();
                applied_diffs.clear();
                if num_lines > 0 {
                    applied_diffs.push(DiffDelta {
                        replacement_line_range: 1..num_lines.saturating_add(1),
                        insertion: String::new(),
                    });
                }
            }

            FileDiff::new(dummy_content, path, DiffType::update(applied_diffs, None))
        })
        .collect()
}

impl BackingView for CodeDiffView {
    type PaneHeaderOverflowMenuAction = CodeDiffViewAction;
    type CustomAction = ();
    type AssociatedData = ();

    fn handle_pane_header_overflow_menu_action(
        &mut self,
        action: &Self::PaneHeaderOverflowMenuAction,
        ctx: &mut ViewContext<Self>,
    ) {
        self.handle_action(action, ctx);
    }

    fn close(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(CodeDiffViewEvent::Pane(PaneEvent::Close));
    }

    fn handle_custom_action(
        &mut self,
        _custom_action: &Self::CustomAction,
        _ctx: &mut ViewContext<Self>,
    ) {
    }

    fn focus_contents(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.focus_self();
    }

    fn render_header_content(
        &self,
        _ctx: &view::HeaderRenderContext<'_>,
        _app: &AppContext,
    ) -> view::HeaderContent {
        // Code diffs should show "Requested Edit" as the title and hide the close button
        // since they are closed via accept/reject actions.
        view::HeaderContent::Standard(view::StandardHeader {
            title: "Requested Edit".to_string(),
            title_secondary: None,
            title_style: None,
            title_clip_config: warpui::text_layout::ClipConfig::start(),
            title_max_width: None,
            left_of_title: None,
            right_of_title: None,
            left_of_overflow: None,
            options: view::StandardHeaderOptions {
                hide_close_button: true,
                ..Default::default()
            },
        })
    }

    fn set_focus_handle(&mut self, focus_handle: PaneFocusHandle, _ctx: &mut ViewContext<Self>) {
        self.focus_handle = Some(focus_handle);
    }
}

fn accept_keystroke_source(is_passive: bool) -> KeystrokeSource {
    if FeatureFlag::AgentView.is_enabled() && is_passive {
        KeystrokeSource::Binding(ACCEPT_PROMPT_SUGGESTION_KEYBINDING)
    } else {
        KeystrokeSource::Fixed(keystroke_for_mode(ACCEPT_KEY, is_passive))
    }
}

/// Returns a keystroke based on key, OS, and passive state.
///
/// This assumes that when the diff is passive, the keybindings are cmd or ctrl-shift modified,
/// depending on the host OS.
fn keystroke_for_mode(key: &str, is_passive: bool) -> Keystroke {
    let is_mac = OperatingSystem::get().is_mac();
    Keystroke {
        cmd: is_mac && is_passive,
        ctrl: !is_mac && is_passive,
        shift: !is_mac && is_passive,
        key: key.to_owned(),
        ..Default::default()
    }
}
