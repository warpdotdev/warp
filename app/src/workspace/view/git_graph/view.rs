//! Git Graph view.
//!
//! Renders the commit graph list in the left panel (lane graph + short hash +
//! ref labels + subject); clicking a row loads and shows that commit's detail
//! (full info + changed files).
//!
//! State is held directly in the view (single instance, not shared); we don't
//! introduce a separate Model indirection layer — that can be extracted later
//! if cross-view sharing is ever needed.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use async_channel::Sender;
use pathfinder_color::ColorU;
use warpui::clipboard::ClipboardContent;
use warpui::elements::shimmering_text::{
    ShimmerConfig, ShimmeringTextElement, ShimmeringTextStateHandle,
};
use warpui::elements::{
    resizable_state_handle, Align, Border, ChildAnchor, ChildView, ClippedScrollStateHandle,
    ClippedScrollable, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Dismiss,
    DragBarSide, Element, Empty, Expanded, Fill, Flex, Highlight, Hoverable, MainAxisAlignment,
    MainAxisSize, MouseStateHandle, OffsetPositioning, ParentElement, PositionedElementAnchor,
    PositionedElementOffsetBounds, Radius, Resizable, ResizableStateHandle, SavePosition,
    Scrollable, ScrollableElement, ScrollStateHandle, ScrollbarWidth, SelectableArea,
    SelectionHandle, Shrinkable, Stack, Text, UniformList, UniformListState,
};
use warpui::fonts::{Properties, Weight};
use warpui::geometry::vector::{vec2f, Vector2F};
use warpui::keymap::macros::id;
use warpui::keymap::FixedBinding;
use warpui::scene::DropShadow;
use warpui::text_layout::{ClipConfig, ClipDirection, ClipStyle};
use warpui::units::Pixels;
use warpui::{AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle};

use warp_core::ui::color::pick_foreground_color;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::Icon;
use warpui::platform::SaveFilePickerConfiguration;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};

use super::data::{BranchRef, ChangedFile, CommitDetail, CommitNode, RefKind, RefLabel};
use super::layout::{assign_lanes, GraphLayout, GraphRow};
use super::menu::{build_menu, MenuKind, PromptKind};
use super::ops::{archive_format_from_path, GitWriteOp, ResetMode};
use super::row_canvas::GitGraphRowCanvas;
use crate::appearance::Appearance;
use crate::code::editor::{add_color, remove_color};
use crate::editor::{EditorView, Event as EditorEvent, SingleLineEditorOptions};
use crate::features::FeatureFlag;
use crate::menu::{Menu, MenuItem, MenuItemFields};
use crate::settings::{GitSettings, GitSettingsChangedEvent};
use crate::ui_components::buttons::icon_button;
use crate::ui_components::item_highlight::ItemHighlightState;
use crate::view_components::dropdown::{Dropdown, DropdownAction};

/// Number of commits loaded per page.
const COMMIT_PAGE_SIZE: usize = 200;

/// Prefetch the next page once the viewport gets within this many rows of the
/// list end (infinite-scroll lead so we don't wait until the very bottom).
const LOAD_MORE_PREFETCH: usize = 10;

/// Registers the view-level key binding: while the Git Graph panel is focused,
/// Cmd/Ctrl+C copies the text selected in the detail area. Scoped to this view,
/// so it doesn't interfere with copy in the terminal or other contexts.
pub(crate) fn init(app: &mut AppContext) {
    app.register_fixed_bindings([FixedBinding::new(
        "cmdorctrl-c",
        GitGraphAction::CopySelection,
        id!(GitGraphView::ui_name()),
    )]);
}

/// The view's own actions.
/// Implements `PartialEq` to satisfy the [`DropdownItemAction`] bound on the
/// repository dropdown.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum GitGraphAction {
    /// Select the Nth commit row in the list and load its detail.
    SelectCommit(usize),
    /// Switch to the Nth repository in the discovered list (dispatched by the
    /// top dropdown when there are multiple repos).
    SelectRepository(usize),
    /// Expand/collapse the branch filter overlay.
    ToggleBranchFilter,
    /// Close the branch filter overlay (when clicking outside it).
    CloseBranchFilter,
    /// Toggle visibility of a branch ref (the value is the full ref, e.g.
    /// `refs/heads/main`).
    ToggleBranch(String),
    /// Select all branches.
    SelectAllBranches,
    /// Deselect all branches.
    DeselectAllBranches,
    /// Manually rescan the working directory and reload the graph.
    Refresh,
    /// Close the detail area (clear selection).
    CloseDetail,
    /// Copy the text currently selected in the detail area to the clipboard
    /// (Cmd/Ctrl+C).
    CopySelection,
    /// Pull keyboard focus into this view (dispatched when the detail area
    /// starts a drag-selection). Selection relies on hit-testing and doesn't
    /// move focus, so if focus has since left (e.g. pasting into an editor,
    /// clicking elsewhere), not refocusing would keep later Cmd/Ctrl+C from
    /// reaching this view.
    FocusPanel,
    /// Open the Nth changed file's diff from the detail area in a read-only
    /// diff pane in the main area.
    OpenFileDiff(usize),

    // --- Right-click context menu & write operations (gated at build time by
    // [`FeatureFlag::GitGraphWrite`] for the mutating items). ---
    /// Open the context menu for a right-click target at the given offset
    /// (relative to the panel root). `x`/`y` are carried as scalars because
    /// `Vector2F` is not `PartialEq` (required by this enum).
    OpenMenu { kind: MenuKind, x: f32, y: f32 },
    /// Write `text` to the clipboard (commit hash / subject / ref name).
    CopyToClipboard(String),
    /// Open the text-input dialog to collect a name (tag / branch / rename).
    PromptInput(PromptKind),
    /// Submit the text-input dialog: build the op from the entered text and run.
    SubmitInput,
    /// Open the reset-mode dialog (soft / mixed / hard) for the commit.
    PromptResetMode { hash: String },
    /// Run a write op, first showing a confirmation dialog when the op requires
    /// one ([`GitWriteOp::confirm_message`]).
    BeginWriteOp(GitWriteOp),
    /// Run a write op now (dispatched by the confirm dialog's "Confirm" button,
    /// the reset-mode dialog's buttons, and the archive save callback).
    RunOp(GitWriteOp),
    /// Open the OS save dialog for "Create Archive"; the chosen path drives the
    /// archive format and the actual run.
    BeginArchive { rev: String, suggested_name: String },
    /// Cancel any open dialog (input / confirm / reset-mode).
    CancelDialog,
    /// Dismiss the operation-error banner.
    DismissOpError,
}

/// Events the view emits outward.
pub(crate) enum GitGraphEvent {
    /// Request to open "a commit's changes to a given file" in a read-only diff
    /// pane in the main area. Forwarded up by the left panel; the workspace
    /// ultimately builds a [`CommitDiffView`] and opens it as a new pane.
    ///
    /// [`CommitDiffView`]: crate::code::commit_diff_view::CommitDiffView
    #[cfg(not(target_family = "wasm"))]
    OpenCommitFileDiff {
        /// Repo-relative path.
        repo_relative_path: String,
        /// Short commit hash (for the pane title).
        short_hash: String,
        /// The file's full content at the parent commit (the diff base).
        base_content: String,
        /// The commit's unified diff hunks for this file.
        hunks: Vec<crate::code_review::diff_state::DiffHunk>,
    },
}

/// Load state of the commit graph.
enum LoadState {
    /// The working directory isn't inside any git repository, or no directory
    /// has been specified yet.
    NoRepo,
    /// Loading in progress.
    Loading,
    /// Loaded (`commits` is valid; may be empty = repo has no commits).
    Loaded,
    /// Load failed, with the error message.
    Error(String),
}

/// Load state of the selected commit's detail.
enum DetailState {
    /// No commit selected.
    None,
    Loading,
    Loaded(CommitDetail),
    Error(String),
}

/// The modal dialog currently shown over the panel (mutually exclusive with the
/// context menu). Gates each mutating operation behind explicit user input.
enum DialogState {
    /// No dialog open.
    None,
    /// A single-line text prompt (tag / branch name, rename) — the entered text
    /// is read from `dialog_input` on submit.
    Input(PromptKind),
    /// A yes/no confirmation for a (typically destructive) op.
    Confirm { op: GitWriteOp, message: String },
    /// The reset-mode picker for "Reset current branch to this Commit".
    ResetMode { hash: String },
}

/// Width of the right-click context menu.
const CONTEXT_MENU_WIDTH: f32 = 260.;
/// Width of the modal dialogs (input / confirm / reset-mode).
const DIALOG_WIDTH: f32 = 360.;

pub(crate) struct GitGraphView {
    /// Anchor directory for repository discovery (pushed in by the left panel
    /// when the active directory changes): besides the repo it belongs to, we
    /// also scan subdirectories for standalone repos down to
    /// [`GitSettings::git_graph_scan_depth`].
    scan_anchor: Option<PathBuf>,
    /// List of discovered repository roots (the anchor's own repo comes first).
    /// When there's more than one, a repository dropdown is shown at the top.
    repositories: Arc<Vec<PathBuf>>,
    /// Index into `repositories` of the currently selected repo (the one whose
    /// history is being shown).
    selected_repo: Option<usize>,
    /// Repository picker dropdown at the top when there are multiple repos
    /// (child view, dispatches [`GitGraphAction::SelectRepository`]).
    repo_dropdown: ViewHandle<Dropdown<GitGraphAction>>,
    /// The current repo's branch list (local + remote), shown by the branch
    /// filter overlay.
    branches: Arc<Vec<BranchRef>>,
    /// Set of currently checked branch refs (those shown in the graph), stored
    /// as full refs.
    selected_branches: HashSet<String>,
    /// Per repo root → the user's branch selection in that repo (full refs). A
    /// re-discover triggered by switching tab / cd / refresh restores each
    /// repo's selection from this, so the branch filter survives all of them
    /// (refresh included), only dropping branches that no longer exist.
    saved_branch_selections: HashMap<PathBuf, HashSet<String>>,
    /// Per anchor (the tab's working directory) → the repo root the user last
    /// manually picked for that anchor. Lets each tab restore its own repo
    /// choice after switching away and back, instead of snapping to the repo
    /// the anchor lives in. Mirrors `saved_branch_selections`, but keyed by
    /// anchor because "which repo" is a per-tab choice while "which branches"
    /// is a per-repo one.
    saved_repo_selections: HashMap<PathBuf, PathBuf>,
    /// Whether the branch filter overlay is expanded.
    branch_filter_expanded: bool,
    /// Mouse state of the branch filter button.
    branch_filter_button_mouse_state: MouseStateHandle,
    /// Mouse state of the branch overlay's "select all" button.
    branch_select_all_mouse_state: MouseStateHandle,
    /// Mouse state of the branch overlay's "deselect all" button.
    branch_deselect_all_mouse_state: MouseStateHandle,
    /// Per-row mouse state inside the branch overlay (for hover highlight),
    /// same length as `branches`.
    branch_mouse_states: Arc<Vec<MouseStateHandle>>,
    /// Scroll state of the branch overlay list (scrollable when there are many
    /// branches).
    branch_scroll_state: ClippedScrollStateHandle,
    /// Loaded commits (wrapped in `Arc` for zero-copy move into the
    /// [`UniformList`] build closure).
    commits: Arc<Vec<CommitNode>>,
    /// Per-row lane layout computed by [`assign_lanes`], one-to-one with
    /// `commits`.
    layout: Arc<GraphLayout>,
    state: LoadState,
    /// Per-row mouse state handles (used by [`Hoverable`] for click/hover),
    /// same length as `commits`.
    row_mouse_states: Arc<Vec<MouseStateHandle>>,
    /// Index of the currently selected row.
    selected: Option<usize>,
    /// Detail of the selected commit.
    detail: DetailState,
    /// Scroll state of the commit list (virtualization / row range).
    list_state: UniformListState,
    /// Scroll state driving the commit list's overlay scrollbar (paired with
    /// `list_state` the way the file tree pairs its two scroll states).
    commit_scroll_state: ScrollStateHandle,
    /// Scroll state of the detail area as a whole (commit info + changed file
    /// list): info and files share one scrollable region, so long commit
    /// messages can be scrolled to view in full.
    detail_scroll_state: ClippedScrollStateHandle,
    /// Mouse state of the refresh button.
    refresh_mouse_state: MouseStateHandle,
    /// Whether more commits might be loadable (assumed true if the last page
    /// came back full).
    has_more: bool,
    /// Whether the next page is currently loading (reentrancy guard).
    loading_more: bool,
    /// Sender for the list's visible row range: [`UniformList`] reports the
    /// visible range, driving auto-load on scroll to bottom.
    visible_range_sender: Sender<Range<usize>>,
    /// Pulse animation state of the bottom "load more" indicator row.
    loading_shimmer: ShimmeringTextStateHandle,
    /// Shimmer animation state of the "Working…" overlay shown while a write op
    /// runs (separate from `loading_shimmer` so the two never share phase).
    op_shimmer: ShimmeringTextStateHandle,
    /// Draggable state for the detail area's height. The initial pixel value is
    /// just a placeholder; on the first frame's layout the bounds callback in
    /// [`Self::render_resizable_detail`] overrides it to 1/3 of the window
    /// height.
    detail_resizable_state: ResizableStateHandle,
    /// Whether the detail area's height has had its "default to 1/3 on first
    /// open" initialization: this runs only once; afterwards we keep the height
    /// the user dragged to.
    detail_height_initialized: Arc<AtomicBool>,
    /// Mouse state of the detail area's close button.
    detail_close_mouse_state: MouseStateHandle,
    /// Text selection state of the detail area (drag-selection), preserved
    /// across re-renders.
    detail_selection_handle: SelectionHandle,
    /// Text currently selected in the detail area, for Cmd/Ctrl+C copy; written
    /// by the [`SelectableArea`] callback.
    detail_selected_text: Arc<RwLock<Option<String>>>,
    /// Mouse state of the detail area's changed-file rows (for hover highlight /
    /// click to open diff), same length as the current detail's files.
    detail_file_mouse_states: Arc<Vec<MouseStateHandle>>,

