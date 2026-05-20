use std::{
    collections::{HashMap, HashSet},
    mem,
    ops::Range,
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
};

use crate::{
    ai::{
        agent::{AgentReviewCommentBatch, DiffSetHunk},
        blocklist::agent_view::AgentViewEntryOrigin,
    },
    code::editor::comment_editor::DEFAULT_COMMENT_MAX_WIDTH,
    coding_panel_enablement_state::CodingPanelEnablementState,
};

#[cfg(feature = "local_fs")]
use crate::code_review::context::{
    create_attachment_reference_and_key, register_diffset_attachment,
};
use crate::{
    ai::agent::CurrentHead,
    code::editor::view::CodeEditorRenderOptions,
    code::editor::{CommentEditor, CommentEditorEvent, EditorCommentsModel, EditorReviewComment},
    code_review::{comments::ReviewCommentBatch, DiffSetScope},
};
use crate::{
    ai::agent::{AIAgentAttachment, DiffBase},
    code::{
        editor::{
            view::{CodeEditorEvent, CodeEditorView},
            GutterHoverTarget,
        },
        editor_management::CodeEditorStatus,
        local_code_editor::{
            render_unsaved_circle_with_tooltip, LocalCodeEditorEvent, LocalCodeEditorView,
        },
        view::PendingSaveIntent,
    },
    code_review::{
        comments::AttachedReviewCommentTarget,
        context::convert_file_diffs_to_diffset_hunks,
        diff_state::{
            DiffHunk, DiffLineType, DiffMode, DiffState, DiffStateModel, DiffStateModelEvent,
            DiffStats, FileDiff, FileDiffAndContent, FileStatusInfo, GitDiffWithBaseContent,
            GitFileStatus,
        },
        editor_state::CodeReviewEditorState,
        hidden_lines::calculate_hidden_lines,
        telemetry_event::{
            AddToContextOrigin, CodeReviewContextDestination, CodeReviewTelemetryEvent,
            GitButtonKind, PaneStateChange,
        },
    },
};

#[cfg(feature = "local_fs")]
use crate::code_review::telemetry_event::DiffSetContextScope;

use crate::{
    code::editor::line::EditorLineLocation,
    ui_components::dialog::{dialog_styles, Dialog},
};
use crate::{
    code::global_buffer_model::GlobalBufferModel, code_review::comments::ReviewCommentBatchEvent,
};
use crate::{
    menu::{Event as MenuEvent, Menu, MenuItem, MenuItemFields},
    pane_group::{
        focus_state::{PaneFocusHandle, PaneGroupFocusEvent},
        PaneId,
    },
    quit_warning::UnsavedStateSummary,
    terminal::input::MenuPositioning,
    terminal::view::{CliAgentRouting, InitProjectModel, TerminalAction, TerminalView},
    util::bindings::{custom_tag_to_keystroke, CustomAction},
    view_components::{
        action_button::{
            ActionButton, ActionButtonTheme, AdjoinedSide, ButtonSize, DangerPrimaryTheme,
            KeystrokeSource, NakedTheme, PaneHeaderTheme, SecondaryTheme,
        },
        DismissibleToast,
    },
    workspace::{ToastStack, Workspace, WorkspaceAction},
};

use crate::code_review::find_model::CodeReviewFindModel;
#[cfg(feature = "local_fs")]
use crate::server::telemetry::CodePanelsFileOpenEntrypoint;
use crate::terminal::cli_agent::{
    build_selection_line_range_prompt, build_selection_substring_prompt,
};
#[cfg(feature = "local_fs")]
use crate::util::file::external_editor::EditorSettings;
use crate::util::git::{get_all_branches, BranchEntry};
#[cfg(feature = "local_fs")]
use crate::util::openable_file_type::resolve_file_target_with_editor_choice;
#[cfg(feature = "local_fs")]
use crate::util::openable_file_type::FileTarget;
use crate::view_components::find::{Event as FindViewEvent, Find, FindEvent, FindWithinBlockState};
use ai::project_context::model::ProjectContextModel;
#[cfg(feature = "local_fs")]
use num_traits::SaturatingSub;
use string_offset::CharOffset;

use indexmap::IndexMap;
use itertools::Itertools;
use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::{vec2f, Vector2F};
use rand::{distributions::Alphanumeric, Rng};
use warp_core::{
    channel::{Channel, ChannelState},
    features::FeatureFlag,
    safe_error, safe_info,
    ui::theme::color::internal_colors,
};
use warpui::{
    clipboard::ClipboardContent,
    elements::{
        new_scrollable::{
            NewScrollable, NewScrollableElement, ScrollableAppearance, SingleAxisConfig,
        },
        resizable_state_handle, Align, Border, ChildAnchor, ChildView, ClippedScrollStateHandle,
        ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, DispatchEventResult,
        DragBarSide, Element, Empty, EventHandler, Flex, List, ListState, MainAxisAlignment,
        MouseStateHandle, OffsetPositioning, ParentAnchor, ParentElement, ParentOffsetBounds,
        Percentage, PositionedElementAnchor, PositionedElementOffsetBounds, Radius, Rect,
        Resizable, ResizableStateHandle, ScrollOffset, ScrollStateHandle, ScrollbarWidth, Stack,
        Text, DEFAULT_UI_LINE_HEIGHT_RATIO,
    },
    keymap::Keystroke,
    ui_components::{
        button::{ButtonVariant, TextAndIcon, TextAndIconAlignment},
        components::{Coords, UiComponentStyles},
    },
    units::Pixels,
    AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle, WindowId,
};
use warpui::{
    elements::{Clipped, MainAxisSize, Shrinkable},
    text_layout::{default_compute_baseline_position, ClipConfig},
};
use warpui::{
    elements::{Hoverable, SavePosition},
    platform::Cursor,
    ui_components::components::UiComponent,
};
use warpui::{
    fonts::{Properties, Weight},
    ModelHandle, WeakViewHandle,
};

use crate::code::footer::{CodeFooterView, CodeFooterViewEvent};
use crate::settings::AISettings;
use crate::settings_view::SettingsSection;
use crate::ui_components::{
    blended_colors::{neutral_2, neutral_3},
    buttons::icon_button_with_color,
    icons::Icon,
};
use crate::view_components::action_button::TooltipAlignment;
#[cfg(feature = "local_fs")]
use crate::TelemetryEvent;
use crate::{
    appearance::Appearance,
    code::editor::{add_color, remove_color},
    code_review::diff_selector::{DiffSelector, DiffSelectorEvent, DiffTarget},
    editor::InteractionState,
    pane_group::pane::{view, BackingView, PaneEvent},
    send_telemetry_from_ctx,
    themes::theme::WarpTheme,
};

use vec1::Vec1;

use super::{
    code_review_header::CodeReviewHeader,
    comment_list_view::{CommentListDebugState, CommentListEvent, CommentListView},
    comments::{attach_pending_imported_comments, AttachedReviewComment, CommentOrigin},
    diff_size_limits::DiffSize,
    git_dialog::{GitDialog, GitDialogEvent, GitDialogKind},
    GlobalCodeReviewEvent, GlobalCodeReviewModel,
};
#[cfg(not(target_family = "wasm"))]
use crate::code::buffer_location::LocalOrRemotePath;
use crate::code::ShowCommentEditorProvider;
#[cfg(not(target_family = "wasm"))]
use crate::code::ShowFindReferencesCard;
use crate::code_review::comments::CommentId;
use crate::ui_components::render_file_search_row::{render_file_search_row, FileSearchRowOptions};
use crate::workspace::view::right_panel::{ReviewDestination, ReviewSubmissionResult};
use warp_editor::model::CoreEditorModel;
#[cfg(not(target_family = "wasm"))]
use warp_editor::render::model::AutoScrollMode;
use warp_editor::{
    content::buffer::{AutoScrollBehavior, InitialBufferState, SelectionOffsets},
    render::{element::VerticalExpansionBehavior, model::LineCount},
};
use warp_util::{
    content_version::ContentVersion,
    file::{FileLoadError, FileSaveError},
    path::LineAndColumnArg,
    standardized_path::StandardizedPath,
};

pub struct CodeReviewHeaderFields {
    pub is_in_split_pane: bool,
    pub diff_state_model: ModelHandle<DiffStateModel>,
    pub maximize_button: ViewHandle<ActionButton>,
    pub diff_selector: ViewHandle<DiffSelector>,
    pub header_menu: ViewHandle<Menu<CodeReviewAction>>,
    pub header_menu_open: bool,
    pub header_dropdown_button: ViewHandle<ActionButton>,
    pub has_header_menu_items: bool,
    pub file_nav_button: Option<ViewHandle<ActionButton>>,
    pub primary_git_action_mode: PrimaryGitActionMode,
    pub git_primary_action_button: ViewHandle<ActionButton>,
    pub git_operations_chevron: ViewHandle<ActionButton>,
    pub git_operations_menu: ViewHandle<Menu<CodeReviewAction>>,
    pub git_operations_menu_open: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CodeReviewCommentDebugState {
    pub repo_path: Option<PathBuf>,
    pub has_active_comment_model: bool,
    pub comment_list: CommentListDebugState,
}

/// Renders a file navigation button (sidebar toggle) that can be reused across views.
pub fn render_file_navigation_button<F>(
    appearance: &Appearance,
    is_sidebar_expanded: bool,
    mouse_state: MouseStateHandle,
    on_click: F,
) -> Box<dyn Element>
where
    F: Fn(&mut warpui::EventContext<'_>) + 'static,
{
    let ui_builder = appearance.ui_builder().clone();
    let icon_color = appearance
        .theme()
        .sub_text_color(appearance.theme().background());
    let button = icon_button_with_color(
        appearance,
        if is_sidebar_expanded {
            Icon::LeftSidebarClose
        } else {
            Icon::LeftSidebarOpen
        },
        false,
        mouse_state,
        icon_color,
    )
    .with_tooltip(move || {
        ui_builder
            .tool_tip(if is_sidebar_expanded {
                "Hide file navigation".to_owned()
            } else {
                "Show file navigation".to_owned()
            })
            .build()
            .finish()
    })
    .with_tooltip_position(warpui::ui_components::button::ButtonTooltipPosition::BelowLeft)
    .build()
    .on_click(move |ctx: &mut warpui::EventContext<'_>, _, _| {
        on_click(ctx);
    });

    Container::new(
        ConstrainedBox::new(button.finish())
            .with_height(24.)
            .with_width(24.)
            .finish(),
    )
    .with_margin_right(4.)
    .finish()
}

/// Determines which primary git action the code review header should present.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PrimaryGitActionMode {
    /// There are uncommitted changes. Primary = Commit, dropdown shows
    /// Commit / Push-or-Publish / Create PR with per-item disabled states.
    Commit,
    /// Branch has an upstream and unpushed local commits. Primary = Push,
    /// dropdown shows Commit (greyed) / Push / Create PR.
    Push,
    /// Nothing to commit or push, and no existing PR. Primary = Create PR, chevron hidden.
    CreatePr,
    /// Nothing to commit or push, and a PR exists for this branch. Primary = PR #N, chevron hidden.
    ViewPr,
    /// No upstream tracking branch, but local commits exist. Primary = Publish, chevron hidden.
    Publish,
}

const DEFAULT_FILE_SIDEBAR_WIDTH: f32 = 250.;
const FILE_SIDEBAR_MIN_WIDTH: f32 = 150.;
const FILE_SIDEBAR_MAX_WIDTH: f32 = 800.;
/// This is a best guess effort at getting the height of the file header.
/// If the file header is changed, we should update this value.
const FILE_HEADER_HEIGHT: f32 = 41.;
/// The gap between editors in the viewported list.
const EDITOR_GAP: f32 = 12.;
const FILE_SIDEBAR_PANE_WIDTH_PERCENTAGE: f32 = 0.25;
/// Vertical gap between the right panel header row and the code review content below it
/// (sub-header in loaded state, loading text in loading state).
pub(super) const CONTENT_TOP_MARGIN: f32 = 4.;
/// Horizontal margins for the code review content area. The right panel header
/// uses these same values so its edges align with the content below.
pub(crate) const CONTENT_LEFT_MARGIN: f32 = 16.;
pub(crate) const CONTENT_RIGHT_MARGIN: f32 = 4.;
const CODE_REVIEW_EDITOR_LINE_HEIGHT_RATIO: f32 = 1.4;
/// Extra scroll buffer (in pixels) added when scrolling to a line that has a comment editor below it.
const COMMENT_EDITOR_SCROLL_BUFFER: f32 = 200.0;

pub const CODE_REVIEW_TOOLTIP_TEXT: &str = "View changes";
const REMOTE_TEXT: &str = "Diffs only work for local workspaces.";
const DISABLED_TEXT: &str = "Diffs only work for git repositories.";
const WSL_TEXT: &str = "Diffs don't currently work in WSL.";

#[cfg(not(target_family = "wasm"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
enum InitButtons {
    OpenRepository,
    InitProject,
    None,
}

pub fn get_discard_button_disabled_tooltip(git_operation_blocked: bool) -> String {
    if git_operation_blocked {
        "Cannot discard changes while a git operation (merge, rebase, etc.) is in progress"
            .to_string()
    } else {
        "No changes to discard".to_string()
    }
}

/// Returns true if the file status changed between Deleted and non-Deleted states,
/// which requires rebuilding the editor state because we can't use global buffer
/// for files that don't exist on the file system.
#[cfg(not(target_family = "wasm"))]
fn file_status_changed_deleted_state(
    current_status: &GitFileStatus,
    new_status: &GitFileStatus,
) -> bool {
    matches!(current_status, GitFileStatus::Deleted) != matches!(new_status, GitFileStatus::Deleted)
}

#[cfg(target_family = "wasm")]
fn file_status_changed_deleted_state(
    _current_status: &GitFileStatus,
    _new_status: &GitFileStatus,
) -> bool {
    false
}

#[derive(Clone, Debug, PartialEq)]
pub enum CodeReviewAction {
    OpenInNewTab {
        path: PathBuf,
        line_and_column: Option<LineAndColumnArg>,
    },
    ToggleFileExpanded(PathBuf),
    OpenHeaderMenu,
    SetDiffMode(DiffMode),
    ToggleFileSidebar,
    FileSelected(usize),
    ToggleMaximize,
    SaveAllUnsavedFiles,
    SaveAllFiles {
        paths: Vec<PathBuf>,
    },
    RefreshGitState,
    UndoRevert,
    Close,
    EmitPaneEvent(PaneEvent),
    ShowDiscardConfirmDialog(Option<PathBuf>),
    ConfirmDiscardFile,
    CancelDiscardFile,
    ToggleStashChanges,
    ToggleFileSelection(PathBuf),
    AddDiffSetAsContext(DiffSetScope),
    CopyFilePath(PathBuf),
    OpenCommentComposerFromHeader,
    ShowFindBar,
    FocusView,
    InitProjectForCurrentDirectory,
    OpenRepository,
    OpenCommitDialog,
    ToggleGitOperationsMenu,
    OpenPushDialog,
    OpenCreatePrDialog,
    ViewPr(String),
    PublishBranch,
}

pub struct FileState {
    pub file_diff: FileDiff,
    pub editor_state: Option<CodeReviewEditorState>,
    pub is_expanded: bool,
    sidebar_mouse_state: MouseStateHandle,
    header_mouse_state: MouseStateHandle,
    chevron_button: ViewHandle<ActionButton>,
    open_in_tab_button: ViewHandle<ActionButton>,
    discard_button: ViewHandle<ActionButton>,
    add_context_button: ViewHandle<ActionButton>,
    copy_path_button: ViewHandle<ActionButton>,
}

pub(crate) struct LoadedState {
    pub(crate) file_states: IndexMap<PathBuf, FileState>,
    pub(crate) total_additions: usize,
    pub(crate) total_deletions: usize,
    pub(crate) files_changed: usize,
}

impl LoadedState {
    pub(crate) fn to_diff_stats(&self) -> DiffStats {
        DiffStats {
            files_changed: self.files_changed,
            total_additions: self.total_additions,
            total_deletions: self.total_deletions,
        }
    }

    /// Returns a list of pairs of code editor views and the absolute paths of their underlying files.
    fn editor_absolute_file_paths(
        &self,
        repo_path: &Path,
    ) -> Vec<(ViewHandle<LocalCodeEditorView>, PathBuf)> {
        self.file_states
            .values()
            .filter_map(|file_state| {
                let editor = file_state.editor_state.as_ref()?.editor().clone();
                let file_path = repo_path.join(&file_state.file_diff.file_path);
                Some((editor, file_path))
            })
            .collect()
    }
}

/// State of the code review view
enum CodeReviewViewState {
    None,
    Loaded(LoadedState),
    Error(String),
    NoRepoFound,
}

struct UiStateHandles {
    sidebar_scroll_state: ClippedScrollStateHandle,
    sidebar_resizable_state: ResizableStateHandle,
    retry_button_mouse_state: MouseStateHandle,
}

impl Default for UiStateHandles {
    fn default() -> Self {
        Self {
            sidebar_scroll_state: Default::default(),
            sidebar_resizable_state: resizable_state_handle(DEFAULT_FILE_SIDEBAR_WIDTH),
            retry_button_mouse_state: Default::default(),
        }
    }
}

#[cfg_attr(target_family = "wasm", allow(dead_code))]
struct GitSessionState {
    enablement: CodingPanelEnablementState,
}