    // --- Right-click context menu & write operations ---
    /// Stable position id for the panel root, so a right-click can compute the
    /// menu offset relative to the panel.
    position_id: String,
    /// The shared context-menu child view; its items dispatch `GitGraphAction`s.
    context_menu: ViewHandle<Menu<GitGraphAction>>,
    /// The right-click target whose menu is open (`None` = menu closed).
    open_menu: Option<MenuKind>,
    /// Offset (relative to the panel root) at which the open menu is anchored.
    menu_offset: Vector2F,
    /// The modal dialog currently shown (input / confirm / reset-mode).
    dialog: DialogState,
    /// Single-line editor backing the text-input dialog (reused across prompts).
    dialog_input: ViewHandle<EditorView>,
    /// Per-button mouse state for the dialog buttons (hover highlight); indexed
    /// 0..N by button position within the current dialog.
    dialog_button_mouse_states: Vec<MouseStateHandle>,
    /// True while a write op is running (reentrancy guard: blocks a second op and
    /// dims the panel).
    op_running: bool,
    /// Last write-op error, shown in a dismissable banner at the top of the panel.
    op_error: Option<String>,
    /// Mouse state of the op-error banner's dismiss button.
    op_error_dismiss_mouse_state: MouseStateHandle,
}

/// Empty layout, used when not loaded / on error.
fn empty_layout() -> GraphLayout {
    GraphLayout {
        rows: Vec::new(),
        max_lanes: 0,
    }
}

/// Decides which repo index to select once a discovery completes.
///
/// When following the anchor (cd / switch tab), priority is:
/// 1. the repo the user last manually picked for this anchor (if still present)
///    — this is what lets a tab keep its own repo choice across tab switches;
/// 2. the repo the anchor itself lives in (the directory cd'd into);
/// 3. the previously selected repo (if still present);
/// 4. the first repo.
///
/// When not following (manual refresh / scan-depth change), only the previous
/// selection (3) and the first-repo fallback (4) apply.
#[cfg(not(target_family = "wasm"))]
fn pick_selected_repo(
    repos: &[PathBuf],
    anchor: &Path,
    follow_anchor: bool,
    previous: Option<&Path>,
    saved_for_anchor: Option<&PathBuf>,
) -> Option<usize> {
    let keep_previous = || previous.and_then(|p| repos.iter().position(|r| r == p));
    let first = || (!repos.is_empty()).then_some(0);
    if follow_anchor {
        saved_for_anchor
            .and_then(|saved| repos.iter().position(|r| r == saved))
            .or_else(|| repos.iter().position(|r| anchor.starts_with(r)))
            .or_else(keep_previous)
            .or_else(first)
    } else {
        keep_previous().or_else(first)
    }
}

impl GitGraphView {
    pub(crate) fn new(ctx: &mut ViewContext<Self>) -> Self {
        // UniformList reports its current visible row range over this channel,
        // triggering auto-load when scrolled to the bottom.
        let (visible_range_sender, visible_range_receiver) = async_channel::unbounded();
        let _ = ctx.spawn_stream_local(visible_range_receiver, Self::on_visible_range, |_, _| {});

        let repo_dropdown = ctx.add_typed_action_view(Dropdown::new);
        // Shrink to the repo name's width so that, when placed at the left of
        // the top bar, it doesn't stretch out and push the refresh button off
        // the right edge.
        repo_dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_main_axis_size(MainAxisSize::Min, ctx);
        });

        // When the scan depth changes, re-discover repositories for the current
        // anchor (so the panel reflects a depth change made in settings
        // immediately).
        ctx.subscribe_to_model(&GitSettings::handle(ctx), |me, _, event, ctx| {
            if matches!(event, GitSettingsChangedEvent::GitGraphScanDepth { .. }) {
                // Changing the scan depth only re-discovers repos; keep the
                // currently selected repo instead of following the anchor.
                me.discover(false, ctx);
            }
        });

        // Right-click context menu (shared across all targets); its items
        // dispatch `GitGraphAction`s, and we clear `open_menu` when it closes.
        // Matches the Project Explorer (file tree): with
        // `prevent_interaction_with_other_elements`, a click/right-click outside
        // the open menu dismisses it (rather than immediately acting on, or
        // switching to, the element under the cursor).
        let context_menu = ctx.add_typed_action_view(|_| {
            Menu::new()
                .with_width(CONTEXT_MENU_WIDTH)
                .prevent_interaction_with_other_elements()
                .with_drop_shadow()
        });
        ctx.subscribe_to_view(&context_menu, |me, _, event, ctx| {
            if let crate::menu::Event::Close { .. } = event {
                me.open_menu = None;
                ctx.notify();
            }
        });

        // Single-line editor backing the text-input dialog. Enter submits,
        // Escape cancels.
        let dialog_input =
            ctx.add_view(|ctx| EditorView::single_line(SingleLineEditorOptions::default(), ctx));
        ctx.subscribe_to_view(&dialog_input, |me, _, event, ctx| match event {
            EditorEvent::Enter => me.submit_input(ctx),
            EditorEvent::Escape => me.cancel_dialog(ctx),
            _ => {}
        });

        Self {
            scan_anchor: None,
            repositories: Arc::new(Vec::new()),
            selected_repo: None,
            repo_dropdown,
            branches: Arc::new(Vec::new()),
            selected_branches: HashSet::new(),
            saved_branch_selections: HashMap::new(),
            saved_repo_selections: HashMap::new(),
            branch_filter_expanded: false,
            branch_filter_button_mouse_state: MouseStateHandle::default(),
            branch_select_all_mouse_state: MouseStateHandle::default(),
            branch_deselect_all_mouse_state: MouseStateHandle::default(),
            branch_mouse_states: Arc::new(Vec::new()),
            branch_scroll_state: ClippedScrollStateHandle::new(),
            commits: Arc::new(Vec::new()),
            layout: Arc::new(empty_layout()),
            state: LoadState::NoRepo,
            row_mouse_states: Arc::new(Vec::new()),
            selected: None,
            detail: DetailState::None,
            list_state: UniformListState::new(),
            commit_scroll_state: ScrollStateHandle::default(),
            detail_scroll_state: ClippedScrollStateHandle::new(),
            refresh_mouse_state: MouseStateHandle::default(),
            has_more: false,
            loading_more: false,
            visible_range_sender,
            loading_shimmer: ShimmeringTextStateHandle::new(),
            op_shimmer: ShimmeringTextStateHandle::new(),
            detail_resizable_state: resizable_state_handle(220.0),
            detail_height_initialized: Arc::new(AtomicBool::new(false)),
            detail_close_mouse_state: MouseStateHandle::default(),
            detail_selection_handle: SelectionHandle::default(),
            detail_selected_text: Arc::new(RwLock::new(None)),
            detail_file_mouse_states: Arc::new(Vec::new()),
            position_id: format!("git_graph_{}", ctx.view_id()),
            context_menu,
            open_menu: None,
            menu_offset: Vector2F::zero(),
            dialog: DialogState::None,
            dialog_input,
            dialog_button_mouse_states: (0..4).map(|_| MouseStateHandle::default()).collect(),
            op_running: false,
            op_error: None,
            op_error_dismiss_mouse_state: MouseStateHandle::default(),
        }
    }

    /// Sets the anchor directory for repository discovery; a change triggers
    /// re-discovery of repositories.
    pub(crate) fn set_working_directory(
        &mut self,
        dir: Option<PathBuf>,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.scan_anchor == dir {
            return;
        }
        self.scan_anchor = dir;
        // Working directory changed (cd / switch tab): follow, selecting the
        // repo that the current anchor belongs to.
        self.discover(true, ctx);
    }

    /// Path of the currently selected repository.
    fn current_repo_path(&self) -> Option<PathBuf> {
        self.selected_repo
            .and_then(|i| self.repositories.get(i).cloned())
    }

    /// Whether the commit graph has finished loading. Exposed for integration
    /// tests (see `crates/integration`), since `LoadState` is private.
    pub(crate) fn is_loaded(&self) -> bool {
        matches!(self.state, LoadState::Loaded)
    }

    /// Number of currently loaded commits. Exposed for integration tests.
    pub(crate) fn loaded_commit_count(&self) -> usize {
        self.commits.len()
    }

    /// Hash of the first (newest) loaded commit, if any. Exposed for integration
    /// tests to drive a write op against a real commit.
    pub(crate) fn first_commit_hash_for_test(&self) -> Option<String> {
        self.commits.first().map(|c| c.hash.clone())
    }

    /// Whether a local branch named `name` is currently known (used by
    /// integration tests to assert a branch write op took effect after reload).
    pub(crate) fn has_local_branch_for_test(&self, name: &str) -> bool {
        self.branches
            .iter()
            .any(|b| b.kind == RefKind::LocalBranch && b.display_name == name)
    }

    /// Whether the op-error banner is showing. Exposed for the integration test
    /// that drives a failing write op and asserts the banner surfaces *and*
    /// renders without panicking.
    pub(crate) fn has_op_error_for_test(&self) -> bool {
        self.op_error.is_some()
    }

    /// Scans the anchor directory and discovers all git repositories within it
    /// (asynchronously); on completion, populates the repository list and loads
    /// the selected repo.
    ///
    /// `follow_anchor` controls which repo is selected:
    /// - `true` (working directory changed / cd / switch tab): follow — prefer
    ///   the **repo containing the current anchor** (i.e. the one cd'd into);
    /// - `false` (manual refresh / scan depth change): keep the previously
    ///   selected repo.
    /// Both cases fall back to the first repo.
    /// Manual refresh (the toolbar refresh button): runs `git fetch --prune` on
    /// the current repo first — so the graph picks up new remote commits and
    /// drops branches deleted on the remote — then rediscovers + reloads. The
    /// fetch is async (never blocks the UI) and fail-soft: a repo with no
    /// remote, an offline machine, or an auth failure falls straight through to
    /// a normal local reload.
    fn refresh(&mut self, ctx: &mut ViewContext<Self>) {
        #[cfg(not(target_family = "wasm"))]
        if let Some(repo) = self.current_repo_path() {
            self.state = LoadState::Loading;
            ctx.notify();
            let expected = repo.clone();
            ctx.spawn(
                async move { super::data::fetch_remotes(&repo).await },
                move |view, _result, ctx| {
                    // Fetch result intentionally ignored (fail-soft). Only skip
                    // the reload if the user switched repos while fetching.
                    if view.current_repo_path().as_deref() == Some(expected.as_path()) {
                        view.discover(false, ctx);
                    }
                },
            );
            return;
        }

        self.discover(false, ctx);
    }

    fn discover(&mut self, follow_anchor: bool, ctx: &mut ViewContext<Self>) {
        // Remember the currently selected repo; once discovery completes,
        // `follow_anchor` decides whether to keep it or follow the anchor.
        let previous = self.current_repo_path();
        self.clear_selection();

        let Some(anchor) = self.scan_anchor.clone() else {
            self.set_repositories(Vec::new(), None, ctx);
            return;
        };

        self.state = LoadState::Loading;
        ctx.notify();

        #[cfg(not(target_family = "wasm"))]
        {
            let depth = *GitSettings::as_ref(ctx).git_graph_scan_depth as usize;
            let expected = anchor.clone();
            ctx.spawn(
                async move { super::data::discover_repositories(&anchor, depth).await },
                move |view, repos, ctx| {
                    if view.scan_anchor.as_deref() != Some(expected.as_path()) {
                        // Anchor has changed; discard the stale result.
                        return;
                    }
                    let selected = pick_selected_repo(
                        &repos,
                        &expected,
                        follow_anchor,
                        previous.as_deref(),
                        view.saved_repo_selections.get(&expected),
                    );
                    view.set_repositories(repos, selected, ctx);
                },
            );
        }
        #[cfg(target_family = "wasm")]
        {
            let _ = (anchor, previous, follow_anchor);
            self.set_repositories(Vec::new(), None, ctx);
        }
    }

    /// Applies the result of a repository discovery: updates the list and
    /// dropdown, then loads the selected repo (entering the NoRepo placeholder
    /// if none is selected).
    fn set_repositories(
        &mut self,
        repos: Vec<PathBuf>,
        selected: Option<usize>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.repositories = Arc::new(repos);
        self.selected_repo = selected;
        self.update_repo_dropdown(ctx);

        if self.selected_repo.is_some() {
            self.reload(ctx);
        } else {
            self.commits = Arc::new(Vec::new());
            self.layout = Arc::new(empty_layout());
            self.row_mouse_states = Arc::new(Vec::new());
            self.state = LoadState::NoRepo;
            ctx.notify();
        }
    }

    /// Refreshes the top repository dropdown's menu items and selection from the
    /// current repository list and selected index.
    ///
    /// Uses rich items to give the **currently selected repo row** its own
    /// distinct background color (neutral highlight `fg_overlay_4`), clearly
    /// different from other rows' hover `accent_button_color` (accent pink) —
    /// the shared [`Menu`] uses the accent family for both "selected" and
    /// "hovered" by default, which makes the two nearly the same color and the
    /// current repo hard to tell apart; here we override just that item without
    /// touching the global behavior. Long repo names are clipped with an
    /// ellipsis to keep the menu from getting too wide to read.
    fn update_repo_dropdown(&self, ctx: &mut ViewContext<Self>) {
        let repos = self.repositories.clone();
        let selected = self.selected_repo;
        let selected_bg = internal_colors::fg_overlay_4(Appearance::as_ref(ctx).theme());
        self.repo_dropdown.update(ctx, |dropdown, ctx| {
            let items: Vec<MenuItem<DropdownAction>> = repos
                .iter()
                .enumerate()
                .map(|(i, path)| {
                    // Show the directory name; the hover tooltip gives the full
                    // path (so repos with the same name can be told apart).
                    let mut item = MenuItemFields::new(repo_display_name(path))
                        .with_on_select_action(DropdownAction::select_action_and_close(
                            GitGraphAction::SelectRepository(i),
                        ))
                        .with_tooltip(path.to_string_lossy().to_string())
                        .with_clip_config(ClipConfig::ellipsis());
                    if selected == Some(i) {
                        item = item.with_override_hover_background_color(selected_bg);
                    }
                    item.into_item()
                })
                .collect();
            dropdown.set_rich_items(items, ctx);
            if let Some(sel) = selected {
                dropdown.set_selected_by_index(sel, ctx);
            }
            // With a single repo there's nothing to switch to, so disable it
            // (it's only there to consistently show the current repo name).
            if repos.len() <= 1 {
                dropdown.set_disabled(ctx);
            } else {
                dropdown.set_enabled(ctx);
            }
        });
    }

    /// Switches the currently displayed repository.
    ///
    /// We don't call [`Self::update_repo_dropdown`] synchronously here: this
    /// method bubbles up **synchronously** from a dropdown item click via
    /// `dispatch_typed_action`, at which point the [`Dropdown`] view is already
    /// mutably borrowed by its own `handle_action`, so calling `.update()` on it
    /// would reentrantly borrow and crash. The header's selected state updates
    /// itself when the [`Dropdown`] receives `ItemSelected`, so we don't need to
    /// intervene; the authoritative rebuild of the list/selection happens only
    /// in the async [`Self::set_repositories`] (where there's no reentrancy).
    fn select_repository(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        if self.selected_repo == Some(index) || index >= self.repositories.len() {
            return;
        }
        self.selected_repo = Some(index);
        self.persist_repo_selection();
        self.reload(ctx);
    }

    /// Toggles a branch's visibility and reloads the graph. The overlay stays
    /// open: this method only mutates `self` state and calls `ctx.notify()` to
    /// re-render the overlay (so the check marks update along with it); it calls
    /// no child view's `update()`, so there's no reentrant-borrow crash like the
    /// one described in [`Self::select_repository`]'s comment.
    fn toggle_branch(&mut self, ref_name: &str, ctx: &mut ViewContext<Self>) {
        if !self.selected_branches.remove(ref_name) {
            self.selected_branches.insert(ref_name.to_string());
        }
        self.persist_branch_selection();
        self.load_commits(ctx);
    }

    /// Selects all branches (skips if already all selected, to avoid a needless
    /// reload).
    fn select_all_branches(&mut self, ctx: &mut ViewContext<Self>) {
        if self.branches.is_empty() || self.selected_branches.len() == self.branches.len() {
            return;
        }
        self.selected_branches = self.branches.iter().map(|b| b.ref_name.clone()).collect();
        self.persist_branch_selection();
        self.load_commits(ctx);
    }

    /// Deselects all branches (skips if already none selected).
    fn deselect_all_branches(&mut self, ctx: &mut ViewContext<Self>) {
        if self.selected_branches.is_empty() {
            return;
        }
        self.selected_branches.clear();
        self.persist_branch_selection();
        self.load_commits(ctx);
    }

    /// Persists the current branch selection back to its repo (called after the
    /// user changes the branch filter), so it can be restored per repo after
    /// switching tab / cd.
    fn persist_branch_selection(&mut self) {
        if let Some(repo) = self.current_repo_path() {
            self.saved_branch_selections
                .insert(repo, self.selected_branches.clone());
        }
    }

    /// Persists the current repo selection keyed by the anchor (the tab's
    /// working directory), so the tab restores its own choice after switching
    /// away and back instead of snapping to the repo the anchor lives in.
    fn persist_repo_selection(&mut self) {
        if let (Some(anchor), Some(repo)) = (self.scan_anchor.clone(), self.current_repo_path()) {
            self.saved_repo_selections.insert(anchor, repo);
        }
    }

    /// Clears the selection and detail (called on repo change / reload).
    fn clear_selection(&mut self) {
        self.selected = None;
        self.detail = DetailState::None;
        self.clear_detail_text_selection();
    }

    /// Clears the detail area's text selection state (called when switching
    /// commits / closing the detail, to avoid stale selection coordinates).
    fn clear_detail_text_selection(&mut self) {
        self.detail_selection_handle.clear();
        if let Ok(mut guard) = self.detail_selected_text.write() {
            *guard = None;
        }
    }

    /// Reloads the currently selected repo: first fetch the branch list (all
    /// selected by default), then load the commit graph for the selected
    /// branches. Switching repos resets the branch filter (different repos have
    /// different branches) and collapses the overlay.
    fn reload(&mut self, ctx: &mut ViewContext<Self>) {
        self.branch_filter_expanded = false;

        let Some(dir) = self.current_repo_path() else {
            self.branches = Arc::new(Vec::new());
            self.selected_branches.clear();
            self.branch_mouse_states = Arc::new(Vec::new());
            self.clear_selection();
            self.commits = Arc::new(Vec::new());
            self.layout = Arc::new(empty_layout());
            self.row_mouse_states = Arc::new(Vec::new());
            self.has_more = false;
            self.state = LoadState::NoRepo;
            ctx.notify();
            return;
        };

        self.state = LoadState::Loading;
        ctx.notify();

        #[cfg(not(target_family = "wasm"))]
        {
            // Used when the result returns to check whether the repo has been
            // switched away (the task is detached, no handle needed).
            let expected = dir.clone();
            ctx.spawn(
                async move { super::data::load_branches(&dir).await },
                move |view, result, ctx| {
                    if view.current_repo_path().as_deref() != Some(expected.as_path()) {
                        return;
                    }
                    let branches = result.unwrap_or_default();
                    let new_refs: HashSet<String> =
                        branches.iter().map(|b| b.ref_name.clone()).collect();
                    // Restore the repo's saved branch selection (intersected
                    // with the new branch list, dropping branches that have
                    // vanished); if it was never saved (first time seeing this
                    // repo), default to all selected. Then persist it back as
                    // the repo's current selection.
                    view.selected_branches = match view.saved_branch_selections.get(&expected) {
                        Some(saved) => saved.intersection(&new_refs).cloned().collect(),
                        None => new_refs,
                    };
                    view.saved_branch_selections
                        .insert(expected.clone(), view.selected_branches.clone());
                    view.branch_mouse_states = Arc::new(
                        (0..branches.len())
                            .map(|_| MouseStateHandle::default())
                            .collect(),
                    );
                    view.branches = Arc::new(branches);
                    view.load_commits(ctx);
                },
            );
        }
        #[cfg(target_family = "wasm")]
        {
            let _ = dir;
            self.state = LoadState::NoRepo;
            ctx.notify();
        }
    }

    /// The current branch filter: returns `None` when the branch list is empty
    /// (unknown / failed to load) — which falls back to `--all` to avoid an
    /// empty graph; otherwise returns the selected branch refs (which may be
    /// empty = the user deselected all branches = empty graph).
    fn branch_filter(&self) -> Option<Vec<String>> {
        if self.branches.is_empty() {
            None
        } else {
            Some(self.selected_branches.iter().cloned().collect())
        }
    }

    /// Loads the first page of the commit graph for the current repo + current
    /// branch filter (called when the branch selection changes, or after the
    /// branch list finishes loading).
    fn load_commits(&mut self, ctx: &mut ViewContext<Self>) {
        self.clear_selection();
        self.has_more = false;
        self.loading_more = false;
        // Reloading resets commits back to the first page and the scroll
        // position back to the top (the top being the newest commit).
        self.list_state.scroll_to(0);

        let Some(dir) = self.current_repo_path() else {
            self.commits = Arc::new(Vec::new());
            self.layout = Arc::new(empty_layout());
            self.row_mouse_states = Arc::new(Vec::new());
            self.state = LoadState::NoRepo;
            ctx.notify();
            return;
        };

        self.state = LoadState::Loading;
        ctx.notify();

        #[cfg(not(target_family = "wasm"))]
        {
            let expected = dir.clone();
            let filter = self.branch_filter();
            ctx.spawn(
                async move {
                    super::data::load_commit_graph(&dir, filter.as_deref(), COMMIT_PAGE_SIZE, 0)
                        .await
                },
                move |view, result, ctx| {
                    if view.current_repo_path().as_deref() != Some(expected.as_path()) {
                        // Repo has switched; discard the stale result.
                        return;
                    }
                    match result {
                        Ok(commits) => {
                            view.has_more = commits.len() == COMMIT_PAGE_SIZE;
                            view.layout = Arc::new(assign_lanes(&commits));
                            view.row_mouse_states = Arc::new(
                                (0..commits.len())
                                    .map(|_| MouseStateHandle::default())
                                    .collect(),
                            );
                            view.commits = Arc::new(commits);
                            view.state = LoadState::Loaded;
                        }
                        Err(err) => {
                            view.commits = Arc::new(Vec::new());
                            view.layout = Arc::new(empty_layout());
                            view.row_mouse_states = Arc::new(Vec::new());
                            view.has_more = false;
                            let raw = err.to_string();
                            // When the directory isn't inside any git repo,
                            // `git log` reports "not a git repository"; this
                            // isn't an error, so normalize it to the NoRepo
                            // placeholder (rather than showing the scary raw
                            // error).
                            view.state = if raw.contains("not a git repository") {
                                LoadState::NoRepo
                            } else {
                                LoadState::Error(clean_git_error(&raw))
                            };
                        }
                    }
                    ctx.notify();
                },
            );
        }
        #[cfg(target_family = "wasm")]
        {
            let _ = dir;
            self.state = LoadState::NoRepo;
            ctx.notify();
        }
    }

    /// Loads the next page of commits and appends it to the end of the list.
    fn load_more(&mut self, ctx: &mut ViewContext<Self>) {
        if self.loading_more || !self.has_more {
            return;
        }
        let Some(dir) = self.current_repo_path() else {
            return;
        };
        let skip = self.commits.len();
        self.loading_more = true;
        ctx.notify();

        #[cfg(not(target_family = "wasm"))]
        {
            let expected = dir.clone();
            let filter = self.branch_filter();
            ctx.spawn(
                async move {
                    super::data::load_commit_graph(&dir, filter.as_deref(), COMMIT_PAGE_SIZE, skip)
                        .await
                },
                move |view, result, ctx| {
                    view.loading_more = false;
                    // Discard the stale result if the repo has switched or the
                    // start offset has changed (interrupted by a reload).
                    if view.current_repo_path().as_deref() != Some(expected.as_path())
                        || view.commits.len() != skip
                    {
                        ctx.notify();
                        return;
                    }
                    match result {
                        Ok(batch) => {
                            view.has_more = batch.len() == COMMIT_PAGE_SIZE;
                            let mut combined = (*view.commits).clone();
                            combined.extend(batch);
                            view.layout = Arc::new(assign_lanes(&combined));
                            view.row_mouse_states = Arc::new(
                                (0..combined.len())
                                    .map(|_| MouseStateHandle::default())
                                    .collect(),
                            );
                            view.commits = Arc::new(combined);
                        }
                        Err(_) => {
                            view.has_more = false;
                        }
                    }
                    ctx.notify();
                },
            );
        }
        #[cfg(target_family = "wasm")]
        {
            let _ = (dir, skip);
            self.loading_more = false;
        }
    }

    /// Callback for the current visible row range reported by [`UniformList`].
    /// When the visible range approaches the list end and there are more pages,
    /// auto-loads the next page (infinite scroll). `load_more` itself guards
    /// against reentrancy and the "no more pages" case.
    fn on_visible_range(&mut self, range: Range<usize>, ctx: &mut ViewContext<Self>) {
        if range.end + LOAD_MORE_PREFETCH >= self.commits.len() {
            self.load_more(ctx);
        }
    }

    /// Selects a row and asynchronously loads its detail.
    fn select_commit(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        // Pull keyboard focus into this view so it enters the responder chain —
        // a prerequisite for the detail area's `cmdorctrl-c` →
        // [`GitGraphAction::CopySelection`] binding to fire (key bindings only
        // trigger on views in the focus chain, whereas mouse drag-selection
        // relies on hit-testing and doesn't depend on focus, hence "can select
        // but can't copy"). Selecting a commit is the necessary entry point for
        // viewing / copying detail, so bootstrapping focus here covers the
        // drag-select copy flow; focus is naturally handed back when clicking
        // into the terminal.
        ctx.focus_self();
        let Some(commit) = self.commits.get(index) else {
            return;
        };
        let hash = commit.hash.clone();
        self.selected = Some(index);
        self.detail = DetailState::Loading;
        self.clear_detail_text_selection();
        // After switching commits the detail content is replaced wholesale, so
        // reset the scroll position to the top (otherwise it would stay at the
        // previous commit's offset).
        self.detail_scroll_state.scroll_to(Pixels::zero());
        ctx.notify();

        #[cfg(not(target_family = "wasm"))]
        {
            let Some(dir) = self.current_repo_path() else {
                return;
            };
            ctx.spawn(
                async move { super::data::load_commit_detail(&dir, &hash).await },
                move |view, result, ctx| {
                    if view.selected != Some(index) {
                        // Selection has changed; discard the stale result.
                        return;
                    }
                    view.detail = match result {
                        Ok(detail) => {
                            // Prepare mouse state for each changed-file row
                            // (hover highlight / click to open diff).
                            view.detail_file_mouse_states = Arc::new(
                                (0..detail.files.len())
                                    .map(|_| MouseStateHandle::default())
                                    .collect(),
                            );
                            DetailState::Loaded(detail)
                        }
                        Err(err) => {
                            view.detail_file_mouse_states = Arc::new(Vec::new());
                            DetailState::Error(err.to_string())
                        }
                    };
                    ctx.notify();
                },
            );
        }
        #[cfg(target_family = "wasm")]
        {
            let _ = hash;
            self.detail = DetailState::None;
            ctx.notify();
        }
    }

    /// Handles clicking the `file_index`th changed file in the detail area:
    /// asynchronously loads that commit's changes to the file, and on
    /// completion emits [`GitGraphEvent::OpenCommitFileDiff`], which the upper
    /// layer opens as a read-only diff pane in the main area.
    #[cfg(not(target_family = "wasm"))]
    fn open_file_diff(&mut self, file_index: usize, ctx: &mut ViewContext<Self>) {
        let DetailState::Loaded(detail) = &self.detail else {
            return;
        };
        let Some(file) = detail.files.get(file_index) else {
            return;
        };
        let Some(commit) = self.selected.and_then(|i| self.commits.get(i)) else {
            return;
        };
        let Some(dir) = self.current_repo_path() else {
            return;
        };
        let hash = commit.hash.clone();
        let short_hash = commit.short_hash.clone();
        let path = file.path.clone();
        let load_path = path.clone();

        ctx.spawn(
            async move { super::data::load_file_diff_at_commit(&dir, &hash, &load_path).await },
            move |_view, result, ctx| match result {
                Ok(diff) => {
                    ctx.emit(GitGraphEvent::OpenCommitFileDiff {
                        repo_relative_path: path,
                        short_hash,
                        base_content: diff.base_content,
                        hunks: diff.hunks,
                    });
                }
                Err(err) => {
                    log::warn!("Failed to load commit file diff: {err}");
                }
            },
        );
    }

    /// On wasm, git data fetching isn't supported (the detail area doesn't show
    /// the file list either), so a click is a no-op.
    #[cfg(target_family = "wasm")]
    fn open_file_diff(&mut self, _file_index: usize, _ctx: &mut ViewContext<Self>) {}

    /// Renders the clickable commit list (each row = lane + text, wrapped in a
    /// [`Hoverable`] that dispatches the selection). When there are more pages, a
    /// "load more" indicator row with a pulse animation is appended at the end;
    /// scrolling to it auto-loads the next page (infinite scroll).
    fn render_commit_list(&self, appearance: &Appearance) -> Box<dyn Element> {
        let commits = self.commits.clone();
        let layout = self.layout.clone();
        let mouse_states = self.row_mouse_states.clone();
        let has_more = self.has_more;
        let shimmer = self.loading_shimmer.clone();
        let selected = self.selected;
        let commit_count = commits.len();
        let total = commit_count + usize::from(has_more);
        let position_id = self.position_id.clone();

        let list = UniformList::new(self.list_state.clone(), total, move |range, app| {
            let appearance = Appearance::as_ref(app);
            let lane_count = layout.max_lanes;
            let rows: Vec<Box<dyn Element>> = range
                .filter_map(|i| {
                    if i < commit_count {
                        let commit = commits.get(i)?;
                        let row = layout.rows.get(i)?;
                        let element =
                            render_graph_row(row, lane_count, commit, i, &position_id, appearance);
                        let state = mouse_states.get(i).cloned().unwrap_or_default();
                        let is_selected = selected == Some(i);
                        let row_position_id = position_id.clone();
                        Some(
                            // Wrap a highlight background on hover/selection
                            // (reusing the left panel list's common
                            // [`ItemHighlightState`]: faint on hover, slightly
                            // deeper when selected, switching instantly as the
                            // mouse enters/leaves).
                            Hoverable::new(state, move |mouse_state| {
                                let highlight = ItemHighlightState::new(is_selected, mouse_state);
                                let mut container = Container::new(element);
                                if let Some(bg) = highlight.background_color(appearance) {
                                    container = container.with_background_color(bg.into_solid());
                                }
                                if let Some(radius) = highlight.corner_radius() {
                                    container = container.with_corner_radius(radius);
                                }
                                container.finish()
                            })
                            .on_click(move |ctx, _, _| {
                                ctx.dispatch_typed_action(GitGraphAction::SelectCommit(i));
                            })
                            // Right-click the row (off any ref badge) opens the
                            // commit context menu.
                            .on_right_click(move |ctx, _, position| {
                                let Some(bounds) = ctx.element_position_by_id(&row_position_id)
                                else {
                                    return;
                                };
                                let offset = position - bounds.origin();
                                ctx.dispatch_typed_action(GitGraphAction::OpenMenu {
                                    kind: MenuKind::Commit { index: i },
                                    x: offset.x(),
                                    y: offset.y(),
                                });
                            })
                            .finish(),
                        )
                    } else {
                        // Last row: load-more indicator (pulse animation;
                        // scrolling here auto-triggers loading).
                        Some(render_loading_more_row(appearance, shimmer.clone()))
                    }
                })
                .collect();
            rows.into_iter()
        })
        // Report the visible row range; as it approaches the end,
        // on_visible_range triggers the auto-load.
        .notify_visible_items(self.visible_range_sender.clone())
        .finish_scrollable();

        // Wrap in a vertical scrollable with an overlay scrollbar, matching the
        // Project Explorer (file tree) list. The scrollbar floats over the right
        // edge; commit rows reserve that width on the right (see
        // `render_commit_text`) so the subject text clips before it rather than
        // under it.
        let theme = appearance.theme();
        Scrollable::vertical(
            self.commit_scroll_state.clone(),
            list,
            ScrollbarWidth::Auto,
            theme.nonactive_ui_detail().into(),
            theme.active_ui_detail().into(),
            Fill::None,
        )
        .with_overlayed_scrollbar()
        .finish()
    }

    /// Wraps the detail area in a height-draggable [`Resizable`] (drag the top
    /// bar up/down); the list takes the remaining space.
    fn render_resizable_detail(&self, appearance: &Appearance) -> Box<dyn Element> {
        // The top 5px drag bar is itself transparent; on hover / drag it's
        // highlighted with a neutral overlay color to hint that it can be
        // dragged up/down. internal_colors returns a theme::Fill, which needs
        // converting to a warpui elements::Fill.
        let dragbar_hover_color: Fill = internal_colors::fg_overlay_5(appearance.theme()).into();
        let state = self.detail_resizable_state.clone();
        let initialized = self.detail_height_initialized.clone();
        Resizable::new(
            self.detail_resizable_state.clone(),
            self.render_detail(appearance),
        )
        .with_dragbar_side(DragBarSide::Top)
        .with_dragbar_hover_color(dragbar_hover_color)
        .on_resize(move |ctx, _| {
            ctx.notify();
        })
        .with_bounds_callback(Box::new(move |window_size| {
            let min = 100.0;
            let max = (window_size.y() * 0.7).max(min);
            // The first time the detail area appears, default it to 1/3 of the
            // window height (taking effect on the first frame's layout, with no
            // flicker); afterwards keep the height the user dragged to and don't
            // override it again.
            if !initialized.swap(true, Ordering::Relaxed) {
                if let Ok(mut s) = state.lock() {
                    s.set_size((window_size.y() / 3.0).clamp(min, max));
                }
            }
            (min, max)
        }))
        .finish()
    }

    /// Renders the selected commit's detail area (with a close button at the
    /// top).
    fn render_detail(&self, appearance: &Appearance) -> Box<dyn Element> {
        let body: Box<dyn Element> = match &self.detail {
            DetailState::None => Empty::new().finish(),
            DetailState::Loading => {
                render_message("Loading commit details…".to_string(), appearance)
            }
            DetailState::Error(err) => {
                render_message(format!("Failed to load details: {err}"), appearance)
            }
            DetailState::Loaded(detail) => {
                let commit = self.selected.and_then(|i| self.commits.get(i));
                render_detail_body(
                    commit,
                    detail,
                    self.detail_scroll_state.clone(),
                    self.detail_selection_handle.clone(),
                    self.detail_selected_text.clone(),
                    &self.detail_file_mouse_states,
                    appearance,
                )
            }
        };

        let close = icon_button(
            appearance,
            Icon::X,
            false,
            self.detail_close_mouse_state.clone(),
        )
        .build()
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(GitGraphAction::CloseDetail);
        })
        .finish();

        let header = Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(text_line("Commit details".to_string(), appearance, true))
                .with_child(close)
                .finish(),
        )
        .with_horizontal_padding(12.)
        .with_vertical_padding(4.)
        .finish();

        Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(header)
            .with_child(Shrinkable::new(1.0, body).finish())
            .finish()
    }

    /// Top bar: repository dropdown + branch filter dropdown on the left, refresh
    /// button on the right.
    fn render_header(&self, appearance: &Appearance) -> Box<dyn Element> {
        // Left control group: show the repository dropdown when there are repos
        // (disabled and just showing the current repo name for a single repo),
        // and the branch filter when there are branches.
        let mut left = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
        if !self.repositories.is_empty() {
            left = left.with_child(ChildView::new(&self.repo_dropdown).finish());
        }
        if !self.branches.is_empty() {
            left = left.with_child(
                Container::new(self.render_branch_filter(appearance))
                    .with_padding_left(6.)
                    .finish(),
            );
        }

        let refresh = icon_button(
            appearance,
            Icon::Refresh,
            false,
            self.refresh_mouse_state.clone(),
        )
        .build()
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(GitGraphAction::Refresh);
        })
        .finish();

        Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(left.finish())
                .with_child(refresh)
                .finish(),
        )
        .with_horizontal_padding(12.)
        .with_vertical_padding(6.)
        .finish()
    }

    /// Branch filter control: a button plus, when expanded, an overlay anchored
    /// below the button ([`Stack`] layered with [`OffsetPositioning`]).
    fn render_branch_filter(&self, appearance: &Appearance) -> Box<dyn Element> {
        // Overlay anchor label: use [`SavePosition`] to record the button's
        // position, from which the overlay is placed directly below the button.
        let save_label = "git_graph_branch_filter".to_string();
        let button =
            SavePosition::new(self.render_branch_filter_button(appearance), &save_label).finish();
        let mut stack = Stack::new().with_child(button);
        if self.branch_filter_expanded {
            let positioning = OffsetPositioning::offset_from_save_position_element(
                save_label,
                vec2f(0., 4.),
                PositionedElementOffsetBounds::WindowByPosition,
                PositionedElementAnchor::BottomLeft,
                ChildAnchor::TopLeft,
            );
            stack.add_positioned_overlay_child(self.render_branch_popup(appearance), positioning);
        }
        stack.finish()
    }

    /// Branch filter button (shows a summary of the current selection + a
    /// dropdown chevron).
    fn render_branch_filter_button(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let label = self.branch_filter_summary();
        let expanded = self.branch_filter_expanded;
        let state = self.branch_filter_button_mouse_state.clone();
        Hoverable::new(state, move |mouse_state| {
            let chevron = ConstrainedBox::new(
                Icon::ChevronDown
                    .to_warpui_icon(theme.sub_text_color(theme.background()))
                    .finish(),
            )
            .with_width(14.)
            .with_height(14.)
            .finish();
            let row = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(
                    // Cap the max width + ellipsis so an over-long branch name
                    // is truncated rather than stretching the button (and the
                    // refresh button to its right) out.
                    ConstrainedBox::new(
                        Text::new_inline(
                            label,
                            appearance.ui_font_family(),
                            appearance.ui_font_size(),
                        )
                        .with_color(theme.foreground().into())
                        .with_clip(ClipConfig::ellipsis())
                        .finish(),
                    )
                    .with_max_width(120.)
                    .finish(),
                )
                .with_child(Container::new(chevron).with_padding_left(4.).finish())
                .finish();
            // When expanded, highlight as selected; otherwise only on hover
            // (reusing the left panel's common highlight).
            let highlight = ItemHighlightState::new(expanded, mouse_state);
            let mut container = Container::new(row)
                .with_horizontal_padding(8.)
                .with_vertical_padding(4.)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)));
            if let Some(bg) = highlight.background_color(appearance) {
                container = container.with_background_color(bg.into_solid());
            }
            container.finish()
        })
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(GitGraphAction::ToggleBranchFilter);
        })
        .finish()
    }

    /// Branch filter overlay: a scrollable list of branch checkboxes, wrapped in
    /// a [`Dismiss`] to close on clicking outside.
    fn render_branch_popup(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let mut col = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Start);
        for (i, branch) in self.branches.iter().enumerate() {
            col = col.with_child(self.render_branch_row(i, branch, appearance));
        }

        let scrollable = ClippedScrollable::vertical(
            self.branch_scroll_state.clone(),
            col.finish(),
            ScrollbarWidth::Auto,
            theme.nonactive_ui_detail().into(),
            theme.active_ui_detail().into(),
            Fill::None,
        )
        .with_overlayed_scrollbar()
        .finish();

        // The "select all / deselect all" action row is pinned at the top (it
        // doesn't scroll with the branch list), so batch toggling is one click
        // even with many branches.
        let body = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(self.render_branch_filter_actions(appearance))
            .with_child(
                ConstrainedBox::new(scrollable)
                    .with_max_height(280.)
                    .finish(),
            )
            .finish();

        let panel = Container::new(ConstrainedBox::new(body).with_width(220.).finish())
            .with_background_color(theme.background().into_solid())
            .with_border(Border::all(1.).with_border_fill(theme.outline()))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
            .with_drop_shadow(DropShadow::default())
            .with_vertical_padding(4.)
            .finish();

        Dismiss::new(panel)
            .on_dismiss(|ctx, _| {
                ctx.dispatch_typed_action(GitGraphAction::CloseBranchFilter);
            })
            .prevent_interaction_with_other_elements()
            .finish()
    }

    /// A single branch row in the overlay: a check mark (✓ when selected, an
    /// equally-sized blank placeholder for alignment when not) + the branch
    /// name; the whole row is clickable to toggle.
    fn render_branch_row(
        &self,
        index: usize,
        branch: &BranchRef,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let selected = self.selected_branches.contains(&branch.ref_name);
        let is_remote = branch.kind == RefKind::RemoteBranch;
        let display = branch.display_name.clone();
        let ref_name = branch.ref_name.clone();
        let state = self
            .branch_mouse_states
            .get(index)
            .cloned()
            .unwrap_or_default();
        Hoverable::new(state, move |mouse_state| {
            let check: Box<dyn Element> = if selected {
                ConstrainedBox::new(Icon::Check.to_warpui_icon(theme.foreground()).finish())
                    .with_width(14.)
                    .with_height(14.)
                    .finish()
            } else {
                ConstrainedBox::new(Empty::new().finish())
                    .with_width(14.)
                    .with_height(14.)
                    .finish()
            };
            // Remote branches use the secondary color to distinguish them from
            // local branches.
            let name_color = if is_remote {
                theme.sub_text_color(theme.background())
            } else {
                theme.foreground()
            };
            // The row fills the overlay width so the entire row (including the
            // blank space on the right) is a click target, not just the text.
            // The name uses Shrinkable to take the remaining width + ellipsis,
            // so an over-long branch name is truncated rather than overflowing
            // into the commit list.
            let row = Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(Container::new(check).with_padding_right(6.).finish())
                .with_child(
                    Shrinkable::new(
                        1.0,
                        Text::new_inline(
                            display,
                            appearance.ui_font_family(),
                            appearance.ui_font_size(),
                        )
                        .with_color(name_color.into())
                        .with_clip(ClipConfig::ellipsis())
                        .finish(),
                    )
                    .finish(),
                )
                .finish();
            let highlight = ItemHighlightState::new(false, mouse_state);
            let mut container = Container::new(row)
                .with_horizontal_padding(8.)
                .with_vertical_padding(4.);
            if let Some(bg) = highlight.background_color(appearance) {
                container = container.with_background_color(bg.into_solid());
            }
            container.finish()
        })
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(GitGraphAction::ToggleBranch(ref_name.clone()));
        })
        .finish()
    }

    /// The "select all / deselect all" action row at the top of the overlay.
    fn render_branch_filter_actions(&self, appearance: &Appearance) -> Box<dyn Element> {
        let select_all = self.render_branch_action_button(
            "Select all",
            GitGraphAction::SelectAllBranches,
            self.branch_select_all_mouse_state.clone(),
            appearance,
        );
        let deselect_all = self.render_branch_action_button(
            "Deselect all",
            GitGraphAction::DeselectAllBranches,
            self.branch_deselect_all_mouse_state.clone(),
            appearance,
        );
        Container::new(
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(select_all)
                .with_child(Container::new(deselect_all).with_padding_left(8.).finish())
                .finish(),
        )
        .with_horizontal_padding(4.)
        .with_vertical_padding(2.)
        .finish()
    }

    /// A small overlay action button (accent-colored text + hover highlight).
    fn render_branch_action_button(
        &self,
        label: &'static str,
        action: GitGraphAction,
        state: MouseStateHandle,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        Hoverable::new(state, move |mouse_state| {
            let mut container = Container::new(
                Text::new_inline(
                    label,
                    appearance.ui_font_family(),
                    appearance.ui_font_size(),
                )
                .with_color(theme.accent().into())
                .finish(),
            )
            .with_horizontal_padding(6.)
            .with_vertical_padding(3.)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)));
            let highlight = ItemHighlightState::new(false, mouse_state);
            if let Some(bg) = highlight.background_color(appearance) {
                container = container.with_background_color(bg.into_solid());
            }
            container.finish()
        })
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(action.clone());
        })
        .finish()
    }

    /// Summary text on the branch filter button: all selected / none selected /
    /// the branch name directly when exactly one is selected / a count
    /// otherwise.
    fn branch_filter_summary(&self) -> String {
        let total = self.branches.len();
        let selected = self.selected_branches.len().min(total);
        if selected == total {
            "All branches".to_string()
        } else if selected == 0 {
            "No branches".to_string()
        } else if selected == 1 {
            // When only one branch is selected, show its name directly — more
            // intuitive than "1/N branches".
            self.branches
                .iter()
                .find(|b| self.selected_branches.contains(&b.ref_name))
                .map(|b| b.display_name.clone())
                .unwrap_or_else(|| "1 branch".to_string())
        } else {
            format!("{selected}/{total} branches")
        }
    }

    /// Opens the context menu for `kind` at `offset` (relative to the panel
    /// root). Items are built fresh from the current commit / branch state and
    /// the [`FeatureFlag::GitGraphWrite`] flag; a stale anchor index no-ops.
    fn open_context_menu(&mut self, kind: MenuKind, offset: Vector2F, ctx: &mut ViewContext<Self>) {
        let Some(commit) = self.commits.get(kind.index()).cloned() else {
            return;
        };
        let write_enabled = FeatureFlag::GitGraphWrite.is_enabled();
        let items = build_menu(&kind, &commit, write_enabled);
        self.context_menu.update(ctx, move |menu, ctx| {
            menu.set_items(items, ctx);
            ctx.notify();
        });
        self.open_menu = Some(kind);
        self.menu_offset = offset;
        self.dialog = DialogState::None;
        ctx.focus(&self.context_menu);
        ctx.notify();
    }

    /// Begins a write op: shows a confirmation dialog when the op requires one
    /// ([`GitWriteOp::confirm_message`]), otherwise runs it immediately.
    fn begin_write_op(&mut self, op: GitWriteOp, ctx: &mut ViewContext<Self>) {
        self.open_menu = None;
        match op.confirm_message() {
            Some(message) => {
                self.dialog = DialogState::Confirm { op, message };
                ctx.notify();
            }
            None => self.run_op(op, ctx),
        }
    }

    /// Opens the text-input dialog for `kind`, pre-filling and focusing the
    /// shared editor.
    fn open_input_dialog(&mut self, kind: PromptKind, ctx: &mut ViewContext<Self>) {
        self.open_menu = None;
        let initial = kind.initial_text();
        self.dialog_input.update(ctx, |editor, ctx| {
            editor.set_buffer_text(&initial, ctx);
        });
        self.dialog = DialogState::Input(kind);
        ctx.focus(&self.dialog_input);
        ctx.notify();
    }

    /// Submits the text-input dialog: builds the op from the trimmed text (a
    /// blank entry keeps the dialog open) and runs it.
    fn submit_input(&mut self, ctx: &mut ViewContext<Self>) {
        let DialogState::Input(kind) = &self.dialog else {
            return;
        };
        let text = self
            .dialog_input
            .as_ref(ctx)
            .buffer_text(ctx)
            .trim()
            .to_string();
        if text.is_empty() {
            return;
        }
        let op = kind.clone().into_op(text);
        self.dialog = DialogState::None;
        // Pull focus off the now-hidden input editor back to the panel so the
        // view's key bindings (e.g. Cmd/Ctrl+C) keep working.
        ctx.focus_self();
        self.run_op(op, ctx);
    }

    /// Cancels any open dialog and returns focus to the panel.
    fn cancel_dialog(&mut self, ctx: &mut ViewContext<Self>) {
        self.dialog = DialogState::None;
        ctx.focus_self();
        ctx.notify();
    }

    /// Opens the OS save dialog for an archive of `rev`; the chosen path's
    /// extension selects the format, and the archive runs on confirm.
    fn begin_archive(&mut self, rev: String, suggested_name: String, ctx: &mut ViewContext<Self>) {
        self.open_menu = None;
        ctx.notify();
        let config = SaveFilePickerConfiguration::new().with_default_filename(suggested_name);
        ctx.open_save_file_picker(
            move |path_opt, view, ctx| {
                let Some(path) = path_opt else {
                    return;
                };
                let output = PathBuf::from(path);
                let format = archive_format_from_path(&output);
                view.run_op(
                    GitWriteOp::Archive {
                        rev: rev.clone(),
                        output,
                        format,
                    },
                    ctx,
                );
            },
            config,
        );
    }

    /// Runs a write op in the current repo. Guards against re-entrancy, closes
    /// any open menu/dialog, and on completion updates the view: a `git fetch
    /// --prune` reload for a remote-branch deletion (so the pruned ref
    /// disappears), a plain reload for everything else — including push, which
    /// updates the local remote-tracking ref (and may create a new one), so the
    /// graph must redraw to show it — except an archive, which only writes a file
    /// and changes nothing in the graph; failures surface in the error banner.
    fn run_op(&mut self, op: GitWriteOp, ctx: &mut ViewContext<Self>) {
        self.open_menu = None;
        self.dialog = DialogState::None;
        if self.op_running {
            return;
        }
        let Some(repo) = self.current_repo_path() else {
            return;
        };

        #[cfg(not(target_family = "wasm"))]
        {
            self.op_running = true;
            self.op_error = None;
            ctx.notify();
            let expected = repo.clone();
            ctx.spawn(
                async move { super::ops::run_write_op(&repo, &op).await.map(|()| op) },
                move |view, result, ctx| {
                    view.op_running = false;
                    // Drop the result if the user switched repos mid-flight.
                    if view.current_repo_path().as_deref() != Some(expected.as_path()) {
                        ctx.notify();
                        return;
                    }
                    match result {
                        // An archive just writes a file; nothing in the graph changed.
                        Ok(GitWriteOp::Archive { .. }) => ctx.notify(),
                        // Deleting a remote branch: fetch --prune so the dropped
                        // remote-tracking ref disappears from the graph.
                        Ok(GitWriteOp::DeleteRemoteBranch { .. }) => view.refresh(ctx),
                        // Everything else — including push, which updates (or
                        // creates) the local remote-tracking ref — reloads so the
                        // graph reflects the new ref positions.
                        Ok(_) => view.reload(ctx),
                        Err(err) => {
                            view.op_error = Some(clean_git_error(&err.to_string()));
                            ctx.notify();
                        }
                    }
                },
            );
        }
        #[cfg(target_family = "wasm")]
        {
            let _ = (op, repo);
        }
    }

    /// Dismissable banner under the header showing the last write-op error.
    /// `None` when there's no error. (A running op is shown by the centered
    /// [`Self::render_working_overlay`] scrim instead.)
    fn render_op_banner(&self, appearance: &Appearance) -> Option<Box<dyn Element>> {
        let theme = appearance.theme();
        let font = appearance.ui_font_family();
        let size = appearance.ui_font_size();

        let error = self.op_error.as_ref()?;
        let icon_color = theme.sub_text_color(theme.background());
        let message = Expanded::new(
            1.0,
            Text::new(error.clone(), font, size)
                .with_color(remove_color(appearance))
                .finish(),
        )
        .finish();
        let dismiss = Hoverable::new(self.op_error_dismiss_mouse_state.clone(), move |_| {
            // The icon must be size-constrained: an unsized `to_warpui_icon`
            // element has no intrinsic bounds and produces an infinite layout
            // rect, which trips a paint-time assertion (and aborts, since the
            // panic can't unwind out of the paint callback).
            Container::new(
                ConstrainedBox::new(Icon::X.to_warpui_icon(icon_color).finish())
                    .with_width(14.)
                    .with_height(14.)
                    .finish(),
            )
            .with_padding_left(8.)
            .finish()
        })
        .on_click(|ctx, _, _| ctx.dispatch_typed_action(GitGraphAction::DismissOpError))
        .finish();

        Some(
            Container::new(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(message)
                    .with_child(dismiss)
                    .finish(),
            )
            .with_horizontal_padding(12.)
            .with_vertical_padding(6.)
            .with_background_color(internal_colors::fg_overlay_4(theme).into_solid())
            .finish(),
        )
    }

    /// Full-panel scrim shown while a write op runs: dims the graph and centers a
    /// rounded "Working…" pill with the same shimmer animation the panel uses for
    /// "Loading more commits…". (The render layer has no rotation, so this is a
    /// shimmer sweep rather than a spinning ring.) The dim also reads as "busy /
    /// don't touch"; re-entrancy is already guarded in `run_op`.
    fn render_working_overlay(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let base_color = theme.sub_text_color(theme.background()).into_solid();
        let shimmer_color = theme.foreground().into_solid();
        let label = ShimmeringTextElement::new(
            "Working…",
            appearance.ui_font_family(),
            appearance.ui_font_size(),
            base_color,
            shimmer_color,
            ShimmerConfig::default(),
            self.op_shimmer.clone(),
        )
        .finish();
        let pill = Container::new(label)
            .with_horizontal_padding(16.)
            .with_vertical_padding(10.)
            .with_background(theme.surface_2())
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
            .with_border(Border::all(1.0).with_border_fill(theme.outline()))
            .finish();

        // Dim scrim: the panel background at high (but not full) opacity, filling
        // the panel (Align stretches to the overlay bounds) with the pill centered.
        let mut scrim_bg = theme.background().into_solid();
        scrim_bg.a = 0xB8;
        Container::new(Align::new(pill).finish())
            .with_background_color(scrim_bg)
            .finish()
    }

    /// A pill button for a dialog. `primary` gives it the accent background;
    /// others get the list hover treatment.
    fn dialog_button(
        &self,
        label: String,
        action: GitGraphAction,
        state: MouseStateHandle,
        primary: bool,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let accent: ColorU = theme.accent().into();
        Hoverable::new(state, move |mouse_state| {
            let text_color: ColorU = if primary {
                pick_foreground_color(accent)
            } else {
                theme.foreground().into()
            };
            let mut container = Container::new(
                Text::new_inline(
                    label.clone(),
                    appearance.ui_font_family(),
                    appearance.ui_font_size(),
                )
                .with_color(text_color.into())
                .finish(),
            )
            .with_horizontal_padding(12.)
            .with_vertical_padding(5.)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(5.)));
            if primary {
                container = container.with_background_color(accent);
            } else if let Some(bg) =
                ItemHighlightState::new(false, mouse_state).background_color(appearance)
            {
                container = container.with_background_color(bg.into_solid());
            }
            container.finish()
        })
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(action.clone());
        })
        .finish()
    }

    /// Renders the modal dialog card for the current [`DialogState`] (caller only
    /// invokes this when a dialog is open).
    fn render_dialog(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let font = appearance.ui_font_family();
        let size = appearance.ui_font_size();

        // Button mouse states (indexed; the reset dialog uses all four).
        let st = |i: usize| {
            self.dialog_button_mouse_states
                .get(i)
                .cloned()
                .unwrap_or_default()
        };

        let (title, body, buttons): (String, Box<dyn Element>, Vec<Box<dyn Element>>) =
            match &self.dialog {
                DialogState::None => (String::new(), Empty::new().finish(), vec![]),
                DialogState::Input(kind) => {
                    // Pad the editor (rather than fixing a height) so the text is
                    // vertically centered and not clipped by a too-short box.
                    let input = appearance
                        .ui_builder()
                        .text_input(self.dialog_input.clone())
                        .with_style(UiComponentStyles {
                            border_width: Some(1.),
                            border_color: Some(theme.outline().into()),
                            border_radius: Some(CornerRadius::with_all(Radius::Pixels(4.))),
                            padding: Some(Coords {
                                top: 7.,
                                bottom: 7.,
                                left: 8.,
                                right: 8.,
                            }),
                            ..Default::default()
                        })
                        .build()
                        .finish();
                    (
                        kind.title().to_string(),
                        input,
                        vec![
                            self.dialog_button(
                                "Cancel".to_string(),
                                GitGraphAction::CancelDialog,
                                st(0),
                                false,
                                appearance,
                            ),
                            self.dialog_button(
                                kind.title().to_string(),
                                GitGraphAction::SubmitInput,
                                st(1),
                                true,
                                appearance,
                            ),
                        ],
                    )
                }
                DialogState::Confirm { op, message } => (
                    "Confirm".to_string(),
                    dialog_message(message.clone(), appearance),
                    vec![
                        self.dialog_button(
                            "Cancel".to_string(),
                            GitGraphAction::CancelDialog,
                            st(0),
                            false,
                            appearance,
                        ),
                        self.dialog_button(
                            "Confirm".to_string(),
                            GitGraphAction::RunOp(op.clone()),
                            st(1),
                            true,
                            appearance,
                        ),
                    ],
                ),
                DialogState::ResetMode { hash } => (
                    "Reset current branch".to_string(),
                    dialog_message(
                        "Move the current branch to this commit. Soft keeps your changes \
                         staged, Mixed keeps them unstaged, Hard discards all uncommitted \
                         changes."
                            .to_string(),
                        appearance,
                    ),
                    vec![
                        self.dialog_button(
                            "Cancel".to_string(),
                            GitGraphAction::CancelDialog,
                            st(0),
                            false,
                            appearance,
                        ),
                        self.dialog_button(
                            "Soft".to_string(),
                            GitGraphAction::RunOp(GitWriteOp::Reset {
                                hash: hash.clone(),
                                mode: ResetMode::Soft,
                            }),
                            st(1),
                            false,
                            appearance,
                        ),
                        self.dialog_button(
                            "Mixed".to_string(),
                            GitGraphAction::RunOp(GitWriteOp::Reset {
                                hash: hash.clone(),
                                mode: ResetMode::Mixed,
                            }),
                            st(2),
                            false,
                            appearance,
                        ),
                        self.dialog_button(
                            "Hard".to_string(),
                            GitGraphAction::RunOp(GitWriteOp::Reset {
                                hash: hash.clone(),
                                mode: ResetMode::Hard,
                            }),
                            st(3),
                            true,
                            appearance,
                        ),
                    ],
                ),
            };

        let mut button_row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::End)
            .with_cross_axis_alignment(CrossAxisAlignment::Center);
        for (i, button) in buttons.into_iter().enumerate() {
            if i > 0 {
                button_row = button_row
                    .with_child(Container::new(button).with_padding_left(8.).finish());
            } else {
                button_row = button_row.with_child(button);
            }
        }

        let title_chars = title.chars().count();
        let title_el = Text::new_inline(title, font, size)
            .with_color(theme.foreground().into())
            .with_single_highlight(
                Highlight::new().with_properties(Properties::default().weight(Weight::Bold)),
                (0..title_chars).collect(),
            )
            .finish();

        let card = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(Container::new(title_el).with_margin_bottom(10.).finish())
            .with_child(Container::new(body).with_margin_bottom(12.).finish())
            .with_child(button_row.finish());

        ConstrainedBox::new(
            Container::new(card.finish())
                .with_padding_left(16.)
                .with_padding_right(16.)
                .with_padding_top(14.)
                .with_padding_bottom(14.)
                .with_background(theme.surface_2())
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
                .with_border(Border::all(1.0).with_border_fill(theme.outline()))
                .finish(),
        )
        .with_width(DIALOG_WIDTH)
        .finish()
    }
}