#[derive(Clone, Debug)]
pub enum CodeReviewViewEvent {
    Pane(PaneEvent),
    FileEdited {
        path: PathBuf,
    },
    FileSaved {
        path: PathBuf,
    },
    FileLoadError {
        path: PathBuf,
        error: Rc<FileLoadError>,
    },
    FileSaveError {
        path: PathBuf,
        error: Rc<FileSaveError>,
    },
    #[cfg(feature = "local_fs")]
    OpenFileWithTarget {
        path: PathBuf,
        target: FileTarget,
        line_col: Option<LineAndColumnArg>,
    },
    ReviewSubmitted,
    /// Emitted when review comments are ready to be submitted.
    /// A higher-level view (RightPanelView) handles routing to an available terminal.
    SubmitReviewComments {
        comments: AgentReviewCommentBatch,
        repo_path: PathBuf,
    },
    /// Request to open a file in a new tab (e.g. goto-definition).
    OpenFileInNewTab {
        path: PathBuf,
        line_and_column: Option<LineAndColumnArg>,
    },
    /// Request to open LSP logs for the given log file path.
    #[cfg(not(target_family = "wasm"))]
    OpenLspLogs {
        log_path: PathBuf,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum DiscardOperationType {
    AllUncommittedChanges,
    FileUncommittedChanges,
    AllChangesAgainstBranch(Option<String>),
    FileChangesAgainstBranch(Option<String>),
}

impl DiscardOperationType {
    pub fn title(&self) -> String {
        match self {
            DiscardOperationType::AllUncommittedChanges => {
                "Discard uncommitted changes?".to_string()
            }
            DiscardOperationType::FileUncommittedChanges => {
                "Discard all uncommitted changes to file?".to_string()
            }
            DiscardOperationType::AllChangesAgainstBranch(_) => "Discard all changes?".to_string(),
            DiscardOperationType::FileChangesAgainstBranch(_) => {
                "Discard all changes to file?".to_string()
            }
        }
    }

    pub fn description(&self) -> Option<String> {
        match self {
            DiscardOperationType::AllUncommittedChanges => Some("You're about to discard all local changes that haven't been committed.".to_string()),
            DiscardOperationType::FileUncommittedChanges => Some("This will restore this file to the last committed version and discard local edits.".to_string()),
            DiscardOperationType::AllChangesAgainstBranch(None) => Some("You're about to discard all committed and uncommitted changes.".to_string()),
            DiscardOperationType::FileChangesAgainstBranch(None) => Some("This will restore this file to the main branch version and discard all committed and uncommitted edits.".to_string()),
            DiscardOperationType::AllChangesAgainstBranch(Some(_)) => Some("You're about to discard all committed and uncommitted changes.".to_string()),
            DiscardOperationType::FileChangesAgainstBranch(Some(branch)) => Some(format!("This will reset this file to the {branch} branch version and discard all committed and uncommitted edits.")),
        }
    }

    fn is_uncommitted_changes(&self) -> bool {
        matches!(
            self,
            DiscardOperationType::AllUncommittedChanges
                | DiscardOperationType::FileUncommittedChanges
        )
    }
}

pub struct DiscardDialogState {
    show_discard_confirm_dialog: bool,
    discard_file_paths: Vec<PathBuf>,
    selected_files: HashMap<PathBuf, bool>,
    file_checkbox_mouse_states: HashMap<PathBuf, MouseStateHandle>,
    discard_confirm_button: ViewHandle<ActionButton>,
    discard_cancel_button: ViewHandle<ActionButton>,
    stash_changes_enabled: bool,
    stash_changes_checkbox_mouse_state: MouseStateHandle,
    operation_type: DiscardOperationType,
    file_list_scroll_state: ClippedScrollStateHandle,
}

#[cfg_attr(target_family = "wasm", allow(dead_code))]
struct PendingPreciseScroll {
    editor_index: usize,
    /// Starting character offset of the target range to scroll to.
    start_offset: CharOffset,
    /// Ending character offset of the target range to scroll to.
    end_offset: CharOffset,
    /// Extra scroll buffer (in pixels) to scroll past the target line.
    buffer: f32,
}

/// Per-repository state container.
struct RepositoryState {
    repo_path: PathBuf,
    state: CodeReviewViewState,
    available_branches: Vec<BranchEntry>,

    /// Whether a file has been explicitly expanded (true) or collapsed (false).
    file_expanded: HashMap<PathBuf, bool>,
}

impl RepositoryState {
    fn new(repo_path: PathBuf) -> Self {
        Self {
            repo_path,
            state: CodeReviewViewState::None,
            available_branches: Vec::new(),
            file_expanded: HashMap::new(),
        }
    }

    /// If the current state is Loaded, replace it with None and return the LoadedState.
    /// Otherwise leave the state unchanged and return None.
    fn pop_loaded_state(&mut self) -> Option<LoadedState> {
        if !matches!(self.state, CodeReviewViewState::Loaded(_)) {
            return None;
        }

        let CodeReviewViewState::Loaded(loaded_state) =
            mem::replace(&mut self.state, CodeReviewViewState::None)
        else {
            unreachable!("state was verified as Loaded");
        };

        Some(loaded_state)
    }

    fn should_auto_expand_file(&self, file: &FileDiff) -> bool {
        if let Some(manually_expanded) = self.file_expanded.get(&file.file_path) {
            return *manually_expanded;
        }

        if file.is_binary {
            return false;
        }

        if file.size != DiffSize::Normal {
            return false;
        }

        !file.is_autogenerated
    }
}

struct RelocateCommentsResult {
    comments: Vec<AttachedReviewComment>,
    fallback_count: usize,
}

/// State shared among the entire code review view.
pub struct CodeReviewView {
    active_repo: Option<RepositoryState>,

    focus_handle: Option<PaneFocusHandle>,
    maximize_button: ViewHandle<ActionButton>,
    header_dropdown_button: ViewHandle<ActionButton>,
    file_nav_button: ViewHandle<ActionButton>,
    git_primary_action_button: ViewHandle<ActionButton>,
    git_operations_chevron: ViewHandle<ActionButton>,
    git_operations_menu: ViewHandle<Menu<CodeReviewAction>>,
    git_operations_menu_open: bool,
    file_sidebar_expanded: bool,
    /// The file sidebar state from before a code review panel is maximized.
    file_sidebar_expanded_before_maximize: Option<bool>,
    scroll_state: ScrollStateHandle,
    viewported_list_state: ListState<RelocatableScrollContext>,

    window_id: WindowId,

    undo_action_button: ViewHandle<ActionButton>,
    last_revert: Option<(ViewHandle<CodeEditorView>, ContentVersion)>,
    containing_pane_id: Option<PaneId>,
    // Header-specific dropdown menu ("Add diff set as context" / "Add comment")
    header_menu: ViewHandle<Menu<CodeReviewAction>>,
    header_menu_open: bool,
    view_position_id: String,
    /// Position ID of the code review list within this view.
    code_review_list_position_id: String,
    /// Position ID of the header (used for anchoring overlays like the comment composer).
    header_position_id: String,
    discard_dialog_state: DiscardDialogState,

    find_model: ModelHandle<CodeReviewFindModel>,
    find_bar: ViewHandle<Find<CodeReviewFindModel>>,
    comment_list_view: ViewHandle<crate::code_review::comment_list_view::CommentListView>,
    /// Optional overlay composer for creating a new review-level comment.
    comment_composer: Option<ViewHandle<CommentEditor>>,

    /// Precise position to auto-scroll to once editor layout completes
    pending_precise_scroll: Option<PendingPreciseScroll>,
    /// Comment to scroll to once the view finishes loading.
    pending_jump_to_comment: Option<CommentId>,

    active_comment_model: Option<ModelHandle<ReviewCommentBatch>>,

    init_project_button: ViewHandle<ActionButton>,
    #[cfg(not(target_family = "wasm"))]
    open_repository_button: ViewHandle<ActionButton>,

    ui_state_handles: UiStateHandles,
    diff_state_model: ModelHandle<DiffStateModel>,
    diff_selector: ViewHandle<DiffSelector>,
    header: CodeReviewHeader,
    terminal_view: Option<WeakViewHandle<TerminalView>>,
    position_id_prefix: String,
    /// Whether the view is currently open (subscribed to diff state model).
    is_open: bool,
    /// Global LSP footer for the code review panel (workspace mode).
    code_review_footer: Option<ViewHandle<CodeFooterView>>,
    /// Active git-operation dialog overlay (commit / push / publish), if open.
    git_dialog: Option<ViewHandle<GitDialog>>,
}

impl CodeReviewView {
    pub fn repo_path(&self) -> Option<&PathBuf> {
        self.active_repo.as_ref().map(|repo| &repo.repo_path)
    }

    fn to_standardized_path(&self, path: &PathBuf) -> Option<StandardizedPath> {
        if path.is_absolute() {
            Some(StandardizedPath::from_local_absolute_unchecked(path))
        } else {
            let repo_path = self.repo_path()?;
            let absolute_path = repo_path.join(path);
            Some(StandardizedPath::from_local_absolute_unchecked(
                &absolute_path,
            ))
        }
    }

    pub fn diff_state_model(&self) -> &ModelHandle<DiffStateModel> {
        &self.diff_state_model
    }

    /// Called when the code review view is opened/attached to a pane group.
    /// Subscribes to the diff state model and enables metadata refresh.
    pub fn on_open(&mut self, ctx: &mut ViewContext<Self>) {
        if self.is_open {
            return;
        }
        self.is_open = true;

        ctx.subscribe_to_model(&self.diff_state_model, Self::handle_diff_state_model_event);
        if self.repo_path().is_some() {
            self.fetch_branches_and_setup_dropdown(ctx);
        }
        ctx.notify();

        // Create global LSP footer for the code review panel
        if let Some(repo_path) = self.repo_path().cloned() {
            let footer =
                ctx.add_typed_action_view(|ctx| CodeFooterView::new_for_workspace(repo_path, ctx));
            ctx.subscribe_to_view(&footer, Self::handle_footer_event);
            self.code_review_footer = Some(footer);

            // Subscribe to PersistedWorkspace events to refresh the footer
            // UI after LSP installation succeeds or fails.
            #[cfg(feature = "local_fs")]
            {
                use crate::ai::persisted_workspace::{PersistedWorkspace, PersistedWorkspaceEvent};

                // PersistedWorkspace handles spawning the server after install;
                // we only subscribe to refresh the footer UI.
                ctx.subscribe_to_model(&PersistedWorkspace::handle(ctx), |me, _, event, ctx| {
                    match event {
                        PersistedWorkspaceEvent::InstallationSucceeded
                        | PersistedWorkspaceEvent::InstallationFailed => {
                            if let Some(footer) = &me.code_review_footer {
                                footer.update(ctx, |_, ctx| ctx.notify());
                            }
                        }
                        _ => {}
                    }
                });
            }
        }

        self.diff_state_model.update(ctx, |model, ctx| {
            model.set_code_review_metadata_refresh_enabled(true, ctx);
            model.load_diffs_for_current_repo(false, ctx);
        });
    }

    /// Called when the code review view is closed/detached.
    /// Unsubscribes from the diff state model.
    pub fn on_close(&mut self, ctx: &mut ViewContext<Self>) {
        self.is_open = false;

        if self
            .find_model
            .read(ctx, |model, _| model.is_find_bar_open())
        {
            self.close_find_bar(ctx);
        }

        ctx.unsubscribe_to_model(&self.diff_state_model);

        self.code_review_footer = None;

        self.diff_state_model.update(ctx, |model, ctx| {
            model.set_code_review_metadata_refresh_enabled(false, ctx);
        });
    }

    /// Handles events from the global LSP footer in workspace mode.
    fn handle_footer_event(
        &mut self,
        _footer: ViewHandle<CodeFooterView>,
        event: &CodeFooterViewEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            CodeFooterViewEvent::RunTabConfigSkill { .. } => {}
            CodeFooterViewEvent::RestartAllServers { servers } => {
                for server in servers {
                    server.update(ctx, |server, ctx| {
                        server.restart(ctx);
                    });
                }
            }
            CodeFooterViewEvent::StopAllServers { servers } => {
                for server in servers {
                    server.update(ctx, |server, ctx| {
                        let _ = server.stop(true, ctx);
                    });
                }
            }
            CodeFooterViewEvent::StartAllServers { servers } => {
                for server in servers {
                    server.update(ctx, |server, ctx| {
                        let _ = server.manual_start(ctx);
                    });
                }
            }
            CodeFooterViewEvent::ManageServers => {
                ctx.dispatch_typed_action(&WorkspaceAction::ShowSettingsPage(
                    SettingsSection::EditorAndCodeReview,
                ));
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
            CodeFooterViewEvent::OpenLogs { path } => {
                #[cfg(not(target_family = "wasm"))]
                {
                    // Look up the LSP server for this path and emit the log path
                    let lsp_manager = lsp::LspManagerModel::handle(ctx);
                    if let Some(server) = lsp_manager.as_ref(ctx).server_for_path(path, ctx) {
                        let repo_root = server.as_ref(ctx).initial_workspace().to_path_buf();
                        let server_type = server.as_ref(ctx).server_type();
                        let log_path =
                            crate::code::lsp_logs::log_file_path(server_type, &repo_root);
                        ctx.emit(CodeReviewViewEvent::OpenLspLogs { log_path });
                    }
                }
                let _ = path;
            }
            CodeFooterViewEvent::EnableLSP { path, server_type } => {
                Self::handle_enable_lsp(path, *server_type, ctx);
            }
            CodeFooterViewEvent::InstallAndEnableLSP { path, server_type } => {
                Self::handle_install_and_enable_lsp(path, *server_type, ctx);
            }
        }
    }

    /// Enables an LSP server for the workspace. Uses the provided server_type if given,
    /// otherwise derives it from the path.
    #[cfg(feature = "local_fs")]
    fn handle_enable_lsp(
        path: &Path,
        server_type: Option<lsp::supported_servers::LSPServerType>,
        ctx: &mut ViewContext<Self>,
    ) {
        use crate::ai::persisted_workspace::{LspTask, PersistedWorkspace};

        let server_type =
            server_type.or_else(|| lsp::LanguageId::from_path(path).map(|id| id.server_type()));
        let Some(server_type) = server_type else {
            return;
        };

        let repo_root = PersistedWorkspace::as_ref(ctx)
            .root_for_workspace(path)
            .map(|p| p.to_path_buf())
            .or_else(|| {
                repo_metadata::repositories::DetectedRepositories::as_ref(ctx)
                    .get_root_for_path(&warp_util::local_or_remote_path::LocalOrRemotePath::Local(
                        path.to_path_buf(),
                    ))
                    .and_then(|r| r.to_local_path().map(std::path::Path::to_path_buf))
            })
            .or_else(|| path.parent().map(|p| p.to_path_buf()));

        let Some(repo_root) = repo_root else {
            return;
        };

        PersistedWorkspace::handle(ctx).update(ctx, |workspace, _ctx| {
            workspace.enable_lsp_server_for_path(&repo_root, server_type);
        });

        PersistedWorkspace::handle(ctx).update(ctx, |workspace, ctx| {
            workspace.execute_lsp_task(
                LspTask::Spawn {
                    file_path: path.to_path_buf(),
                },
                ctx,
            );
        });
    }

    /// Installs and enables an LSP server for the workspace.
    #[cfg(feature = "local_fs")]
    fn handle_install_and_enable_lsp(
        path: &Path,
        server_type: Option<lsp::supported_servers::LSPServerType>,
        ctx: &mut ViewContext<Self>,
    ) {
        use crate::ai::persisted_workspace::{LspTask, PersistedWorkspace};

        let server_type =
            server_type.or_else(|| lsp::LanguageId::from_path(path).map(|id| id.server_type()));
        let Some(server_type) = server_type else {
            return;
        };

        let repo_root = PersistedWorkspace::as_ref(ctx)
            .root_for_workspace(path)
            .map(|p| p.to_path_buf())
            .or_else(|| {
                repo_metadata::repositories::DetectedRepositories::as_ref(ctx)
                    .get_root_for_path(&warp_util::local_or_remote_path::LocalOrRemotePath::Local(
                        path.to_path_buf(),
                    ))
                    .and_then(|r| r.to_local_path().map(std::path::Path::to_path_buf))
            })
            .or_else(|| path.parent().map(|p| p.to_path_buf()));

        let Some(repo_root) = repo_root else {
            return;
        };

        PersistedWorkspace::handle(ctx).update(ctx, |workspace, ctx| {
            workspace.execute_lsp_task(
                LspTask::Install {
                    file_path: path.to_path_buf(),
                    repo_root,
                    server_type,
                },
                ctx,
            );
        });
    }

    #[cfg(not(feature = "local_fs"))]
    fn handle_enable_lsp(
        _path: &Path,
        _server_type: Option<lsp::supported_servers::LSPServerType>,
        _ctx: &mut ViewContext<Self>,
    ) {
    }

    #[cfg(not(feature = "local_fs"))]
    fn handle_install_and_enable_lsp(
        _path: &Path,
        _server_type: Option<lsp::supported_servers::LSPServerType>,
        _ctx: &mut ViewContext<Self>,
    ) {
    }

    fn set_active_repo_comment_model(
        &mut self,
        new_handle: Option<ModelHandle<ReviewCommentBatch>>,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.active_comment_model == new_handle {
            return;
        }

        if let Some(old_model) = &self.active_comment_model {
            ctx.unsubscribe_to_model(old_model);
        }

        self.active_comment_model = new_handle.clone();

        if let Some(new_model) = new_handle.clone() {
            ctx.subscribe_to_model(&new_model, Self::handle_comment_model_event);
        }

        self.comment_list_view.update(ctx, |view, ctx| {
            view.set_comment_model(new_handle, ctx);
        });

        self.update_editor_comment_markers(ctx);
    }

    fn handle_comment_model_event(
        &mut self,
        model: ModelHandle<ReviewCommentBatch>,
        event: &ReviewCommentBatchEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.active_comment_model.as_ref() != Some(&model) {
            return;
        }

        self.update_editor_comment_markers(ctx);

        if let ReviewCommentBatchEvent::Changed {
            should_reposition_comments: true,
        } = event
        {
            if self.all_editors_loaded() {
                let diff_mode = self.diff_state_model.as_ref(ctx).diff_mode(ctx);
                self.reposition_comments_in_file(&diff_mode, ctx);
            }
        }
    }

    fn clear_comment_locations(
        &self,
        editor_file_paths: &[(ViewHandle<LocalCodeEditorView>, PathBuf)],
        ctx: &mut ViewContext<Self>,
    ) {
        for (editor, _) in editor_file_paths {
            editor.update(ctx, |local_editor, ctx| {
                local_editor.editor().update(ctx, |code_editor, ctx| {
                    code_editor.clear_comment_locations(ctx);
                });
            });
        }
    }

    fn collect_comments_by_file(
        &self,
        model: &ModelHandle<ReviewCommentBatch>,
        editor_file_paths: &[(ViewHandle<LocalCodeEditorView>, PathBuf)],
        ctx: &mut ViewContext<Self>,
    ) -> HashMap<PathBuf, Vec<EditorReviewComment>> {
        model.read(ctx, |batch, _| {
            editor_file_paths
                .iter()
                .map(|(_, file_path)| {
                    (file_path.clone(), batch.editor_comments_for_file(file_path))
                })
                .collect::<HashMap<_, _>>()
        })
    }

    /// Creates a new `ListState` with scroll preservation and sets up debounced
    /// scroll tracking. Used during construction and full invalidation.
    fn create_list_state(ctx: &mut ViewContext<Self>) -> ListState<RelocatableScrollContext> {
        let view_handle: WeakViewHandle<Self> = ctx.handle();
        let render_handle = view_handle.clone();
        #[cfg(not(target_family = "wasm"))]
        let adjustment_handle = view_handle;

        let (list_state, scroll_rx) = ListState::new_with_scroll_preservation(
            move |index, scroll_offset, app| {
                let view_handle = render_handle
                    .upgrade(app)
                    .expect("CodeReviewView dropped during render");
                view_handle
                    .as_ref(app)
                    .render_diff_at_index(index, scroll_offset, app)
            },
            #[cfg(not(target_family = "wasm"))]
            move |index, captured_context, app| {
                Self::adjust_scroll_offset(&adjustment_handle, index, captured_context, app)
            },
            #[cfg(target_family = "wasm")]
            move |_index, _captured_context, _app| None,
        );

        Self::setup_scroll_tracking(scroll_rx, ctx);
        list_state
    }

    fn update_editor_comment_markers(&mut self, ctx: &mut ViewContext<Self>) {
        let CodeReviewViewState::Loaded(state) = self.state() else {
            return;
        };

        let Some(repo_path) = self.repo_path().cloned() else {
            return;
        };

        let editor_file_paths = state.editor_absolute_file_paths(&repo_path);

        let Some(model) = self.active_comment_model.as_ref() else {
            self.clear_comment_locations(&editor_file_paths, ctx);
            return;
        };

        let comments_by_file = self.collect_comments_by_file(model, &editor_file_paths, ctx);

        for (editor, file_path) in editor_file_paths {
            let comments = comments_by_file
                .get(&file_path)
                .cloned()
                .unwrap_or_default();

            editor.update(ctx, |local_editor, ctx| {
                local_editor.editor().update(ctx, |code_editor, ctx| {
                    code_editor.set_comment_locations(comments.into_iter(), ctx);
                });
            });
        }
    }

    pub fn new(
        repo_path: Option<PathBuf>,
        diff_state_model: ModelHandle<DiffStateModel>,
        comment_batch_model: Option<ModelHandle<ReviewCommentBatch>>,
        terminal_view: Option<WeakViewHandle<TerminalView>>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        // TODO(asweet): Migrate subscription and event handling of diff_state_model to RepositoryState

        let diff_selector = ctx.add_typed_action_view(DiffSelector::new);
        ctx.subscribe_to_view(&diff_selector, |me, _, event, ctx| {
            me.handle_diff_selector_event(event, ctx);
        });

        let random_str = rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(8)
            .map(char::from)
            .collect();

        let maximize_button = ctx.add_typed_action_view(move |_| {
            // Since the view isn't part of a pane group yet, default to not-maximized. The button will be updated
            //when focus state changes.
            let (icon, tooltip_text) = (Icon::Maximize, "Maximize");

            ActionButton::new("", NakedTheme)
                .with_icon(icon)
                .with_tooltip(tooltip_text)
                .with_tooltip_positioning_provider(Arc::new(MenuPositioning::BelowInputBox))
                .on_click(|ctx| ctx.dispatch_typed_action(CodeReviewAction::ToggleMaximize))
        });

        let header_dropdown_button = ctx.add_typed_action_view(|_ctx| {
            let theme: Arc<dyn ActionButtonTheme> =
                if FeatureFlag::GitOperationsInCodeReview.is_enabled() {
                    Arc::new(NakedTheme)
                } else {
                    Arc::new(PaneHeaderTheme)
                };
            ActionButton::new_with_boxed_theme("", theme)
                .with_icon(Icon::DotsVertical)
                .on_click(|ctx| ctx.dispatch_typed_action(CodeReviewAction::OpenHeaderMenu))
        });

        let file_nav_button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("", NakedTheme)
                .with_icon(Icon::FileCopy)
                .with_tooltip("Show file navigation")
                .on_click(|ctx| ctx.dispatch_typed_action(CodeReviewAction::ToggleFileSidebar))
        });

        let git_primary_action_button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("Commit", SecondaryTheme)
                .with_size(ButtonSize::Small)
                .with_icon(Icon::GitCommit)
                .with_adjoined_side(AdjoinedSide::Right)
                .on_click(|ctx| ctx.dispatch_typed_action(CodeReviewAction::OpenCommitDialog))
        });

        let git_operations_chevron = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("", SecondaryTheme)
                .with_size(ButtonSize::Small)
                .with_icon(Icon::ChevronDown)
                .with_adjoined_side(AdjoinedSide::Left)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(CodeReviewAction::ToggleGitOperationsMenu)
                })
        });

        let git_operations_menu = ctx.add_typed_action_view(|_| {
            Menu::new()
                .prevent_interaction_with_other_elements()
                .with_drop_shadow()
                .with_width(140.)
        });
        ctx.subscribe_to_view(&git_operations_menu, |me, _, event, ctx| match event {
            MenuEvent::ItemSelected | MenuEvent::Close { .. } => {
                me.git_operations_menu_open = false;
                me.git_operations_chevron.update(ctx, |button, ctx| {
                    button.set_active(false, ctx);
                });
                ctx.notify();
            }
            MenuEvent::ItemHovered => {}
        });

        let list_state = Self::create_list_state(ctx);

        let window_id = ctx.window_id();
        let view_id = ctx.view_id();

        let undo_action_button = ctx.add_typed_action_view(move |ctx| {
            let keybinding = custom_tag_to_keystroke(CustomAction::Undo.into());
            let mut action_button = ActionButton::new("Undo", NakedTheme)
                .with_size(ButtonSize::Small)
                .on_click(move |ctx| {
                    ctx.dispatch_typed_action(WorkspaceAction::UndoRevertInCodeReviewPane {
                        window_id,
                        view_id,
                    })
                });

            if let Some(keybinding) = keybinding {
                action_button =
                    action_button.with_keybinding(KeystrokeSource::Fixed(keybinding), ctx);
            }
            action_button
        });

        let discard_confirm_button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("Discard changes", DangerPrimaryTheme)
                .on_click(|ctx| ctx.dispatch_typed_action(CodeReviewAction::ConfirmDiscardFile))
        });

        let discard_cancel_button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("Cancel", NakedTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(CodeReviewAction::CancelDiscardFile);
            })
        });

        ctx.subscribe_to_model(&GlobalCodeReviewModel::handle(ctx), |me, _, event, ctx| {
            let GlobalCodeReviewEvent::DiffReverted { window_id, view_id } = event;
            if ctx.window_id() == *window_id && ctx.view_id() == *view_id {
                me.maybe_undo_revert(ctx);
            }
        });
        // The diff selector re-reads Appearance on render, so theme / font
        // changes are picked up automatically.

        // Header dropdown menu
        let header_menu = ctx.add_typed_action_view(|_| {
            Menu::new()
                .prevent_interaction_with_other_elements()
                .with_drop_shadow()
        });
        ctx.subscribe_to_view(&header_menu, move |me, _, event, ctx| match event {
            MenuEvent::ItemSelected | MenuEvent::Close { .. } => {
                me.header_menu_open = false;
                me.update_header_dropdown_active_state(ctx);
                ctx.notify();
            }
            MenuEvent::ItemHovered => {
                // No-op for now.
            }
        });

        let discard_dialog_state = DiscardDialogState {
            show_discard_confirm_dialog: false,
            discard_file_paths: Vec::new(),
            selected_files: HashMap::new(),
            file_checkbox_mouse_states: HashMap::new(),
            discard_confirm_button,
            discard_cancel_button,
            stash_changes_enabled: false,
            stash_changes_checkbox_mouse_state: MouseStateHandle::default(),
            operation_type: DiscardOperationType::AllUncommittedChanges,
            file_list_scroll_state: ClippedScrollStateHandle::default(),
        };

        let self_handle = ctx.handle();
        let find_model = ctx.add_model(|ctx| CodeReviewFindModel::new(self_handle.clone(), ctx));
        let find_bar = ctx.add_typed_action_view(|ctx| {
            let mut view = Find::new(find_model.clone(), ctx);
            view.display_find_within_block = FindWithinBlockState::Hidden;
            view
        });
        ctx.subscribe_to_view(&find_bar, move |me, view_handle, event, ctx| {
            me.handle_find_event(view_handle, event, ctx);
        });
        ctx.subscribe_to_model(&find_model, |me, _, event, ctx| {
            me.handle_find_model_event(event, ctx);
        });

        let comment_list_view = ctx
            .add_typed_action_view(|ctx| CommentListView::new(repo_path.clone(), self_handle, ctx));
        ctx.subscribe_to_view(&comment_list_view, |me, _, event, ctx| {
            me.handle_comment_list_event(event, ctx);
        });

        let ui_state_handles = UiStateHandles::default();
        let header = CodeReviewHeader::new();

        let init_project_button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("Initialize codebase", NakedTheme)
                .with_size(ButtonSize::Small)
                .with_tooltip("Enables codebase indexing and WARP.md")
                .with_tooltip_alignment(TooltipAlignment::Center)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(CodeReviewAction::InitProjectForCurrentDirectory)
                })
        });

        #[cfg(not(target_family = "wasm"))]
        let open_repository_button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("Open repository", NakedTheme)
                .with_size(ButtonSize::Small)
                .with_tooltip("Navigate to a repo and initialize it for coding")
                .with_tooltip_alignment(TooltipAlignment::Center)
                .on_click(|ctx| ctx.dispatch_typed_action(CodeReviewAction::OpenRepository))
        });

        let has_repo = repo_path.is_some();
        let active_repo = repo_path.map(|repo_path| RepositoryState::new(repo_path.clone()));

        let mut view = Self {
            active_repo,
            ui_state_handles,
            diff_state_model,
            focus_handle: None,
            diff_selector,
            maximize_button,
            header_dropdown_button,
            file_nav_button,
            git_primary_action_button,
            git_operations_chevron,
            git_operations_menu,
            git_operations_menu_open: false,
            file_sidebar_expanded: false,
            file_sidebar_expanded_before_maximize: None,
            position_id_prefix: random_str,
            viewported_list_state: list_state,
            scroll_state: ScrollStateHandle::default(),
            terminal_view,
            window_id: ctx.window_id(),
            undo_action_button,
            last_revert: None,
            containing_pane_id: None,
            header_menu,
            header_menu_open: false,
            view_position_id: format!("code_review_view_{}", ctx.view_id()),
            code_review_list_position_id: format!("code_review_view_list_{}", ctx.view_id()),
            header_position_id: format!("code_review_view_header_{}", ctx.view_id()),
            discard_dialog_state,
            header,
            find_model,
            find_bar,
            comment_list_view,
            comment_composer: None,
            pending_precise_scroll: None,
            pending_jump_to_comment: None,
            active_comment_model: None,
            init_project_button,
            #[cfg(not(target_family = "wasm"))]
            open_repository_button,
            is_open: false,
            code_review_footer: None,
            git_dialog: None,
        };
        view.set_active_repo_comment_model(comment_batch_model, ctx);
        if has_repo {
            view.fetch_branches_and_setup_dropdown(ctx);
            view.invalidate_all(None, ctx);
        }

        view
    }

    pub fn set_terminal_view(&mut self, terminal_view: WeakViewHandle<TerminalView>) {
        self.terminal_view = Some(terminal_view);
    }

    pub fn set_review_destination(
        &mut self,
        destination: ReviewDestination,
        ctx: &mut ViewContext<Self>,
    ) {
        self.comment_list_view
            .update(ctx, |comment_list_view, ctx| {
                comment_list_view.set_review_destination(destination, ctx);
            });
    }

    pub fn debug_review_comment_state(&self, ctx: &AppContext) -> CodeReviewCommentDebugState {
        let comment_list = self.comment_list_view.as_ref(ctx).debug_state(ctx);

        CodeReviewCommentDebugState {
            repo_path: self.repo_path().cloned(),
            has_active_comment_model: self.active_comment_model.is_some(),
            comment_list,
        }
    }

    fn handle_focus_state_event(
        &mut self,
        event: &PaneGroupFocusEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        if self
            .focus_handle
            .as_ref()
            .is_some_and(|handle| handle.is_affected(event))
        {
            self.update_maximize_button(ctx);
        }
    }

    fn update_maximize_button(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(focus_handle) = &self.focus_handle else {
            return;
        };

        let is_maximized = focus_handle.is_maximized(ctx);
        let (icon, tooltip) = if is_maximized {
            (Icon::Minimize, "Restore")
        } else {
            (Icon::Maximize, "Maximize")
        };

        self.maximize_button.update(ctx, |button, ctx| {
            button.set_icon(Some(icon), ctx);
            button.set_tooltip(Some(tooltip), ctx);
        });
    }

    fn update_file_nav_button_tooltip(&self, ctx: &mut ViewContext<Self>) {
        let tooltip = if self.file_sidebar_expanded {
            "Hide file navigation"
        } else {
            "Show file navigation"
        };
        self.file_nav_button.update(ctx, |button, ctx| {
            button.set_tooltip(Some(tooltip), ctx);
        });
    }

    fn open_file_sidebar(&mut self, ctx: &mut ViewContext<Self>) {
        self.file_sidebar_expanded = true;
        if let Some(containing_pane_id) = self.containing_pane_id {
            if let Some(pane_width) = ctx
                .element_position_by_id(containing_pane_id.position_id())
                .map(|rect| rect.width())
            {
                if let Ok(mut state) = self.ui_state_handles.sidebar_resizable_state.lock() {
                    state.set_size(pane_width * FILE_SIDEBAR_PANE_WIDTH_PERCENTAGE);
                }
            }
        }
    }

    /// Handles file sidebar state transitions when the maximize state changes.
    /// On maximize: saves the current sidebar state and opens the sidebar.
    /// On minimize: restores the sidebar to its pre-maximize state.
    pub fn handle_maximization_toggle(&mut self, is_maximized: bool, ctx: &mut ViewContext<Self>) {
        if is_maximized && self.file_sidebar_expanded_before_maximize.is_none() {
            // Transitioning to maximized: save current sidebar state and open it
            self.file_sidebar_expanded_before_maximize = Some(self.file_sidebar_expanded);
            if !self.file_sidebar_expanded {
                self.open_file_sidebar(ctx);
                self.update_file_nav_button_tooltip(ctx);
                ctx.notify();
            }
        } else if !is_maximized {
            if let Some(was_expanded) = self.file_sidebar_expanded_before_maximize.take() {
                // Transitioning to minimized: restore saved sidebar state
                if self.file_sidebar_expanded != was_expanded {
                    self.file_sidebar_expanded = was_expanded;
                    self.update_file_nav_button_tooltip(ctx);
                    ctx.notify();
                }
            }
        }
    }

    fn fetch_branches_and_setup_dropdown(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(repo_path) = self.repo_path().cloned() else {
            return;
        };
        let fetched_repo_path = repo_path.clone();
        ctx.spawn(
            async move {
                get_all_branches(&repo_path, None, false /* include_remotes */)
                    .await
            },
            move |me, branches_result, ctx| {
                // If the active repo changed while branches were being fetched,
                // discard the stale result.
                if me.repo_path() != Some(&fetched_repo_path) {
                    return;
                }
                match branches_result {
                    Ok(branches) => {
                        if let Some(repo) = me.active_repo.as_mut() {
                            let branch_count = branches.len();
                            let repo_path = &repo.repo_path;
                            safe_info!(
                                safe: ("Code Review: Set available_branches with {} branches", branch_count),
                                full: (
                                    "Code Review: Set available_branches for repo {:?} with {} branches",
                                    repo_path,
                                    branch_count
                                )
                            );
                            repo.available_branches = branches;
                        }
                        me.update_diff_selector_selection(ctx);
                    }
                    Err(err) => {
                        log::warn!("Failed to fetch branches: {err}");
                        // Fallback to default dropdown with just uncommitted changes and main branch
                        if let Some(repo) = me.active_repo.as_mut() {
                            let repo_path = &repo.repo_path;
                            safe_info!(
                                safe: ("Code Review: Set available_branches to empty (fallback after error)"),
                                full: (
                                    "Code Review: Set available_branches to empty for repo {:?} (fallback after error)",
                                    repo_path
                                )
                            );
                            repo.available_branches = vec![];
                        }
                        me.update_diff_selector_selection(ctx);
                    }
                }
            },
        );
    }

    pub(crate) fn build_diff_targets(&self, ctx: &ViewContext<Self>) -> Vec<DiffTarget> {
        let Some(repo) = self.active_repo.as_ref() else {
            return Vec::new();
        };

        let (current_mode, current_branch_name) = self.diff_state_model.read(ctx, |model, ctx| {
            (model.diff_mode(ctx), model.get_current_branch_name(ctx))
        });

        let mut targets = Vec::new();

        // 1. Always add "Uncommitted changes" first.
        targets.push(DiffTarget::new(
            "Uncommitted changes",
            DiffMode::Head,
            matches!(current_mode, DiffMode::Head),
        ));

        // 2. If the current mode targets a branch not in the local branch
        // list, add it after "Uncommitted changes".
        if let DiffMode::OtherBranch(ref branch_name) = current_mode {
            let already_present = repo
                .available_branches
                .iter()
                .any(|entry| entry.name == *branch_name);
            if !already_present {
                targets.push(DiffTarget::new(
                    branch_name.clone(),
                    DiffMode::OtherBranch(branch_name.clone()),
                    true,
                ));
            }
        }

        // 3. Main branch, if known.
        let main_branch = repo.available_branches.iter().find(|entry| entry.is_main);
        if let Some(main_entry) = main_branch {
            targets.push(DiffTarget::new(
                main_entry.name.clone(),
                DiffMode::MainBranch,
                matches!(current_mode, DiffMode::MainBranch),
            ));
        }

        // 4. Other branches, filtered to exclude main and the currently
        // checked-out branch (the latter is functionally the same as
        // "Uncommitted changes").
        for entry in repo.available_branches.iter() {
            if entry.is_main {
                continue;
            }
            if let Some(current_name) = &current_branch_name {
                if entry.name == *current_name {
                    continue;
                }
            }
            let is_selected = match &current_mode {
                DiffMode::OtherBranch(name) => name == &entry.name,
                DiffMode::Head | DiffMode::MainBranch => false,
            };
            targets.push(DiffTarget::new(
                entry.name.clone(),
                DiffMode::OtherBranch(entry.name.clone()),
                is_selected,
            ));
        }

        targets
    }

    fn update_diff_selector_selection(&mut self, ctx: &mut ViewContext<Self>) {
        let targets = self.build_diff_targets(ctx);
        let selector = self.diff_selector.clone();
        selector.update(ctx, |selector, ctx| {
            selector.set_targets(targets, ctx);
        });
    }

    fn handle_diff_selector_event(
        &mut self,
        event: &DiffSelectorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            DiffSelectorEvent::SelectMode(mode) => {
                // Dispatching a typed action from this view's own event callback
                // would bubble past `CodeReviewView` instead of re-entering
                // `handle_action`, so call the shared helper directly.
                self.apply_diff_mode(mode.clone(), ctx);
            }
        }
    }

    /// Shared body for `CodeReviewAction::SetDiffMode` and
    /// `DiffSelectorEvent::SelectMode`: sends a telemetry event for the
    /// mode change and updates the diff state model.
    fn apply_diff_mode(&mut self, mode: DiffMode, ctx: &mut ViewContext<Self>) {
        if self
            .diff_state_model
            .read(ctx, |model, ctx| model.diff_mode(ctx))
            == mode
        {
            return;
        }

        send_telemetry_from_ctx!(
            CodeReviewTelemetryEvent::BaseChanged { mode: mode.clone() },
            ctx
        );

        self.diff_state_model.update(ctx, |model, ctx| {
            model.set_diff_mode(mode, false, ctx);
        });
        self.update_diff_selector_selection(ctx);
        self.invalidate_all(None, ctx);
    }

    fn handle_find_event(
        &mut self,
        _find_bar: ViewHandle<Find<CodeReviewFindModel>>,
        event: &FindViewEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            FindViewEvent::CloseFindBar => {
                self.close_find_bar(ctx);
                ctx.focus_self();
            }
            FindViewEvent::Update { query } => {
                self.find_model.update(ctx, |model, model_ctx| {
                    model.update_query(query.clone(), self.editor_handles(), model_ctx);
                });
            }
            #[cfg_attr(target_family = "wasm", allow(unused_variables))]
            FindViewEvent::NextMatch { direction } => {
                #[cfg(not(target_family = "wasm"))]
                self.find_model.update(ctx, |model, model_ctx| {
                    model.focus_next_find_match(*direction, self.editor_handles(), model_ctx);
                });
            }
            FindViewEvent::ToggleCaseSensitivity { is_case_sensitive } => {
                self.find_model.update(ctx, |model, model_ctx| {
                    model.set_case_sensitive(*is_case_sensitive, self.editor_handles(), model_ctx);
                });
            }
            FindViewEvent::ToggleRegexSearch { is_regex_enabled } => {
                self.find_model.update(ctx, |model, model_ctx| {
                    model.set_regex(*is_regex_enabled, self.editor_handles(), model_ctx);
                });
            }
            FindViewEvent::ToggleFindInBlock { .. } => {
                log::warn!("Toggle find in block is not supported for code review");
            }
        }
    }

    fn handle_find_model_event(&mut self, event: &FindEvent, ctx: &mut ViewContext<Self>) {
        match event {
            FindEvent::RanFind => {
                self.update_search_decorations(ctx);
            }
            FindEvent::UpdatedFocusedMatch => {
                self.update_search_decorations(ctx);
                self.scroll_to_selected_match(ctx);
            }
        }
    }

    fn handle_comment_list_event(&mut self, event: &CommentListEvent, ctx: &mut ViewContext<Self>) {
        match event {
            CommentListEvent::Submitted => {
                self.handle_submit_review_with_comments(ctx);
                ctx.notify();
            }
            CommentListEvent::Cancelled => {
                self.clear_review_comments(ctx);
                ctx.notify();
            }
            CommentListEvent::DeleteComment { comment_id } => {
                self.delete_comment_by_id(*comment_id, ctx);
            }
            CommentListEvent::EditComment(comment_id) => {
                self.handle_edit_comment(comment_id, ctx);
                ctx.notify();
            }
            CommentListEvent::JumpToCommentLocation(comment_id) => {
                self.handle_jump_to_comment_location(comment_id, ctx);
                ctx.notify();
            }
        }
    }

    #[cfg(not(target_family = "wasm"))]
    fn update_search_decorations(&mut self, ctx: &mut ViewContext<Self>) {
        let CodeReviewViewState::Loaded(state) = self.state() else {
            return;
        };

        let mut matches_by_editor = self.find_model.as_ref(ctx).matches_by_editor();
        let selected_match = self.find_model.as_ref(ctx).selected_match_info();

        for file_state in state.file_states.values() {
            let Some(editor_state) = &file_state.editor_state else {
                continue;
            };

            let editor_id = editor_state.editor.id();
            let ranges = matches_by_editor.remove(&editor_id).unwrap_or_default();
            let selected_range_index = selected_match
                .as_ref()
                .filter(|info| info.editor_id == editor_id)
                .map(|info| info.index_within_editor);

            editor_state.editor.update(ctx, |local_editor, ctx| {
                local_editor.editor().update(ctx, |editor, ctx| {
                    editor.set_find_highlights(ranges, selected_range_index, ctx);
                });
            });
        }
    }

    #[cfg(target_family = "wasm")]
    fn update_search_decorations(&mut self, _ctx: &mut ViewContext<Self>) {
        unreachable!("Code review is not available on wasm")
    }

    fn open_review_comment_composer(
        &mut self,
        existing_comment: Option<AttachedReviewComment>,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.comment_composer.is_some() {
            return;
        }

        let comment_model = ctx.add_model(EditorCommentsModel::new);
        let comment_model_handle = comment_model.clone();

        let composer = ctx.add_typed_action_view(move |ctx| {
            CommentEditor::new(ctx, comment_model_handle.clone())
        });

        if let Some(comment) = &existing_comment {
            composer.update(ctx, |editor, ctx| {
                editor.reopen_saved_comment(
                    &comment.id,
                    None,
                    &comment.content,
                    &comment.origin,
                    ctx,
                );
            });
        }

        ctx.subscribe_to_view(&composer, |me, _, event, ctx| match event {
            CommentEditorEvent::CommentSaved {
                id, comment_text, ..
            } => {
                if let Some(id) = id {
                    if let Some(comment) = me.get_comment_by_id(*id, ctx) {
                        let mut updated = comment;
                        updated.content = comment_text.clone();
                        updated.last_update_time = chrono::Local::now();
                        me.update_review_comment(updated, ctx);
                    }
                } else {
                    let base = me.get_diff_base(ctx).ok();
                    let head = me.get_current_head(ctx);
                    let new_comment = AttachedReviewComment {
                        id: CommentId::new(),
                        content: comment_text.clone(),
                        target: AttachedReviewCommentTarget::General,
                        last_update_time: chrono::Local::now(),
                        base,
                        head,
                        outdated: false,
                        origin: CommentOrigin::Native,
                    };

                    me.update_review_comment(new_comment, ctx);
                }
            }
            CommentEditorEvent::DeleteComment { id } => {
                me.delete_comment_by_id(*id, ctx);
                me.comment_composer = None;
                me.update_header_dropdown_active_state(ctx);
                ctx.notify();
            }
            CommentEditorEvent::CloseEditor => {
                me.comment_composer = None;
                me.update_header_dropdown_active_state(ctx);
                ctx.notify();
            }
            _ => {}
        });

        self.comment_composer = Some(composer.clone());

        // Focus the comment editor when the composer opens
        ctx.focus(&composer);

        self.update_header_dropdown_active_state(ctx);
        ctx.notify();
    }

    fn handle_edit_comment(&mut self, comment_id: &CommentId, ctx: &mut ViewContext<Self>) {
        let Some(comment) = self.get_comment_by_id(*comment_id, ctx) else {
            log::error!("Couldn't find code review comment by ID");
            return;
        };

        let CodeReviewViewState::Loaded(state) = self.state() else {
            return;
        };
        match &comment.target {
            AttachedReviewCommentTarget::Line {
                absolute_file_path,
                line,
                ..
            } => {
                let Some((editor_index, file_state)) = Self::editor_for_comment(&comment, state)
                else {
                    log::warn!("Couldn't find editor for file: {absolute_file_path:?}");
                    return;
                };

                let Some(editor_state) = &file_state.editor_state.as_ref() else {
                    log::error!(
                        "CodeReviewView could not fetch editor for file {:?}",
                        file_state.file_diff.file_path
                    );
                    return;
                };

                editor_state.editor().update(ctx, |local_editor, ctx| {
                    local_editor.editor().update(ctx, |editor, ctx| {
                        editor.open_existing_comment(
                            &comment.id,
                            line,
                            &comment.content,
                            comment.origin(),
                            ctx,
                        );
                    });
                });

                self.scroll_to_line(editor_index, line, COMMENT_EDITOR_SCROLL_BUFFER, ctx);
            }
            AttachedReviewCommentTarget::General => {
                self.open_review_comment_composer(Some(comment), ctx);
            }
            AttachedReviewCommentTarget::File { .. } => {
                log::error!(
                    "Attempted to edit a file-level comment; file-level comments are not editable"
                );
            }
        }
    }

    pub fn handle_jump_to_comment_location(
        &mut self,
        comment_id: &CommentId,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(comment) = self.get_comment_by_id(*comment_id, ctx) else {
            if !matches!(self.state(), CodeReviewViewState::Loaded(_)) {
                self.pending_jump_to_comment = Some(*comment_id);
            } else {
                log::warn!("CodeReviewView couldn't find code review comment by ID");
            }
            return;
        };

        self.comment_list_view.update(ctx, |comment_list, ctx| {
            comment_list.scroll_to_comment(*comment_id, ctx);
        });

        let CodeReviewViewState::Loaded(state) = &self.state() else {
            self.pending_jump_to_comment = Some(*comment_id);
            return;
        };

        match &comment.target {
            AttachedReviewCommentTarget::Line {
                absolute_file_path, ..
            }
            | AttachedReviewCommentTarget::File { absolute_file_path } => {
                let Some((editor_index, _file_state)) = Self::editor_for_comment(&comment, state)
                else {
                    log::warn!("Couldn't find editor for file: {absolute_file_path:?}");
                    return;
                };

                if let AttachedReviewCommentTarget::Line { line, .. } = &comment.target {
                    self.scroll_to_line(editor_index, line, 0.0, ctx);
                } else {
                    self.viewported_list_state
                        .scroll_to_with_offset(editor_index, Pixels::new(0.0));
                }
            }
            AttachedReviewCommentTarget::General => {
                // Review-level comments only need to be scrolled into view in the comment list.
            }
        }
    }

    fn editor_for_comment<'a>(
        comment: &AttachedReviewComment,
        state: &'a LoadedState,
    ) -> Option<(usize, &'a FileState)> {
        let file_path = match &comment.target {
            AttachedReviewCommentTarget::Line {
                absolute_file_path, ..
            }
            | AttachedReviewCommentTarget::File { absolute_file_path } => absolute_file_path,
            AttachedReviewCommentTarget::General => return None,
        };

        state
            .file_states
            .values()
            .enumerate()
            .find(|(_, file_state)| {
                let editor_filepath = &file_state.file_diff.file_path;
                // Editor file paths are relative, while comment filepaths are absolute.
                file_path.ends_with(editor_filepath)
            })
    }

    fn scroll_to_line(
        &mut self,
        editor_index: usize,
        line: &EditorLineLocation,
        buffer: f32,
        ctx: &mut ViewContext<Self>,
    ) {
        let CodeReviewViewState::Loaded(state) = self.state() else {
            return;
        };

        let Some(editor_state) = state
            .file_states
            .get_index(editor_index)
            .map(|(_, fs)| fs)
            .and_then(|fs| fs.editor_state.as_ref())
        else {
            log::warn!("No editor state found for index {editor_index}");
            return;
        };

        let (start_offset, end_offset) = editor_state
            .editor
            .as_ref(ctx)
            .editor()
            .read(ctx, |code_editor_view, ctx| {
                code_editor_view.line_location_to_offsets(line, ctx)
            });

        self.scroll_to_position(editor_index, start_offset, end_offset, buffer, ctx);
    }

    fn scroll_to_selected_match(&mut self, ctx: &mut ViewContext<Self>) {
        let CodeReviewViewState::Loaded(state) = self.state() else {
            return;
        };

        let Some(selected_match_info) = self.find_model.as_ref(ctx).selected_match_info() else {
            return;
        };

        let Some(editor_index) = state.file_states.values().position(|file_state| {
            file_state
                .editor_state
                .as_ref()
                .map(|es| es.editor.id() == selected_match_info.editor_id)
                .unwrap_or(false)
        }) else {
            return;
        };

        // Buffer is 3 times the editor line height
        let buffer = Appearance::as_ref(ctx).monospace_font_size()
            * CODE_REVIEW_EDITOR_LINE_HEIGHT_RATIO
            * 3.;

        self.scroll_to_position(
            editor_index,
            selected_match_info.start_offset,
            selected_match_info.end_offset,
            buffer,
            ctx,
        )
    }

    fn scroll_to_position(
        &mut self,
        editor_index: usize,
        start_offset: CharOffset,
        end_offset: CharOffset,
        buffer: f32,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some((start_top_y, end_bottom_y)) =
            self.get_match_character_bounds(editor_index, start_offset, end_offset, ctx)
        {
            // Bounds are available - apply scrolling immediately
            self.vertically_scroll_to_match(editor_index, start_top_y, end_bottom_y, buffer);
            self.horizontally_scroll_to_match(editor_index, start_offset, end_offset, ctx);
        } else {
            // Character bounds aren't available, editor hasn't been laid out yet
            // Scroll conservatively to trigger layout without overshooting
            if editor_index > self.viewported_list_state.get_scroll_index() {
                // Moving DOWN - scroll until the file header is a bit above the bottom of the viewport
                self.viewported_list_state.scroll_to_with_offset(
                    editor_index,
                    Pixels::new(FILE_HEADER_HEIGHT)
                        - self.viewported_list_state.get_viewport_height()
                        + Pixels::new(5.0),
                );
            } else {
                // Moving UP - scroll until the editor beneath target is a bit below the top of the viewport
                self.viewported_list_state.scroll_to_with_offset(
                    editor_index + 1,
                    -Pixels::new(EDITOR_GAP) - Pixels::new(5.0),
                );
            }

            // Scroll to the precise position once the editor is laid out
            self.pending_precise_scroll = Some(PendingPreciseScroll {
                editor_index,
                start_offset,
                end_offset,
                buffer,
            });

            #[cfg(not(target_family = "wasm"))]
            {
                let CodeReviewViewState::Loaded(state) = self.state() else {
                    return;
                };

                ctx.subscribe_to_view(
                    &state.file_states[editor_index]
                        .editor_state
                        .as_ref()
                        .unwrap()
                        .editor,
                    move |view, editor_view, event, ctx| {
                        if let LocalCodeEditorEvent::ViewportUpdated = event {
                            let Some(pending) = view.pending_precise_scroll.take() else {
                                return;
                            };

                            let CodeReviewViewState::Loaded(state) = view.state() else {
                                // Put it back if we're not in the right state yet.
                                view.pending_precise_scroll = Some(pending);
                                return;
                            };

                            let firing_editor_index =
                                state.file_states.values().position(|file_state| {
                                    file_state
                                        .editor_state
                                        .as_ref()
                                        .map(|es| es.editor.id() == editor_view.id())
                                        .unwrap_or(false)
                                });

                            // Only apply if the firing editor matches the pending target to avoid race conditions
                            if firing_editor_index == Some(pending.editor_index) {
                                // Character bounds are available now after layout
                                if let Some((start_top_y, end_bottom_y)) = view
                                    .get_match_character_bounds(
                                        pending.editor_index,
                                        pending.start_offset,
                                        pending.end_offset,
                                        ctx,
                                    )
                                {
                                    view.vertically_scroll_to_match(
                                        pending.editor_index,
                                        start_top_y,
                                        end_bottom_y,
                                        pending.buffer,
                                    );
                                    view.horizontally_scroll_to_match(
                                        pending.editor_index,
                                        pending.start_offset,
                                        pending.end_offset,
                                        ctx,
                                    );
                                }
                            } else {
                                // Wrong editor fired - put pending back
                                view.pending_precise_scroll = Some(pending);
                            }
                        }
                    },
                );
            }
        }
    }

    #[cfg(target_family = "wasm")]
    fn get_match_character_bounds(
        &self,
        _editor_index: usize,
        _start_offset: CharOffset,
        _end_offset: CharOffset,
        _ctx: &ViewContext<Self>,
    ) -> Option<(Pixels, Pixels)> {
        unreachable!("get_match_character_bounds should not run on wasm");
    }

    #[cfg(not(target_family = "wasm"))]
    fn get_match_character_bounds(
        &self,
        editor_index: usize,
        start_offset: CharOffset,
        end_offset: CharOffset,
        ctx: &ViewContext<Self>,
    ) -> Option<(Pixels, Pixels)> {
        let CodeReviewViewState::Loaded(state) = self.state() else {
            return None;
        };

        let render_state = state
            .file_states
            .get_index(editor_index)
            .map(|(_, fs)| fs)
            .and_then(|file_state| file_state.editor_state.as_ref())
            .map(|editor_state| {
                editor_state
                    .editor
                    .as_ref(ctx)
                    .editor()
                    .as_ref(ctx)
                    .model
                    .as_ref(ctx)
                    .render_state()
                    .as_ref(ctx)
            })?;

        if let (Some((start_top_y, _)), Some((_, end_bottom_y))) = (
            render_state.character_vertical_bounds(start_offset),
            render_state.character_vertical_bounds(end_offset.saturating_sub(&CharOffset::from(1))),
        ) {
            Some((start_top_y, end_bottom_y))
        } else {
            None
        }
    }

    /// Applies vertical scrolling to bring a match into view vertically
    fn vertically_scroll_to_match(
        &mut self,
        editor_index: usize,
        start_top_y: Pixels,
        end_bottom_y: Pixels,
        buffer: f32,
    ) {
        // Don't scroll if match is already visible (check without buffer)
        if self.viewported_list_state.is_vertical_range_visible(
            editor_index,
            start_top_y,
            Pixels::new(FILE_HEADER_HEIGHT) + end_bottom_y,
        ) {
            return;
        }

        let current_item = self.viewported_list_state.get_scroll_index();
        if (current_item == editor_index
            && start_top_y < self.viewported_list_state.get_scroll_offset())
            || editor_index < current_item
        {
            // Scroll to top: same file with match above, or moving up to higher file
            self.viewported_list_state
                .scroll_to_with_offset(editor_index, start_top_y - Pixels::new(buffer));
        } else {
            // Scroll to bottom: same file with match below, or moving down to lower file
            self.viewported_list_state.scroll_to_with_offset(
                editor_index,
                Pixels::new(FILE_HEADER_HEIGHT) + end_bottom_y + Pixels::new(buffer)
                    - self.viewported_list_state.get_viewport_height(),
            );
        }
    }

    #[cfg(target_family = "wasm")]
    fn horizontally_scroll_to_match(
        &self,
        _editor_index: usize,
        _start_offset: CharOffset,
        _end_offset: CharOffset,
        _ctx: &mut ViewContext<Self>,
    ) {
        unreachable!("horizontally_scroll_to_match should not run on wasm");
    }

    #[cfg(not(target_family = "wasm"))]
    fn horizontally_scroll_to_match(
        &self,
        editor_index: usize,
        start_offset: CharOffset,
        end_offset: CharOffset,
        ctx: &mut ViewContext<Self>,
    ) {
        let CodeReviewViewState::Loaded(state) = self.state() else {
            return;
        };

        let Some(editor_state) = state
            .file_states
            .get_index(editor_index)
            .map(|(_, fs)| fs)
            .and_then(|fs| fs.editor_state.as_ref())
        else {
            return;
        };

        editor_state.editor.update(ctx, |local_editor, ctx| {
            local_editor.editor().update(ctx, |editor, ctx| {
                editor
                    .model
                    .as_ref(ctx)
                    .render_state()
                    .clone()
                    .update(ctx, |render, _| {
                        render.request_autoscroll_to(AutoScrollMode::ScrollOffsetsIntoViewport(
                            start_offset..end_offset,
                        ));
                    });
            });
        });
    }

    fn show_find_bar(&mut self, ctx: &mut ViewContext<Self>) {
        let selected_text = self.editor_handles().find_map(|editor_handle| {
            editor_handle
                .as_ref(ctx)
                .editor()
                .as_ref(ctx)
                .selected_text(ctx)
                .filter(|text| !text.contains('\n'))
        });

        self.find_model.update(ctx, |model, _| {
            model.set_is_find_bar_open(true);
        });

        if let Some(text) = selected_text {
            self.find_bar.update(ctx, |find_bar, ctx| {
                find_bar.set_query_text(&text, ctx);
            });
            self.find_model.update(ctx, |model, model_ctx| {
                model.update_query(Some(text), self.editor_handles(), model_ctx);
            });
        } else {
            self.find_model.update(ctx, |model, model_ctx| {
                model.run_search(self.editor_handles(), model_ctx);
            });
        }

        send_telemetry_from_ctx!(
            CodeReviewTelemetryEvent::FindBarToggled { is_open: true },
            ctx
        );
        ctx.focus(&self.find_bar);
        self.update_search_decorations(ctx);
        ctx.notify();
    }

    fn close_find_bar(&mut self, ctx: &mut ViewContext<Self>) {
        self.find_bar.update(ctx, |find_bar, ctx| {
            find_bar.set_query_text("", ctx);
        });

        let editor_handles = self.editor_handles().collect::<Vec<_>>();
        self.find_model.update(ctx, |model, model_ctx| {
            model.set_is_find_bar_open(false);
            model.update_query(None, editor_handles.into_iter(), model_ctx);
            model.clear_results();
        });

        send_telemetry_from_ctx!(
            CodeReviewTelemetryEvent::FindBarToggled { is_open: false },
            ctx
        );

        // Clear finder match decorations
        #[cfg(not(target_family = "wasm"))]
        if let CodeReviewViewState::Loaded(state) = self.state() {
            for file_state in state.file_states.values() {
                if let Some(editor_state) = &file_state.editor_state {
                    editor_state.editor.update(ctx, |local_editor, ctx| {
                        local_editor.editor().update(ctx, |editor, ctx| {
                            editor.set_find_highlights(Vec::new(), None, ctx)
                        });
                    });
                }
            }
        }

        ctx.notify();
    }

    fn handle_diff_state_model_event(
        &mut self,
        _diff_state_model: ModelHandle<DiffStateModel>,
        event: &DiffStateModelEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            DiffStateModelEvent::CurrentBranchChanged => {
                self.fetch_branches_and_setup_dropdown(ctx);
                self.update_diff_selector_selection(ctx);
            }
            DiffStateModelEvent::NewDiffsComputed(diffs) => {
                self.invalidate_all(diffs.as_ref().map(|d| d.as_ref()), ctx);
                if FeatureFlag::GitOperationsInCodeReview.is_enabled() {
                    self.update_git_operations_ui(ctx);
                }
            }
            DiffStateModelEvent::SingleFileUpdated { path, diff } => {
                self.update_from_single_file_diff_result(path.clone(), diff.clone(), ctx);
                if FeatureFlag::GitOperationsInCodeReview.is_enabled() {
                    self.update_git_operations_ui(ctx);
                }
            }
            DiffStateModelEvent::MetadataRefreshed(metadata) => {
                let mode = self.diff_state_model.as_ref(ctx).diff_mode(ctx);
                if let Some(CodeReviewViewState::Loaded(loaded_state)) = self.state_mut() {
                    let stats = match mode {
                        DiffMode::Head => Some(metadata.against_head.aggregate_stats),
                        DiffMode::MainBranch => metadata
                            .against_base_branch
                            .as_ref()
                            .map(|base| base.aggregate_stats),
                        DiffMode::OtherBranch(_) => None,
                    };
                    if let Some(stats) = stats {
                        loaded_state.total_additions = stats.total_additions;
                        loaded_state.total_deletions = stats.total_deletions;
                        loaded_state.files_changed = stats.files_changed;
                    }
                }
                if FeatureFlag::GitOperationsInCodeReview.is_enabled() {
                    self.update_git_operations_ui(ctx);
                }
                ctx.notify();
            }
            DiffStateModelEvent::ConnectionLost => {
                // Don't clear loaded state — keep stale diffs visible
                // so the user can still see what they were looking at.
                ctx.notify();
            }
        }
    }

    fn update_header_dropdown_active_state(&mut self, ctx: &mut ViewContext<Self>) {
        let is_active = self.header_menu_open || self.comment_composer.is_some();
        self.header_dropdown_button.update(ctx, |button, ctx| {
            button.set_active(is_active, ctx);
        });
    }

    /// Updates the code review view with the diff result for a single updated
    /// file from the queue-based invalidation path.
    fn update_from_single_file_diff_result(
        &mut self,
        file_path: PathBuf,
        updated_diff: Option<Arc<FileDiffAndContent>>,
        ctx: &mut ViewContext<Self>,
    ) {
        let mut diff_data = {
            let Some(repo) = self.active_repo.as_mut() else {
                return;
            };
            match repo.pop_loaded_state() {
                Some(data) => data,
                None => return,
            }
        };

        let existing_index = diff_data.file_states.get_index_of(&file_path);

        match (existing_index, updated_diff) {
            (Some(index), Some(diff)) => {
                let diff = diff.as_ref();
                let status_changed = file_status_changed_deleted_state(
                    &diff_data.file_states[index].file_diff.status,
                    &diff.file_diff.status,
                );

                if status_changed {
                    diff_data.file_states.shift_remove_index(index);
                    self.viewported_list_state.remove(index);
                    let new_states =
                        self.build_view_state_for_file_diffs(std::slice::from_ref(diff), ctx);
                    diff_data.file_states.extend(
                        new_states
                            .into_iter()
                            .map(|state| (state.file_diff.file_path.clone(), state)),
                    );
                } else {
                    let current = &mut diff_data.file_states[index];
                    let should_apply = current
                        .editor_state
                        .as_ref()
                        .map(|es| !es.has_unsaved_changes(ctx))
                        .unwrap_or(true);
                    if should_apply {
                        current.file_diff = diff.file_diff.clone();
                    }
                    self.viewported_list_state
                        .invalidate_height_for_index(index);
                }
            }
            (Some(index), None) => {
                diff_data.file_states.shift_remove_index(index);
                self.viewported_list_state.remove(index);
            }
            (None, Some(diff)) => {
                let new_states =
                    self.build_view_state_for_file_diffs(std::slice::from_ref(diff.as_ref()), ctx);
                diff_data.file_states.extend(
                    new_states
                        .into_iter()
                        .map(|state| (state.file_diff.file_path.clone(), state)),
                );
            }
            (None, None) => {}
        }

        if let Some(repo) = self.active_repo.as_mut() {
            repo.state = CodeReviewViewState::Loaded(diff_data);
        }

        self.update_editor_comment_markers(ctx);
        GlobalBufferModel::handle(ctx).update(ctx, |model, ctx| {
            model.remove_deallocated_buffers(ctx);
        });
        ctx.notify();
    }

    /// Updates state for the view when new git diffs come in.
    fn invalidate_all(
        &mut self,
        diff_data: Option<&GitDiffWithBaseContent>,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.active_repo.is_none() {
            return;
        };

        match self.diff_state(ctx) {
            DiffState::Loading => {
                if let Some(repo) = self.active_repo.as_mut() {
                    log::info!(
                        "Code Review Panel: Setting state to loading after receiving 'loading' message."
                    );
                    repo.state = CodeReviewViewState::None;
                }
                ctx.notify();
                return;
            }
            DiffState::NotInRepository => {
                if let Some(repo) = self.active_repo.as_mut() {
                    if repo.repo_path.as_os_str().is_empty() {
                        repo.state = CodeReviewViewState::NoRepoFound;
                    } else {
                        log::info!(
                            "Code Review Panel: Setting state to loading after receiving 'not in repository' message."
                        );
                        repo.state = CodeReviewViewState::None;
                    }
                }
                ctx.notify();
                return;
            }
            DiffState::Error(err) => {
                if let Some(repo) = self.active_repo.as_mut() {
                    repo.state = CodeReviewViewState::Error(err);
                }
                ctx.notify();
                return;
            }
            DiffState::Disconnected => {
                // Disconnected state is handled via the ConnectionLost event
                // path, which preserves stale diffs. If invalidate_all is
                // called while disconnected (e.g. from a stale push), ignore.
                return;
            }
            DiffState::Loaded => (),
        };

        let Some(diff_data) = diff_data else {
            log::warn!("Trying to reload diff but there is no git diff base");
            return;
        };

        // Deallocate global buffers that are going to be invalidated.
        if let Some(repo) = self.active_repo.as_mut() {
            repo.state = CodeReviewViewState::None;
            GlobalBufferModel::handle(ctx).update(ctx, |model, ctx| {
                model.remove_deallocated_buffers(ctx);
            });
        }

        // Create a new list state for this update
        self.viewported_list_state = Self::create_list_state(ctx);

        let file_states_vec = self.build_view_state_for_file_diffs(&diff_data.files, ctx);

        if let Some(repo) = self.active_repo.as_mut() {
            repo.state = CodeReviewViewState::Loaded(LoadedState {
                file_states: file_states_vec
                    .into_iter()
                    .map(|fs| (fs.file_diff.file_path.clone(), fs))
                    .collect(),
                total_additions: diff_data.total_additions,
                total_deletions: diff_data.total_deletions,
                files_changed: diff_data.files_changed,
            });
        }

        if self.all_editors_loaded() {
            let diff_mode = self.diff_state_model.as_ref(ctx).diff_mode(ctx);
            self.reposition_comments_in_file(&diff_mode, ctx);
        }

        self.update_editor_comment_markers(ctx);

        if let Some(comment_id) = self.pending_jump_to_comment.take() {
            self.handle_jump_to_comment_location(&comment_id, ctx);
        }

        ctx.notify();
    }

    /// Builds view state for the given file diffs, returning the list of newly created file states.
    fn build_view_state_for_file_diffs(
        &self,
        files: &[FileDiffAndContent],
        ctx: &mut ViewContext<Self>,
    ) -> Vec<FileState> {
        let git_operation_blocked = self
            .diff_state_model
            .as_ref(ctx)
            .is_git_operation_blocked(ctx);
        let discard_tooltip_text = if git_operation_blocked {
            get_discard_button_disabled_tooltip(git_operation_blocked)
        } else {
            "Discard changes".to_string()
        };

        let mut file_states = vec![];
        for file in files {
            let editor_state = {
                #[cfg(not(target_family = "wasm"))]
                {
                    self.create_code_review_model_with_global_buffer(file, ctx)
                }
                #[cfg(target_family = "wasm")]
                {
                    self.create_code_review_model(file, ctx)
                }
            };
            let is_expanded = self.should_auto_expand_file(&file.file_diff);

            let file_path = file.file_diff.file_path.clone();
            let file_line = file_line_for_open(&file.file_diff);

            let chevron_path = file_path.clone();
            let initial_icon = if is_expanded {
                Icon::ChevronDown
            } else {
                Icon::ChevronRight
            };
            let chevron_button = ctx.add_typed_action_view(move |_ctx| {
                ActionButton::new("", NakedTheme)
                    .with_icon(initial_icon)
                    .with_size(ButtonSize::InlineActionHeader)
                    .on_click(move |ctx| {
                        ctx.dispatch_typed_action(CodeReviewAction::ToggleFileExpanded(
                            chevron_path.clone(),
                        ))
                    })
            });

            let open_tab_path = file_path.clone();
            let open_in_tab_button = ctx.add_typed_action_view(move |_ctx| {
                ActionButton::new("", NakedTheme)
                    .with_icon(Icon::LinkExternal)
                    .with_size(ButtonSize::InlineActionHeader)
                    .with_tooltip("Open file")
                    .on_click(move |ctx| {
                        ctx.dispatch_typed_action(CodeReviewAction::OpenInNewTab {
                            path: open_tab_path.clone(),
                            line_and_column: file_line.map(|line| LineAndColumnArg {
                                line_num: line,
                                column_num: None,
                            }),
                        })
                    })
            });

            let discard_path = file.file_diff.file_path.clone();
            let discard_tooltip = discard_tooltip_text.clone();
            let discard_button = ctx.add_typed_action_view(move |ctx| {
                let mut button = ActionButton::new("", NakedTheme)
                    .with_icon(Icon::ReverseLeft)
                    .with_size(ButtonSize::InlineActionHeader)
                    .with_tooltip(discard_tooltip);

                if git_operation_blocked {
                    button.set_disabled(true, ctx);
                } else {
                    button = button.on_click(move |ctx| {
                        ctx.dispatch_typed_action(CodeReviewAction::ShowDiscardConfirmDialog(Some(
                            discard_path.clone(),
                        )))
                    });
                }
                button
            });

            let context_path = file.file_diff.file_path.clone();
            let add_context_button = ctx.add_typed_action_view(move |_ctx| {
                ActionButton::new("", NakedTheme)
                    .with_icon(Icon::Paperclip)
                    .with_size(ButtonSize::InlineActionHeader)
                    .with_tooltip("Add file diff as context")
                    .on_click(move |ctx| {
                        ctx.dispatch_typed_action(CodeReviewAction::AddDiffSetAsContext(
                            DiffSetScope::File(context_path.clone()),
                        ))
                    })
            });

            let copy_path = file.file_diff.file_path.clone();
            let copy_path_button = ctx.add_typed_action_view(move |_ctx| {
                ActionButton::new("", NakedTheme)
                    .with_icon(Icon::Copy)
                    .with_size(ButtonSize::InlineActionHeader)
                    .with_tooltip("Copy file path")
                    .on_click(move |ctx| {
                        ctx.dispatch_typed_action(CodeReviewAction::CopyFilePath(copy_path.clone()))
                    })
            });

            file_states.push(FileState {
                file_diff: file.file_diff.clone(),
                editor_state,
                is_expanded,
                chevron_button,
                open_in_tab_button,
                discard_button,
                add_context_button,
                copy_path_button,
                sidebar_mouse_state: MouseStateHandle::default(),
                header_mouse_state: MouseStateHandle::default(),
            })
        }

        // Populate the viewported list with file diffs
        for _ in file_states.iter() {
            self.viewported_list_state.add_item();
        }
        file_states
    }

    fn render_diff_at_index(
        &self,
        index: usize,
        scroll_offset: ScrollOffset,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let diff_state = match self.state() {
            CodeReviewViewState::Loaded(LoadedState { file_states, .. }) => file_states,
            // This should never happen--we only render the list when in the loaded state.
            _ => return Empty::new().finish(),
        };
        let Some((_, file_state)) = diff_state.get_index(index) else {
            return Empty::new().finish();
        };

        self.render_file_diff(file_state, index, scroll_offset, appearance, app)
    }

    fn should_auto_expand_file(&self, file: &FileDiff) -> bool {
        self.active_repo
            .as_ref()
            .map(|repo| repo.should_auto_expand_file(file))
            .unwrap_or(false)
    }

    fn get_existing_diffset_comment(&self, app: &AppContext) -> Option<AttachedReviewComment> {
        self.active_comment_model
            .as_ref()
            .and_then(|model| model.read(app, |batch, _| batch.diffset_comment().cloned()))
    }

    /// Updates or adds a review comment.
    fn update_review_comment(
        &mut self,
        comment: AttachedReviewComment,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(model) = self.active_comment_model.clone() else {
            return;
        };

        let is_existing = model.read(ctx, |batch, _| {
            batch.get_review_comment_by_id(comment.id).is_some()
        });

        model.update(ctx, move |batch, ctx| {
            batch.upsert_comment(comment.clone(), ctx);
        });

        // Telemetry: record whether this was a new comment or an edit.
        if is_existing {
            send_telemetry_from_ctx!(CodeReviewTelemetryEvent::CommentEdited, ctx);
        } else {
            send_telemetry_from_ctx!(CodeReviewTelemetryEvent::CommentAdded, ctx);
        }
    }

    /// Clears all review comments.
    fn clear_review_comments(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(model) = self.active_comment_model.clone() {
            model.update(ctx, |batch, ctx| {
                batch.clear_all(ctx);
            });
        }
    }

    fn delete_comment_by_id(&mut self, id: CommentId, ctx: &mut ViewContext<Self>) {
        if let Some(model) = self.active_comment_model.clone() {
            let is_imported = model
                .read(ctx, |batch, _| {
                    batch
                        .get_review_comment_by_id(id)
                        .map(|c| c.origin.is_imported_from_github())
                })
                .unwrap_or(false);

            model.update(ctx, |batch, ctx| {
                batch.delete_comment(id, ctx);
            });

            send_telemetry_from_ctx!(
                CodeReviewTelemetryEvent::CommentDeleted { is_imported },
                ctx
            );
        }
    }

    pub fn editor_lens_for_location(
        &self,
        path: &Path,
        line: Range<EditorLineLocation>,
        ctx: &AppContext,
    ) -> Option<Box<dyn Element>> {
        let editor = self.editor_for_path(path, ctx)?;
        Some(
            editor
                .as_ref(ctx)
                .editor()
                .as_ref(ctx)
                .lens_for_line_range(line, ctx),
        )
    }

    /// Get the terminal view for the current repo. Returns None if no repo or no terminal.
    pub fn terminal_view(&self, app: &AppContext) -> Option<ViewHandle<TerminalView>> {
        self.terminal_view.as_ref().and_then(|tv| tv.upgrade(app))
    }

    fn diff_state(&self, app: &AppContext) -> DiffState {
        self.diff_state_model.read(app, |model, ctx| model.get(ctx))
    }

    /// Get the state of the current repo. Returns None if no repo.
    fn state(&self) -> &CodeReviewViewState {
        if let Some(repo) = self.active_repo.as_ref() {
            &repo.state
        } else {
            &CodeReviewViewState::NoRepoFound
        }
    }

    /// Get mutable state of the current repo. Returns None if no repo.
    fn state_mut(&mut self) -> Option<&mut CodeReviewViewState> {
        Some(&mut self.active_repo.as_mut()?.state)
    }

    #[cfg(not(target_family = "wasm"))]
    fn session_env(&self, app: &AppContext) -> Option<GitSessionState> {
        let terminal_view = self.terminal_view.as_ref()?.upgrade(app)?;
        terminal_view.read(app, |terminal, ctx| {
            let session = terminal
                .active_block_session_id()
                .and_then(|id| terminal.sessions_model().as_ref(ctx).get(id));
            let is_local = terminal.active_session_is_local(ctx);
            let is_remote = matches!(is_local, Some(false));
            let is_wsl = session.as_ref().map(|s| s.is_wsl()).unwrap_or(false);

            let enablement = if is_remote {
                CodingPanelEnablementState::RemoteSession {
                    has_remote_server: false,
                }
            } else if is_wsl {
                CodingPanelEnablementState::UnsupportedSession
            } else {
                CodingPanelEnablementState::Enabled
            };

            Some(GitSessionState { enablement })
        })
    }

    #[cfg(target_family = "wasm")]
    fn render_no_repo_for_env(
        &self,
        _app: &AppContext,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        Self::render_wsl_state(appearance, None)
    }

    #[cfg(not(target_family = "wasm"))]
    fn render_no_repo_for_env(
        &self,
        app: &AppContext,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        match self.session_env(app) {
            Some(state)
                if matches!(
                    state.enablement,
                    CodingPanelEnablementState::RemoteSession { .. }
                ) =>
            {
                self.render_remote_state_with_buttons(appearance)
            }
            Some(state)
                if matches!(
                    state.enablement,
                    CodingPanelEnablementState::UnsupportedSession
                ) =>
            {
                self.render_wsl_state_with_buttons(appearance)
            }
            None => self.render_not_repo_state_with_buttons(appearance),
            Some(_) => self.render_not_repo_state_with_buttons(appearance),
        }
    }

    /// Converts GitDiffData hunks to DiffDelta format for CodeEditorView.apply_diffs
    fn convert_hunks_to_diff_deltas(hunks: &[DiffHunk]) -> Vec<ai::diff_validation::DiffDelta> {
        let mut diff_deltas = Vec::new();

        for hunk in hunks {
            let mut current_replacement_start: Option<usize> = None;
            let mut current_insertion = String::new();
            let mut has_removals = false;
            let mut old_line = hunk.old_start_line;

            for line in &hunk.lines {
                match line.line_type {
                    DiffLineType::Add => {
                        if current_replacement_start.is_none() {
                            current_replacement_start = Some(old_line);
                        }

                        current_insertion.push_str(&line.text);
                        current_insertion.push('\n');
                    }
                    DiffLineType::Delete => {
                        if current_replacement_start.is_none() {
                            current_replacement_start = Some(old_line);
                        }
                        has_removals = true;
                        old_line += 1;
                    }
                    DiffLineType::Context => {
                        if let Some(start) = current_replacement_start.take() {
                            let end = if has_removals { old_line } else { start };

                            diff_deltas.push(ai::diff_validation::DiffDelta {
                                replacement_line_range: start..end,
                                insertion: current_insertion.clone(),
                            });
                            current_insertion.clear();
                            has_removals = false;
                        }
                        old_line += 1;
                    }
                    DiffLineType::HunkHeader => {
                        continue;
                    }
                }
            }

            if let Some(start) = current_replacement_start.take() {
                let end = if has_removals { old_line } else { start };
                diff_deltas.push(ai::diff_validation::DiffDelta {
                    replacement_line_range: start..end,
                    insertion: current_insertion,
                });
            }
        }

        diff_deltas
    }

    #[cfg(not(target_family = "wasm"))]
    fn create_code_review_model_with_global_buffer(
        &self,
        file: &FileDiffAndContent,
        ctx: &mut ViewContext<Self>,
    ) -> Option<CodeReviewEditorState> {
        let repo_path = self.repo_path()?;
        // Skip editor creation for binary files or files without content (e.g., pure renames)
        if file.file_diff.is_binary || file.content_at_head.is_none() {
            None
        } else if matches!(file.file_diff.status, GitFileStatus::Deleted) {
            // For deleted files, the file doesn't exist on disk anymore, so we can't use
            // GlobalBufferModel. Instead, use the non-global buffer approach which directly
            // populates the editor with content_at_head.
            self.create_code_review_model(file, ctx)
        } else {
            let self_handle = ctx.handle();
            let full_file_path = repo_path.join(&file.file_diff.file_path);

            let local_code_view = ctx.add_typed_action_view(|ctx| {
                let editor = LocalCodeEditorView::new_with_global_buffer(
                    LocalOrRemotePath::Local(full_file_path.clone()),
                    |buffer_state, ctx| {
                        ctx.add_typed_action_view(|ctx| {
                            let mut editor_view = CodeEditorView::new(
                                None,
                                Some(buffer_state.buffer),
                                CodeEditorRenderOptions::new(
                                    VerticalExpansionBehavior::InfiniteHeight,
                                )
                                .lazy_layout()
                                .line_height_override(CODE_REVIEW_EDITOR_LINE_HEIGHT_RATIO)
                                .with_show_comment_editor_provider(ShowCommentEditor {
                                    comment_list_save_position_id: self
                                        .code_review_list_position_id
                                        .clone(),
                                    window_id: ctx.window_id(),
                                })
                                .with_show_find_references_provider(ShowFindReferencesCard {
                                    editor_window_id: ctx.window_id(),
                                    parent_scrollable_position_id: Some(
                                        self.code_review_list_position_id.clone(),
                                    ),
                                }),
                                ctx,
                            )
                            .with_add_context_button() // Enable add context button for code review
                            .with_revert_diff_hunk_button() // Enable revert diff button for code review
                            .with_comment_button() // Enable comment button for code review
                            .with_collapsible_diffs(false) // Disable collapsible diffs
                            .disable_diff_indicator_expansion_on_hover()
                            .with_gutter_hover_target(GutterHoverTarget::Line) // Show gutter element when hovering the line.
                            .disable_find_and_replace(); // Disable find and replace since parts of the file are hidden from view

                            editor_view.set_show_nav_bar(false);

                            // Now we hand off hidden lines calculation to the editor model itself.
                            editor_view.hide_lines_outside_of_active_diff(4, ctx);
                            editor_view
                        })
                    },
                    false,
                    None,
                    ctx,
                )
                .with_selection_as_context(Box::new(move |_, app| {
                    self_handle.upgrade(app).and_then(|code_review_view| {
                        code_review_view.as_ref(app).terminal_view(app)
                    })
                }));

                editor
            });

            let inner_editor = local_code_view.as_ref(ctx).editor().clone();
            ctx.subscribe_to_view(&inner_editor, {
                let file_path = file.file_diff.file_path.clone();
                move |this, editor, event, ctx| {
                    this.handle_code_editor_event(file_path.clone(), editor, event, ctx);
                }
            });

            Self::apply_diff_to_code_editor(
                &local_code_view,
                file,
                true,
                &self.comment_line_numbers_for_file(&file.file_diff.file_path, ctx),
                ctx,
            );

            ctx.subscribe_to_view(&local_code_view, {
                let diff_file_path = file.file_diff.file_path.clone();
                move |me, editor, event, ctx| {
                    me.handle_local_code_editor_events(
                        editor,
                        event,
                        &full_file_path,
                        &diff_file_path,
                        ctx,
                    );
                }
            });

            Some(CodeReviewEditorState::new(local_code_view))
        }
    }

    /// Creates CodeReviewModel for each file, containing the CodeEditorView and file data
    fn create_code_review_model(
        &self,
        file: &FileDiffAndContent,
        ctx: &mut ViewContext<Self>,
    ) -> Option<CodeReviewEditorState> {
        let repo_path = self.repo_path()?;

        if file.file_diff.is_binary {
            None
        } else {
            let self_handle = ctx.handle();
            let code_editor_view = ctx.add_typed_action_view(|ctx| {
                let mut editor_view = CodeEditorView::new(
                    None,
                    None,
                    CodeEditorRenderOptions::new(VerticalExpansionBehavior::InfiniteHeight)
                        .lazy_layout()
                        .line_height_override(CODE_REVIEW_EDITOR_LINE_HEIGHT_RATIO)
                        .with_show_comment_editor_provider(ShowCommentEditor {
                            comment_list_save_position_id: self
                                .code_review_list_position_id
                                .clone(),
                            window_id: ctx.window_id(),
                        }),
                    ctx,
                )
                .with_add_context_button() // Enable add context button for code review
                .with_revert_diff_hunk_button() // Enable revert diff button for code review
                .with_comment_button() // Enable comment button for code review
                .with_collapsible_diffs(false) // Disable collapsible diffs
                .disable_diff_indicator_expansion_on_hover()
                .with_gutter_hover_target(GutterHoverTarget::Line) // Show gutter element when hovering the line.
                .disable_find_and_replace(); // Disable find and replace since parts of the file are hidden from view

                editor_view.set_show_nav_bar(false);
                editor_view
            });

            let full_file_path = repo_path.join(&file.file_diff.file_path);
            code_editor_view.update(ctx, |editor, ctx| {
                editor.set_language_with_path(&full_file_path, ctx);
            });

            ctx.subscribe_to_view(&code_editor_view, {
                let file_path = file.file_diff.file_path.clone();
                move |this, editor, event, ctx| {
                    this.handle_code_editor_event(file_path.clone(), editor, event, ctx);
                }
            });

            let local_code_view = ctx.add_typed_action_view(|ctx| {
                let mut local_code_view =
                    LocalCodeEditorView::new(code_editor_view, None, false, None, ctx);
                if FeatureFlag::HoaCodeReview.is_enabled() {
                    local_code_view =
                        local_code_view.with_selection_as_context(Box::new(move |_, app| {
                            self_handle.upgrade(app).and_then(|code_review_view| {
                                code_review_view.as_ref(app).terminal_view(app)
                            })
                        }));
                }
                // Deleted files have no file backing — no FileModel, no GlobalBufferModel.
                // file_id() will be None for these editors; no downstream code in code_review
                // relies on file_id for deleted entries (save/conflict flows early-return on None).
                // Content is populated via reset_with_state in apply_diff_to_code_editor.
                local_code_view
            });

            let comment_line_numbers =
                self.comment_line_numbers_for_file(&file.file_diff.file_path, ctx);

            Self::apply_diff_to_code_editor(
                &local_code_view,
                file,
                true,
                &comment_line_numbers,
                ctx,
            );

            ctx.subscribe_to_view(&local_code_view, {
                let diff_file_path = file.file_diff.file_path.clone();
                move |me, editor, event, ctx| {
                    me.handle_local_code_editor_events(
                        editor,
                        event,
                        &full_file_path,
                        &diff_file_path,
                        ctx,
                    );
                }
            });

            // For non-global buffer mode, content is loaded synchronously via apply_diff_to_code_editor,
            // so mark as loaded immediately.
            Some(CodeReviewEditorState::new_loaded(local_code_view))
        }
    }

    fn handle_local_code_editor_events(
        &mut self,
        editor: ViewHandle<LocalCodeEditorView>,
        event: &LocalCodeEditorEvent,
        full_file_path: &Path,
        diff_file_path: &Path,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            LocalCodeEditorEvent::FileSaved => {
                send_telemetry_from_ctx!(CodeReviewTelemetryEvent::FileSaved, ctx);

                ctx.emit(CodeReviewViewEvent::FileSaved {
                    path: full_file_path.to_path_buf(),
                });
            }
            LocalCodeEditorEvent::FailedToSave { error } => {
                ctx.emit(CodeReviewViewEvent::FileSaveError {
                    path: full_file_path.to_path_buf(),
                    error: error.clone(),
                });
            }
            LocalCodeEditorEvent::DelayedRenderingFlushed => {
                // Mark the editor as loaded so we can render it.
                // This is only relevant for global buffer mode.
                self.mark_editor_loaded_for_file(diff_file_path, ctx);
                ctx.notify();
            }
            LocalCodeEditorEvent::FailedToLoad { error } => {
                // Also mark as loaded on failure so we don't wait forever.
                self.mark_editor_loaded_for_file(diff_file_path, ctx);
                ctx.emit(CodeReviewViewEvent::FileLoadError {
                    path: full_file_path.to_path_buf(),
                    error: error.clone(),
                });
                ctx.notify();
            }
            LocalCodeEditorEvent::SelectionAddedAsContext {
                relative_file_path,
                line_range,
                selected_text,
            } => {
                self.insert_selection_as_context(
                    relative_file_path.clone(),
                    line_range.start.as_usize(),
                    line_range.end.as_usize(),
                    selected_text.clone(),
                    ctx,
                );
            }
            LocalCodeEditorEvent::DiscardUnsavedChanges { path: _path } => {
                #[cfg(feature = "local_fs")]
                GlobalBufferModel::handle(ctx).update(ctx, |global_buffer, ctx| {
                    global_buffer.discard_unsaved_changes(_path, ctx);
                });
            }
            LocalCodeEditorEvent::CommentSaved { comment } => {
                let Some(file_path) = editor.as_ref(ctx).file_path() else {
                    log::error!(
                        "Attempted to attach code review comment to a LocalCodeEditorView without a file path"
                    );
                    return;
                };
                let base = self.get_diff_base(ctx).ok();
                let head = self.get_current_head(ctx);
                let comment_with_file_context = AttachedReviewComment::from_editor_review_comment(
                    comment.clone(),
                    file_path.to_path_buf(),
                    base,
                    head,
                );
                self.update_review_comment(comment_with_file_context, ctx);
                ctx.notify();
            }
            LocalCodeEditorEvent::DeleteComment { id } => {
                self.delete_comment_by_id(*id, ctx);
            }
            LocalCodeEditorEvent::RequestOpenComment(comment_id) => {
                let Some(existing_comment) = self.get_comment_by_id(*comment_id, ctx) else {
                    log::warn!("Tried to reopen non-existent comment with ID {comment_id:?}");
                    return;
                };
                match &existing_comment.target {
                    AttachedReviewCommentTarget::Line { line, .. } => {
                        let comment_text = &existing_comment.content;
                        let origin = &existing_comment.origin;
                        editor.update(ctx, |local_code_editor, ctx| {
                            local_code_editor.editor().update(ctx, |code_editor, ctx| {
                                code_editor.open_existing_comment(
                                    comment_id,
                                    line,
                                    comment_text,
                                    origin,
                                    ctx,
                                );
                            });
                        });
                    }
                    AttachedReviewCommentTarget::File { .. }
                    | AttachedReviewCommentTarget::General => {
                        log::error!("Tried to reopen a non-line review comment.");
                    }
                }
            }
            LocalCodeEditorEvent::DiffStatusUpdated => {}
            LocalCodeEditorEvent::ViewportUpdated => {}
            LocalCodeEditorEvent::LayoutInvalidated => {
                if let CodeReviewViewState::Loaded(state) = self.state() {
                    if let Some(index) = state
                        .file_states
                        .iter()
                        .position(|f| f.1.file_diff.file_path == *diff_file_path)
                    {
                        self.viewported_list_state
                            .invalidate_height_for_index(index);
                        ctx.notify();
                    }
                }
            }
            #[cfg(not(target_family = "wasm"))]
            LocalCodeEditorEvent::GotoDefinition {
                path,
                line,
                column,
                source_server_id,
            } => {
                // Register the external file so it can use LSP features.
                // The manager will skip registration if the path is under an existing workspace.
                let lsp_manager = lsp::LspManagerModel::handle(ctx);
                lsp_manager.update(ctx, |mgr, _| {
                    mgr.maybe_register_external_file(path, *source_server_id);
                });

                self.open_file_in_tab(
                    path,
                    Some(LineAndColumnArg {
                        // LSP uses 0-indexed lines, but we display 1-indexed
                        line_num: *line + 1,
                        column_num: Some(*column),
                    }),
                    ctx,
                );
            }
            #[cfg(not(target_family = "wasm"))]
            LocalCodeEditorEvent::OpenLspLogs { log_path } => {
                ctx.emit(CodeReviewViewEvent::OpenLspLogs {
                    log_path: log_path.clone(),
                });
            }
            _ => (),
        }
    }

    fn get_comment_by_id(&self, id: CommentId, app: &AppContext) -> Option<AttachedReviewComment> {
        self.active_comment_model.as_ref().and_then(|model| {
            model.read(app, |batch, _| batch.get_review_comment_by_id(id).cloned())
        })
    }

    fn comment_line_numbers_for_file(&self, file_path: &Path, app: &AppContext) -> Vec<LineCount> {
        self.active_comment_model
            .as_ref()
            .map(|model| {
                model.read(app, |batch, _| {
                    batch.comment_line_numbers_for_file(file_path).collect_vec()
                })
            })
            .unwrap_or_default()
    }

    /// Marks the editor for the given file path as loaded.
    /// This is called when LocalCodeEditorEvent::DelayedRenderingFlushed or FailedToLoad fires.
    fn mark_editor_loaded_for_file(&mut self, file_path: &Path, ctx: &mut ViewContext<Self>) {
        let Some(repo) = self.active_repo.as_mut() else {
            return;
        };

        let CodeReviewViewState::Loaded(loaded_state) = &mut repo.state else {
            return;
        };

        if let Some(file_state) = loaded_state.file_states.get_mut(file_path) {
            if let Some(editor_state) = &mut file_state.editor_state {
                editor_state.set_loaded();
            }
        }

        if self.all_editors_loaded() {
            let diff_mode = self.diff_state_model.as_ref(ctx).diff_mode(ctx);
            self.reposition_comments_in_file(&diff_mode, ctx);
        }
    }

    /// Returns true if all editors with editor_state have finished loading their buffer content.
    /// For global buffer mode, this checks the is_loaded flag on each CodeReviewEditorState.
    /// Returns true if there are no file states or if global buffer is not enabled.
    fn all_editors_loaded(&self) -> bool {
        let Some(repo) = self.active_repo.as_ref() else {
            return true;
        };

        let CodeReviewViewState::Loaded(loaded_state) = &repo.state else {
            return true;
        };

        // Check if all editors with editor_state are loaded
        loaded_state
            .file_states
            .values()
            .filter_map(|file_state| file_state.editor_state.as_ref())
            .all(|editor_state| editor_state.is_loaded())
    }

    fn apply_diff_to_code_editor(
        code_editor_view: &ViewHandle<LocalCodeEditorView>,
        file: &FileDiffAndContent,
        is_initial_setup: bool,
        comment_line_numbers: &[LineCount],
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(file_content) = &file.content_at_head {
            let version = ContentVersion::new();
            let diff_deltas = Self::convert_hunks_to_diff_deltas(&file.file_diff.hunks);

            // For deleted files, we need to populate the buffer directly since the file
            // doesn't exist on disk and can't be loaded via GlobalBufferModel.
            let is_deleted_file = matches!(file.file_diff.status, GitFileStatus::Deleted);

            code_editor_view.update(ctx, |local_editor, ctx| {
                // Preserve cursor position for non-initial setups (when refreshing diffs)
                let saved_selections = if !is_initial_setup {
                    Some(Self::capture_cursor_position(
                        local_editor.editor().as_ref(ctx),
                        ctx,
                    ))
                } else {
                    None
                };

                let mut range = None;

                // When global buffer is enabled (and file is not deleted), we only need to set the base to the content at HEAD.
                // For deleted files or when global buffer is disabled, we need to populate the buffer directly.
                if is_deleted_file {
                    let line_count = file_content.lines().count();
                    range = Some(calculate_hidden_lines(
                        &diff_deltas,
                        line_count,
                        comment_line_numbers,
                    ));
                    let state = InitialBufferState::plain_text(file_content).with_version(version);
                    // Reset editor state with incoming content.
                    local_editor.reset_with_state(state, ctx);
                }
                #[cfg(not(target_family = "wasm"))]
                if !is_deleted_file {
                    // We only want to recompute diff is the file is loaded. If not, we can rely on the file load event
                    // for diff computation.
                    let file_loaded = local_editor.file_loaded(ctx);

                    local_editor.editor().update(ctx, |editor, ctx| {
                        editor.set_base(file_content, file_loaded, ctx);
                    });
                }

                local_editor.editor().update(ctx, |editor, ctx| {
                    // When global buffer is enabled (and file is not deleted), hidden line configuration is handed off to the model itself.
                    if is_deleted_file {
                        if let Some(range) = range {
                            editor.model.update(ctx, |model, ctx| {
                                model.set_hidden_lines(range, ctx);
                            });
                        }

                        if !diff_deltas.is_empty() {
                            editor.apply_diffs(diff_deltas, ctx);
                        }
                    }
                    editor.expand_diffs(ctx);

                    // Restore cursor position if it was saved
                    if let Some(selections) = saved_selections {
                        Self::restore_cursor_position(editor, selections, ctx);
                    }

                    if is_initial_setup {
                        if FeatureFlag::CodeReviewSaveChanges.is_enabled() {
                            editor.set_interaction_state(InteractionState::Editable, ctx);
                        } else {
                            editor.set_interaction_state(InteractionState::Selectable, ctx);
                        }
                    }
                });
            });
        }
    }

    /// Relocates comments by updating their line locations based on the current state of editors.
    ///
    /// This is a pure function that takes comments and returns updated comments without mutation.
    /// For each `Line` comment, it finds the matching editor via file path and computes the new
    /// target location. Comments without matching editors or without matching line content are
    /// marked as outdated.
    ///
    /// Returns a tuple of (all_comments_with_updated_targets, fallback_count).
    fn relocate_comments(
        comments: impl IntoIterator<Item = AttachedReviewComment>,
        state: &LoadedState,
        repo_path: &Path,
        ctx: &mut ViewContext<Self>,
    ) -> RelocateCommentsResult {
        let mut fallback_count = 0;
        let editor_file_paths = state.editor_absolute_file_paths(repo_path);

        let relocated_comments = comments
            .into_iter()
            .map(|mut comment| {
                if comment.target.absolute_file_path().is_none() {
                    // General comments pass through unchanged.
                    return comment;
                };

                let matching_editor = match &comment.target {
                    AttachedReviewCommentTarget::Line {
                        absolute_file_path, ..
                    }
                    | AttachedReviewCommentTarget::File { absolute_file_path } => editor_file_paths
                        .iter()
                        .find(|(_, editor_path)| editor_path == absolute_file_path)
                        .map(|(editor, _)| editor),
                    AttachedReviewCommentTarget::General => None,
                };

                let Some(editor_view) = matching_editor else {
                    // If there's no matching editor, mark the comment as outdated.
                    // The comment retains its original content so it can still be displayed.
                    if FeatureFlag::PRCommentsSlashCommand.is_enabled() {
                        comment.outdated = true;
                    }
                    return comment;
                };

                let AttachedReviewCommentTarget::Line {
                    absolute_file_path,
                    line,
                    content,
                } = &comment.target
                else {
                    // File-level comments with matching editors pass through unchanged.
                    return comment;
                };

                let (new_location, new_content, used_fallback) =
                    editor_view.update(ctx, |local_editor, ctx| {
                        local_editor.editor().update(ctx, |editor, ctx| {
                            editor.model.update(ctx, |model, ctx| {
                                model.get_new_line_location(line, content.original_text(), ctx)
                            })
                        })
                    });

                if used_fallback {
                    fallback_count += 1;
                    if FeatureFlag::PRCommentsSlashCommand.is_enabled() {
                        comment.outdated = true;
                    }
                } else {
                    comment.outdated = false;
                    comment.target = AttachedReviewCommentTarget::Line {
                        absolute_file_path: absolute_file_path.to_path_buf(),
                        line: new_location,
                        content: new_content,
                    };
                }

                comment
            })
            .collect();

        RelocateCommentsResult {
            comments: relocated_comments,
            fallback_count,
        }
    }

    fn reposition_comments_in_file(&mut self, diff_mode: &DiffMode, ctx: &mut ViewContext<Self>) {
        let Some(model) = &self.active_comment_model else {
            log::error!("Failed to relocate PR comments: CodeReviewView diff state not loaded",);
            return;
        };

        let Some(repo_path) = self.repo_path() else {
            log::error!("Failed to relocate PR comments: CodeReviewView has no repo path");
            return;
        };

        let CodeReviewViewState::Loaded(state) = self.state() else {
            log::warn!("Failed to relocate PR comments: CodeReviewView diff state not loaded");
            return;
        };

        let mut comments = model.update(ctx, |batch, _| batch.take_comments());
        let pending_imported = model.update(ctx, |batch, _| {
            batch.take_pending_imported_comments_for_branch(diff_mode)
        });

        let newly_imported = attach_pending_imported_comments(pending_imported, repo_path);
        let newly_imported_ids: HashSet<CommentId> = newly_imported.iter().map(|c| c.id).collect();
        comments.extend(newly_imported);

        if comments.is_empty() {
            return;
        }

        let RelocateCommentsResult {
            comments: relocated_comments,
            fallback_count,
        } = Self::relocate_comments(comments, state, repo_path, ctx);

        if fallback_count > 0 {
            send_telemetry_from_ctx!(
                CodeReviewTelemetryEvent::CommentRelocationFailed { fallback_count },
                ctx
            );
        }

        if !newly_imported_ids.is_empty() {
            let (active_count, outdated_count) = relocated_comments
                .iter()
                .filter(|c| newly_imported_ids.contains(&c.id))
                .fold((0usize, 0usize), |(active, outdated), c| {
                    if c.outdated {
                        (active, outdated + 1)
                    } else {
                        (active + 1, outdated)
                    }
                });
            send_telemetry_from_ctx!(
                CodeReviewTelemetryEvent::CommentsAttached {
                    active_count,
                    outdated_count,
                },
                ctx
            );
        }

        model.update(ctx, |batch, ctx| {
            batch.upsert_comments(relocated_comments, ctx);
        });

        ctx.notify();
    }

    /// Opens the comment list tray to display comments.
    pub(crate) fn expand_comment_list(&mut self, ctx: &mut ViewContext<Self>) {
        self.comment_list_view.update(ctx, |comment_list, ctx| {
            comment_list.expand(ctx);
        });
    }
    /// Opens the comment list tray and scrolls to the given comment.
    pub(crate) fn expand_comment_list_and_scroll_to_comment(
        &mut self,
        comment_id: CommentId,
        ctx: &mut ViewContext<Self>,
    ) {
        self.comment_list_view.update(ctx, |comment_list, ctx| {
            comment_list.expand(ctx);
            comment_list.scroll_to_comment(comment_id, ctx);
        });
    }

    fn render_placeholder_header(appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();

        let header_text = "Loading open changes...";
        let loading_icon = Icon::Loading
            .to_warpui_icon(warp_core::ui::theme::Fill::Solid(
                internal_colors::neutral_6(theme),
            ))
            .finish();
        let loading_icon = Container::new(
            ConstrainedBox::new(loading_icon)
                .with_height(appearance.ui_font_size())
                .with_width(appearance.ui_font_size())
                .finish(),
        )
        .with_margin_right(10.)
        .finish();

        let mut flex = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::Start)
            .with_cross_axis_alignment(CrossAxisAlignment::Center);
        flex.add_child(loading_icon);

        flex.add_child(
            Container::new(
                Text::new(
                    header_text,
                    appearance.ui_font_family(),
                    appearance.ui_font_size(),
                )
                .with_style(Properties::default().weight(Weight::Semibold))
                .with_color(theme.main_text_color(theme.background()).into())
                .finish(),
            )
            .finish(),
        );
        flex.finish()
    }

    /// Renders the loading state
    pub fn render_loading_state(appearance: &Appearance) -> Box<dyn Element> {
        let placeholder = (0..4).map(|_| {
            Shrinkable::new(
                1.,
                Container::new(CodeReviewView::render_code_diff_placeholder(appearance))
                    .with_margin_bottom(EDITOR_GAP)
                    .finish(),
            )
            .finish()
        });
        Container::new(
            Flex::column()
                .with_child(
                    Container::new(CodeReviewView::render_placeholder_header(appearance))
                        .with_padding_bottom(12.)
                        .finish(),
                )
                .with_children(placeholder)
                .finish(),
        )
        .with_uniform_padding(16.)
        .with_padding_top(CONTENT_TOP_MARGIN)
        .finish()
    }

    fn render_code_diff_placeholder(appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();

        let base_gradient_color_start = internal_colors::neutral_2(theme);

        // Make the end of gradient the same color but with 10% opacity.
        let mut base_gradient_color_end = base_gradient_color_start;
        base_gradient_color_end.a = 26;

        // Percent widths to render for each line in the diff placeholder.
        let percent_widths = vec![0.6, 0.77, 0.34, 0.88, 0.25, 0.07, 0.7];
        let lines = percent_widths.into_iter().map(|percent_width| {
            let rect = ConstrainedBox::new(
                Rect::new()
                    .with_horizontal_background_gradient(
                        base_gradient_color_start,
                        base_gradient_color_end,
                    )
                    .finish(),
            )
            .with_height(18.)
            .finish();

            let rect = Align::new(Percentage::width(percent_width, rect).finish())
                .left()
                .finish();

            Shrinkable::new(1., Container::new(rect).with_vertical_padding(1.).finish()).finish()
        });

        let lines = Flex::column().with_children(lines).finish();
        let corner_radius = Radius::Pixels(8.);

        Container::new(
            Flex::column()
                .with_main_axis_alignment(MainAxisAlignment::Center)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(
                    ConstrainedBox::new(
                        Rect::new()
                            .with_horizontal_background_gradient(
                                base_gradient_color_start,
                                base_gradient_color_end,
                            )
                            .with_corner_radius(CornerRadius::with_top(corner_radius))
                            .finish(),
                    )
                    .with_height(24.)
                    .finish(),
                )
                .with_child(
                    Shrinkable::new(
                        1.,
                        Container::new(lines)
                            .with_horizontal_padding(12.)
                            .with_vertical_padding(4.)
                            .finish(),
                    )
                    .finish(),
                )
                .finish(),
        )
        .with_corner_radius(CornerRadius::with_all(corner_radius))
        .with_border(Border::all(1.).with_border_fill(base_gradient_color_start))
        .finish()
    }

    fn render_error_state(&self, error: &str, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();

        let main_column = Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::Center)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Container::new(
                    ConstrainedBox::new(
                        Icon::AlertTriangle
                            .to_warpui_icon(warp_core::ui::theme::Fill::Solid(
                                internal_colors::neutral_6(theme),
                            ))
                            .finish(),
                    )
                    .with_width(24.)
                    .with_height(24.)
                    .finish(),
                )
                .with_margin_bottom(12.)
                .finish(),
            )
            .with_child(
                Text::new(
                    "Error loading diffs",
                    appearance.ui_font_family(),
                    appearance.ui_font_size() + 2.,
                )
                .with_style(Properties::default().weight(Weight::Semibold))
                .with_color(theme.sub_text_color(theme.background()).into())
                .finish(),
            )
            .with_child(
                ConstrainedBox::new(
                    Shrinkable::new(
                        1.,
                        Container::new(
                            Shrinkable::new(
                                1.,
                                Text::new(
                                    error.to_string(),
                                    appearance.ui_font_family(),
                                    appearance.ui_font_size() + 2.,
                                )
                                .with_color(theme.disabled_text_color(theme.background()).into())
                                .finish(),
                            )
                            .finish(),
                        )
                        .with_margin_top(4.)
                        .finish(),
                    )
                    .finish(),
                )
                .with_max_width(425.)
                .finish(),
            )
            .with_child(
                Container::new(
                    appearance
                        .ui_builder()
                        .button(
                            ButtonVariant::Secondary,
                            self.ui_state_handles.retry_button_mouse_state.clone(),
                        )
                        .with_text_and_icon_label(TextAndIcon::new(
                            TextAndIconAlignment::IconFirst,
                            " Retry".to_string(),
                            Icon::Refresh.to_warpui_icon(warp_core::ui::theme::Fill::Solid(
                                theme.main_text_color(theme.background()).into(),
                            )),
                            MainAxisSize::Min,
                            MainAxisAlignment::SpaceBetween,
                            vec2f(16., 16.),
                        ))
                        .with_style(UiComponentStyles {
                            font_weight: Some(Weight::Semibold),
                            padding: Some(Coords {
                                top: 4.,
                                bottom: 4.,
                                left: 8.,
                                right: 8.,
                            }),
                            ..Default::default()
                        })
                        .build()
                        .on_click(|ctx, _, _| {
                            ctx.dispatch_typed_action(CodeReviewAction::RefreshGitState)
                        })
                        .finish(),
                )
                .with_margin_top(12.)
                .finish(),
            )
            .finish();

        Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::Center)
            .with_child(main_column)
            .finish()
    }

    #[cfg(not(target_family = "wasm"))]
    fn render_no_repo_found_state_with_buttons(
        &self,
        appearance: &Appearance,
        message: &'static str,
        buttons: InitButtons,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();

        let main_column = Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::Center)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Container::new(
                    ConstrainedBox::new(
                        Icon::FolderClosed
                            .to_warpui_icon(warp_core::ui::theme::Fill::Solid(
                                internal_colors::neutral_6(theme),
                            ))
                            .finish(),
                    )
                    .with_width(24.)
                    .with_height(24.)
                    .finish(),
                )
                .with_margin_bottom(12.)
                .finish(),
            )
            .with_child(
                Text::new(
                    "Cannot detect diffs for this folder",
                    appearance.ui_font_family(),
                    appearance.ui_font_size() + 2.,
                )
                .with_style(Properties::default().weight(Weight::Semibold))
                .with_color(theme.sub_text_color(theme.background()).into())
                .finish(),
            )
            .with_child(
                Container::new(
                    Text::new(
                        message,
                        appearance.ui_font_family(),
                        appearance.ui_font_size() + 2.,
                    )
                    .with_color(theme.disabled_text_color(theme.background()).into())
                    .finish(),
                )
                .with_margin_top(4.)
                .finish(),
            );

        let mut buttons_row = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::Center)
            .with_cross_axis_alignment(CrossAxisAlignment::Center);

        match buttons {
            InitButtons::OpenRepository => {
                buttons_row.add_child(ChildView::new(&self.open_repository_button).finish());
            }
            InitButtons::InitProject => {
                buttons_row.add_child(ChildView::new(&self.init_project_button).finish());
            }
            InitButtons::None => {}
        }

        let main_column = main_column
            .with_child(
                Container::new(buttons_row.finish())
                    .with_margin_top(16.)
                    .finish(),
            )
            .finish();

        Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::Center)
            .with_child(main_column)
            .finish()
    }

    pub fn render_no_repo_found_state(
        appearance: &Appearance,
        message: &'static str,
        open_repo_button: Option<Box<dyn Element>>,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();

        let main_column = Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::Center)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Container::new(
                    ConstrainedBox::new(
                        Icon::FolderClosed
                            .to_warpui_icon(warp_core::ui::theme::Fill::Solid(
                                internal_colors::neutral_6(theme),
                            ))
                            .finish(),
                    )
                    .with_width(24.)
                    .with_height(24.)
                    .finish(),
                )
                .with_margin_bottom(12.)
                .finish(),
            )
            .with_child(
                Text::new(
                    "Cannot detect diffs for this folder",
                    appearance.ui_font_family(),
                    appearance.ui_font_size() + 2.,
                )
                .with_style(Properties::default().weight(Weight::Semibold))
                .with_color(theme.sub_text_color(theme.background()).into())
                .finish(),
            )
            .with_child(
                Container::new(
                    Text::new(
                        message,
                        appearance.ui_font_family(),
                        appearance.ui_font_size() + 2.,
                    )
                    .with_color(theme.disabled_text_color(theme.background()).into())
                    .finish(),
                )
                .with_margin_top(4.)
                .finish(),
            );

        let main_column = if let Some(button) = open_repo_button {
            main_column
                .with_child(Container::new(button).with_margin_top(16.).finish())
                .finish()
        } else {
            main_column.finish()
        };

        Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::Center)
            .with_child(main_column)
            .finish()
    }

    pub fn render_remote_state(
        appearance: &Appearance,
        open_repo_button: Option<Box<dyn Element>>,
    ) -> Box<dyn Element> {
        Self::render_no_repo_found_state(appearance, REMOTE_TEXT, open_repo_button)
    }

    pub fn render_wsl_state(
        appearance: &Appearance,
        open_repo_button: Option<Box<dyn Element>>,
    ) -> Box<dyn Element> {
        Self::render_no_repo_found_state(appearance, WSL_TEXT, open_repo_button)
    }

    pub fn render_not_repo_state(
        appearance: &Appearance,
        open_repo_button: Option<Box<dyn Element>>,
    ) -> Box<dyn Element> {
        Self::render_no_repo_found_state(appearance, DISABLED_TEXT, open_repo_button)
    }

    #[cfg(not(target_family = "wasm"))]
    fn render_remote_state_with_buttons(&self, appearance: &Appearance) -> Box<dyn Element> {
        self.render_no_repo_found_state_with_buttons(
            appearance,
            REMOTE_TEXT,
            InitButtons::OpenRepository,
        )
    }

    #[cfg(not(target_family = "wasm"))]
    fn render_wsl_state_with_buttons(&self, appearance: &Appearance) -> Box<dyn Element> {
        self.render_no_repo_found_state_with_buttons(
            appearance,
            WSL_TEXT,
            InitButtons::OpenRepository,
        )
    }

    #[cfg(not(target_family = "wasm"))]
    fn render_not_repo_state_with_buttons(&self, appearance: &Appearance) -> Box<dyn Element> {
        self.render_no_repo_found_state_with_buttons(
            appearance,
            DISABLED_TEXT,
            InitButtons::OpenRepository,
        )
    }

    fn render_loaded_state(
        &self,
        state: &LoadedState,
        appearance: &Appearance,
        is_in_split_pane: bool,
        app: &warpui::AppContext,
    ) -> Box<dyn Element> {
        let top_section = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_main_axis_alignment(MainAxisAlignment::Start)
            .with_child(self.render_header(state, appearance, is_in_split_pane, app))
            .with_child(self.render_content(state, appearance, app));

        let top_section_with_margin = ConstrainedBox::new(
            Container::new(Shrinkable::new(1., top_section.finish()).finish())
                .with_margin_left(CONTENT_LEFT_MARGIN)
                .with_margin_right(CONTENT_RIGHT_MARGIN)
                .with_margin_bottom(5.)
                .finish(),
        )
        .with_min_width(180.)
        .finish();

        // Bottom section: comment list view
        // The view handles its own resizable logic internally
        let bottom_section = ChildView::new(&self.comment_list_view).finish();

        // Global LSP footer (below comment list)
        // Only show the footer if there are changes and all editors have finished loading.
        let footer_section = if !state.file_states.is_empty() && self.all_editors_loaded() {
            self.code_review_footer
                .as_ref()
                .map(|footer| ChildView::new(footer).finish())
        } else {
            None
        };

        // Wrap all sections in a vertical flex that takes full height
        let mut col = Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(Shrinkable::new(1., top_section_with_margin).finish())
            .with_child(bottom_section);

        if let Some(footer) = footer_section {
            col.add_child(footer);
        }

        col.finish()
    }

    /// Renders the state when there are no changes on the current branch.
    fn render_no_changes_state(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();

        let mut main_row = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max);

        let mut zero_state_column = Flex::column()
            .with_main_axis_alignment(MainAxisAlignment::Center)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Container::new(
                    ConstrainedBox::new(
                        Icon::Diff
                            .to_warpui_icon(warp_core::ui::theme::Fill::Solid(
                                internal_colors::neutral_6(theme),
                            ))
                            .finish(),
                    )
                    .with_width(48.)
                    .with_height(48.)
                    .finish(),
                )
                .with_margin_bottom(16.)
                .finish(),
            )
            .with_child(
                Text::new("No open changes", appearance.ui_font_family(), 16.)
                    .with_style(Properties::default().weight(Weight::Semibold))
                    .with_color(theme.main_text_color(theme.surface_2()).into())
                    .finish(),
            )
            .with_child(
                Container::new(
                    Text::new(
                        "As you or the Agent make changes, you'll be able to track them here.",
                        appearance.ui_font_family(),
                        14.,
                    )
                    .with_color(theme.sub_text_color(theme.surface_2()).into())
                    .finish(),
                )
                .with_margin_top(8.)
                .finish(),
            );

        let should_show_init = self
            .repo_path()
            .map(|path| {
                let has_steps = InitProjectModel::should_have_available_steps(path, app);
                let is_terminal_in_correct_dir = self
                    .terminal_view(app)
                    .and_then(|view| {
                        view.read(app, |t, _| t.pwd().map(|pwd| pwd == path.to_string_lossy()))
                    })
                    .unwrap_or(false);
                has_steps && is_terminal_in_correct_dir
            })
            .unwrap_or(false);

        if should_show_init {
            zero_state_column.add_child(
                Container::new(ChildView::new(&self.init_project_button).finish())
                    .with_margin_top(16.)
                    .finish(),
            );
        } else if let Some(repo_path) = self.repo_path() {
            // Check for initialized project-scoped rules.
            if let Some(rules) =
                ProjectContextModel::as_ref(app).find_applicable_project_rules(repo_path)
            {
                if let Some(first_rule) = rules.active_rules.first() {
                    if let Some(file_name) = first_rule.path.file_name().and_then(|n| n.to_str()) {
                        zero_state_column.add_child(
                            Container::new(
                                Text::new(
                                    format!("Repo is initialized with a {file_name} file."),
                                    appearance.ui_font_family(),
                                    12.,
                                )
                                .with_color(theme.sub_text_color(theme.surface_2()).into())
                                .finish(),
                            )
                            .with_margin_top(8.)
                            .finish(),
                        );
                    }
                }
            }
        }

        let zero_state_content = Container::new(zero_state_column.finish()).finish();

        // Add expandable spacers on left and right to center the content and force full width.
        main_row.add_child(Shrinkable::new(1., Empty::new().finish()).finish());
        main_row.add_child(zero_state_content);
        main_row.add_child(Shrinkable::new(1., Empty::new().finish()).finish());

        Shrinkable::new(
            1.,
            Container::new(Clipped::new(main_row.finish()).finish())
                .with_border(Border::new(1.0).with_border_fill(theme.outline()))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
                .with_uniform_padding(16.)
                .with_margin_bottom(16.)
                .finish(),
        )
        .finish()
    }

    /// Renders the header with diff mode dropdown and overflow menu.
    fn render_header(
        &self,
        state: &LoadedState,
        appearance: &Appearance,
        is_in_split_pane: bool,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let has_menu_flags = FeatureFlag::DiscardPerFileAndAllChanges.is_enabled()
            || FeatureFlag::DiffSetAsContext.is_enabled()
            || FeatureFlag::FileAndDiffSetComments.is_enabled();
        let has_changes = matches!(self.state(), CodeReviewViewState::Loaded(loaded) if !loaded.to_diff_stats().has_no_changes());
        let has_header_menu_items =
            has_menu_flags && (!FeatureFlag::GitOperationsInCodeReview.is_enabled() || has_changes);

        let code_review_header_fields = CodeReviewHeaderFields {
            is_in_split_pane,
            maximize_button: self.maximize_button.clone(),
            diff_selector: self.diff_selector.clone(),
            header_menu: self.header_menu.clone(),
            header_menu_open: self.header_menu_open,
            diff_state_model: self.diff_state_model.clone(),
            header_dropdown_button: self.header_dropdown_button.clone(),
            has_header_menu_items,
            file_nav_button: if FeatureFlag::GitOperationsInCodeReview.is_enabled()
                && self.has_file_states()
            {
                Some(self.file_nav_button.clone())
            } else {
                None
            },
            primary_git_action_mode: self.primary_git_action_mode(app),
            git_primary_action_button: self.git_primary_action_button.clone(),
            git_operations_chevron: self.git_operations_chevron.clone(),
            git_operations_menu: self.git_operations_menu.clone(),
            git_operations_menu_open: self.git_operations_menu_open,
        };

        let header = if FeatureFlag::GitOperationsInCodeReview.is_enabled() {
            self.header
                .render_new(appearance, &code_review_header_fields)
        } else {
            self.header
                .render(state, appearance, &code_review_header_fields, app)
        };
        SavePosition::new(header, &self.header_position_id).finish()
    }

    /// Prepares review comments and emits an event for a higher-level view to route
    /// them to an available terminal.
    fn handle_submit_review_with_comments(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(model) = self.active_comment_model.as_ref() else {
            return;
        };

        let review_comments = model.read(ctx, |batch, _| batch.clone());

        let active_comments: Vec<_> = review_comments
            .comments
            .into_iter()
            .filter(|c| !c.outdated)
            .collect();

        if active_comments.is_empty() {
            log::info!("No review comments to submit");
            return;
        }

        let Some(repo_path) = self.repo_path().cloned() else {
            log::warn!("No active repo path for submitting review");
            return;
        };

        let active_batch = ReviewCommentBatch::from_comments(active_comments);
        let diff_set = self.collect_diff_set(&active_batch);
        let agent_comment_batch = AgentReviewCommentBatch {
            comments: active_batch.comments,
            diff_set,
        };

        ctx.emit(CodeReviewViewEvent::SubmitReviewComments {
            comments: agent_comment_batch,
            repo_path,
        });
    }

    /// Called by the routing layer (RightPanelView) after attempting to submit review
    /// comments to a terminal.
    pub fn handle_review_submission_result(
        &mut self,
        result: ReviewSubmissionResult,
        ctx: &mut ViewContext<Self>,
    ) {
        match result {
            ReviewSubmissionResult::Success {
                comment_count,
                file_count,
                destination,
            } => {
                log::info!("Successfully submitted review comments to terminal");

                send_telemetry_from_ctx!(
                    CodeReviewTelemetryEvent::ReviewSubmitted {
                        comment_count,
                        file_count,
                        destination,
                    },
                    ctx
                );

                self.clear_review_comments(ctx);
                ToastStack::handle(ctx).update(ctx, |stack, ctx| {
                    let toast = DismissibleToast::default("Comments sent to agent".into());
                    stack.add_ephemeral_toast(toast, self.window_id, ctx);
                });
                ctx.emit(CodeReviewViewEvent::ReviewSubmitted);
                ctx.notify();
            }
            ReviewSubmissionResult::Error => {
                log::error!("Failed to submit review comments");
                let error_message = "Could not submit comments to the agent".to_string();
                ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                    let toast = DismissibleToast::error(error_message);
                    toast_stack.add_ephemeral_toast(toast, self.window_id, ctx);
                });
            }
        }
    }

    /// TODO(CODE-1649): de-duplicate entries in the diff set.
    fn collect_diff_set(
        &self,
        review_comments: &ReviewCommentBatch,
    ) -> HashMap<String, Vec<DiffSetHunk>> {
        let mut diff_set: HashMap<String, Vec<DiffSetHunk>> = HashMap::new();
        let repo_path = self.repo_path();

        for comment in &review_comments.comments {
            match &comment.target {
                AttachedReviewCommentTarget::Line {
                    absolute_file_path,
                    line,
                    content,
                } => {
                    if let Some(line_number) = line.line_number() {
                        let hunk = DiffSetHunk {
                            line_range: line_number..line_number + 1,
                            diff_content: content.content.clone(),
                            lines_added: content.lines_added.as_u32(),
                            lines_removed: content.lines_removed.as_u32(),
                        };

                        let file_key = repo_path
                            .and_then(|repo_path| absolute_file_path.strip_prefix(repo_path).ok())
                            .unwrap_or(absolute_file_path.as_path())
                            .to_string_lossy()
                            .to_string();

                        diff_set.entry(file_key).or_default().push(hunk);
                    }
                }
                AttachedReviewCommentTarget::File { absolute_file_path } => {
                    if let CodeReviewViewState::Loaded(loaded_state) = self.state() {
                        let file_diffs = loaded_state
                            .file_states
                            .values()
                            .filter(|fs| absolute_file_path.ends_with(&fs.file_diff.file_path))
                            .map(|fs| &fs.file_diff);
                        let hunks = convert_file_diffs_to_diffset_hunks(file_diffs);
                        diff_set.extend(hunks);
                    }
                }
                AttachedReviewCommentTarget::General => {
                    if let CodeReviewViewState::Loaded(loaded_state) = self.state() {
                        let file_diffs = loaded_state.file_states.values().map(|fs| &fs.file_diff);
                        let hunks = convert_file_diffs_to_diffset_hunks(file_diffs);
                        diff_set.extend(hunks);
                    }
                }
            }
        }
        diff_set
    }

    /// Renders additions and deletions counts
    pub fn render_additions_and_deletions(
        stats: &DiffStats,
        appearance: &Appearance,
        minus_px: f32,
    ) -> Flex {
        let mut counts_row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);

        // Add additions count
        counts_row.add_child(
            Container::new(
                Text::new(
                    format!("+{}", stats.total_additions),
                    appearance.ui_font_family(),
                    appearance.ui_font_size() - minus_px,
                )
                .with_color(add_color(appearance))
                .finish(),
            )
            .with_margin_right(4.)
            .finish(),
        );

        // Add deletions count
        counts_row.add_child(
            Container::new(
                Text::new(
                    format!("-{}", stats.total_deletions),
                    appearance.ui_font_family(),
                    appearance.ui_font_size() - minus_px,
                )
                .with_color(remove_color(appearance))
                .finish(),
            )
            .finish(),
        );

        counts_row
    }

    /// Renders the diff statistics
    pub fn render_diff_stats(stats: &DiffStats, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();

        // Create diff stats chip content
        let mut counts_row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);

        // Add file icon
        let file_icon = Icon::File
            .to_warpui_icon(warp_core::ui::theme::Fill::Solid(
                internal_colors::neutral_6(theme),
            ))
            .finish();
        counts_row.add_child(
            Container::new(
                ConstrainedBox::new(file_icon)
                    .with_height(appearance.ui_font_size())
                    .with_width(appearance.ui_font_size())
                    .finish(),
            )
            .with_margin_right(4.)
            .finish(),
        );

        // Add file count
        counts_row.add_child(
            Container::new(
                Text::new_inline(
                    stats.files_changed.to_string(),
                    appearance.ui_font_family(),
                    appearance.ui_font_size(),
                )
                .with_color(
                    warp_core::ui::theme::Fill::Solid(internal_colors::neutral_6(theme)).into(),
                )
                .with_line_height_ratio(appearance.line_height_ratio())
                .with_style(Properties::default().weight(Weight::Semibold))
                .finish(),
            )
            .with_margin_right(4.)
            .finish(),
        );

        if stats.has_no_changes() {
            return counts_row.finish();
        }

        // Add separator
        counts_row.add_child(
            Container::new(
                Text::new(
                    " • ",
                    appearance.ui_font_family(),
                    appearance.ui_font_size(),
                )
                .with_style(Properties::default().weight(Weight::Bold))
                .with_color(
                    warp_core::ui::theme::Fill::Solid(internal_colors::neutral_6(theme)).into(),
                )
                .finish(),
            )
            .with_margin_right(4.)
            .finish(),
        );

        counts_row.add_child(Self::render_additions_and_deletions(stats, appearance, 0.).finish());

        counts_row.finish()
    }

    /// Renders the content area with all file diffs
    fn render_content(
        &self,
        state: &LoadedState,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        if state.file_states.is_empty() {
            return self.render_no_changes_state(appearance, app);
        }

        let mut sidebar_and_diffs_row =
            Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

        let sidebar_on_right = FeatureFlag::GitOperationsInCodeReview.is_enabled();

        // When the flag is off, sidebar goes on the left (legacy).
        if !sidebar_on_right && self.file_sidebar_expanded && !state.file_states.is_empty() {
            sidebar_and_diffs_row
                .add_child(Container::new(self.render_file_sidebar(state, appearance)).finish());

            let vertical_separator = ConstrainedBox::new(
                Rect::new()
                    .with_background(appearance.theme().outline())
                    .finish(),
            )
            .with_width(1.)
            .finish();

            sidebar_and_diffs_row.add_child(vertical_separator);
        }

        let axis_config = SingleAxisConfig::Manual {
            handle: self.scroll_state.clone(),
            child: NewScrollableElement::finish_scrollable(List::new(
                self.viewported_list_state.clone(),
            )),
        };
        let scrollable_diffs = NewScrollable::vertical(
            axis_config,
            appearance.theme().nonactive_ui_detail().into(),
            appearance.theme().active_ui_detail().into(),
            warpui::elements::Fill::None,
        )
        .with_vertical_scrollbar(ScrollableAppearance::new(ScrollbarWidth::Auto, false))
        .with_propagate_mousewheel_if_not_handled(true)
        .with_always_handle_events_first(false)
        .finish();
        let scrollable_diffs =
            SavePosition::new(scrollable_diffs, &self.code_review_list_position_id).finish();

        let diffs_container = if self.file_sidebar_expanded && !state.file_states.is_empty() {
            let margin = if sidebar_on_right {
                Container::new(scrollable_diffs).with_margin_right(15.)
            } else {
                Container::new(scrollable_diffs).with_margin_left(15.)
            };
            margin.finish()
        } else {
            scrollable_diffs
        };

        sidebar_and_diffs_row.add_child(Shrinkable::new(1., diffs_container).finish());

        // When the flag is on, sidebar goes on the right (new layout).
        if sidebar_on_right && self.file_sidebar_expanded && !state.file_states.is_empty() {
            let vertical_separator = ConstrainedBox::new(
                Rect::new()
                    .with_background(appearance.theme().outline())
                    .finish(),
            )
            .with_width(1.)
            .finish();

            sidebar_and_diffs_row.add_child(vertical_separator);
            sidebar_and_diffs_row
                .add_child(Container::new(self.render_file_sidebar(state, appearance)).finish());
        }

        Shrinkable::new(1., sidebar_and_diffs_row.finish()).finish()
    }

    fn render_file_sidebar(
        &self,
        state: &LoadedState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let mut column = Flex::column()
            .with_main_axis_alignment(MainAxisAlignment::Start)
            .with_cross_axis_alignment(CrossAxisAlignment::Start);

        for (file_index, file_state) in state.file_states.values().enumerate() {
            let file_row = self.render_file_sidebar_row(file_state, appearance);
            column.add_child(
                Hoverable::new(file_state.sidebar_mouse_state.clone(), |mouse_state| {
                    let mut container = Container::new(Shrinkable::new(1., file_row).finish())
                        .with_vertical_padding(5.)
                        .with_horizontal_padding(8.)
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)));

                    if mouse_state.is_hovered() {
                        container = container.with_background(warp_core::ui::theme::Fill::Solid(
                            internal_colors::neutral_3(appearance.theme()),
                        ))
                    }
                    container.finish()
                })
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(CodeReviewAction::FileSelected(file_index));
                })
                .with_cursor(Cursor::PointingHand)
                .finish(),
            );
        }

        let scrollable_content = NewScrollable::vertical(
            SingleAxisConfig::Clipped {
                handle: self.ui_state_handles.sidebar_scroll_state.clone(),
                child: column.finish(),
            },
            appearance.theme().nonactive_ui_detail().into(),
            appearance.theme().active_ui_detail().into(),
            warpui::elements::Fill::None,
        )
        .with_vertical_scrollbar(ScrollableAppearance::new(ScrollbarWidth::Auto, false))
        .finish();

        // We need an Align to ensure the Resizable takes up the full height of the sidebar.
        // This way, the click target for resizing doesn't shrink with a short or empty file list.
        let sidebar_on_right = FeatureFlag::GitOperationsInCodeReview.is_enabled();
        let sidebar_content = if sidebar_on_right {
            Container::new(scrollable_content)
                .with_padding_left(8.)
                .finish()
        } else {
            Container::new(scrollable_content)
                .with_padding_right(8.)
                .finish()
        };
        let mut resizable = Resizable::new(
            self.ui_state_handles.sidebar_resizable_state.clone(),
            sidebar_content,
        );
        if sidebar_on_right {
            resizable = resizable.with_dragbar_side(DragBarSide::Left);
        }
        resizable
            .on_resize(move |ctx, _| {
                ctx.notify();
            })
            .with_bounds_callback(Box::new(Self::file_sidebar_bounds_callback))
            .finish()
    }

    fn file_sidebar_bounds_callback(_window_bounds: Vector2F) -> (f32, f32) {
        (FILE_SIDEBAR_MIN_WIDTH, FILE_SIDEBAR_MAX_WIDTH)
    }

    fn render_file_sidebar_row(
        &self,
        file_state: &FileState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let file_name = file_state
            .file_diff
            .file_path
            .file_name()
            .and_then(|file_name| file_name.to_str())
            .unwrap_or_default();
        let dir_path = file_state
            .file_diff
            .file_path
            .parent()
            .and_then(|parent| parent.to_str())
            .unwrap_or_default();
        let additions = file_state.file_diff.additions();
        let deletions = file_state.file_diff.deletions();

        // Create the main row for the file entry
        let mut file_row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween);

        let mut file_and_directory = Flex::row();

        const SMALLER_TEXT_RATIO: f32 = 0.9;

        // File name (prominent)
        file_and_directory.add_child(
            Container::new(
                ConstrainedBox::new(
                    Text::new(
                        file_name.to_string(),
                        appearance.ui_font_family(),
                        appearance.ui_font_size(),
                    )
                    .with_color(
                        appearance
                            .theme()
                            .main_text_color(appearance.theme().surface_2())
                            .into(),
                    )
                    .soft_wrap(false)
                    .finish(),
                )
                .with_max_width(190.)
                .finish(),
            )
            .with_margin_right(4.)
            .finish(),
        );

        // Directory path (muted and smaller)
        if !dir_path.is_empty() {
            file_and_directory.add_child(
                Shrinkable::new(
                    1.,
                    Text::new(
                        dir_path.to_string(),
                        appearance.ui_font_family(),
                        appearance.ui_font_size() * SMALLER_TEXT_RATIO, // Slightly smaller
                    )
                    .with_color(
                        appearance
                            .theme()
                            .sub_text_color(appearance.theme().surface_2())
                            .into(),
                    )
                    .with_clip(ClipConfig::end())
                    .soft_wrap(false)
                    .with_line_height_ratio(DEFAULT_UI_LINE_HEIGHT_RATIO / SMALLER_TEXT_RATIO)
                    .with_compute_baseline_position_fn(Box::new(|args| {
                        // Calculate baseline position as if we were using the larger font size.
                        // This ensures both text elements have the same baseline.
                        let larger_font_size = args.font_size / SMALLER_TEXT_RATIO;
                        default_compute_baseline_position(
                            larger_font_size,
                            DEFAULT_UI_LINE_HEIGHT_RATIO,
                            args.ascent * (larger_font_size / args.font_size),
                            args.descent * (larger_font_size / args.font_size),
                        )
                    }))
                    .finish(),
                )
                .finish(),
            );
        }

        file_row.add_child(
            Shrinkable::new(1., Clipped::new(file_and_directory.finish()).finish()).finish(),
        );

        // Right side: additions/deletions
        let mut changes_text = Text::new(
            "",
            appearance.ui_font_family(),
            appearance.ui_font_size() * SMALLER_TEXT_RATIO,
        )
        .with_line_height_ratio(DEFAULT_UI_LINE_HEIGHT_RATIO / SMALLER_TEXT_RATIO)
        .with_compute_baseline_position_fn(Box::new(|args| {
            // Calculate baseline position as if we were using the larger font size.
            // This ensures all text elements have the same baseline.
            let larger_font_size = args.font_size / SMALLER_TEXT_RATIO;
            default_compute_baseline_position(
                larger_font_size,
                DEFAULT_UI_LINE_HEIGHT_RATIO,
                args.ascent * (larger_font_size / args.font_size),
                args.descent * (larger_font_size / args.font_size),
            )
        }));
        if additions > 0 {
            changes_text.add_text_with_highlights(
                format!("+{additions}"),
                add_color(appearance),
                Properties::default(),
            );
        }
        if deletions > 0 {
            if !changes_text.text().is_empty() {
                changes_text.add_text_with_highlights(
                    " ",
                    remove_color(appearance),
                    Properties::default(),
                );
            }
            changes_text.add_text_with_highlights(
                format!("-{deletions}"),
                remove_color(appearance),
                Properties::default(),
            );
        }

        if !changes_text.text().is_empty() {
            file_row.add_child(
                Container::new(changes_text.finish())
                    .with_margin_left(8.)
                    .finish(),
            );
        }

        file_row.finish()
    }

    fn file_index_position(&self, file_index: usize) -> String {
        format!("CodeReviewView-{}-{file_index}", self.position_id_prefix)
    }

    fn file_diff_header_position(&self, file_index: usize) -> String {
        format!(
            "CodeReviewView-{}-DiffHeader-{file_index}",
            self.position_id_prefix
        )
    }

    /// Renders a single file's diff
    fn render_file_diff(
        &self,
        file: &FileState,
        file_index: usize,
        scroll_offset_from_top: ScrollOffset,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let is_item_being_scrolled = file_index == scroll_offset_from_top.list_item_index();
        // This helps us avoid rendering the sticky header for the first time when scrolled to the very top.
        let is_first_item_with_no_scroll = file_index == 0
            && scroll_offset_from_top.list_item_index() == 0
            && scroll_offset_from_top.offset_from_start().as_f32() < 1.;

        let file_header =
            if is_item_being_scrolled && file.is_expanded && !is_first_item_with_no_scroll {
                Empty::new().finish()
            } else {
                let header = SavePosition::new(
                    self.render_file_header(file, appearance, app),
                    &self.file_diff_header_position(file_index),
                )
                .finish();
                if file.is_expanded {
                    header
                } else {
                    // The file header is saved as the position if the diff is not expanded.
                    SavePosition::new(header, &self.file_index_position(file_index)).finish()
                }
            };

        let mut content = Flex::column().with_child(file_header);

        let mut stack = Stack::new().with_constrain_absolute_children();
        // Only show file content if expanded.
        if file.is_expanded {
            stack.add_child(
                SavePosition::new(
                    Container::new(self.render_file_content(file, appearance))
                        .with_margin_top(
                            if is_item_being_scrolled && !is_first_item_with_no_scroll {
                                // This is the height of the header bar needs to be present. Otherwise,
                                // the file contents shift up by this amount.
                                if let Some(header_rect) = app.element_position_by_id_at_last_frame(
                                    self.window_id,
                                    self.file_diff_header_position(file_index),
                                ) {
                                    header_rect.height()
                                } else {
                                    FILE_HEADER_HEIGHT
                                }
                            } else {
                                0.
                            },
                        )
                        .finish(),
                    &self.file_index_position(file_index),
                )
                .finish(),
            );
            if is_item_being_scrolled && !is_first_item_with_no_scroll {
                let sticky_file_header = self.render_file_header(file, appearance, app);
                stack.add_positioned_child(
                    sticky_file_header,
                    // We effectively make this an absolutely positioned header.
                    OffsetPositioning::offset_from_parent(
                        vec2f(0., scroll_offset_from_top.offset_from_start().as_f32()),
                        warpui::elements::ParentOffsetBounds::ParentByPosition,
                        warpui::elements::ParentAnchor::TopMiddle,
                        warpui::elements::ChildAnchor::TopMiddle,
                    ),
                );
            }
            content.add_child(stack.finish());
        }

        Container::new(Shrinkable::new(1., content.finish()).finish())
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
            .with_margin_bottom(EDITOR_GAP)
            .finish()
    }

    /// Renders the file header with name and status
    fn render_file_header(
        &self,
        file: &FileState,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();

        let file_name = file.file_diff.file_path.display().to_string();

        let mut left_section = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Min)
            .with_child(
                Container::new(ChildView::new(&file.chevron_button).finish())
                    .with_margin_right(8.)
                    .finish(),
            );
        if let GitFileStatus::Renamed { old_path } = &file.file_diff.status {
            left_section.add_children([
                Shrinkable::new(
                    1.,
                    Text::new(
                        old_path.clone(),
                        appearance.ui_font_family(),
                        appearance.ui_font_size() + 2.,
                    )
                    .with_color(theme.main_text_color(theme.surface_2()).into())
                    .with_clip(ClipConfig::start())
                    .soft_wrap(false)
                    .finish(),
                )
                .finish(),
                Text::new(
                    " → ",
                    appearance.ui_font_family(),
                    appearance.ui_font_size() + 2.,
                )
                .with_color(theme.main_text_color(theme.surface_2()).into())
                .finish(),
                Shrinkable::new(
                    1.,
                    Container::new(
                        Text::new(
                            file_name,
                            appearance.ui_font_family(),
                            appearance.ui_font_size() + 2.,
                        )
                        .with_color(theme.main_text_color(theme.surface_2()).into())
                        .with_clip(ClipConfig::start())
                        .soft_wrap(false)
                        .finish(),
                    )
                    .with_margin_right(8.)
                    .finish(),
                )
                .finish(),
            ]);
        } else {
            left_section.add_child(
                Shrinkable::new(
                    1.,
                    Container::new(
                        Shrinkable::new(
                            1.,
                            Text::new(
                                file_name,
                                appearance.ui_font_family(),
                                appearance.ui_font_size() + 2.,
                            )
                            .with_color(theme.main_text_color(theme.surface_2()).into())
                            .with_clip(ClipConfig::start())
                            .soft_wrap(false)
                            .finish(),
                        )
                        .finish(),
                    )
                    .with_margin_right(8.)
                    .finish(),
                )
                .finish(),
            );
        }

        left_section.add_child(if let Some(editor_state) = &file.editor_state {
            if editor_state.has_unsaved_changes(app) {
                let save_keystroke = Keystroke::parse("cmdorctrl-s").unwrap_or_default();
                let save_shortcut = save_keystroke.displayed();
                let tooltip_text =
                    format!("This file has unsaved changes. {save_shortcut} to save");
                render_unsaved_circle_with_tooltip(
                    editor_state.unsaved_changes_mouse_state(),
                    tooltip_text,
                    10.67,
                    8.,
                    appearance,
                )
            } else {
                Empty::new().finish()
            }
        } else {
            Empty::new().finish()
        });
        left_section.add_child(
            EventHandler::new(
                Container::new(ChildView::new(&file.copy_path_button).finish())
                    .with_margin_right(8.)
                    .finish(),
            )
            .on_left_mouse_up(|_, _, _| DispatchEventResult::StopPropagation)
            .on_left_mouse_down(|_, _, _| DispatchEventResult::StopPropagation)
            .finish(),
        );
        left_section.add_child(self.render_file_stats(&file.file_diff, appearance));

        let mut right_row = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::End)
            .with_cross_axis_alignment(CrossAxisAlignment::Center);

        // Add file diff as context button (before remove button)
        if FeatureFlag::DiffSetAsContext.is_enabled() {
            right_row.add_child(
                EventHandler::new(
                    Container::new(ChildView::new(&file.add_context_button).finish())
                        .with_margin_left(4.)
                        .finish(),
                )
                .on_left_mouse_up(|_, _, _| DispatchEventResult::StopPropagation)
                .on_left_mouse_down(|_, _, _| DispatchEventResult::StopPropagation)
                .finish(),
            );
        }

        if FeatureFlag::DiscardPerFileAndAllChanges.is_enabled() {
            right_row.add_child(
                EventHandler::new(
                    Container::new(ChildView::new(&file.discard_button).finish())
                        .with_margin_left(4.)
                        .finish(),
                )
                .on_left_mouse_up(|_, _, _| DispatchEventResult::StopPropagation)
                .on_left_mouse_down(|_, _, _| DispatchEventResult::StopPropagation)
                .finish(),
            );
        }

        right_row.add_child(
            EventHandler::new(
                Container::new(ChildView::new(&file.open_in_tab_button).finish())
                    .with_margin_left(4.)
                    .finish(),
            )
            .on_left_mouse_up(|_, _, _| DispatchEventResult::StopPropagation)
            .on_left_mouse_down(|_, _, _| DispatchEventResult::StopPropagation)
            .finish(),
        );

        let right_section = right_row.finish();

        let file_path_for_toggle = file.file_diff.file_path.clone();

        let outer_bg = theme.background();
        let inner_corner_radius = if file.is_expanded {
            CornerRadius::with_top(Radius::Pixels(8.))
        } else {
            CornerRadius::with_all(Radius::Pixels(8.))
        };

        let inner_header = Hoverable::new(file.header_mouse_state.clone(), |mouse_state| {
            let header_bg = if mouse_state.is_hovered() {
                neutral_3(appearance.theme())
            } else {
                neutral_2(appearance.theme())
            };
            Container::new(
                Clipped::new(
                    Shrinkable::new(
                        1.,
                        Flex::row()
                            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                            .with_main_axis_size(MainAxisSize::Max)
                            .with_cross_axis_alignment(CrossAxisAlignment::Center)
                            .with_child(Shrinkable::new(1., left_section.finish()).finish())
                            .with_child(right_section)
                            .finish(),
                    )
                    .finish(),
                )
                .finish(),
            )
            .with_background(header_bg)
            .with_vertical_padding(8.)
            .with_horizontal_padding(16.)
            .with_corner_radius(inner_corner_radius)
            .with_border(
                Border::new(1.)
                    .with_sides(
                        true,  /* top */
                        true,  /* left */
                        false, /* bottom */
                        true,  /* right */
                    )
                    .with_border_fill(theme.surface_3()),
            )
            .finish()
        })
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(CodeReviewAction::ToggleFileExpanded(
                file_path_for_toggle.clone(),
            ))
        })
        .with_defer_events_to_children()
        .finish();

        Container::new(inner_header)
            .with_background(outer_bg)
            .finish()
    }

    /// Renders file-specific statistics
    fn render_file_stats(&self, file: &FileDiff, appearance: &Appearance) -> Box<dyn Element> {
        let mut row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);

        let no_line_changes_present = file.additions() == 0 && file.deletions() == 0;
        if file.is_binary || no_line_changes_present {
            row.add_child(
                Container::new(
                    Text::new("0", appearance.ui_font_family(), appearance.ui_font_size())
                        .with_color(
                            appearance
                                .theme()
                                .disabled_text_color(appearance.theme().background())
                                .into(),
                        )
                        .finish(),
                )
                .finish(),
            )
        } else {
            row.add_children([
                Container::new(
                    Text::new(
                        format!("+{}", file.additions()),
                        appearance.ui_font_family(),
                        appearance.ui_font_size(),
                    )
                    .with_color(add_color(appearance))
                    .finish(),
                )
                .with_margin_right(4.)
                .finish(),
                Container::new(
                    Text::new("•", appearance.ui_font_family(), appearance.ui_font_size())
                        .with_style(Properties::default().weight(Weight::Bold))
                        .with_color(
                            warp_core::ui::theme::Fill::Solid(internal_colors::neutral_6(
                                appearance.theme(),
                            ))
                            .into(),
                        )
                        .finish(),
                )
                .with_margin_right(4.)
                .finish(),
                Container::new(
                    Text::new(
                        format!("-{}", file.deletions()),
                        appearance.ui_font_family(),
                        appearance.ui_font_size(),
                    )
                    .with_color(remove_color(appearance))
                    .finish(),
                )
                .with_margin_left(4.)
                .finish(),
            ]);
        }
        Container::new(row.finish())
            .with_background(appearance.theme().background())
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(2.)))
            .with_horizontal_padding(8.)
            .with_vertical_padding(4.)
            .with_border(
                Border::all(1.).with_border_fill(warp_core::ui::theme::Fill::Solid(
                    internal_colors::neutral_4(appearance.theme()),
                )),
            )
            .finish()
    }

    fn styled_file_content_container(
        content: Box<dyn Element>,
        theme: &WarpTheme,
    ) -> Box<dyn Element> {
        Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_child(Shrinkable::new(1., content).finish())
                .finish(),
        )
        .with_corner_radius(CornerRadius::with_bottom(Radius::Pixels(8.)))
        .with_background(theme.background())
        .with_vertical_padding(8.)
        .with_horizontal_padding(16.)
        .with_border(
            Border::new(1.)
                .with_sides(
                    false, /* top */
                    true,  /* left */
                    true,  /* bottom */
                    true,  /* right */
                )
                .with_border_fill(theme.surface_3()),
        )
        .finish()
    }

    /// Renders the file content (hunks for text files using LocalCodeEditorView, placeholder for binary)
    fn render_file_content(&self, file: &FileState, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();

        let diff_size = file.file_diff.size;
        if diff_size == DiffSize::Unrenderable {
            return Self::styled_file_content_container(
                Text::new(
                    "Diff is too large to render",
                    appearance.monospace_font_family(),
                    appearance.monospace_font_size(),
                )
                .with_color(remove_color(appearance))
                .finish(),
                theme,
            );
        }

        if file.file_diff.is_binary {
            Self::styled_file_content_container(
                Text::new(
                    "Binary file - no diff available",
                    appearance.monospace_font_family(),
                    appearance.monospace_font_size(),
                )
                .with_color(theme.main_text_color(theme.background()).into())
                .finish(),
                theme,
            )
        } else if file.file_diff.status.is_renamed() && file.file_diff.is_empty() {
            Self::styled_file_content_container(
                Text::new(
                    "File renamed without changes",
                    appearance.monospace_font_family(),
                    appearance.monospace_font_size(),
                )
                .with_color(theme.main_text_color(theme.background()).into())
                .finish(),
                theme,
            )
        } else if file.file_diff.status.is_new_file() && file.file_diff.is_empty() {
            Self::styled_file_content_container(
                Text::new(
                    "New empty file",
                    appearance.monospace_font_family(),
                    appearance.monospace_font_size(),
                )
                .with_color(theme.main_text_color(theme.background()).into())
                .finish(),
                theme,
            )
        } else if let Some(editor_state) = file.editor_state.as_ref() {
            Hoverable::new(editor_state.editor_mouse_state.clone(), |_| {
                Container::new(ChildView::new(&editor_state.editor).finish())
                    .with_corner_radius(CornerRadius::with_bottom(Radius::Pixels(8.)))
                    .with_background(theme.background())
                    .with_border(
                        Border::new(1.)
                            .with_sides(
                                false, /* top */
                                true,  /* left */
                                true,  /* bottom */
                                true,  /* right */
                            )
                            .with_border_fill(theme.surface_3()),
                    )
                    .finish()
            })
            .with_cursor(Cursor::IBeam)
            .finish()
        } else {
            Self::styled_file_content_container(
                Text::new(
                    "Unable to load file content",
                    appearance.ui_font_family(),
                    appearance.ui_font_size(),
                )
                .with_color(theme.sub_text_color(theme.background()).into())
                .finish(),
                theme,
            )
        }
    }

    fn revert_hunk_toast_id(&self, ctx: &mut ViewContext<Self>) -> String {
        format!("diff_removed_{}", ctx.view_id())
    }

    #[cfg(feature = "local_fs")]
    fn attach_diff_not_allowed_toast_id(&self, ctx: &mut ViewContext<Self>) -> String {
        format!("attach_diff_not_allowed_{}", ctx.view_id())
    }

    fn attach_context_not_allowed_toast_id(&self, ctx: &mut ViewContext<Self>) -> String {
        format!("attach_context_not_allowed_{}", ctx.view_id())
    }

    fn render_stats_fallback(appearance: &Appearance) -> Box<dyn Element> {
        Container::new(
            Text::new("0", appearance.ui_font_family(), appearance.ui_font_size())
                .with_color(
                    appearance
                        .theme()
                        .disabled_text_color(appearance.theme().background())
                        .into(),
                )
                .finish(),
        )
        .finish()
    }

    fn get_file_stats(
        file_path: &PathBuf,
        loaded: &LoadedState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        if let Some(file_state) = loaded.file_states.get(file_path) {
            let additions = file_state.file_diff.additions();
            let deletions = file_state.file_diff.deletions();
            let file_diff_stats = DiffStats {
                files_changed: 1,
                total_additions: additions,
                total_deletions: deletions,
            };
            Self::render_additions_and_deletions(&file_diff_stats, appearance, 1.).finish()
        } else {
            Self::render_stats_fallback(appearance)
        }
    }

    /// If checkbox_element is Some, it will be rendered on the left
    fn render_single_file_row(
        file_path: &Path,
        stats: Box<dyn Element>,
        checkbox_element: Option<Box<dyn Element>>,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let file_path_element = render_file_search_row(
            file_path,
            FileSearchRowOptions {
                ..Default::default()
            },
            app,
        );

        let mut left = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
        if let Some(checkbox) = checkbox_element {
            left.add_child(checkbox);
        }
        left.add_child(Shrinkable::new(1., file_path_element).finish());

        Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(Shrinkable::new(1., left.finish()).finish())
            .with_child(stats)
            .finish()
    }

    fn render_file_row(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        if self.discard_dialog_state.discard_file_paths.is_empty() {
            return Text::new(
                "No file selected",
                appearance.ui_font_family(),
                appearance.ui_font_size(),
            )
            .with_color(
                appearance
                    .theme()
                    .sub_text_color(appearance.theme().background())
                    .into(),
            )
            .finish();
        }

        let CodeReviewViewState::Loaded(loaded) = self.state() else {
            return Text::new(
                "No files to discard",
                appearance.ui_font_family(),
                appearance.ui_font_size(),
            )
            .with_color(
                appearance
                    .theme()
                    .sub_text_color(appearance.theme().background())
                    .into(),
            )
            .finish();
        };

        let is_discard_all = matches!(
            self.discard_dialog_state.operation_type,
            DiscardOperationType::AllUncommittedChanges
                | DiscardOperationType::AllChangesAgainstBranch(_)
        );

        if is_discard_all {
            // Removing all files: show list of all files with individual stats and checkboxes
            let mut file_list = Flex::column();

            for file_path in &self.discard_dialog_state.discard_file_paths {
                let file_stats = Self::get_file_stats(file_path, loaded, appearance);
                let is_selected = self
                    .discard_dialog_state
                    .selected_files
                    .get(file_path)
                    .copied()
                    .unwrap_or(true);
                let mouse_state = self
                    .discard_dialog_state
                    .file_checkbox_mouse_states
                    .get(file_path)
                    .cloned()
                    .unwrap_or_default();
                let file_path_for_closure = file_path.clone();

                let checkbox = appearance
                    .ui_builder()
                    .checkbox(mouse_state, Some(10.5))
                    .check(is_selected)
                    .build()
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(CodeReviewAction::ToggleFileSelection(
                            file_path_for_closure.clone(),
                        ));
                    })
                    .finish();

                let row = Self::render_single_file_row(file_path, file_stats, Some(checkbox), app);
                file_list.add_child(row);
            }

            file_list.finish()
        } else {
            // Removing single file: show that file with individual stats (no checkbox)
            let file_path = &self.discard_dialog_state.discard_file_paths[0];
            let file_stats = Self::get_file_stats(file_path, loaded, appearance);
            Self::render_single_file_row(file_path, file_stats, None, app)
        }
    }

    fn render_discard_confirm_dialog(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        let discard_button = Container::new(
            ChildView::new(&self.discard_dialog_state.discard_confirm_button).finish(),
        )
        .with_margin_left(8.)
        .finish();
        let cancel_button =
            ChildView::new(&self.discard_dialog_state.discard_cancel_button).finish();

        let file_row_content = self.render_file_row(app);
        let scrollable_file_row = NewScrollable::vertical(
            SingleAxisConfig::Clipped {
                handle: self.discard_dialog_state.file_list_scroll_state.clone(),
                child: file_row_content,
            },
            appearance.theme().nonactive_ui_detail().into(),
            appearance.theme().active_ui_detail().into(),
            warpui::elements::Fill::None,
        )
        .with_vertical_scrollbar(ScrollableAppearance::new(ScrollbarWidth::Auto, false))
        .finish();
        let file_row = ConstrainedBox::new(scrollable_file_row)
            .with_max_height(200.0)
            .finish();

        let (title, description) = (
            self.discard_dialog_state.operation_type.title(),
            self.discard_dialog_state.operation_type.description(),
        );

        let mut dialog_builder = Dialog::new(
            title,
            description,
            UiComponentStyles {
                width: Some(460.),
                padding: Some(Coords::uniform(24.).bottom(12.)),
                ..dialog_styles(appearance)
            },
        )
        .with_child(Container::new(file_row).finish())
        .with_separator();

        // hide stash option entirely if there's no HEAD (git doesn't let you stash with no HEAD)
        let can_stash = self.diff_state_model.as_ref(app).has_head(app);

        if self
            .discard_dialog_state
            .operation_type
            .is_uncommitted_changes()
            && can_stash
        {
            let stash_checkbox = Container::new(
                appearance
                    .ui_builder()
                    .checkbox(
                        self.discard_dialog_state
                            .stash_changes_checkbox_mouse_state
                            .clone(),
                        Some(16.),
                    )
                    .check(self.discard_dialog_state.stash_changes_enabled)
                    .with_label(
                        appearance.ui_builder().span("Stash changes").with_style(
                            UiComponentStyles {
                                font_size: Some(appearance.ui_font_size()),
                                font_color: Some(
                                    appearance
                                        .theme()
                                        .main_text_color(appearance.theme().background())
                                        .into(),
                                ),
                                ..Default::default()
                            },
                        ),
                    )
                    .build()
                    .on_click(|ctx, _, _| {
                        ctx.dispatch_typed_action(CodeReviewAction::ToggleStashChanges);
                    })
                    .finish(),
            )
            .finish();
            dialog_builder = dialog_builder.with_bottom_row_left_child(stash_checkbox);
        }

        let dialog = Container::new(
            dialog_builder
                .with_bottom_row_child(cancel_button)
                .with_bottom_row_child(discard_button)
                .build()
                .finish(),
        )
        .with_margin_top(35.)
        .finish();

        // Stack needed so that dialog can get bounds information,
        // specifically to ensure no overlap with the window's traffic lights
        let mut stack = Stack::new();
        stack.add_positioned_child(
            dialog,
            OffsetPositioning::offset_from_parent(
                vec2f(0., 0.),
                ParentOffsetBounds::WindowByPosition,
                ParentAnchor::Center,
                ChildAnchor::Center,
            ),
        );

        // This blurs the background and makes it uninteractable
        Container::new(Align::new(stack.finish()).finish())
            .with_background_color(appearance.theme().blurred_background_overlay().into())
            .with_corner_radius(app.windows().window_corner_radius())
            .finish()
    }

    fn create_file_status_info(&self, path: StandardizedPath) -> FileStatusInfo {
        let Some(local_path) = path.to_local_path() else {
            return FileStatusInfo {
                path,
                status: GitFileStatus::Modified,
            };
        };
        let status = match self.state() {
            CodeReviewViewState::Loaded(loaded_state) => loaded_state
                .file_states
                .get(&local_path)
                .map(|fs| fs.file_diff.status.clone())
                .unwrap_or(GitFileStatus::Modified),
            _ => GitFileStatus::Modified,
        };
        FileStatusInfo { path, status }
    }

    fn discard_file(
        &mut self,
        path: StandardizedPath,
        should_stash: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        let file_info = self.create_file_status_info(path);

        let branch_name = match &self.discard_dialog_state.operation_type {
            DiscardOperationType::FileChangesAgainstBranch(None) => {
                Some(self.diff_state_model.as_ref(ctx).get_main_branch_name(ctx))
            }
            DiscardOperationType::FileChangesAgainstBranch(Some(branch)) => {
                Some(Some(branch.clone()))
            }
            _ => None,
        };
        self.diff_state_model.update(ctx, |model, ctx| {
            model.discard_files(vec![file_info], should_stash, branch_name.flatten(), ctx);
        });
        self.discard_dialog_state.stash_changes_enabled = false;
    }

    fn discard_multiple_files(
        &mut self,
        file_paths: Vec<StandardizedPath>,
        should_stash: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        let file_infos: Vec<FileStatusInfo> = file_paths
            .into_iter()
            .map(|path| self.create_file_status_info(path))
            .collect();

        let branch_name = match &self.discard_dialog_state.operation_type {
            DiscardOperationType::AllChangesAgainstBranch(None) => {
                Some(self.diff_state_model.as_ref(ctx).get_main_branch_name(ctx))
            }
            DiscardOperationType::AllChangesAgainstBranch(Some(branch)) => {
                Some(Some(branch.clone()))
            }
            _ => None,
        };
        self.diff_state_model.update(ctx, |model, ctx| {
            model.discard_files(file_infos, should_stash, branch_name.flatten(), ctx);
        });
        self.discard_dialog_state.stash_changes_enabled = false;
    }

    fn handle_code_editor_event(
        &mut self,
        file_path: PathBuf,
        editor: ViewHandle<CodeEditorView>,
        event: &CodeEditorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            CodeEditorEvent::DiffHunkContextAdded { line_range } => {
                self.insert_diff_hunk_as_context(file_path, line_range.clone(), ctx);
            }
            CodeEditorEvent::DiffReverted => {
                // Show toast notification that diff was removed.
                let version = editor.as_ref(ctx).version(ctx);
                self.last_revert = Some((editor, version));

                let toast_id = self.revert_hunk_toast_id(ctx);
                crate::workspace::ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                    let toast = crate::view_components::DismissibleToast::default(
                        "Diff removed".to_string(),
                    )
                    .with_object_id(toast_id)
                    .with_action_button(self.undo_action_button.clone());
                    toast_stack.add_ephemeral_toast(toast, self.window_id, ctx);
                });

                // Focus the view in case the focus was in a different pane before clicking the revert button.
                // Otherwise, the undo and save actions will not work.
                ctx.focus_self();
            }
            CodeEditorEvent::HiddenSectionExpanded => {
                if let CodeReviewViewState::Loaded(LoadedState { file_states, .. }) = self.state() {
                    if let Some(index) = file_states.get_index_of(&file_path) {
                        self.viewported_list_state
                            .invalidate_height_for_index(index);
                        ctx.notify();
                    }
                }
            }
            CodeEditorEvent::Focused => {
                ctx.emit(CodeReviewViewEvent::Pane(PaneEvent::FocusSelf));
            }
            CodeEditorEvent::ContentChanged { origin, .. } => {
                if origin.from_user() {
                    ctx.emit(CodeReviewViewEvent::FileEdited { path: file_path });

                    if let Some((view_handle, content_version)) = self.last_revert.take() {
                        let same_content_version =
                            content_version == editor.as_ref(ctx).version(ctx);

                        // If the revert was for a different editor or the content version is the same, keep the revert.
                        if view_handle.id() != editor.id() || same_content_version {
                            self.last_revert = Some((view_handle, content_version));
                        } else {
                            self.last_revert = None;
                            self.dismiss_revert_toast(ctx);
                        }
                    }

                    if self.find_model.as_ref(ctx).is_find_bar_open()
                        && FeatureFlag::CodeReviewFind.is_enabled()
                    {
                        self.find_model.update(ctx, |model, model_ctx| {
                            model.run_search(self.editor_handles(), model_ctx);
                        });
                    }
                }
            }
            _ => {}
        }
    }

    fn insert_selection_as_context(
        &mut self,
        file_path: String,
        start_line: usize,
        end_line: usize,
        selected_text: String,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(terminal_view) = self.terminal_view.as_ref().and_then(|tv| tv.upgrade(ctx)) {
            // If a CLI agent is active, send appropriate content to the PTY.
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
                    CliAgentRouting::RichInput => CodeReviewContextDestination::RichInput,
                    CliAgentRouting::Pty => CodeReviewContextDestination::Pty,
                };
                send_telemetry_from_ctx!(
                    CodeReviewTelemetryEvent::AddToContext {
                        origin: AddToContextOrigin::SelectedText,
                        destination,
                        diff_set_scope: None,
                    },
                    ctx
                );
                return;
            }

            let is_long_running =
                terminal_view.read(ctx, |terminal_view, _| terminal_view.is_long_running());

            if is_long_running {
                let toast_id = self.attach_context_not_allowed_toast_id(ctx);
                crate::workspace::ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                    let toast = crate::view_components::DismissibleToast::default(
                        "Cannot attach context when terminal is running".to_string(),
                    )
                    .with_object_id(toast_id);
                    toast_stack.add_ephemeral_toast(toast, self.window_id, ctx);
                });
                return;
            }

            // Otherwise insert the location snippet into the input buffer (original behavior).
            let location = format!("{file_path}:{start_line}-{end_line} ");
            send_telemetry_from_ctx!(
                CodeReviewTelemetryEvent::AddToContext {
                    origin: AddToContextOrigin::SelectedText,
                    destination: CodeReviewContextDestination::AgentInput,
                    diff_set_scope: None,
                },
                ctx
            );
            terminal_view.update(ctx, |terminal_view, ctx| {
                terminal_view.input().update(ctx, |input, ctx| {
                    input.append_to_buffer(&location, ctx);
                    // Ensure agent mode for AI features
                    input.ensure_agent_mode_for_ai_features(true, ctx);
                });
            });
        }
    }

    fn maybe_undo_revert(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some((editor, _)) = self.last_revert.take() {
            editor.update(ctx, |editor, ctx| {
                editor.undo(ctx);
            });

            self.dismiss_revert_toast(ctx);
        }
    }

    fn dismiss_revert_toast(&mut self, ctx: &mut ViewContext<Self>) {
        let window_id = ctx.window_id();
        let revert_hunk_toast_id = self.revert_hunk_toast_id(ctx);

        ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
            toast_stack.remove_toast_by_identifier(revert_hunk_toast_id, window_id, ctx);
        });
    }

    pub fn has_unsaved_changes(&self, ctx: &AppContext) -> bool {
        !self.get_unsaved_file_paths(ctx).is_empty()
    }

    /// Insert diff set as context in the terminal input (either all files or a specific file)
    #[cfg(feature = "local_fs")]
    fn insert_diff_as_context(&mut self, scope: DiffSetScope, ctx: &mut ViewContext<Self>) {
        let Some(repo_path) = self.repo_path() else {
            return;
        };
        if let Some(terminal_view) = self
            .terminal_view
            .as_ref()
            .and_then(|view| view.upgrade(ctx))
        {
            let active_cli_agent = terminal_view.read(ctx, |tv, ctx| tv.active_cli_agent(ctx));

            let diff_set_scope = match &scope {
                DiffSetScope::All => DiffSetContextScope::All,
                DiffSetScope::File(_) => DiffSetContextScope::File,
            };
            // CLI agent path: write per-file hunk ranges to the PTY (or rich input if open).
            if active_cli_agent.is_some() {
                if let CodeReviewViewState::Loaded(state) = self.state() {
                    let files_to_process = match &scope {
                        DiffSetScope::All => state
                            .file_states
                            .values()
                            .map(|fs| &fs.file_diff)
                            .collect_vec(),
                        DiffSetScope::File(target_path) => state
                            .file_states
                            .values()
                            .filter(|fs| fs.file_diff.file_path == *target_path)
                            .map(|fs| &fs.file_diff)
                            .collect_vec(),
                    };
                    let file_diffs =
                        convert_file_diffs_to_diffset_hunks(files_to_process.into_iter());
                    let routing = terminal_view.update(ctx, |tv, ctx| {
                        tv.send_diff_context_to_cli_agent_or_rich_input(&file_diffs, ctx)
                    });
                    let destination = match routing {
                        Some(CliAgentRouting::RichInput) => CodeReviewContextDestination::RichInput,
                        _ => CodeReviewContextDestination::Pty,
                    };
                    send_telemetry_from_ctx!(
                        CodeReviewTelemetryEvent::AddToContext {
                            origin: AddToContextOrigin::CodeReviewHeader,
                            destination,
                            diff_set_scope: Some(diff_set_scope),
                        },
                        ctx
                    );
                }
                return;
            }

            let is_input_box_visible = terminal_view.read(ctx, |terminal_view, _| {
                terminal_view.is_input_box_visible(&terminal_view.model.lock(), ctx)
            });

            if !is_input_box_visible {
                let toast_id = self.attach_diff_not_allowed_toast_id(ctx);
                ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                    let toast = DismissibleToast::default(
                        "Cannot attach diff while input is not available".to_string(),
                    )
                    .with_object_id(toast_id);
                    toast_stack.add_ephemeral_toast(toast, self.window_id, ctx);
                });
                return;
            }

            if let CodeReviewViewState::Loaded(state) = self.state() {
                // Filter files based on scope
                let files_to_process = match &scope {
                    DiffSetScope::All => state
                        .file_states
                        .values()
                        .map(|fs| &fs.file_diff)
                        .collect_vec(),
                    DiffSetScope::File(target_path) => state
                        .file_states
                        .get(target_path)
                        .into_iter()
                        .map(|fs| &fs.file_diff)
                        .collect_vec(),
                };

                if files_to_process.is_empty() {
                    if let DiffSetScope::File(path) = &scope {
                        log::warn!("Could not find file state for path: {}", path.display());
                    }
                    return;
                }

                // Use the shared function to convert diff data with relative paths
                let file_diffs = convert_file_diffs_to_diffset_hunks(files_to_process.into_iter());

                let base = match self.get_diff_base(ctx) {
                    Ok(base) => base,
                    Err(err) => {
                        log::error!(
                            "CodeReviewView could not find diff base when attaching diff as context: {err:?}"
                        );
                        return;
                    }
                };

                // Create attachment reference and key based on scope
                let main_branch_name = self.diff_state_model.as_ref(ctx).get_main_branch_name(ctx);
                let (attachment_reference, attachment_key) = create_attachment_reference_and_key(
                    &scope,
                    &self.diff_state_model.as_ref(ctx).diff_mode(ctx),
                    main_branch_name.as_deref(),
                    repo_path,
                );

                // Insert the reference into the terminal input
                terminal_view.update(ctx, |terminal_view, ctx| {
                    terminal_view.input().update(ctx, |input, ctx| {
                        input.append_to_buffer(&format!("{attachment_reference} "), ctx);
                        input.ensure_agent_mode_for_ai_features(true, ctx);
                    });
                });

                send_telemetry_from_ctx!(
                    CodeReviewTelemetryEvent::AddToContext {
                        origin: AddToContextOrigin::CodeReviewHeader,
                        destination: CodeReviewContextDestination::AgentAttachment,
                        diff_set_scope: Some(diff_set_scope),
                    },
                    ctx
                );

                // Register the DiffSet attachment in the terminal view's AI context model.
                let current = self.get_current_head(ctx);
                terminal_view.update(ctx, |terminal_view, ctx| {
                    register_diffset_attachment(
                        terminal_view.ai_context_model(),
                        attachment_key,
                        file_diffs,
                        current,
                        base,
                        ctx,
                    );

                    // Enter agent view if enabled and not already active
                    if FeatureFlag::AgentView.is_enabled()
                        && !terminal_view
                            .agent_view_controller()
                            .as_ref(ctx)
                            .is_active()
                    {
                        terminal_view.enter_agent_view_for_new_conversation(
                            None,
                            AgentViewEntryOrigin::CodeReviewContext,
                            ctx,
                        );
                    }
                });
            }
        }
    }

    #[cfg(not(feature = "local_fs"))]
    fn insert_diff_as_context(&mut self, _scope: DiffSetScope, _ctx: &mut ViewContext<Self>) {
        log::error!("insert_diff_as_context is not supported without the local_fs feature");
    }

    fn get_current_head(&self, ctx: &ViewContext<Self>) -> Option<CurrentHead> {
        self.diff_state_model
            .as_ref(ctx)
            .get_current_branch_name(ctx)
            .map(CurrentHead::BranchName)
    }

    fn get_diff_base(&self, ctx: &ViewContext<Self>) -> anyhow::Result<DiffBase> {
        match self.diff_state_model.as_ref(ctx).diff_mode(ctx) {
            DiffMode::Head => Ok(DiffBase::UncommittedChanges),
            DiffMode::MainBranch => {
                let main_branch_name = self.diff_state_model.as_ref(ctx).get_main_branch_name(ctx);
                match main_branch_name {
                    Some(name) => Ok(DiffBase::BranchName(name)),
                    None => Err(anyhow::anyhow!("unable to determine main branch name")),
                }
            }
            DiffMode::OtherBranch(branch_name) => Ok(DiffBase::BranchName(branch_name)),
        }
    }

    /// Configures the code review view to display and scroll to a specific imported comment.
    /// Sets the diff base, expands the comment list, and queues a jump to the comment location.
    pub(crate) fn navigate_to_imported_comment(
        &mut self,
        comment_id: CommentId,
        diff_mode: DiffMode,
        ctx: &mut ViewContext<Self>,
    ) {
        self.set_diff_base(diff_mode, ctx);
        self.expand_comment_list_and_scroll_to_comment(comment_id, ctx);
        self.pending_jump_to_comment = Some(comment_id);
    }

    pub(crate) fn set_diff_base(&mut self, diff_mode: DiffMode, ctx: &mut ViewContext<Self>) {
        self.diff_state_model.update(ctx, |diff_state_model, ctx| {
            diff_state_model.set_diff_mode_and_fetch_base(diff_mode, ctx);
        });
        self.update_diff_selector_selection(ctx);
        self.invalidate_all(None, ctx);
    }

    /// Insert diff hunk as an inline attachment in the terminal input
    fn insert_diff_hunk_as_context(
        &mut self,
        file_path: PathBuf,
        line_range: Range<warp_editor::render::model::LineCount>,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(repo_path) = self.repo_path() else {
            return;
        };
        // Try to get the terminal view and insert the context
        if let Some(terminal_view) = self.terminal_view.as_ref().and_then(|tv| tv.upgrade(ctx)) {
            let is_long_running =
                terminal_view.read(ctx, |terminal_view, _| terminal_view.is_long_running());
            let active_cli_agent = terminal_view.read(ctx, |tv, ctx| tv.active_cli_agent(ctx));

            let relative_path = if file_path.is_absolute() {
                file_path
                    .strip_prefix(repo_path)
                    .unwrap_or(&file_path)
                    .to_path_buf()
            } else {
                file_path.clone()
            };

            // Case 1: CLI agent — send location + change stats to PTY or rich input
            if active_cli_agent.is_some() {
                if let Some((_, lines_added, lines_removed)) =
                    self.extract_diff_hunk_data(&relative_path, &line_range)
                {
                    // Use relative_path so the prompt shows repo-relative paths (e.g. src/foo.rs)
                    // rather than absolute machine-specific paths.
                    let start_line = line_range.start.as_usize() + 1;
                    let end_line = line_range.end.as_usize();
                    let routing = terminal_view.update(ctx, |tv, ctx| {
                        tv.send_diff_hunk_to_cli_agent_or_rich_input(
                            &relative_path,
                            start_line,
                            end_line,
                            lines_added,
                            lines_removed,
                            ctx,
                        )
                    });
                    let destination = match routing {
                        Some(CliAgentRouting::RichInput) => CodeReviewContextDestination::RichInput,
                        _ => CodeReviewContextDestination::Pty,
                    };
                    send_telemetry_from_ctx!(
                        CodeReviewTelemetryEvent::AddToContext {
                            origin: AddToContextOrigin::Gutter,
                            destination,
                            diff_set_scope: None,
                        },
                        ctx
                    );
                }
                return;
            }

            // Case 2: Generic long-running (non-CLI-agent) — insert file path + line range as text
            if is_long_running {
                // When a command is running, just insert the file path and line range as text
                // (similar to file tree drag/drop behavior)
                let full_path = repo_path.join(&file_path);
                let start_line = line_range.start.as_usize() + 1;
                let end_line = line_range.end.as_usize();
                let path_with_range = format!("{}:{start_line}-{end_line} ", full_path.display());
                terminal_view.update(ctx, |terminal_view, ctx| {
                    terminal_view.handle_file_tree_drop_on_active_command(&path_with_range, ctx);
                });
                send_telemetry_from_ctx!(
                    CodeReviewTelemetryEvent::AddToContext {
                        origin: AddToContextOrigin::Gutter,
                        destination: CodeReviewContextDestination::ActiveCommandBuffer,
                        diff_set_scope: None,
                    },
                    ctx
                );
                return;
            }
            if let Some((hunk, lines_added, lines_removed)) =
                self.extract_diff_hunk_data(&relative_path, &line_range)
            {
                // Create a descriptive key using filename and line range
                let filename = relative_path.display().to_string();
                // Use 1-indexed, inclusive line numbers for the user visible range.

                let diff_hunk_key =
                    format!("{filename}:{}-{}", line_range.start + 1, line_range.end);

                let attachment_reference = format!("<change:{diff_hunk_key}>",);

                // Insert the reference into the terminal input and lock into agent mode
                terminal_view.update(ctx, |terminal_view, ctx| {
                    terminal_view.input().update(ctx, |input, ctx| {
                        input.append_to_buffer(&format!("{attachment_reference} "), ctx);
                        input.ensure_agent_mode_for_ai_features(true, ctx);
                    });
                });

                // Convert the diff hunk to a formatted diff string
                let diff_content = self.format_diff_hunk_content(&hunk);

                // Determine the diff base from the current diff state
                let diff_base = match self
                    .diff_state_model
                    .read(ctx, |model, ctx| model.diff_mode(ctx))
                {
                    DiffMode::Head => DiffBase::UncommittedChanges,
                    DiffMode::MainBranch => {
                        let main_branch_name = self
                            .diff_state_model
                            .read(ctx, |model, ctx| model.get_main_branch_name(ctx));

                        match main_branch_name {
                            Some(name) => DiffBase::BranchName(name),
                            None => {
                                log::warn!(
                                    "Unable to determine main branch name when inserting diff hunk context."
                                );
                                return;
                            }
                        }
                    }
                    DiffMode::OtherBranch(branch_name) => DiffBase::BranchName(branch_name),
                };

                send_telemetry_from_ctx!(
                    CodeReviewTelemetryEvent::AddToContext {
                        origin: AddToContextOrigin::Gutter,
                        destination: CodeReviewContextDestination::AgentAttachment,
                        diff_set_scope: None,
                    },
                    ctx
                );
                // Create the DiffHunk attachment
                let attachment = AIAgentAttachment::DiffHunk {
                    file_path: filename.clone(),
                    line_range: line_range.clone(),
                    diff_content,
                    lines_added,
                    lines_removed,
                    current: None, // We don't have current branch info here
                    base: diff_base,
                };

                // Register the attachment with the terminal's AI controller using the new key format
                terminal_view.update(ctx, |terminal_view, ctx| {
                    terminal_view
                        .ai_context_model()
                        .update(ctx, |context_model, _| {
                            context_model.register_diff_hunk_attachment(diff_hunk_key, attachment);
                        });

                    // Enter agent view if enabled and not already active
                    if FeatureFlag::AgentView.is_enabled()
                        && !terminal_view
                            .agent_view_controller()
                            .as_ref(ctx)
                            .is_active()
                    {
                        terminal_view.enter_agent_view_for_new_conversation(
                            None,
                            AgentViewEntryOrigin::CodeReviewContext,
                            ctx,
                        );
                    }
                });
            }
        }
    }

    /// Extract diff hunk data for the given file and line range
    fn extract_diff_hunk_data(
        &self,
        file_path: &PathBuf,
        line_range: &Range<warp_editor::render::model::LineCount>,
    ) -> Option<(DiffHunk, u32, u32)> {
        if let CodeReviewViewState::Loaded(state) = self.state() {
            // Find the file state that matches the given file path
            let file_state = state.file_states.get(file_path)?;

            let file_diff = &file_state.file_diff;

            // Convert editor line range to 1-indexed, exclusive line numbers
            let requested_start = line_range.start.as_usize() + 1;
            let requested_end = line_range.end.as_usize() + 1;

            // Find the diff hunk that contains this line range
            for hunk in file_diff.hunks.iter() {
                // Check if this hunk overlaps with the requested line range
                let hunk_start = hunk.new_start_line;
                let hunk_end = hunk_start + hunk.lines.len();

                if requested_start <= hunk_end && requested_end >= hunk_start {
                    // Filter the hunk lines to only include those within the requested range
                    let mut filtered_lines = Vec::new();
                    let mut current_line = hunk.new_start_line;
                    let mut lines_added = 0u32;
                    let mut lines_removed = 0u32;

                    for line in &hunk.lines {
                        // For additions and context lines, check if they're in the requested range
                        let include_line = match line.line_type {
                            DiffLineType::Add | DiffLineType::Context => {
                                current_line >= requested_start && current_line < requested_end
                            }
                            DiffLineType::Delete => {
                                // Include deletions if they're relevant to the range.
                                // CODE-1638: Deletion hunks are anchored to the line after the removed line,
                                // so allow one extra line past requested_end.
                                current_line >= requested_start && current_line <= requested_end
                            }
                            DiffLineType::HunkHeader => false,
                        };

                        if include_line {
                            filtered_lines.push(line.clone());
                            match line.line_type {
                                DiffLineType::Add => lines_added += 1,
                                DiffLineType::Delete => lines_removed += 1,
                                _ => {}
                            }
                        }

                        // Advance line counter for non-deletion lines
                        if !matches!(line.line_type, DiffLineType::Delete) {
                            current_line += 1;
                        }
                    }

                    // Create a filtered hunk with only the relevant lines
                    let filtered_hunk = DiffHunk {
                        old_start_line: hunk.old_start_line,
                        old_line_count: hunk.old_line_count,
                        new_start_line: requested_start,
                        new_line_count: filtered_lines.len(),
                        lines: filtered_lines,
                        unified_diff_start: hunk.unified_diff_start,
                        unified_diff_end: hunk.unified_diff_end,
                    };

                    return Some((filtered_hunk, lines_added, lines_removed));
                }
            }
        }
        None
    }

    /// Format a diff hunk into a standard diff format string
    fn format_diff_hunk_content(&self, hunk: &DiffHunk) -> String {
        let mut diff_lines = Vec::new();

        for line in &hunk.lines {
            match line.line_type {
                DiffLineType::Add => diff_lines.push(format!("+{}", line.text)),
                DiffLineType::Delete => diff_lines.push(format!("-{}", line.text)),
                DiffLineType::Context => diff_lines.push(line.text.clone()),
                DiffLineType::HunkHeader => continue,
            }
        }

        diff_lines.join("\n")
    }

    fn save_files(&mut self, paths: &[PathBuf], ctx: &mut ViewContext<Self>) {
        for path in paths {
            self.save_file(path, ctx);
        }
    }

    fn save_file(&mut self, path: &PathBuf, ctx: &mut ViewContext<CodeReviewView>) {
        if let CodeReviewViewState::Loaded(state) = self.state() {
            if let Some(file_state) = state.file_states.get(path) {
                if let Some(editor) = file_state.editor_state.as_ref().map(|state| state.editor()) {
                    if let Err(err) =
                        editor.update(ctx, |local_editor, ctx| local_editor.save_local(ctx))
                    {
                        safe_error!(
                            safe: ("Failed to save file: {err}"),
                            full: ("Failed to save file {}: {err:?}", path.display())
                        );
                    }
                }
            }
        }
    }

    /// Captures the current cursor position and selections from the editor
    fn capture_cursor_position(editor: &CodeEditorView, app: &AppContext) -> Vec<SelectionOffsets> {
        editor
            .model
            .as_ref(app)
            .selections(app)
            .mapped(|selection| SelectionOffsets {
                head: selection.head,
                tail: selection.tail,
            })
            .into_vec()
    }

    /// Restores cursor position and selections to the editor
    fn restore_cursor_position(
        editor: &CodeEditorView,
        selections: Vec<SelectionOffsets>,
        ctx: &mut warpui::ViewContext<CodeEditorView>,
    ) {
        if let Ok(selections_vec1) = Vec1::try_from_vec(selections) {
            editor.model.update(ctx, |model, ctx| {
                model.vim_set_selections(
                    selections_vec1,
                    AutoScrollBehavior::None, // Don't auto-scroll when restoring position
                    ctx,
                );
            });
        }
    }

    /// Refreshes metadata and PR info after a git operation (commit, push, etc.).
    /// Diff reloading is left to the file watcher (for commits) or skipped
    /// entirely (for push/create-PR where the working directory is unchanged).
    fn refresh_after_git_operation(&mut self, ctx: &mut ViewContext<Self>) {
        self.diff_state_model.update(ctx, |model, ctx| {
            model.refresh_metadata_and_pr_info(ctx);
        });
        ctx.notify();
    }

    /// Returns whether the working tree has uncommitted changes.
    ///
    /// This reads the `against_head` metadata directly rather than the loaded
    /// view state, because the loaded state reflects whatever diff mode the
    /// user currently has selected (e.g. `MainBranch`). In non-`Head` modes
    /// committed changes still appear in the stats, so using them here would
    /// make the button stay on "Commit" after a successful commit.
    fn has_uncommitted_changes(&self, app: &AppContext) -> bool {
        self.diff_state_model
            .as_ref(app)
            .get_uncommitted_stats(app)
            .is_some_and(|stats| !stats.has_no_changes())
    }

    /// Opens a `GitDialog` overlay for the given `kind`. Centralizes the
    /// common guards (single-dialog invariant, git-ops blocked check, repo
    /// + branch lookup), the per-kind dialog construction, and the event
    /// subscription that clears `git_dialog` + refreshes repo state when the
    /// dialog completes. Each dialog mode handles its own success/failure
    /// toasts internally.
    fn open_git_dialog(&mut self, kind: GitDialogKind, ctx: &mut ViewContext<Self>) {
        if self.git_dialog.is_some() {
            return;
        }
        if self
            .diff_state_model
            .as_ref(ctx)
            .is_git_operation_blocked(ctx)
        {
            return;
        }
        let Some(repo_path) = self.repo_path().cloned() else {
            return;
        };
        let branch_name = self
            .diff_state_model
            .read(ctx, |model, ctx| model.get_current_branch_name(ctx))
            .unwrap_or_default();

        let dialog = match kind {
            GitDialogKind::Commit => {
                // Hide the "Commit and create PR" intent when it wouldn't make
                // sense: a PR already exists for this branch, or we're on the
                // repo's main branch (creating a PR from main is invalid).
                // `has_upstream` controls the label/icon on the push-chained
                // intent (Commit and push vs Commit and publish).
                let diff_state = self.diff_state_model.as_ref(ctx);
                let allow_create_pr = diff_state.pr_info(ctx).is_none()
                    && !diff_state.is_pr_info_refreshing(ctx)
                    && !diff_state.is_on_main_branch(ctx);
                let has_upstream = diff_state.upstream_ref(ctx).is_some();
                ctx.add_typed_action_view(|ctx| {
                    GitDialog::new_for_commit(
                        repo_path,
                        branch_name,
                        allow_create_pr,
                        has_upstream,
                        ctx,
                    )
                })
            }
            GitDialogKind::Push { publish } => {
                let commits = self
                    .diff_state_model
                    .read(ctx, |model, ctx| model.unpushed_commits(ctx).to_vec());
                ctx.add_typed_action_view(|ctx| {
                    GitDialog::new_for_push(repo_path, branch_name, publish, commits, ctx)
                })
            }
            GitDialogKind::CreatePr => {
                let base_branch_name = self
                    .diff_state_model
                    .read(ctx, |model, ctx| model.get_main_branch_name(ctx));
                ctx.add_typed_action_view(|ctx| {
                    GitDialog::new_for_pr(repo_path, branch_name, base_branch_name, ctx)
                })
            }
        };

        ctx.subscribe_to_view(&dialog, move |me, _, event, ctx| {
            match event {
                GitDialogEvent::Completed => {
                    me.git_dialog = None;
                    me.refresh_after_git_operation(ctx);
                }
                GitDialogEvent::Cancelled => {
                    me.git_dialog = None;
                }
            }
            ctx.notify();
        });
        self.git_dialog = Some(dialog.clone());
        ctx.focus(&dialog);
        ctx.notify();
    }

    /// Computes the current primary git action from diff stats and the diff state model.
    fn primary_git_action_mode(&self, app: &AppContext) -> PrimaryGitActionMode {
        let diff_state = self.diff_state_model.as_ref(app);
        let has_uncommitted_changes = self.has_uncommitted_changes(app);
        let has_upstream = diff_state.upstream_ref(app).is_some();
        let has_local_commits = !diff_state.unpushed_commits(app).is_empty();
        let is_pr_info_refreshing = diff_state.is_pr_info_refreshing(app);
        // False when upstream == main (e.g. after `git checkout -b feature origin/master`),
        // which means the branch hasn't been pushed to its own remote ref yet.
        let upstream_differs_from_main = diff_state.upstream_differs_from_main(app);

        if has_uncommitted_changes {
            PrimaryGitActionMode::Commit
        } else if !has_upstream && has_local_commits {
            PrimaryGitActionMode::Publish
        } else if has_local_commits {
            PrimaryGitActionMode::Push
        } else if diff_state.pr_info(app).is_some() {
            PrimaryGitActionMode::ViewPr
        } else if !is_pr_info_refreshing
            && has_upstream
            && !diff_state.is_on_main_branch(app)
            && upstream_differs_from_main
        {
            PrimaryGitActionMode::CreatePr
        } else {
            // Nothing actionable — show Commit disabled.
            PrimaryGitActionMode::Commit
        }
    }

    /// Updates the primary git operations button, chevron visibility, and
    /// related state to match the current [`PrimaryGitActionMode`].
    fn update_git_operations_ui(&mut self, ctx: &mut ViewContext<Self>) {
        let mode = self.primary_git_action_mode(ctx);

        match mode {
            PrimaryGitActionMode::Commit => {
                let disabled = !self.has_uncommitted_changes(ctx);
                self.git_primary_action_button.update(ctx, |button, ctx| {
                    button.set_label("Commit", ctx);
                    button.set_icon(Some(Icon::GitCommit), ctx);
                    button.set_disabled(disabled, ctx);
                    button.set_tooltip(disabled.then_some("No changes to commit"), ctx);
                    button.set_on_click(
                        |ctx| ctx.dispatch_typed_action(CodeReviewAction::OpenCommitDialog),
                        ctx,
                    );
                    button.set_adjoined_side(AdjoinedSide::Right, ctx);
                });
                self.git_operations_chevron.update(ctx, |button, ctx| {
                    button.set_disabled(disabled, ctx);
                    button.set_tooltip(disabled.then_some("No git actions available"), ctx);
                });
            }
            PrimaryGitActionMode::Push => {
                self.git_primary_action_button.update(ctx, |button, ctx| {
                    button.set_label("Push", ctx);
                    button.set_icon(Some(Icon::ArrowUp), ctx);
                    button.set_disabled(false, ctx);
                    button.clear_tooltip(ctx);
                    button.set_on_click(
                        |ctx| ctx.dispatch_typed_action(CodeReviewAction::OpenPushDialog),
                        ctx,
                    );
                    button.set_adjoined_side(AdjoinedSide::Right, ctx);
                });
                self.git_operations_chevron.update(ctx, |button, ctx| {
                    button.set_disabled(false, ctx);
                });
            }
            PrimaryGitActionMode::CreatePr => {
                self.git_primary_action_button.update(ctx, |button, ctx| {
                    button.set_label("Create PR", ctx);
                    button.set_icon(Some(Icon::Github), ctx);
                    button.set_disabled(false, ctx);
                    button.clear_tooltip(ctx);
                    button.set_on_click(
                        |ctx| ctx.dispatch_typed_action(CodeReviewAction::OpenCreatePrDialog),
                        ctx,
                    );
                    button.clear_adjoined_side(ctx);
                });
            }
            PrimaryGitActionMode::ViewPr => {
                let diff_state = self.diff_state_model.as_ref(ctx);
                let pr_info = diff_state.pr_info(ctx).cloned();
                let is_pr_info_refreshing = diff_state.is_pr_info_refreshing(ctx);
                if let Some(pr_info) = pr_info {
                    let url = pr_info.url.clone();
                    let number = pr_info.number;
                    let label = format!("PR #{number}");
                    self.git_primary_action_button.update(ctx, |button, ctx| {
                        button.set_label(label, ctx);
                        button.set_icon(Some(Icon::Github), ctx);
                        button.set_disabled(is_pr_info_refreshing, ctx);
                        button.set_tooltip(
                            is_pr_info_refreshing.then_some("Refreshing PR info"),
                            ctx,
                        );
                        button.set_on_click(
                            move |ctx| {
                                ctx.dispatch_typed_action(CodeReviewAction::ViewPr(url.clone()))
                            },
                            ctx,
                        );
                        button.clear_adjoined_side(ctx);
                    });
                }
            }
            PrimaryGitActionMode::Publish => {
                self.git_primary_action_button.update(ctx, |button, ctx| {
                    button.set_label("Publish", ctx);
                    button.set_icon(Some(Icon::UploadCloud), ctx);
                    button.set_disabled(false, ctx);
                    button.clear_tooltip(ctx);
                    button.set_on_click(
                        |ctx| ctx.dispatch_typed_action(CodeReviewAction::PublishBranch),
                        ctx,
                    );
                    button.clear_adjoined_side(ctx);
                });
            }
        }

        ctx.notify();
    }

    /// Returns the "Commit" dropdown item. Label/icon/action are fixed;
    /// only the disabled state flips across modes (enabled in Commit mode,
    /// disabled in Push mode where there's nothing to commit).
    fn commit_menu_item(disabled: bool) -> MenuItem<CodeReviewAction> {
        MenuItemFields::new("Commit")
            .with_icon(Icon::GitCommit)
            .with_on_select_action(CodeReviewAction::OpenCommitDialog)
            .with_disabled(disabled)
            .into_item()
    }

    /// Returns the "send commits to remote" dropdown item: `Push` when the
    /// branch already has an upstream, `Publish` otherwise (first push also
    /// sets the upstream).
    fn push_or_publish_menu_item(has_upstream: bool, disabled: bool) -> MenuItem<CodeReviewAction> {
        if has_upstream {
            MenuItemFields::new("Push")
                .with_icon(Icon::ArrowUp)
                .with_on_select_action(CodeReviewAction::OpenPushDialog)
                .with_disabled(disabled)
                .into_item()
        } else {
            MenuItemFields::new("Publish")
                .with_icon(Icon::UploadCloud)
                .with_on_select_action(CodeReviewAction::PublishBranch)
                .with_disabled(disabled)
                .into_item()
        }
    }

    /// Returns the PR dropdown item: "PR #N" linking to the existing PR, or
    /// "Create PR" to open the dialog. Create PR is disabled on main, when the
    /// branch has no upstream, or when the upstream is the same ref as main
    /// (e.g. a worktree branch whose tracking was auto-set to origin/master).
    fn pr_menu_item(&self, app: &AppContext) -> MenuItem<CodeReviewAction> {
        let diff_state = self.diff_state_model.as_ref(app);
        let is_pr_info_refreshing = diff_state.is_pr_info_refreshing(app);
        if let Some(pr_info) = diff_state.pr_info(app).cloned() {
            MenuItemFields::new(format!("PR #{}", pr_info.number))
                .with_icon(Icon::Github)
                .with_on_select_action(CodeReviewAction::ViewPr(pr_info.url))
                .with_disabled(is_pr_info_refreshing)
                .into_item()
        } else {
            let is_on_main = diff_state.is_on_main_branch(app);
            let has_upstream = diff_state.upstream_ref(app).is_some();
            let upstream_differs_from_main = diff_state.upstream_differs_from_main(app);
            MenuItemFields::new("Create PR")
                .with_icon(Icon::Github)
                .with_on_select_action(CodeReviewAction::OpenCreatePrDialog)
                .with_disabled(
                    is_pr_info_refreshing
                        || is_on_main
                        || !has_upstream
                        || !upstream_differs_from_main,
                )
                .into_item()
        }
    }

    /// Items for the git operations dropdown (chevron button). All three
    /// operations (Commit / Push / Create PR) are always listed so the
    /// dropdown shape is stable across modes; the primary mode determines
    /// which are enabled.
    fn git_operations_menu_items(&self, app: &AppContext) -> Vec<MenuItem<CodeReviewAction>> {
        let diff_state = self.diff_state_model.as_ref(app);
        let has_local_commits = !diff_state.unpushed_commits(app).is_empty();
        let has_upstream = diff_state.upstream_ref(app).is_some();
        match self.primary_git_action_mode(app) {
            PrimaryGitActionMode::Commit => vec![
                Self::commit_menu_item(false),
                // Middle item sends existing commits to the remote. Uncommitted
                // changes in the working tree don't block this — only whether
                // there are local commits to send.
                Self::push_or_publish_menu_item(has_upstream, !has_local_commits),
                // PR item handles its own disabled state (main branch, no
                // upstream). Uncommitted changes don't block it: the PR is
                // based on whatever's already been pushed.
                self.pr_menu_item(app),
            ],
            PrimaryGitActionMode::Push => vec![
                Self::commit_menu_item(true),
                Self::push_or_publish_menu_item(has_upstream, false),
                self.pr_menu_item(app),
            ],
            PrimaryGitActionMode::CreatePr
            | PrimaryGitActionMode::ViewPr
            | PrimaryGitActionMode::Publish => {
                // Chevron is hidden in these modes, so the menu is never opened.
                vec![]
            }
        }
    }

    /// Items for the header overflow menu (three-dots button).
    fn header_menu_items(&self, ctx: &mut ViewContext<Self>) -> Vec<MenuItem<CodeReviewAction>> {
        if FeatureFlag::GitOperationsInCodeReview.is_enabled() {
            self.header_menu_items_new(ctx)
        } else {
            self.header_menu_items_legacy(ctx)
        }
    }

    /// Legacy menu items — gated on FileAndDiffSetComments only.
    fn header_menu_items_legacy(
        &self,
        ctx: &mut ViewContext<Self>,
    ) -> Vec<MenuItem<CodeReviewAction>> {
        let mut items = Vec::new();

        if !FeatureFlag::FileAndDiffSetComments.is_enabled() {
            return items;
        }

        let mut has_changes = false;
        if let CodeReviewViewState::Loaded(loaded) = self.state() {
            has_changes = !loaded.to_diff_stats().has_no_changes();
        }

        if FeatureFlag::DiffSetAsContext.is_enabled() && has_changes {
            items.push(
                MenuItemFields::new("Add diff set as context")
                    .with_icon(Icon::Paperclip)
                    .with_on_select_action(CodeReviewAction::AddDiffSetAsContext(DiffSetScope::All))
                    .into_item(),
            );
        }

        let (comment_label, comment_icon) = if self.get_existing_diffset_comment(ctx).is_some() {
            ("Show saved comment", Icon::MessageText)
        } else {
            ("Add comment", Icon::MessagePlusSquare)
        };

        items.push(
            MenuItemFields::new(comment_label)
                .with_icon(comment_icon)
                .with_on_select_action(CodeReviewAction::OpenCommentComposerFromHeader)
                .into_item(),
        );

        items
    }

    /// New menu items — individually gated, includes discard and AI check.
    fn header_menu_items_new(
        &self,
        ctx: &mut ViewContext<Self>,
    ) -> Vec<MenuItem<CodeReviewAction>> {
        let mut items = Vec::new();

        let has_changes = matches!(self.state(), CodeReviewViewState::Loaded(loaded) if !loaded.to_diff_stats().has_no_changes());

        let is_ai_enabled = AISettings::as_ref(ctx).is_any_ai_enabled(ctx);
        if is_ai_enabled && FeatureFlag::DiffSetAsContext.is_enabled() && has_changes {
            items.push(
                MenuItemFields::new("Add diff set as context")
                    .with_icon(Icon::Paperclip)
                    .with_on_select_action(CodeReviewAction::AddDiffSetAsContext(DiffSetScope::All))
                    .into_item(),
            );
        }

        if FeatureFlag::FileAndDiffSetComments.is_enabled() && has_changes {
            let (comment_label, comment_icon) = if self.get_existing_diffset_comment(ctx).is_some()
            {
                ("Show saved comment", Icon::MessageText)
            } else {
                ("Add comment", Icon::MessagePlusSquare)
            };

            items.push(
                MenuItemFields::new(comment_label)
                    .with_icon(comment_icon)
                    .with_on_select_action(CodeReviewAction::OpenCommentComposerFromHeader)
                    .into_item(),
            );
        }

        if FeatureFlag::DiscardPerFileAndAllChanges.is_enabled() && has_changes {
            items.push(
                MenuItemFields::new("Discard all")
                    .with_icon(Icon::ReverseLeft)
                    .with_on_select_action(CodeReviewAction::ShowDiscardConfirmDialog(None))
                    .into_item(),
            );
        }

        items
    }

    fn get_unsaved_file_paths(&self, app: &AppContext) -> Vec<PathBuf> {
        let mut unsaved_paths = Vec::new();
        if let CodeReviewViewState::Loaded(state) = self.state() {
            for file_state in state.file_states.values() {
                if let Some(model) = &file_state.editor_state {
                    if model.has_unsaved_changes(app) {
                        unsaved_paths.push(file_state.file_diff.file_path.clone());
                    }
                }
            }
        }
        unsaved_paths
    }
    pub fn set_pane_id(&mut self, pane_id: PaneId) {
        self.containing_pane_id = Some(pane_id);
    }

    pub fn file_sidebar_expanded(&self) -> bool {
        self.file_sidebar_expanded
    }

    pub fn has_file_states(&self) -> bool {
        if let CodeReviewViewState::Loaded(loaded_state) = self.state() {
            !loaded_state.file_states.is_empty()
        } else {
            false
        }
    }

    /// Returns the diff stats from the loaded state — the same source used by
    /// the inner `CodeReviewHeader`. Returns `None` if diffs haven't loaded yet.
    pub fn loaded_diff_stats(&self) -> Option<DiffStats> {
        if let CodeReviewViewState::Loaded(loaded_state) = self.state() {
            Some(loaded_state.to_diff_stats())
        } else {
            None
        }
    }

    pub fn open_file_in_tab(
        &self,
        path: &Path,
        line_and_column: Option<LineAndColumnArg>,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(repo_path) = self.repo_path() else {
            return;
        };
        let full_path = repo_path.join(path);
        ctx.emit(CodeReviewViewEvent::OpenFileInNewTab {
            path: full_path,
            line_and_column,
        });
    }

    #[cfg(not(feature = "local_fs"))]
    fn open_code_review_file(
        &self,
        _full_path: PathBuf,
        _line_and_column: Option<LineAndColumnArg>,
        _ctx: &mut ViewContext<Self>,
    ) {
    }

    #[cfg(feature = "local_fs")]
    fn open_code_review_file(
        &self,
        full_path: PathBuf,
        line_and_column: Option<LineAndColumnArg>,
        ctx: &mut ViewContext<Self>,
    ) {
        let settings = EditorSettings::as_ref(ctx);
        let target = resolve_file_target_with_editor_choice(
            &full_path,
            *settings.open_code_panels_file_editor,
            *settings.prefer_markdown_viewer,
            *settings.open_file_layout,
            None,
        );

        send_telemetry_from_ctx!(
            TelemetryEvent::CodePanelsFileOpened {
                entrypoint: CodePanelsFileOpenEntrypoint::CodeReview,
                target: target.clone(),
            },
            ctx
        );

        ctx.emit(CodeReviewViewEvent::OpenFileWithTarget {
            path: full_path,
            target,
            line_col: line_and_column,
        });
    }

    pub(super) fn editor_for_path(
        &self,
        path: &Path,
        ctx: &AppContext,
    ) -> Option<ViewHandle<LocalCodeEditorView>> {
        match self.state() {
            CodeReviewViewState::Loaded(loaded) => loaded
                .file_states
                .values()
                .filter_map(|file| file.editor_state.as_ref())
                .find_map(|state| {
                    let editor_path = state.editor.as_ref(ctx).file_path();
                    if editor_path == Some(path) {
                        Some(state.editor.clone())
                    } else {
                        None
                    }
                }),
            _ => None,
        }
    }

    pub(super) fn editor_handles(
        &self,
    ) -> Box<dyn Iterator<Item = ViewHandle<LocalCodeEditorView>> + '_> {
        match self.state() {
            CodeReviewViewState::Loaded(loaded) => Box::new(
                loaded
                    .file_states
                    .values()
                    .filter(|file| file.is_expanded)
                    .filter_map(|file| file.editor_state.as_ref())
                    .map(|state| state.editor.clone()),
            ),
            _ => Box::new(std::iter::empty()),
        }
    }
}