/// The name shown in the repository dropdown: the directory name (the full path
/// is provided by the tooltip).
fn repo_display_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string())
}

/// A line of plain text (single line, no wrapping).
fn text_line(text: String, appearance: &Appearance, dim: bool) -> Box<dyn Element> {
    let theme = appearance.theme();
    let color = if dim {
        theme.sub_text_color(theme.background())
    } else {
        theme.foreground()
    };
    Text::new_inline(text, appearance.ui_font_family(), appearance.ui_font_size())
        .with_color(color.into())
        .finish()
}

/// Renders the "load more" indicator row at the bottom of the list: a pulsing
/// text animation ([`ShimmeringTextElement`] self-drives its repaint within
/// paint, around 30fps); appears only when there are more pages, and scrolling
/// to it triggers the auto-load.
fn render_loading_more_row(
    appearance: &Appearance,
    shimmer: ShimmeringTextStateHandle,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let bg = theme.background();
    let base_color = theme.sub_text_color(bg).into_solid();
    let shimmer_color = theme.foreground().into_solid();
    let text = ShimmeringTextElement::new(
        "Loading more commits…",
        appearance.ui_font_family(),
        appearance.ui_font_size(),
        base_color,
        shimmer_color,
        ShimmerConfig::default(),
        shimmer,
    )
    .finish();
    Container::new(text)
        .with_horizontal_padding(12.)
        .with_vertical_padding(4.)
        .finish()
}

/// Condenses `run_git_command`'s raw error (of the form
/// `Git command failed: <stderr>, <stdout>`) into a single concise line: strips
/// the prefix, keeps only the first line, and trims trailing stray
/// commas/whitespace.
fn clean_git_error(raw: &str) -> String {
    raw.strip_prefix("Git command failed: ")
        .unwrap_or(raw)
        .lines()
        .next()
        .unwrap_or(raw)
        .trim()
        .trim_end_matches(',')
        .trim()
        .to_string()
}

/// Renders a small hint message inside the detail area (left-aligned, single
/// line; used while detail is loading / on error).
fn render_message(text: String, appearance: &Appearance) -> Box<dyn Element> {
    Container::new(text_line(text, appearance, true))
        .with_horizontal_padding(12.)
        .with_vertical_padding(8.)
        .finish()
}

/// A dimmed, **wrapping** message for the modal dialogs (unlike [`render_message`]
/// which is single-line); used so a long confirmation / reset explanation wraps
/// inside the dialog instead of being clipped.
fn dialog_message(text: String, appearance: &Appearance) -> Box<dyn Element> {
    let theme = appearance.theme();
    Text::new(text, appearance.ui_font_family(), appearance.ui_font_size())
        .with_color(theme.sub_text_color(theme.background()).into())
        .finish()
}