/// Returns the line number of the first line in the file affected by the diff.
fn file_line_for_open(file_diff: &FileDiff) -> Option<usize> {
    file_diff.hunks.first().and_then(|hunk| {
        let mut last_context_line_number = None;
        for line in &hunk.lines {
            match line.line_type {
                // If the hunk has additions, open the file at the start of the addition.
                DiffLineType::Add => return line.new_line_number,
                // If the hunk is only a deletion, open the file where the deleted lines would have been,
                // which we know from the previous context lines.
                DiffLineType::Delete => {
                    return last_context_line_number
                        .map(|context_line: usize| context_line.saturating_add(1));
                }
                DiffLineType::Context => {
                    last_context_line_number = line.new_line_number;
                }
                DiffLineType::HunkHeader => {}
            }
        }
        None
    })
}

impl Entity for CodeReviewView {
    type Event = CodeReviewViewEvent;
}

impl View for CodeReviewView {
    fn render(&self, ctx: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(ctx);

        if self.active_repo.is_none() {
            return self.render_no_repo_for_env(ctx, appearance);
        }

        let is_in_split_pane = self
            .focus_handle
            .as_ref()
            .map(|h| h.split_pane_state(ctx).is_in_split_pane())
            .unwrap_or(false);

        let main_content = match self.state() {
            CodeReviewViewState::None => CodeReviewView::render_loading_state(appearance),
            CodeReviewViewState::Loaded(loaded_state) => {
                // For global buffer mode, show loading state until all editors have loaded
                // their buffer content. This prevents a brief flash of empty editors.
                if !self.all_editors_loaded() {
                    CodeReviewView::render_loading_state(appearance)
                } else {
                    self.render_loaded_state(loaded_state, appearance, is_in_split_pane, ctx)
                }
            }
            CodeReviewViewState::Error(err) => self.render_error_state(err, appearance),
            CodeReviewViewState::NoRepoFound => self.render_no_repo_for_env(ctx, appearance),
        };

        let content_with_handler = EventHandler::new(Container::new(main_content).finish())
            .on_left_mouse_down(|ctx, _, _| {
                ctx.dispatch_typed_action(CodeReviewAction::FocusView);
                DispatchEventResult::StopPropagation
            })
            .finish();

        let mut stack = Stack::new()
            .with_child(content_with_handler)
            .with_constrain_absolute_children();

        // Pane-level comment composer overlay, anchored to the header position.
        if let Some(composer) = &self.comment_composer {
            let editor = ChildView::new(composer).finish();
            let styled_composer = Container::new(
                ConstrainedBox::new(editor)
                    .with_max_width(DEFAULT_COMMENT_MAX_WIDTH)
                    .with_max_height(260.)
                    .finish(),
            )
            .with_margin_left(32.)
            .with_margin_right(32.)
            .finish();

            stack.add_positioned_child(
                styled_composer,
                OffsetPositioning::offset_from_save_position_element(
                    self.header_position_id.as_str(),
                    vec2f(0., -40.),
                    PositionedElementOffsetBounds::ParentByPosition,
                    PositionedElementAnchor::BottomRight,
                    ChildAnchor::TopRight,
                ),
            );
        }

        // Header dropdown menu is rendered inline with the header button in CodeReviewHeader

        if self.find_model.as_ref(ctx).is_find_bar_open() {
            stack.add_child(ChildView::new(&self.find_bar).finish());
        }

        let show_modal =
            self.discard_dialog_state.show_discard_confirm_dialog || self.git_dialog.is_some();

        if show_modal {
            let mut dialog_stack = Stack::new()
                .with_child(SavePosition::new(stack.finish(), &self.view_position_id).finish());

            if self.discard_dialog_state.show_discard_confirm_dialog {
                dialog_stack.add_positioned_overlay_child(
                    self.render_discard_confirm_dialog(ctx),
                    OffsetPositioning::offset_from_parent(
                        vec2f(0., 0.),
                        ParentOffsetBounds::WindowByPosition,
                        ParentAnchor::Center,
                        ChildAnchor::Center,
                    ),
                );
            } else if let Some(git_dialog) = &self.git_dialog {
                dialog_stack.add_positioned_overlay_child(
                    ChildView::new(git_dialog).finish(),
                    OffsetPositioning::offset_from_parent(
                        vec2f(0., 0.),
                        ParentOffsetBounds::WindowByPosition,
                        ParentAnchor::Center,
                        ChildAnchor::Center,
                    ),
                );
            }

            dialog_stack.finish()
        } else {
            SavePosition::new(stack.finish(), &self.view_position_id).finish()
        }
    }