/// Renders a placeholder state for the whole panel: vertically + horizontally
/// centered within the remaining space, with an optional decorative icon, a
/// required title, and an optional subtitle. Used for the NoRepo / Loading /
/// Error / empty-repo "full screen" states, to keep text from cramming into the
/// top-left corner.
fn render_centered_placeholder(
    icon: Option<Icon>,
    title: String,
    subtitle: Option<String>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    // Content block: icon/title/subtitle stacked vertically, horizontally
    // centered relative to each other (default MainAxisSize::Min, shrinks to
    // content).
    let mut content = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Center);

    if let Some(icon) = icon {
        let icon_el = ConstrainedBox::new(
            icon.to_warpui_icon(theme.sub_text_color(theme.background()))
                .finish(),
        )
        .with_width(32.)
        .with_height(32.)
        .finish();
        content = content.with_child(Container::new(icon_el).with_vertical_padding(8.).finish());
    }

    content = content.with_child(
        Text::new_inline(
            title,
            appearance.ui_font_family(),
            appearance.ui_font_size(),
        )
        .with_color(theme.foreground().into())
        .finish(),
    );

    if let Some(subtitle) = subtitle {
        content = content.with_child(
            Container::new(
                Text::new(
                    subtitle,
                    appearance.ui_font_family(),
                    appearance.ui_font_size(),
                )
                .with_color(theme.sub_text_color(theme.background()).into())
                .finish(),
            )
            .with_vertical_padding(4.)
            .with_horizontal_padding(24.)
            .finish(),
        );
    }

    // Shrinkable fills the remaining space (the outer is a MainAxisSize::Max
    // column), and Align centers the content block on both axes within it —
    // which is what gives a width to center against; Flex's CrossAxisAlignment
    // alone would be ineffective since the column width only wraps the text.
    Shrinkable::new(1.0, Align::new(content.finish()).finish()).finish()
}