    fn ui_name() -> &'static str {
        "CodeReviewView"
    }
}

impl TypedActionView for CodeReviewView {
    type Action = CodeReviewAction;

    fn handle_action(&mut self, action: &CodeReviewAction, ctx: &mut ViewContext<Self>) {
        match action {
            CodeReviewAction::OpenInNewTab {
                path,
                line_and_column,
            } => {
                let Some(repo_path) = self.repo_path() else {
                    return;
                };
                let full_path = repo_path.join(path);
                self.open_code_review_file(full_path, *line_and_column, ctx);
            }
            CodeReviewAction::ToggleFileExpanded(path) => {
                let (file_index, now_expanded, chevron_button) = {
                    let Some(repo) = self.active_repo.as_mut() else {
                        return;
                    };

                    if let CodeReviewViewState::Loaded(state) = &mut repo.state {
                        if let Some(index) = state.file_states.get_index_of(path) {
                            let file = &mut state.file_states[index];
                            file.is_expanded = !file.is_expanded;
                            let now_expanded = file.is_expanded;
                            repo.file_expanded
                                .insert(file.file_diff.file_path.clone(), now_expanded);
                            (index, now_expanded, file.chevron_button.clone())
                        } else {
                            return;
                        }
                    } else {
                        return;
                    }
                };

                // Update the chevron button icon based on expanded state
                chevron_button.update(ctx, |button, ctx| {
                    let icon = if now_expanded {
                        Icon::ChevronDown
                    } else {
                        Icon::ChevronRight
                    };
                    button.set_icon(Some(icon), ctx);
                });

                self.viewported_list_state
                    .invalidate_height_for_index(file_index);
                // If the file gets collapsed and had a sticky header, then we scroll to make the header in view.
                if !now_expanded && self.viewported_list_state.is_scrolled_to_item(file_index) {
                    self.viewported_list_state.scroll_to(file_index);
                }

                if self.find_model.as_ref(ctx).is_find_bar_open()
                    && FeatureFlag::CodeReviewFind.is_enabled()
                {
                    self.find_model.update(ctx, |model, model_ctx| {
                        model.run_search(self.editor_handles(), model_ctx);
                    });
                }

                ctx.notify();
            }
            CodeReviewAction::SetDiffMode(mode) => {
                self.apply_diff_mode(mode.clone(), ctx);
            }
            CodeReviewAction::ToggleFileSidebar => {
                if self.file_sidebar_expanded {
                    self.file_sidebar_expanded = false;
                    // If the sidebar is closed while maximized, update the saved
                    // pre-maximize state so we don't reopen it on minimize.
                    if self.file_sidebar_expanded_before_maximize.is_some() {
                        self.file_sidebar_expanded_before_maximize = Some(false);
                    }
                } else {
                    self.open_file_sidebar(ctx);
                }
                self.update_file_nav_button_tooltip(ctx);
                ctx.notify();
            }
            CodeReviewAction::FileSelected(file_index) => {
                // Early-return when repo/state/file is missing to avoid calling
                // invalidate_height_for_index or scroll_to with an invalid index.
                let was_expanded = {
                    let Some(repo) = self.active_repo.as_mut() else {
                        return;
                    };
                    let CodeReviewViewState::Loaded(state) = &mut repo.state else {
                        return;
                    };
                    let Some((_, file)) = state.file_states.get_index_mut(*file_index) else {
                        return;
                    };
                    let was_expanded = file.is_expanded;
                    file.is_expanded = true;
                    was_expanded
                };

                self.viewported_list_state
                    .invalidate_height_for_index(*file_index);

                if !was_expanded
                    && self.find_model.as_ref(ctx).is_find_bar_open()
                    && FeatureFlag::CodeReviewFind.is_enabled()
                {
                    self.find_model.update(ctx, |model, model_ctx| {
                        model.run_search(self.editor_handles(), model_ctx);
                    });
                }

                ctx.notify();

                self.viewported_list_state.scroll_to(*file_index);
                ctx.notify();
            }
            CodeReviewAction::ToggleMaximize => {
                // Determine if we're minimizing or maximizing
                let is_currently_maximized = self
                    .focus_handle
                    .as_ref()
                    .is_some_and(|h| h.is_maximized(ctx));

                let state_change = if is_currently_maximized {
                    PaneStateChange::Minimized
                } else {
                    PaneStateChange::Maximized
                };

                send_telemetry_from_ctx!(
                    CodeReviewTelemetryEvent::PaneStateChanged { state_change },
                    ctx
                );

                ctx.emit(CodeReviewViewEvent::Pane(PaneEvent::ToggleMaximized));
            }
            CodeReviewAction::SaveAllFiles { paths } => {
                self.save_files(paths, ctx);
            }
            CodeReviewAction::SaveAllUnsavedFiles => {
                let unsaved_files = self.get_unsaved_file_paths(ctx);
                self.save_files(unsaved_files.as_slice(), ctx);
            }
            CodeReviewAction::RefreshGitState => {
                self.diff_state_model.update(ctx, |model, ctx| {
                    model.load_diffs_for_current_repo(false, ctx);
                    model.refresh_metadata_and_pr_info(ctx);
                });
            }
            CodeReviewAction::UndoRevert => {
                self.maybe_undo_revert(ctx);
            }
            CodeReviewAction::Close => {
                ctx.emit(CodeReviewViewEvent::Pane(PaneEvent::Close));
            }
            CodeReviewAction::OpenHeaderMenu => {
                // Header dropdown: build items and toggle the menu open/closed.
                let items = self.header_menu_items(ctx);
                if items.is_empty() {
                    return;
                }

                if self.header_menu_open {
                    // Close the menu by clearing items.
                    self.header_menu.update(ctx, |menu, ctx| {
                        menu.set_items(Vec::new(), ctx);
                        ctx.notify();
                    });
                    self.header_menu_open = false;
                } else {
                    self.header_menu.update(ctx, move |menu, ctx| {
                        menu.set_items(items, ctx);
                        ctx.notify();
                    });
                    self.header_menu_open = true;
                }
                self.update_header_dropdown_active_state(ctx);
                ctx.notify();
            }
            CodeReviewAction::OpenCommentComposerFromHeader => {
                // Show the review comment composer overlay if it's not already open.
                let existing_comment = self.get_existing_diffset_comment(ctx);
                self.open_review_comment_composer(existing_comment, ctx);
            }
            CodeReviewAction::EmitPaneEvent(event) => {
                ctx.emit(CodeReviewViewEvent::Pane(event.clone()));
            }
            CodeReviewAction::ShowDiscardConfirmDialog(file_path) => {
                self.discard_dialog_state.show_discard_confirm_dialog = true;

                let current_diff_mode = self.diff_state_model.as_ref(ctx).diff_mode(ctx);

                if let Some(path) = file_path {
                    // Single file remove
                    self.discard_dialog_state.discard_file_paths = vec![path.clone()];
                    self.discard_dialog_state.operation_type = match current_diff_mode {
                        DiffMode::Head => DiscardOperationType::FileUncommittedChanges,
                        DiffMode::MainBranch => {
                            DiscardOperationType::FileChangesAgainstBranch(None)
                        }
                        DiffMode::OtherBranch(branch) => {
                            DiscardOperationType::FileChangesAgainstBranch(Some(branch))
                        }
                    };
                } else {
                    // All files remove
                    self.discard_dialog_state.operation_type = match current_diff_mode {
                        DiffMode::Head => DiscardOperationType::AllUncommittedChanges,
                        DiffMode::MainBranch => DiscardOperationType::AllChangesAgainstBranch(None),
                        DiffMode::OtherBranch(branch) => {
                            DiscardOperationType::AllChangesAgainstBranch(Some(branch))
                        }
                    };

                    // Collect all file paths from loaded state
                    if let CodeReviewViewState::Loaded(loaded) = self.state() {
                        self.discard_dialog_state.discard_file_paths =
                            loaded.file_states.keys().cloned().collect();

                        // Initialize all files as selected  by default
                        self.discard_dialog_state.selected_files.clear();
                        self.discard_dialog_state.file_checkbox_mouse_states.clear();
                        for file_path in &self.discard_dialog_state.discard_file_paths {
                            self.discard_dialog_state
                                .selected_files
                                .insert(file_path.clone(), true);
                            self.discard_dialog_state
                                .file_checkbox_mouse_states
                                .insert(file_path.clone(), MouseStateHandle::default());
                        }
                    }
                }
                ctx.notify();
            }
            CodeReviewAction::ConfirmDiscardFile => {
                let is_discard_all = matches!(
                    self.discard_dialog_state.operation_type,
                    DiscardOperationType::AllUncommittedChanges
                        | DiscardOperationType::AllChangesAgainstBranch(_)
                );

                if is_discard_all {
                    // Get list of selected files
                    let selected_files: Vec<StandardizedPath> = self
                        .discard_dialog_state
                        .discard_file_paths
                        .iter()
                        .filter_map(|path| {
                            if !*self
                                .discard_dialog_state
                                .selected_files
                                .get(path)
                                .unwrap_or(&false)
                            {
                                None
                            } else {
                                self.to_standardized_path(path)
                            }
                        })
                        .collect();

                    if !selected_files.is_empty() {
                        self.discard_multiple_files(
                            selected_files,
                            self.discard_dialog_state.stash_changes_enabled,
                            ctx,
                        );
                    }
                } else {
                    let file_path = self.discard_dialog_state.discard_file_paths[0].clone();
                    if let Some(standardized_path) = self.to_standardized_path(&file_path) {
                        self.discard_file(
                            standardized_path,
                            self.discard_dialog_state.stash_changes_enabled,
                            ctx,
                        );
                    }
                }
                self.discard_dialog_state.show_discard_confirm_dialog = false;
                self.discard_dialog_state.discard_file_paths.clear();
                self.discard_dialog_state.selected_files.clear();
                self.discard_dialog_state.file_checkbox_mouse_states.clear();
                self.discard_dialog_state.stash_changes_enabled = false;
                ctx.notify();
            }
            CodeReviewAction::CancelDiscardFile => {
                self.discard_dialog_state.show_discard_confirm_dialog = false;
                self.discard_dialog_state.discard_file_paths.clear();
                ctx.notify();
            }
            CodeReviewAction::ToggleStashChanges => {
                self.discard_dialog_state.stash_changes_enabled =
                    !self.discard_dialog_state.stash_changes_enabled;
                ctx.notify();
            }
            CodeReviewAction::ToggleFileSelection(ref file_path) => {
                if let Some(selected) = self.discard_dialog_state.selected_files.get_mut(file_path)
                {
                    *selected = !*selected;
                    ctx.notify();
                }
            }
            CodeReviewAction::AddDiffSetAsContext(scope) => {
                self.insert_diff_as_context(scope.clone(), ctx);
            }
            CodeReviewAction::CopyFilePath(path) => {
                if let Some(repo_path) = self.repo_path() {
                    let absolute_path = repo_path.join(path);
                    if let Some(path_str) = absolute_path.to_str() {
                        ctx.clipboard()
                            .write(ClipboardContent::plain_text(path_str.to_string()));
                    }
                }
            }
            CodeReviewAction::ShowFindBar => self.show_find_bar(ctx),
            CodeReviewAction::FocusView => {
                ctx.focus_self();
            }
            CodeReviewAction::OpenRepository => {
                if let Some(terminal_view) = self.terminal_view(ctx) {
                    terminal_view.update(ctx, |terminal, ctx| {
                        terminal.handle_action(&TerminalAction::PickRepoToOpen, ctx);
                    });
                }
            }
            CodeReviewAction::InitProjectForCurrentDirectory => {
                if let Some(terminal_view) = self.terminal_view(ctx) {
                    terminal_view.update(ctx, |terminal, ctx| {
                        terminal.handle_action(&TerminalAction::InitProject, ctx);
                    });
                }
            }
            CodeReviewAction::OpenCommitDialog => {
                send_telemetry_from_ctx!(
                    CodeReviewTelemetryEvent::GitButtonTriggered {
                        button: GitButtonKind::Commit,
                    },
                    ctx
                );
                self.open_git_dialog(GitDialogKind::Commit, ctx);
            }
            CodeReviewAction::PublishBranch => {
                send_telemetry_from_ctx!(
                    CodeReviewTelemetryEvent::GitButtonTriggered {
                        button: GitButtonKind::Publish,
                    },
                    ctx
                );
                self.open_git_dialog(GitDialogKind::Push { publish: true }, ctx);
            }
            CodeReviewAction::OpenPushDialog => {
                send_telemetry_from_ctx!(
                    CodeReviewTelemetryEvent::GitButtonTriggered {
                        button: GitButtonKind::Push,
                    },
                    ctx
                );
                self.open_git_dialog(GitDialogKind::Push { publish: false }, ctx);
            }
            CodeReviewAction::OpenCreatePrDialog => {
                send_telemetry_from_ctx!(
                    CodeReviewTelemetryEvent::GitButtonTriggered {
                        button: GitButtonKind::CreatePr,
                    },
                    ctx
                );
                self.open_git_dialog(GitDialogKind::CreatePr, ctx);
            }
            CodeReviewAction::ViewPr(url) => {
                send_telemetry_from_ctx!(
                    CodeReviewTelemetryEvent::GitButtonTriggered {
                        button: GitButtonKind::ViewPr,
                    },
                    ctx
                );
                ctx.open_url(url);
            }
            CodeReviewAction::ToggleGitOperationsMenu => {
                let items = self.git_operations_menu_items(ctx);
                if items.is_empty() {
                    return;
                }

                if self.git_operations_menu_open {
                    self.git_operations_menu_open = false;
                } else {
                    self.git_operations_menu.update(ctx, move |menu, ctx| {
                        menu.set_items(items, ctx);
                        ctx.notify();
                    });
                    self.git_operations_menu_open = true;
                }
                self.git_operations_chevron.update(ctx, |button, ctx| {
                    button.set_active(self.git_operations_menu_open, ctx);
                });
                ctx.notify();
            }
        }
    }
}

impl BackingView for CodeReviewView {
    type PaneHeaderOverflowMenuAction = CodeReviewAction;
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
        let unsaved_file_paths = self.get_unsaved_file_paths(ctx);

        if !unsaved_file_paths.is_empty() && ChannelState::channel() != Channel::Integration {
            let file_names = unsaved_file_paths
                .iter()
                .filter_map(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .map(|s| s.to_string())
                })
                .collect::<Vec<_>>()
                .join(", ");
            let summary = UnsavedStateSummary::for_editor_tab(
                Some(file_names),
                vec![CodeEditorStatus::new(true)],
                ctx,
            );

            let handle_save_intent = |intent: PendingSaveIntent| {
                let handle = ctx.handle().clone();
                let paths_to_save = unsaved_file_paths.clone();
                move |ctx: &mut AppContext| {
                    if let Some(view) = handle.upgrade(ctx) {
                        view.update(ctx, |view, ctx| match intent {
                            PendingSaveIntent::Save => {
                                view.handle_action(
                                    &CodeReviewAction::SaveAllFiles {
                                        paths: paths_to_save,
                                    },
                                    ctx,
                                );
                                ctx.emit(CodeReviewViewEvent::Pane(PaneEvent::Close));
                            }
                            PendingSaveIntent::Discard => {
                                ctx.emit(CodeReviewViewEvent::Pane(PaneEvent::Close));
                            }
                            PendingSaveIntent::Cancel => {}
                        });
                    }
                }
            };