/// Renders a single graph row: the lane drawing on the left + the commit text on
/// the right. `index` and `position_id` let the ref badges open their context
/// menu.
fn render_graph_row(
    row: &GraphRow,
    lane_count: usize,
    commit: &CommitNode,
    index: usize,
    position_id: &str,
    appearance: &Appearance,
) -> Box<dyn Element> {
    Flex::row()
        .with_main_axis_size(MainAxisSize::Max)
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(GitGraphRowCanvas::new(row.clone(), lane_count).finish())
        .with_child(Expanded::new(1.0, render_commit_text(commit, index, position_id, appearance)).finish())
        .finish()
}

/// Renders the commit text column: short hash + ref labels + subject. `index` /
/// `position_id` thread through to the ref badges' right-click menus.
fn render_commit_text(
    commit: &CommitNode,
    index: usize,
    position_id: &str,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let font = appearance.ui_font_family();
    let size = appearance.ui_font_size();
    let dim = theme.sub_text_color(theme.background());
    let fg = theme.foreground();

    // The short hash carries its own right-click menu (copy the 7-char hash),
    // mirroring the ref badges: its handler sits above the row's commit menu so
    // a right-click landing on the hash copies exactly what's shown rather than
    // the commit menu's full 40-char hash.
    let short_hash = {
        let position_id = position_id.to_string();
        let hash_label = Container::new(
            Text::new_inline(commit.short_hash.clone(), font, size)
                .with_color(dim.into())
                .finish(),
        )
        .with_padding_right(8.)
        .finish();
        Hoverable::new(MouseStateHandle::default(), move |_| hash_label)
            .on_right_click(move |ctx, _app, position| {
                let Some(bounds) = ctx.element_position_by_id(&position_id) else {
                    return;
                };
                let offset = position - bounds.origin();
                ctx.dispatch_typed_action(GitGraphAction::OpenMenu {
                    kind: MenuKind::ShortHash { index },
                    x: offset.x(),
                    y: offset.y(),
                });
            })
            .finish()
    };

    let mut row = Flex::row()
        .with_main_axis_size(MainAxisSize::Max)
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(short_hash);

    for ref_label in &commit.refs {
        row = row.with_child(render_ref_badge(
            ref_label,
            Some((index, position_id)),
            appearance,
        ));
    }

    // The subject takes the remaining width and ellipsis-clips, so a long
    // message ends with "…" rather than overflowing under the overlay scrollbar.
    row = row.with_child(
        Expanded::new(
            1.0,
            Text::new_inline(commit.subject.clone(), font, size)
                .with_color(fg.into())
                .with_clip(ClipConfig::ellipsis())
                .finish(),
        )
        .finish(),
    );

    // Reserve the overlay scrollbar's width on the right (it floats over the
    // content) so the clipped subject stays clear of it.
    Container::new(row.finish())
        .with_padding_left(6.)
        .with_padding_right(ScrollbarWidth::Auto.as_f32() + 6.)
        .finish()
}

/// Badge color for a ref label (by kind).
fn ref_badge_color(kind: RefKind) -> ColorU {
    match kind {
        RefKind::Head => ColorU {
            r: 0x4e,
            g: 0xc9,
            b: 0x7a,
            a: 0xff,
        }, // green
        RefKind::LocalBranch => ColorU {
            r: 0x4f,
            g: 0xc1,
            b: 0xff,
            a: 0xff,
        }, // blue
        RefKind::RemoteBranch => ColorU {
            r: 0xd6,
            g: 0x7c,
            b: 0xff,
            a: 0xff,
        }, // purple
        RefKind::Tag => ColorU {
            r: 0xe6,
            g: 0xd2,
            b: 0x4f,
            a: 0xff,
        }, // yellow
    }
}

/// Maps a ref label to the context menu it opens. The HEAD badge stands in for
/// the branch it points at — it gets the local-branch menu flagged as the
/// current branch (so self-only operations like delete / merge-into-current are
/// omitted).
fn ref_menu_kind(kind: RefKind, name: String, index: usize) -> MenuKind {
    match kind {
        RefKind::Tag => MenuKind::Tag { index, name },
        RefKind::RemoteBranch => MenuKind::RemoteBranch { index, name },
        RefKind::LocalBranch => MenuKind::LocalBranch {
            index,
            name,
            is_current: false,
        },
        RefKind::Head => MenuKind::LocalBranch {
            index,
            name,
            is_current: true,
        },
    }
}

/// Renders a single ref label badge. Most refs use a "ghost pill" (rounded,
/// semi-transparent background + same-colored text). The current branch (HEAD)
/// instead uses a "filled pill" — a solid background with contrasting text — so
/// it stands out from every other ref at a glance.
///
/// When `menu_target` is `Some((row_index, panel_position_id))` the badge is
/// right-clickable, opening the tag/branch context menu for that ref; the detail
/// area passes `None` (its badges are display-only).
fn render_ref_badge(
    label: &RefLabel,
    menu_target: Option<(usize, &str)>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let color = ref_badge_color(label.kind);
    let is_current = label.kind == RefKind::Head;
    let (bg, text_color) = if is_current {
        (color, pick_foreground_color(color))
    } else {
        (ColorU { a: 0x33, ..color }, color)
    };
    let badge = Container::new(
        Text::new_inline(
            label.name.clone(),
            appearance.ui_font_family(),
            appearance.ui_font_size(),
        )
        .with_color(text_color.into())
        .finish(),
    )
    .with_background_color(bg)
    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(3.)))
    .with_horizontal_padding(5.)
    .with_vertical_padding(1.)
    .finish();

    let inner = Container::new(badge).with_padding_right(4.).finish();

    match menu_target {
        Some((index, position_id)) => {
            let position_id = position_id.to_string();
            let menu_kind = ref_menu_kind(label.kind, label.name.clone(), index);
            Hoverable::new(MouseStateHandle::default(), move |_| inner)
                .on_right_click(move |ctx, _app, position| {
                    let Some(bounds) = ctx.element_position_by_id(&position_id) else {
                        return;
                    };
                    let offset = position - bounds.origin();
                    ctx.dispatch_typed_action(GitGraphAction::OpenMenu {
                        kind: menu_kind.clone(),
                        x: offset.x(),
                        y: offset.y(),
                    });
                })
                .finish()
        }
        None => inner,
    }
}

/// Converts a Unix-seconds timestamp into a relative-time string (just now /
/// N minutes ago / N hours ago / N days ago / N months ago / N years ago).
/// Computed against the system's current time; a negative diff (e.g. from a
/// clock that's been set back) falls back to "just now".
fn relative_time(unix_secs: i64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(unix_secs);
    let diff = now - unix_secs;
    match diff {
        i64::MIN..=59 => "just now".to_string(),
        60..=3_599 => {
            let n = diff / 60;
            format!("{n} minute{} ago", if n == 1 { "" } else { "s" })
        }
        3_600..=86_399 => {
            let n = diff / 3_600;
            format!("{n} hour{} ago", if n == 1 { "" } else { "s" })
        }
        86_400..=2_591_999 => {
            let n = diff / 86_400;
            format!("{n} day{} ago", if n == 1 { "" } else { "s" })
        }
        2_592_000..=31_103_999 => {
            let n = diff / 2_592_000;
            format!("{n} month{} ago", if n == 1 { "" } else { "s" })
        }
        _ => {
            let n = diff / 31_536_000;
            format!("{n} year{} ago", if n == 1 { "" } else { "s" })
        }
    }
}

/// Takes the body of the commit's full message (`%B`) with the first line (shown
/// as the title) removed: the first line is usually followed by a blank line,
/// which is removed too, then trailing whitespace is trimmed. Returns an empty
/// string when there's no body.
fn detail_message_body(message: &str) -> String {
    match message.trim_end().split_once('\n') {
        Some((_subject, rest)) => rest.trim_start_matches('\n').trim_end().to_string(),
        None => String::new(),
    }
}

/// Renders a pair of red/green add/delete counts: `+N` in the addition color,
/// `-N` in the deletion color (the same colors as the diff editor opened on
/// click). `add_width` / `del_width` are the character widths to right-align the
/// numbers by digit count — file rows pass this commit's global max digit count,
/// which, with the monospace font, aligns every row's `+` and `-` into columns;
/// single-row cases (e.g. the summary) can just pass their own digit counts.
fn render_diff_counts(
    additions: u32,
    deletions: u32,
    add_width: usize,
    del_width: usize,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let font = appearance.monospace_font_family();
    let size = appearance.ui_font_size();
    Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(
            Text::new_inline(format!("+{:>add_width$}", additions), font, size)
                .with_color(add_color(appearance))
                .finish(),
        )
        .with_child(
            Container::new(
                Text::new_inline(format!("-{:>del_width$}", deletions), font, size)
                    .with_color(remove_color(appearance))
                    .finish(),
            )
            .with_padding_left(8.)
            .finish(),
        )
        .finish()
}