            let dialog = summary
                .dialog()
                .on_save_changes(handle_save_intent(PendingSaveIntent::Save))
                .on_discard_changes(handle_save_intent(PendingSaveIntent::Discard))
                .on_cancel(handle_save_intent(PendingSaveIntent::Cancel))
                .build();

            if cfg!(all(not(target_family = "wasm"), target_os = "macos")) {
                AppContext::show_native_platform_modal(ctx, dialog);
            } else if cfg!(all(
                not(target_family = "wasm"),
                any(
                    target_os = "linux",
                    target_os = "freebsd",
                    target_os = "windows"
                )
            )) {
                // Find the workspace to show the Warp-native modal
                if let Some(workspace) = ctx
                    .views_of_type::<Workspace>(ctx.window_id())
                    .and_then(|workspaces| workspaces.first().cloned())
                {
                    workspace.update(ctx, |view, ctx| {
                        view.show_native_modal(dialog, ctx);
                    });
                }
            }
        } else {
            ctx.emit(CodeReviewViewEvent::Pane(PaneEvent::Close));
        }
    }

    fn focus_contents(&mut self, ctx: &mut ViewContext<Self>) {
        // TODO: Focus the contents of the view.
        ctx.focus_self();
    }

    fn handle_custom_action(&mut self, _action: &Self::CustomAction, _ctx: &mut ViewContext<Self>) {
    }

    fn render_header_content(
        &self,
        _ctx: &view::HeaderRenderContext<'_>,
        _app: &AppContext,
    ) -> view::HeaderContent {
        view::HeaderContent::simple("Reviewing code changes")
    }

    fn set_focus_handle(&mut self, focus_handle: PaneFocusHandle, ctx: &mut ViewContext<Self>) {
        ctx.subscribe_to_model(
            focus_handle.focus_state_handle(),
            |code_review, _handle, event, ctx| {
                code_review.handle_focus_state_event(event, ctx);
            },
        );
        self.focus_handle = Some(focus_handle);
    }
}

#[derive(Debug)]
struct ShowCommentEditor {
    comment_list_save_position_id: String,
    window_id: WindowId,
}

impl ShowCommentEditorProvider for ShowCommentEditor {
    fn should_show_comment_editor(&self, editor_line_location: RectF, app: &AppContext) -> bool {
        let Some(comment_list_view_position) = app.element_position_by_id_at_last_frame(
            self.window_id,
            &self.comment_list_save_position_id,
        ) else {
            return false;
        };

        comment_list_view_position.contains_point(editor_line_location.upper_right())
            || comment_list_view_position.contains_point(editor_line_location.lower_left())
    }
}

#[path = "scroll_preservation.rs"]
mod scroll_preservation;
use scroll_preservation::RelocatableScrollContext;

#[cfg(feature = "integration_tests")]
#[path = "code_review_view_integration.rs"]
mod code_review_view_integration;

#[cfg(feature = "integration_tests")]
pub use code_review_view_integration::CodeReviewVisibleAnchorForTest;

#[cfg(test)]
#[path = "code_review_view_tests.rs"]
mod tests;