/// Renders the body of the detail area: the layered commit metadata (subject /
/// author · time / (committer) / short hash / body) + ref badges + changed-file
/// area.
///
/// The metadata segments build a visual hierarchy (bold subject, dimmed author
/// and hash) but are carried in a single [`Text`] (using char-range highlights
/// for the levels) wrapped in one [`SelectableArea`] — a single Text is what
/// makes its drag-select copy work reliably; splitting into multiple Texts would
/// break cross-segment selection so a drag-select couldn't be copied. The ref
/// badges and file area sit outside the SelectableArea and aren't part of the
/// selection.
fn render_detail_body(
    commit: Option<&CommitNode>,
    detail: &CommitDetail,
    scroll_state: ClippedScrollStateHandle,
    selection_handle: SelectionHandle,
    selected_text: Arc<RwLock<Option<String>>>,
    file_mouse_states: &[MouseStateHandle],
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let font = appearance.ui_font_family();
    let size = appearance.ui_font_size();
    let fg: ColorU = theme.foreground().into();
    let dim: ColorU = theme.sub_text_color(theme.background()).into();

    // ---- Selectable metadata: subject (bold) / author · time / (committer) /
    // full hash / body ----
    // Carried in a single [`Text`], using char-range highlights for the levels.
    // A single Text is the prerequisite for [`SelectableArea`]'s drag-select
    // copy to work reliably — splitting into multiple Texts would break
    // cross-segment selection, so a drag-select couldn't be copied.
    let subject = commit.map(|c| c.subject.clone()).unwrap_or_else(|| {
        detail
            .message
            .lines()
            .next()
            .unwrap_or_default()
            .to_string()
    });
    let mut meta_text = subject;
    let subject_chars = meta_text.chars().count();

    // Dimmed segment: author · time / (committer · time) / full hash.
    let mut dim_range: Option<Range<usize>> = None;
    if let Some(c) = commit {
        meta_text.push_str("\n\n");
        let start = meta_text.chars().count();
        meta_text.push_str(&format!(
            "{} · {}",
            c.author_name,
            relative_time(c.author_time)
        ));
        // Add a line only when the committer differs from the author
        // (cherry-pick / rebase / amend, etc.).
        if detail.committer_name != c.author_name {
            meta_text.push_str(&format!(
                "\ncommitted by {} · {}",
                detail.committer_name,
                relative_time(detail.committer_time)
            ));
        }
        meta_text.push('\n');
        meta_text.push_str(&c.hash);
        dim_range = Some(start..meta_text.chars().count());
    }

    // Body: the full message with the first line (used as the subject) removed;
    // if empty, append nothing.
    let body = detail_message_body(&detail.message);
    if !body.is_empty() {
        meta_text.push_str("\n\n");
        meta_text.push_str(&body);
    }

    let mut meta = Text::new(meta_text, font, size)
        .with_color(fg)
        .with_selectable(true)
        .with_single_highlight(
            Highlight::new().with_properties(Properties::default().weight(Weight::Bold)),
            (0..subject_chars).collect(),
        );
    if let Some(range) = dim_range {
        meta = meta
            .with_single_highlight(Highlight::new().with_foreground_color(dim), range.collect());
    }
    let selectable_meta = SelectableArea::new(
        selection_handle,
        move |args, ctx, _| {
            // When the selection goes from "none" to "some", pull focus back
            // into this view: selection relies on hit-testing and doesn't move
            // focus, so if focus has since left (e.g. pasting the last copied
            // content into an editor, clicking elsewhere), not refocusing would
            // keep later Cmd/Ctrl+C from reaching CopySelection (showing up as
            // "the first copy works, later ones don't").
            let was_empty = selected_text.read().map(|g| g.is_none()).unwrap_or(true);
            if was_empty && args.selection.is_some() {
                ctx.dispatch_typed_action(GitGraphAction::FocusPanel);
            }
            if let Ok(mut guard) = selected_text.write() {
                *guard = args.selection;
            }
        },
        meta.finish(),
    )
    .finish();

    // ---- File area: top divider line + summary (N files changed / total
    // additions and deletions) + file rows ----
    let total_add: u32 = detail.files.iter().map(|f| f.additions).sum();
    let total_del: u32 = detail.files.iter().map(|f| f.deletions).sum();
    // File rows' +/- are right-aligned to the max digit count within this
    // commit, forming columns across rows; the summary is its own row and can
    // just use the totals' own digit counts.
    let add_width = detail
        .files
        .iter()
        .map(|f| f.additions.to_string().len())
        .max()
        .unwrap_or(1);
    let del_width = detail
        .files
        .iter()
        .map(|f| f.deletions.to_string().len())
        .max()
        .unwrap_or(1);
    let summary = Flex::row()
        .with_main_axis_size(MainAxisSize::Max)
        .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(
            Text::new_inline(format!("{} files changed", detail.files.len()), font, size)
                .with_color(dim)
                .finish(),
        )
        .with_child(render_diff_counts(
            total_add,
            total_del,
            total_add.to_string().len(),
            total_del.to_string().len(),
            appearance,
        ))
        .finish();

    // Don't virtualize the file list: a single commit has a limited number of
    // files, and putting the info and files in the same scroll region is what
    // lets a long commit message be scrolled through together with the files.
    let mut files_col = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Start)
        .with_child(Container::new(summary).with_vertical_padding(4.).finish());
    for (index, file) in detail.files.iter().enumerate() {
        // Mouse states are the same length as files; if missing, fall back to a
        // default with no hover highlight (clicking still works).
        let mouse_state = file_mouse_states.get(index).cloned().unwrap_or_default();
        files_col = files_col.with_child(render_file_row(
            index,
            file,
            mouse_state,
            add_width,
            del_width,
            appearance,
        ));
    }
    let files_section = Container::new(files_col.finish())
        .with_border(Border::top(1.).with_border_fill(theme.outline()))
        .with_margin_top(10.)
        .with_padding_top(8.)
        .finish();

    let mut content = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_child(
            Container::new(selectable_meta)
                .with_vertical_padding(6.)
                .finish(),
        );
    // Ref badges (when a branch / tag / HEAD points at this commit): in the
    // narrow panel they take their own row, placed between the metadata and the
    // file area.
    if let Some(c) = commit {
        if !c.refs.is_empty() {
            let mut chips = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
            for ref_label in &c.refs {
                chips = chips.with_child(render_ref_badge(ref_label, None, appearance));
            }
            content = content.with_child(
                Container::new(chips.finish())
                    .with_padding_bottom(4.)
                    .finish(),
            );
        }
    }
    content = content.with_child(files_section);

    // The overlay scrollbar sits 8px in from the content's right edge, so leave
    // a right padding of "scrollbar width + breathing room" for the content;
    // otherwise it would cover the file rows' right-aligned `-N` deletion counts
    // or leave the numbers pressed up against the scrollbar.
    let content = Container::new(content.finish())
        .with_padding_right(ScrollbarWidth::Auto.as_f32() + 6.)
        .finish();
    let scrollable = ClippedScrollable::vertical(
        scroll_state,
        content,
        ScrollbarWidth::Auto,
        theme.nonactive_ui_detail().into(),
        theme.active_ui_detail().into(),
        Fill::None,
    )
    .with_overlayed_scrollbar()
    .finish();

    Container::new(scrollable)
        .with_horizontal_padding(12.)
        .with_vertical_padding(8.)
        .finish()
}

/// Renders a clickable changed-file row: the path (directory dimmed, file name
/// brightened) + red/green `+adds -dels` on the right. Highlights on hover; a
/// click opens a read-only diff pane in the main area.
fn render_file_row(
    index: usize,
    file: &ChangedFile,
    mouse_state: MouseStateHandle,
    add_width: usize,
    del_width: usize,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let font = appearance.ui_font_family();
    let size = appearance.ui_font_size();
    let theme = appearance.theme();
    let fg: ColorU = theme.foreground().into();
    let dim: ColorU = theme.sub_text_color(theme.background()).into();

    // The whole path is dimmed, with only the file name (the last segment)
    // brightened to the foreground color, establishing a "directory / file name"
    // hierarchy; when too narrow, clip from the left (keeping the more
    // informative file name), consistent with the file search row.
    let path = file.path.clone();
    let basename_byte = path.rfind('/').map(|i| i + 1).unwrap_or(0);
    let basename_char_start = path[..basename_byte].chars().count();
    let total_chars = path.chars().count();
    let path_text = Text::new_inline(path, font, size)
        .with_color(dim)
        .with_single_highlight(
            Highlight::new().with_foreground_color(fg),
            (basename_char_start..total_chars).collect(),
        )
        .with_clip(ClipConfig {
            direction: ClipDirection::Start,
            style: ClipStyle::Ellipsis,
        })
        .finish();

    let row = Flex::row()
        .with_main_axis_size(MainAxisSize::Max)
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(Expanded::new(1.0, path_text).finish())
        .with_child(
            Container::new(render_diff_counts(
                file.additions,
                file.deletions,
                add_width,
                del_width,
                appearance,
            ))
            .with_padding_left(8.)
            .finish(),
        )
        .finish();

    // Hover highlight: reuse the list's common [`ItemHighlightState`] (file rows
    // have no "selected" state, only a hover-based background switch).
    Hoverable::new(mouse_state, move |mouse_state| {
        let highlight = ItemHighlightState::new(false, mouse_state);
        let mut container = Container::new(row).with_vertical_padding(2.);
        if let Some(bg) = highlight.background_color(appearance) {
            container = container.with_background_color(bg.into_solid());
        }
        if let Some(radius) = highlight.corner_radius() {
            container = container.with_corner_radius(radius);
        }
        container.finish()
    })
    .on_click(move |ctx, _, _| {
        ctx.dispatch_typed_action(GitGraphAction::OpenFileDiff(index));
    })
    .finish()
}

impl Entity for GitGraphView {
    type Event = GitGraphEvent;
}

impl TypedActionView for GitGraphView {
    type Action = GitGraphAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            GitGraphAction::SelectCommit(index) => self.select_commit(*index, ctx),
            GitGraphAction::SelectRepository(index) => self.select_repository(*index, ctx),
            GitGraphAction::ToggleBranchFilter => {
                self.branch_filter_expanded = !self.branch_filter_expanded;
                ctx.notify();
            }
            GitGraphAction::CloseBranchFilter => {
                self.branch_filter_expanded = false;
                ctx.notify();
            }
            GitGraphAction::ToggleBranch(ref_name) => self.toggle_branch(ref_name, ctx),
            GitGraphAction::SelectAllBranches => self.select_all_branches(ctx),
            GitGraphAction::DeselectAllBranches => self.deselect_all_branches(ctx),
            // Manual refresh: fetch + rescan repos (the user may have
            // added/removed subrepos) while keeping the current repo *and* its
            // branch selection — reload restores the saved selection, only
            // dropping branches that no longer exist after the fetch.
            GitGraphAction::Refresh => self.refresh(ctx),
            GitGraphAction::CloseDetail => {
                self.clear_selection();
                ctx.notify();
            }
            GitGraphAction::CopySelection => {
                let text = self
                    .detail_selected_text
                    .read()
                    .ok()
                    .and_then(|guard| guard.clone())
                    .filter(|t| !t.is_empty());
                if let Some(text) = text {
                    ctx.clipboard().write(ClipboardContent::plain_text(text));
                }
            }
            GitGraphAction::FocusPanel => ctx.focus_self(),
            GitGraphAction::OpenFileDiff(index) => self.open_file_diff(*index, ctx),
            GitGraphAction::OpenMenu { kind, x, y } => {
                self.open_context_menu(kind.clone(), vec2f(*x, *y), ctx)
            }
            GitGraphAction::CopyToClipboard(text) => {
                ctx.clipboard().write(ClipboardContent::plain_text(text.clone()));
            }
            GitGraphAction::PromptInput(kind) => self.open_input_dialog(kind.clone(), ctx),
            GitGraphAction::SubmitInput => self.submit_input(ctx),
            GitGraphAction::PromptResetMode { hash } => {
                self.open_menu = None;
                self.dialog = DialogState::ResetMode { hash: hash.clone() };
                ctx.notify();
            }
            GitGraphAction::BeginWriteOp(op) => self.begin_write_op(op.clone(), ctx),
            GitGraphAction::RunOp(op) => self.run_op(op.clone(), ctx),
            GitGraphAction::BeginArchive {
                rev,
                suggested_name,
            } => self.begin_archive(rev.clone(), suggested_name.clone(), ctx),
            GitGraphAction::CancelDialog => self.cancel_dialog(ctx),
            GitGraphAction::DismissOpError => {
                self.op_error = None;
                ctx.notify();
            }
        }
    }
}

impl View for GitGraphView {
    fn ui_name() -> &'static str {
        "GitGraphView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        // A single vertical column directly takes the panel's bounded height; a
        // Shrinkable factor distributes space between the list and the detail
        // (nesting two MainAxisSize::Max layers would feed the inner one an
        // unbounded constraint and panic).
        let mut column = Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::Start)
            .with_cross_axis_alignment(CrossAxisAlignment::Start);

        // Show the top bar (repository dropdown + refresh button) when there's
        // an anchor directory.
        if self.scan_anchor.is_some() {
            column = column.with_child(self.render_header(appearance));
        }

        // A write op in progress / its error surfaces in a banner under the
        // header, leaving the graph itself intact.
        if let Some(banner) = self.render_op_banner(appearance) {
            column = column.with_child(banner);
        }

        column = match &self.state {
            LoadState::NoRepo => column.with_child(render_centered_placeholder(
                Some(Icon::GitBranch),
                "Not a Git repository".to_string(),
                None,
                appearance,
            )),
            LoadState::Loading => column.with_child(render_centered_placeholder(
                None,
                "Loading commit history…".to_string(),
                None,
                appearance,
            )),
            LoadState::Error(err) => column.with_child(render_centered_placeholder(
                None,
                "Failed to load git history".to_string(),
                Some(err.clone()),
                appearance,
            )),
            LoadState::Loaded if self.commits.is_empty() => column.with_child(
                render_centered_placeholder(None, "No commits yet".to_string(), None, appearance),
            ),
            LoadState::Loaded if self.selected.is_some() => column
                // The list uses Expanded to fill the remaining space above
                // (pushing the detail area to the bottom); the detail area's
                // height is draggable (top drag bar). Expanded rather than
                // Shrinkable: with few commits, Shrinkable would only shrink to
                // the content height, leaving the list and detail crammed at the
                // top with empty space below and the detail's drag misaligned.
                .with_child(Expanded::new(1.0, self.render_commit_list(appearance)).finish())
                .with_child(self.render_resizable_detail(appearance)),
            LoadState::Loaded => {
                column.with_child(Expanded::new(1.0, self.render_commit_list(appearance)).finish())
            }
        };

        // The panel root carries a stable position id so right-clicks can place
        // the context menu relative to it.
        let content = SavePosition::new(column.finish(), &self.position_id).finish();
        let mut stack = Stack::new();
        stack.add_child(content);

        // Context menu, anchored at the click offset (relative to the panel).
        if self.open_menu.is_some() {
            stack.add_positioned_overlay_child(
                ChildView::new(&self.context_menu).finish(),
                OffsetPositioning::offset_from_save_position_element(
                    self.position_id.clone(),
                    self.menu_offset,
                    PositionedElementOffsetBounds::ParentByPosition,
                    PositionedElementAnchor::TopLeft,
                    ChildAnchor::TopLeft,
                ),
            );
        }

        // Modal dialog (input / confirm / reset-mode), centered over the panel,
        // with a click-outside scrim that cancels.
        if !matches!(self.dialog, DialogState::None) {
            stack.add_positioned_overlay_child(
                Dismiss::new(Align::new(self.render_dialog(appearance)).finish())
                    .on_dismiss(|ctx, _app| {
                        ctx.dispatch_typed_action(GitGraphAction::CancelDialog)
                    })
                    .finish(),
                OffsetPositioning::offset_from_save_position_element(
                    self.position_id.clone(),
                    vec2f(0., 0.),
                    PositionedElementOffsetBounds::ParentByPosition,
                    PositionedElementAnchor::Center,
                    ChildAnchor::Center,
                ),
            );
        }

        // While a write op runs, dim the panel with a centered "Working…" scrim.
        // Added as a non-positioned child so it's constrained to the Stack (i.e.
        // the panel) size and fills only the panel — a positioned overlay child
        // would instead be sized to the whole window and dim the entire app.
        if self.op_running {
            stack.add_child(self.render_working_overlay(appearance));
        }

        stack.finish()
    }
}

#[cfg(test)]
#[path = "view_tests.rs"]
mod tests;
