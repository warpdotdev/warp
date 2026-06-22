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
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_channel::Sender;
use pathfinder_color::ColorU;
#[cfg(feature = "local_fs")]
use repo_metadata::repositories::{DetectedRepositories, RepoDetectionSource};
#[cfg(feature = "local_fs")]
use repo_metadata::repository::SubscriberId;
#[cfg(feature = "local_fs")]
use repo_metadata::Repository;
use warp_core::ui::color::pick_foreground_color;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::Icon;
use warpui::clipboard::ClipboardContent;
use warpui::elements::shimmering_text::{
    ShimmerConfig, ShimmeringTextElement, ShimmeringTextStateHandle,
};
use warpui::elements::{
    resizable_state_handle, Align, Border, ChildAnchor, ChildView, ClippedScrollStateHandle,
    ClippedScrollable, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Dismiss,
    DragBarSide, Element, Empty, Expanded, Fill, Flex, Highlight, Hoverable, MainAxisAlignment,
    MainAxisSize, MouseStateHandle, OffsetPositioning, ParentAnchor, ParentElement,
    ParentOffsetBounds, PositionedElementAnchor, PositionedElementOffsetBounds, Radius, Resizable,
    ResizableStateHandle, SavePosition, ScrollStateHandle, Scrollable, ScrollableElement,
    ScrollbarWidth, SelectableArea, SelectionHandle, Shrinkable, Stack, Text, UniformList,
    UniformListState,
};
use warpui::fonts::{Properties, Weight};
use warpui::geometry::vector::{vec2f, Vector2F};
use warpui::keymap::macros::id;
use warpui::keymap::FixedBinding;
use warpui::platform::SaveFilePickerConfiguration;
use warpui::scene::DropShadow;
use warpui::text_layout::ClipConfig;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::units::Pixels;
#[cfg(feature = "local_fs")]
use warpui::ModelHandle;
use warpui::{AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle};

#[cfg(feature = "local_fs")]
use super::auto_refresh;
use super::data::{BranchRef, ChangedFile, CommitDetail, CommitNode, RefKind, RefLabel};
use super::layout::{build_layout, GraphLayout, GraphRow};
use super::menu::{build_menu, MenuKind, PromptKind, DEFAULT_PUSH_REMOTE};
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
use crate::view_components::dropdown::{Dropdown, DropdownAction, DropdownEvent};

/// Number of commits loaded per page.
const COMMIT_PAGE_SIZE: usize = 200;

/// Per-depth horizontal indent (px) of rows in the detail area's file tree
/// (matches the Project Explorer's folder indent).
const FILE_TREE_INDENT: f32 = 16.;
/// Size (px) of the disclosure chevron / type icon columns on each file-tree row.
const FILE_TREE_ICON_SIZE: f32 = 16.;

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
    /// Select the synthetic "uncommitted changes" row and load its diff detail.
    SelectUncommitted,
    /// Switch to the Nth repository in the discovered list (dispatched by the
    /// top dropdown when there are multiple repos).
    SelectRepository(usize),
    /// Expand/collapse the branch filter overlay.
    ToggleBranchFilter,
    /// Close the branch filter overlay (when clicking outside it).
    CloseBranchFilter,
    /// Toggle visibility of a branch ref (the value is the full ref, e.g.
    /// `refs/heads/main`). Selecting a branch while in "Show All" mode switches
    /// out of it (the selection set becomes non-empty).
    ToggleBranch(String),
    /// Clear the branch selection back to "Show All" (no ref filter = every
    /// branch shown). Dispatched by the overlay's pinned "Show All" row.
    ShowAllBranches,
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
    /// Toggle a directory row in the detail area's file tree between expanded and
    /// collapsed (the value is the full directory path, e.g. `src/foo`).
    ToggleDir(String),

    // --- Right-click context menu & write operations (gated at build time by
    // [`FeatureFlag::GitGraphWrite`] for the mutating items). ---
    /// Open the context menu for a right-click target at the given offset
    /// (relative to the panel root). `x`/`y` are carried as scalars because
    /// `Vector2F` is not `PartialEq` (required by this enum).
    OpenMenu { kind: MenuKind, x: f32, y: f32 },
    /// Write `text` to the clipboard (commit hash / subject / ref name).
    CopyToClipboard(String),
    /// Open the text-input dialog to collect a name (branch / rename).
    PromptInput(PromptKind),
    /// Submit the text-input dialog: build the op from the entered text and run.
    SubmitInput,
    /// Open the Add-tag dialog (tag-name input + "Push to remote" checkbox) for
    /// the commit at `hash`. Distinct from [`PromptInput`] because the dialog
    /// owns a checkbox that chains a follow-up [`GitWriteOp::PushTag`] after the
    /// local tag is created.
    OpenAddTag { hash: String },
    /// Toggle the "Push to remote" checkbox in the open Add-tag dialog.
    ToggleAddTagPush,
    /// Submit the Add-tag dialog: build [`GitWriteOp::AddTag`] from the entered
    /// name and, when the checkbox is checked, chain a [`GitWriteOp::PushTag`]
    /// after the tag is created.
    SubmitAddTag,
    /// Open the stash dialog for the uncommitted-changes row (message input +
    /// Include-untracked checkbox).
    PromptStash,
    /// Toggle the "Include untracked" checkbox in the open stash dialog.
    ToggleStashUntracked,
    /// Submit the stash dialog: build the stash op from the entered message and
    /// the untracked toggle, then run it.
    SubmitStash,
    /// Open the reset-mode dialog (soft / mixed / hard) for the commit.
    PromptResetMode { hash: String },
    /// Open the reset dialog for the uncommitted-changes row (mixed / hard reset
    /// to HEAD, discarding working-tree / index changes; soft is a no-op here).
    PromptResetUncommitted,
    /// Run a write op, first showing a confirmation dialog when the op requires
    /// one ([`GitWriteOp::confirm_message`]).
    BeginWriteOp(GitWriteOp),
    /// Run a write op now (dispatched by the confirm dialog's "Confirm" button,
    /// the reset-mode dialog's buttons, and the archive save callback).
    RunOp(GitWriteOp),
    /// Toggle the optional checkbox in the open confirmation dialog (the force
    /// flag on push / checkout / delete branch, or "clean untracked directories").
    ToggleConfirmOption,
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
        /// How the diff pane should present the file (text diff, or a placeholder
        /// for a binary file / symlink).
        preview: crate::code::commit_diff_view::DiffPreview,
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
    /// A single-line text prompt (branch name, rename) — the entered text is
    /// read from `dialog_input` on submit.
    Input(PromptKind),
    /// The Add-tag dialog: a tag-name input (read from `dialog_input`) plus a
    /// "Push to remote" checkbox whose state is held here. When the checkbox is
    /// checked, a [`GitWriteOp::PushTag`] is chained after the local tag is
    /// created (see [`GitGraphView::pending_follow_up`]).
    AddTag { hash: String, push_after: bool },
    /// The stash dialog: a message input (read from `dialog_input`, optional) plus
    /// an Include-untracked checkbox whose state is held here.
    Stash { include_untracked: bool },
    /// A yes/no confirmation for a (typically destructive) op.
    Confirm { op: GitWriteOp, message: String },
    /// The reset-mode picker for "Reset current branch to this Commit".
    ResetMode { hash: String },
    /// The reset picker for the uncommitted-changes row: mixed / hard reset to
    /// HEAD (no commit hash, no soft option).
    ResetUncommitted,
}

/// How a (re)load positions the commit list afterward.
enum LoadAnchor {
    /// Reset to the top (newest commit) and clear the selection — used on repo
    /// switch and branch-filter changes, where the previous position is moot.
    Top,
    /// Preserve the user's place across an auto-refresh: re-select / re-anchor
    /// the list by commit hash. The selection / scroll snapshot is captured
    /// when the reload *lands* (see [`auto_refresh::capture_anchor`] /
    /// [`auto_refresh::relocate_view`]), not when it starts — a selection made
    /// while the reload is in flight must win over the pre-reload state, so
    /// the anchor carries no data of its own.
    #[cfg(feature = "local_fs")]
    Preserve,
}

/// The repository the Git Graph is currently subscribed to for auto-refresh.
#[cfg(feature = "local_fs")]
struct WatchedRepo {
    /// Root path, to detect when the selected repo changes.
    path: PathBuf,
    /// Handle used to unsubscribe (`stop_watching`) when switching away.
    repository: ModelHandle<Repository>,
    /// Subscriber id returned by `start_watching`.
    subscriber_id: SubscriberId,
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
    /// Mouse state of the branch overlay's pinned "Show All" row.
    branch_show_all_mouse_state: MouseStateHandle,
    /// Single-line editor backing the overlay's "Filter Branches…" search box.
    branch_filter_input: ViewHandle<EditorView>,
    /// Current branch-list search query (lower-cased), narrowing which branch
    /// rows the overlay shows. Mirrors `branch_filter_input`'s text; kept in
    /// state because render has no `ctx` to read the editor buffer.
    branch_filter_query: String,
    /// Per-row mouse state inside the branch overlay (for hover highlight),
    /// same length as `branches`.
    branch_mouse_states: Arc<Vec<MouseStateHandle>>,
    /// Scroll state of the branch overlay list (scrollable when there are many
    /// branches).
    branch_scroll_state: ClippedScrollStateHandle,
    /// Max height of the branch overlay's list: 1/3 of the panel's height,
    /// measured each time the overlay opens (see
    /// [`Self::dropdown_max_height`]). Kept in state because render has no
    /// `ctx` to measure the panel with.
    branch_popup_max_height: f32,
    /// Loaded commits (wrapped in `Arc` for zero-copy move into the
    /// [`UniformList`] build closure).
    commits: Arc<Vec<CommitNode>>,
    /// Per-row lane layout computed by [`build_layout`], one-to-one with
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
    /// [`Self::render_resizable_detail`] overrides it to 1/2 of the window
    /// height.
    detail_resizable_state: ResizableStateHandle,
    /// Whether the detail area's height has had its "default to 1/2 on first
    /// open" initialization: this runs only once; afterwards we keep the height
    /// the user dragged to.
    detail_height_initialized: Arc<AtomicBool>,
    /// Mouse state of the detail area's close button.
    detail_close_mouse_state: MouseStateHandle,
    /// Text selection states of the detail area's drag-selectable segments
    /// (subject / hash + body / the identity hover card), preserved across
    /// re-renders. Separate segments because the author / committer rows
    /// between the texts are hover elements rather than selectable text; a
    /// segment's selection callback clears the other handles, since once one
    /// [`SelectableArea`] handles a mouse-down the others no longer see it and
    /// would keep a stale highlight. The author and committer cards share the
    /// third handle (only one card can be open at a time).
    detail_selection_handles: [SelectionHandle; 3],
    /// Text currently selected in the detail area, tagged with the owning
    /// segment's index, for Cmd/Ctrl+C copy; written by the
    /// [`SelectableArea`] callbacks. The tag lets a segment's "no selection"
    /// callback clear only its own text: every [`SelectableArea`] that sees a
    /// mouse-up reports its (possibly absent) selection, so an untagged
    /// shared value would be wiped by whichever segment reports after the
    /// owner.
    detail_selected_text: Arc<RwLock<Option<(usize, String)>>>,
    /// Hover states of the detail area's author and committer rows and their
    /// floating identity cards, as `(row, card)` pairs: hovering the row opens
    /// the card, and hovering the card keeps it open so its `name <email>`
    /// text can be drag-selected.
    detail_author_mouse_states: (MouseStateHandle, MouseStateHandle),
    detail_committer_mouse_states: (MouseStateHandle, MouseStateHandle),
    /// Mouse state of the detail area's changed-file rows (for hover highlight /
    /// click to open diff), same length as the current detail's files.
    detail_file_mouse_states: Arc<Vec<MouseStateHandle>>,
    /// Full directory paths (e.g. `src/foo`) currently collapsed in the detail
    /// area's file tree. Empty = every directory expanded (the default). Reset
    /// whenever a different commit / the uncommitted row is selected.
    detail_collapsed_dirs: HashSet<String>,
    /// Hover mouse-state per directory row in the file tree, keyed by full
    /// directory path. Built alongside [`Self::detail_file_mouse_states`] when a
    /// commit's detail loads.
    detail_dir_mouse_states: Arc<HashMap<String, MouseStateHandle>>,

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
    /// One-shot follow-up to run after the next successful op (currently only
    /// set by [`Self::submit_add_tag`] when "Push to remote" is checked, so a
    /// successful `git tag` is followed by a `git push` of that tag). Cleared
    /// on failure to avoid leaking the queued push to an unrelated next op.
    pending_follow_up: Option<GitWriteOp>,
    /// Last git error (write op, or a failed refresh fetch), shown in a
    /// dismissable banner at the top of the panel.
    op_error: Option<String>,
    /// Mouse state of the op-error banner's dismiss button.
    op_error_dismiss_mouse_state: MouseStateHandle,

    /// Number of uncommitted changed files in the working tree (0 = clean). When
    /// > 0, the graph shows a synthetic "Uncommitted Changes (N)" row at the top.
    uncommitted_count: usize,
    /// Whether the uncommitted row is the currently selected detail target
    /// (mutually exclusive with `selected`, which holds a commit index).
    uncommitted_selected: bool,

    // --- Auto-refresh (watch the selected repo's `.git`) ---
    /// Most recent visible row range reported by the commit list; used as the
    /// scroll anchor when an auto-refresh re-reads the graph, so the view keeps
    /// the user's place instead of snapping to the top.
    #[cfg(feature = "local_fs")]
    last_visible_range: Range<usize>,
    /// The repo whose `.git` we're subscribed to (re-subscribed when the
    /// selected repo changes); `None` until the first repo is watched.
    #[cfg(feature = "local_fs")]
    watched_repo: Option<WatchedRepo>,
    /// Sender the repo subscriber pushes reload signals onto, drained by the
    /// stream spawned in [`Self::new`]; cloned into each new subscriber.
    #[cfg(feature = "local_fs")]
    auto_refresh_tx: Sender<()>,
    /// When the last auto-refresh reload was fired; the throttle clock
    /// (`None` until the first one).
    #[cfg(feature = "local_fs")]
    last_auto_refresh: Option<std::time::Instant>,
    /// Whether a catch-up reload is scheduled for the end of the current
    /// throttle window (set on the first in-window signal, consumed by the
    /// catch-up or absorbed by the next immediate refresh).
    #[cfg(feature = "local_fs")]
    auto_refresh_pending: bool,
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

/// Summary text for the branch filter button (pure; unit-tested). An empty
/// selection is "Show All"; otherwise the selected branches' display names, in
/// `branches` order (the selection set is unordered), comma-joined. The button
/// itself truncates the result with an ellipsis when it's too long.
fn branch_summary_text(branches: &[BranchRef], selected: &HashSet<String>) -> String {
    if selected.is_empty() {
        return "Show All".to_string();
    }
    branches
        .iter()
        .filter(|b| selected.contains(&b.ref_name))
        .map(|b| b.display_name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

impl GitGraphView {
    pub(crate) fn new(ctx: &mut ViewContext<Self>) -> Self {
        // UniformList reports its current visible row range over this channel,
        // triggering auto-load when scrolled to the bottom.
        let (visible_range_sender, visible_range_receiver) = async_channel::unbounded();
        let _ = ctx.spawn_stream_local(visible_range_receiver, Self::on_visible_range, |_, _| {});

        // Auto-refresh: the repo subscriber pushes reload signals here; the
        // stream drains each into a position-preserving reload.
        #[cfg(feature = "local_fs")]
        let auto_refresh_tx = {
            let (tx, rx) = async_channel::unbounded::<()>();
            let _ = ctx.spawn_stream_local(rx, Self::on_auto_refresh_signal, |_, _| {});
            tx
        };

        let repo_dropdown = ctx.add_typed_action_view(Dropdown::new);
        // Shrink to the repo name's width so that, when placed at the left of
        // the top bar, it doesn't stretch out and push the refresh button off
        // the right edge.
        repo_dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_main_axis_size(MainAxisSize::Min, ctx);
        });
        // Cap the repo menu at 1/3 of the panel's height, re-measured on every
        // expand (the panel may have been resized since the last open).
        ctx.subscribe_to_view(&repo_dropdown, |me, _, event, ctx| {
            if matches!(event, DropdownEvent::ToggleExpanded) {
                if let Some(height) = me.dropdown_max_height(ctx) {
                    me.repo_dropdown.update(ctx, |dropdown, ctx| {
                        dropdown.set_menu_max_height(height, ctx);
                    });
                }
            }
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
        // Enter submits the currently open input-bearing dialog (plain prompt or
        // Add-tag); Escape always cancels. Stash dialog ignores Enter on purpose:
        // its message field is optional and empty-Enter would be ambiguous.
        ctx.subscribe_to_view(&dialog_input, |me, _, event, ctx| match event {
            EditorEvent::Enter => match &me.dialog {
                DialogState::Input(_) => me.submit_input(ctx),
                DialogState::AddTag { .. } => me.submit_add_tag(ctx),
                _ => {}
            },
            EditorEvent::Escape => me.cancel_dialog(ctx),
            _ => {}
        });

        // Single-line editor backing the branch overlay's filter box. Each edit
        // re-narrows the visible branch rows; Escape closes the overlay.
        let branch_filter_input = ctx.add_view(|ctx| {
            let mut editor = EditorView::single_line(SingleLineEditorOptions::default(), ctx);
            editor.set_placeholder_text("Filter Branches…", ctx);
            editor
        });
        ctx.subscribe_to_view(&branch_filter_input, |me, _, event, ctx| match event {
            EditorEvent::Edited(_) => me.on_branch_filter_query_changed(ctx),
            EditorEvent::Escape => me.close_branch_filter(ctx),
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
            branch_show_all_mouse_state: MouseStateHandle::default(),
            branch_filter_input,
            branch_filter_query: String::new(),
            branch_mouse_states: Arc::new(Vec::new()),
            branch_scroll_state: ClippedScrollStateHandle::new(),
            // Fallback before the first open ever measures the panel; replaced
            // by 1/3 of the panel height when the overlay opens.
            branch_popup_max_height: 280.,
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
            detail_selection_handles: Default::default(),
            detail_selected_text: Arc::new(RwLock::new(None)),
            detail_author_mouse_states: Default::default(),
            detail_committer_mouse_states: Default::default(),
            detail_file_mouse_states: Arc::new(Vec::new()),
            detail_collapsed_dirs: HashSet::new(),
            detail_dir_mouse_states: Arc::new(HashMap::new()),
            position_id: format!("git_graph_{}", ctx.view_id()),
            context_menu,
            open_menu: None,
            menu_offset: Vector2F::zero(),
            dialog: DialogState::None,
            dialog_input,
            dialog_button_mouse_states: (0..4).map(|_| MouseStateHandle::default()).collect(),
            op_running: false,
            pending_follow_up: None,
            op_error: None,
            op_error_dismiss_mouse_state: MouseStateHandle::default(),
            uncommitted_count: 0,
            uncommitted_selected: false,
            #[cfg(feature = "local_fs")]
            last_visible_range: 0..0,
            #[cfg(feature = "local_fs")]
            watched_repo: None,
            #[cfg(feature = "local_fs")]
            auto_refresh_tx,
            #[cfg(feature = "local_fs")]
            last_auto_refresh: None,
            #[cfg(feature = "local_fs")]
            auto_refresh_pending: false,
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
    #[cfg(feature = "integration_tests")]
    pub(crate) fn is_loaded(&self) -> bool {
        matches!(self.state, LoadState::Loaded)
    }

    /// Number of currently loaded commits. Exposed for integration tests.
    #[cfg(feature = "integration_tests")]
    pub(crate) fn loaded_commit_count(&self) -> usize {
        self.commits.len()
    }

    /// Hash of the first (newest) loaded commit, if any. Exposed for integration
    /// tests to drive a write op against a real commit.
    #[cfg(feature = "integration_tests")]
    pub(crate) fn first_commit_hash_for_test(&self) -> Option<String> {
        self.commits.first().map(|c| c.hash.clone())
    }

    /// Whether a local branch named `name` is currently known (used by
    /// integration tests to assert a branch write op took effect after reload).
    #[cfg(feature = "integration_tests")]
    pub(crate) fn has_local_branch_for_test(&self, name: &str) -> bool {
        self.branches
            .iter()
            .any(|b| b.kind == RefKind::LocalBranch && b.display_name == name)
    }

    /// Whether the op-error banner is showing. Exposed for the integration test
    /// that drives a failing write op and asserts the banner surfaces *and*
    /// renders without panicking.
    #[cfg(feature = "integration_tests")]
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
    /// drops branches deleted on the remote — then reloads the graph in place,
    /// keeping the selection, scroll position, and open detail (a failed
    /// graph falls back to a full repo re-discovery instead). The fetch is
    /// async (never blocks the UI) and fail-soft: a repo with no remote, an
    /// offline machine, or an auth failure falls straight through to a normal
    /// local reload.
    fn refresh(&mut self, ctx: &mut ViewContext<Self>) {
        #[cfg(not(target_family = "wasm"))]
        if let Some(repo) = self.current_repo_path() {
            // Captured before the state flips to Loading: only the failure
            // states (Error / NoRepo) need the full re-discovery below. A
            // Loading graph still holds the previous view — e.g. a second
            // refresh click while the first fetch is in flight — so it
            // preserves too rather than resetting.
            #[cfg(feature = "local_fs")]
            let had_view = !matches!(self.state, LoadState::Error(_) | LoadState::NoRepo);
            self.state = LoadState::Loading;
            // Clear any stale fetch error so a previously-failed refresh doesn't
            // keep showing the banner once a later fetch reaches the remote.
            self.op_error = None;
            ctx.notify();
            let expected = repo.clone();
            ctx.spawn(
                async move { super::data::fetch_remotes(&repo).await },
                move |view, result, ctx| {
                    // Ignore the result if the user switched repos mid-fetch.
                    if view.current_repo_path().as_deref() != Some(expected.as_path()) {
                        return;
                    }
                    // Fail-soft: a failed fetch (unreachable remote / auth /
                    // timeout) still reloads the local graph, but we surface a
                    // banner so the user knows the remote wasn't reached rather
                    // than silently showing stale data.
                    if result.is_err() {
                        view.op_error =
                            Some("Couldn't reach remote — showing local graph.".to_string());
                    }
                    // Reload in place — keeping the selection, scroll
                    // position, and open detail — rather than going through
                    // repo re-discovery, which resets the view. Only a failed
                    // graph (Error / NoRepo) rediscovers, as the recovery
                    // path.
                    #[cfg(feature = "local_fs")]
                    if had_view {
                        view.reload_in_place(ctx);
                        return;
                    }
                    view.discover(false, ctx);
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
            self.reload(LoadAnchor::Top, ctx);
        } else {
            self.enter_no_repo_state(ctx);
        }
        // Re-point the auto-refresh watch at the now-selected repo (or drop it).
        #[cfg(feature = "local_fs")]
        self.sync_auto_refresh_subscription(ctx);
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
        self.reload(LoadAnchor::Top, ctx);
        #[cfg(feature = "local_fs")]
        self.sync_auto_refresh_subscription(ctx);
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
        self.load_commits(LoadAnchor::Top, ctx);
    }

    /// Clears the branch selection back to "Show All" (an empty selection set =
    /// no ref filter = `--all`). Skips the reload if already showing all.
    fn show_all_branches(&mut self, ctx: &mut ViewContext<Self>) {
        if self.selected_branches.is_empty() {
            return;
        }
        self.selected_branches.clear();
        self.persist_branch_selection();
        self.load_commits(LoadAnchor::Top, ctx);
    }

    /// Re-reads the filter box into `branch_filter_query` (lower-cased so the
    /// match is case-insensitive) and re-renders the overlay so the branch rows
    /// narrow as the user types.
    fn on_branch_filter_query_changed(&mut self, ctx: &mut ViewContext<Self>) {
        self.branch_filter_query = self
            .branch_filter_input
            .as_ref(ctx)
            .buffer_text(ctx)
            .trim()
            .to_lowercase();
        ctx.notify();
    }

    /// Max list height for the header's dropdowns (repository menu + branch
    /// overlay): 1/3 of the panel's height, read from the panel root's saved
    /// position (one frame stale at worst). `None` until the panel has
    /// rendered a frame — which can't happen from a dropdown click, since the
    /// click needs a rendered panel; callers then keep their previous height.
    fn dropdown_max_height(&self, ctx: &mut ViewContext<Self>) -> Option<f32> {
        ctx.element_position_by_id(&self.position_id)
            .map(|bounds| bounds.height() / 3.0)
    }

    /// Collapses the branch overlay, clears its filter, and returns focus to the
    /// panel (so the view's key bindings keep working). Shared by the outside-
    /// click dismiss and the filter box's Escape key.
    fn close_branch_filter(&mut self, ctx: &mut ViewContext<Self>) {
        self.branch_filter_expanded = false;
        self.clear_branch_filter_query(ctx);
        ctx.focus_self();
        ctx.notify();
    }

    /// Resets the filter box to empty (text + cached query), so the overlay
    /// opens fresh next time.
    fn clear_branch_filter_query(&mut self, ctx: &mut ViewContext<Self>) {
        self.branch_filter_query.clear();
        self.branch_filter_input.update(ctx, |editor, ctx| {
            editor.set_buffer_text("", ctx);
        });
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
        self.uncommitted_selected = false;
        self.detail = DetailState::None;
        self.clear_detail_text_selection();
    }

    /// Clears the detail area's text selection state (called when switching
    /// commits / closing the detail, to avoid stale selection coordinates).
    fn clear_detail_text_selection(&mut self) {
        for handle in &self.detail_selection_handles {
            handle.clear();
        }
        if let Ok(mut guard) = self.detail_selected_text.write() {
            *guard = None;
        }
    }

    /// Drains an auto-refresh signal through a trailing-edge throttle
    /// ([`auto_refresh::throttle_signal`]): the first signal after a quiet
    /// period reloads the graph immediately, while signals inside the cooldown
    /// window collapse into one catch-up reload at the window's end. Signals
    /// arriving before the first load are ignored (the load itself reads the
    /// latest state).
    #[cfg(feature = "local_fs")]
    fn on_auto_refresh_signal(&mut self, _: (), ctx: &mut ViewContext<Self>) {
        match auto_refresh::throttle_signal(
            self.last_auto_refresh.map(|at| at.elapsed()),
            self.auto_refresh_pending,
            auto_refresh::AUTO_REFRESH_MIN_INTERVAL,
        ) {
            auto_refresh::ThrottleDecision::RefreshNow => {
                if !matches!(self.state, LoadState::Loaded) {
                    return;
                }
                self.reload_in_place(ctx);
            }
            auto_refresh::ThrottleDecision::Defer(delay) => {
                self.auto_refresh_pending = true;
                ctx.spawn(
                    async move {
                        warpui::r#async::Timer::after(delay).await;
                    },
                    |view, _, ctx| view.fire_pending_auto_refresh(ctx),
                );
            }
            auto_refresh::ThrottleDecision::AlreadyDeferred => {}
        }
    }

    /// Fires the catch-up reload scheduled at the end of a throttle window,
    /// unless its pending mark was already absorbed by an immediate refresh.
    #[cfg(feature = "local_fs")]
    fn fire_pending_auto_refresh(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.auto_refresh_pending {
            return;
        }
        self.auto_refresh_pending = false;
        if !matches!(self.state, LoadState::Loaded) {
            return;
        }
        self.reload_in_place(ctx);
    }

    /// Reloads the graph in place — keeping the user's selection, scroll
    /// position, and open detail (re-anchored by commit hash, see
    /// [`auto_refresh::relocate_view`]) — and stamps the throttle clock.
    /// Shared by auto-refresh and the manual refresh button; stamping the
    /// clock also lets the throttle absorb the watcher signals a manual
    /// fetch triggers itself.
    #[cfg(feature = "local_fs")]
    fn reload_in_place(&mut self, ctx: &mut ViewContext<Self>) {
        self.last_auto_refresh = Some(std::time::Instant::now());
        // This refresh covers any catch-up still scheduled for the current
        // window; absorb it so the timer's callback becomes a no-op.
        self.auto_refresh_pending = false;
        self.reload(LoadAnchor::Preserve, ctx);
    }

    /// Re-points the auto-refresh watch at the currently selected repo: detects
    /// it (registering it with `repo_metadata` if it wasn't already watched),
    /// then subscribes to its `.git` changes. No-op when the watched repo hasn't
    /// changed; tears down the previous subscription on a switch.
    #[cfg(feature = "local_fs")]
    fn sync_auto_refresh_subscription(&mut self, ctx: &mut ViewContext<Self>) {
        let current = self.current_repo_path();
        if self.watched_repo.as_ref().map(|w| w.path.as_path()) == current.as_deref() {
            return;
        }
        if let Some(previous) = self.watched_repo.take() {
            previous.repository.update(ctx, |repo, ctx| {
                repo.stop_watching(previous.subscriber_id, ctx);
            });
        }
        let Some(repo_path) = current else {
            return;
        };
        let Some(repo_path_str) = repo_path.to_str().map(str::to_owned) else {
            return;
        };
        // Ensure the repo is detected + watched by `repo_metadata`, then grab its
        // handle and subscribe.
        let detect = DetectedRepositories::handle(ctx).update(ctx, |repos, ctx| {
            repos.detect_possible_local_git_repo(
                &repo_path_str,
                RepoDetectionSource::GitGraphPanel,
                ctx,
            )
        });
        ctx.spawn(detect, move |view, detected, ctx| {
            // The selected repo may have changed (or already been subscribed by
            // a racing sync) while detection ran.
            if view.current_repo_path().as_deref() != Some(repo_path.as_path())
                || view.watched_repo.is_some()
            {
                return;
            }
            let Some(detected) = detected else {
                return;
            };
            let Some(repository) =
                DetectedRepositories::as_ref(ctx).get_local_watched_repo_for_path(&detected, ctx)
            else {
                return;
            };
            let signal_tx = view.auto_refresh_tx.clone();
            let start = repository.update(ctx, |repo, ctx| {
                repo.start_watching(
                    Box::new(auto_refresh::GitGraphRepositorySubscriber { signal_tx }),
                    ctx,
                )
            });
            view.watched_repo = Some(WatchedRepo {
                path: repo_path.clone(),
                repository: repository.clone(),
                subscriber_id: start.subscriber_id,
            });
            // Tear the subscription back down if registration fails, so a later
            // sync can retry.
            ctx.spawn(start.registration_future, move |view, result, ctx| {
                if result.is_err() {
                    if let Some(w) = view.watched_repo.take() {
                        w.repository.update(ctx, |repo, ctx| {
                            repo.stop_watching(w.subscriber_id, ctx);
                        });
                    }
                }
            });
        });
    }

    /// Reloads the currently selected repo: first fetch the branch list
    /// (defaulting to "Show All"), then load the commit graph. Switching repos
    /// resets the branch filter (different repos have different branches) and
    /// collapses the overlay.
    /// Resets the panel to the "no repository" placeholder: clears the commit
    /// graph, the branch list (so the header's branch filter disappears — it's
    /// meaningless without a repo), and any selection. Shared by the two paths
    /// that land on NoRepo — a discovery that finds no repo
    /// ([`Self::set_repositories`]) and a reload with no selected repo
    /// ([`Self::reload`]) — so they can't drift apart.
    fn enter_no_repo_state(&mut self, ctx: &mut ViewContext<Self>) {
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
    }

    fn reload(&mut self, anchor: LoadAnchor, ctx: &mut ViewContext<Self>) {
        // An auto-refresh keeps the branch overlay as-is; a manual repo/branch
        // change collapses it and clears its search box.
        #[cfg(feature = "local_fs")]
        let preserve = matches!(anchor, LoadAnchor::Preserve);
        #[cfg(not(feature = "local_fs"))]
        let preserve = false;
        if !preserve {
            self.branch_filter_expanded = false;
            self.clear_branch_filter_query(ctx);
        }

        let Some(dir) = self.current_repo_path() else {
            self.enter_no_repo_state(ctx);
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
                    // repo), default to an empty set = "Show All". Then persist
                    // it back as the repo's current selection.
                    view.selected_branches = match view.saved_branch_selections.get(&expected) {
                        Some(saved) => saved.intersection(&new_refs).cloned().collect(),
                        None => HashSet::new(),
                    };
                    view.saved_branch_selections
                        .insert(expected.clone(), view.selected_branches.clone());
                    view.branch_mouse_states = Arc::new(
                        (0..branches.len())
                            .map(|_| MouseStateHandle::default())
                            .collect(),
                    );
                    view.branches = Arc::new(branches);
                    view.load_commits(anchor, ctx);
                },
            );
        }
        #[cfg(target_family = "wasm")]
        {
            let _ = (dir, anchor);
            self.state = LoadState::NoRepo;
            ctx.notify();
        }
    }

    /// The current branch filter: an empty selection means "Show All", which
    /// returns `None` to fall back to `--all` (every ref). This also covers the
    /// case where the branch list hasn't loaded yet (selection still empty).
    /// A non-empty selection returns exactly those branch refs.
    fn branch_filter(&self) -> Option<Vec<String>> {
        if self.selected_branches.is_empty() {
            None
        } else {
            Some(self.selected_branches.iter().cloned().collect())
        }
    }

    /// Loads the first page of the commit graph for the current repo + current
    /// branch filter (called when the branch selection changes, or after the
    /// branch list finishes loading).
    fn load_commits(&mut self, anchor: LoadAnchor, ctx: &mut ViewContext<Self>) {
        // `Top` resets the selection and scrolls back to the newest commit;
        // `Preserve` (auto-refresh) keeps them and re-anchors by hash once the
        // new page lands.
        #[cfg(feature = "local_fs")]
        let preserve = matches!(anchor, LoadAnchor::Preserve);
        #[cfg(not(feature = "local_fs"))]
        let preserve = false;
        if !preserve {
            self.clear_selection();
            self.list_state.scroll_to(0);
        }
        self.has_more = false;
        self.loading_more = false;

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
                    let commits = super::data::load_commit_graph(
                        &dir,
                        filter.as_deref(),
                        COMMIT_PAGE_SIZE,
                        0,
                    )
                    .await?;
                    // `has_more` is decided by the raw page size, before stashes
                    // are mixed in (they don't count toward pagination).
                    let has_more = commits.len() == COMMIT_PAGE_SIZE;
                    // Inject stashes as graph nodes, ordered into the page by time.
                    let stashes = super::data::load_stashes(&dir).await.unwrap_or_default();
                    let commits = super::data::merge_stashes(commits, stashes);
                    // Bundle the cheap status query so the uncommitted row lands
                    // together with the first page (no second flash).
                    let uncommitted = super::data::load_working_tree_status(&dir)
                        .await
                        .unwrap_or(0);
                    Ok::<_, anyhow::Error>((commits, has_more, uncommitted))
                },
                move |view, result, ctx| {
                    #[cfg(not(feature = "local_fs"))]
                    let _ = &anchor;
                    if view.current_repo_path().as_deref() != Some(expected.as_path()) {
                        // Repo has switched; discard the stale result.
                        return;
                    }
                    match result {
                        Ok((commits, has_more, uncommitted)) => {
                            // Snapshot the user's place from the outgoing list
                            // NOW — at landing time — so a selection made while
                            // this reload was in flight is what gets restored
                            // (a start-time snapshot would roll it back; e.g.
                            // quick refresh ×2 then clicking another commit).
                            #[cfg(feature = "local_fs")]
                            let (selected_hash, anchor_hash) =
                                if matches!(anchor, LoadAnchor::Preserve) {
                                    auto_refresh::capture_anchor(
                                        &view.commits,
                                        view.selected,
                                        view.last_visible_range.start,
                                        view.uncommitted_count > 0,
                                    )
                                } else {
                                    (None, None)
                                };
                            view.has_more = has_more;
                            view.uncommitted_count = uncommitted;
                            view.layout = Arc::new(build_layout(&commits, uncommitted > 0));
                            view.row_mouse_states = Arc::new(
                                (0..view.layout.rows.len())
                                    .map(|_| MouseStateHandle::default())
                                    .collect(),
                            );
                            view.commits = Arc::new(commits);
                            view.state = LoadState::Loaded;
                            // Auto-refresh: restore the user's place in the new
                            // list by hash (see `auto_refresh::relocate_view`).
                            #[cfg(feature = "local_fs")]
                            if matches!(anchor, LoadAnchor::Preserve) {
                                let placement = auto_refresh::relocate_view(
                                    &view.commits,
                                    selected_hash.as_deref(),
                                    anchor_hash.as_deref(),
                                );
                                view.selected = placement.selected;
                                // Offset the scroll target past the synthetic
                                // uncommitted row (row 0) when present.
                                let offset = usize::from(view.uncommitted_count > 0);
                                view.list_state.scroll_to(placement.scroll_to + offset);
                                match auto_refresh::detail_refresh_after_reload(
                                    view.uncommitted_selected,
                                    view.uncommitted_count,
                                    placement.selected,
                                ) {
                                    auto_refresh::DetailRefresh::Keep => {}
                                    auto_refresh::DetailRefresh::RefreshUncommitted => {
                                        view.refresh_uncommitted_detail(ctx);
                                    }
                                    // The detail's target is gone (commit
                                    // amended away / tree became clean): drop
                                    // the now-stale detail.
                                    auto_refresh::DetailRefresh::Clear => {
                                        view.uncommitted_selected = false;
                                        view.detail = DetailState::None;
                                    }
                                }
                            }
                        }
                        Err(err) => {
                            view.commits = Arc::new(Vec::new());
                            view.layout = Arc::new(empty_layout());
                            view.row_mouse_states = Arc::new(Vec::new());
                            view.has_more = false;
                            view.uncommitted_count = 0;
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
            let _ = (dir, anchor);
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
        // Stash nodes are injected client-side and aren't part of `git log`'s
        // output, so they must not count toward the pagination skip.
        let skip = self
            .commits
            .iter()
            .filter(|c| !super::data::is_stash_node(c))
            .count();
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
                    // start offset has changed (interrupted by a reload). Count
                    // real commits only, matching how `skip` was computed.
                    let real_count = view
                        .commits
                        .iter()
                        .filter(|c| !super::data::is_stash_node(c))
                        .count();
                    if view.current_repo_path().as_deref() != Some(expected.as_path())
                        || real_count != skip
                    {
                        ctx.notify();
                        return;
                    }
                    match result {
                        Ok(batch) => {
                            view.has_more = batch.len() == COMMIT_PAGE_SIZE;
                            let mut combined = (*view.commits).clone();
                            combined.extend(batch);
                            view.layout =
                                Arc::new(build_layout(&combined, view.uncommitted_count > 0));
                            view.row_mouse_states = Arc::new(
                                (0..view.layout.rows.len())
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
        // Remembered as the auto-refresh scroll anchor (local_fs only).
        #[cfg(feature = "local_fs")]
        {
            self.last_visible_range = range.clone();
        }
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
        self.uncommitted_selected = false;
        self.detail = DetailState::Loading;
        self.clear_detail_text_selection();
        // A new commit has its own file set, so start its tree fully expanded.
        self.detail_collapsed_dirs.clear();
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
            let loaded_hash = hash.clone();
            ctx.spawn(
                async move { super::data::load_commit_detail(&dir, &hash).await },
                move |view, result, ctx| {
                    // Stale check by hash, not index: a reload landing while
                    // this detail loads may shift the commit's index when it
                    // re-anchors, but the same commit keeps its in-flight
                    // detail (an index compare would misjudge it as stale and
                    // leave the pane stuck on Loading).
                    let still_selected = view
                        .selected
                        .and_then(|i| view.commits.get(i))
                        .is_some_and(|c| c.hash == loaded_hash);
                    if !still_selected {
                        // Selection has changed; discard the stale result.
                        return;
                    }
                    view.detail = match result {
                        Ok(detail) => {
                            view.rebuild_detail_mouse_states(&detail.files);
                            DetailState::Loaded(detail)
                        }
                        Err(err) => {
                            view.clear_detail_mouse_states();
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

    /// Selects the synthetic "uncommitted changes" row and asynchronously loads
    /// its working-tree-vs-HEAD detail (changed file list).
    fn select_uncommitted(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.focus_self();
        self.selected = None;
        self.uncommitted_selected = true;
        self.detail = DetailState::Loading;
        self.clear_detail_text_selection();
        // A fresh file set starts its tree fully expanded.
        self.detail_collapsed_dirs.clear();
        self.detail_scroll_state.scroll_to(Pixels::zero());
        ctx.notify();

        #[cfg(not(target_family = "wasm"))]
        self.refresh_uncommitted_detail(ctx);
        #[cfg(target_family = "wasm")]
        {
            self.detail = DetailState::None;
            ctx.notify();
        }
    }

    /// (Re-)loads the uncommitted detail without touching focus / scroll /
    /// collapse state: the data half of [`Self::select_uncommitted`], also used
    /// by the auto-refresh reload to keep an open uncommitted detail in sync
    /// with the working tree instead of dropping it.
    #[cfg(not(target_family = "wasm"))]
    fn refresh_uncommitted_detail(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(dir) = self.current_repo_path() else {
            return;
        };
        ctx.spawn(
            async move { super::data::load_uncommitted_detail(&dir).await },
            move |view, result, ctx| {
                if !view.uncommitted_selected {
                    // Selection has changed; discard the stale result.
                    return;
                }
                view.detail = match result {
                    Ok(detail) => {
                        view.rebuild_detail_mouse_states(&detail.files);
                        DetailState::Loaded(detail)
                    }
                    Err(err) => {
                        view.clear_detail_mouse_states();
                        DetailState::Error(err.to_string())
                    }
                };
                ctx.notify();
            },
        );
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
        let Some(dir) = self.current_repo_path() else {
            return;
        };
        let path = file.path.clone();

        // The uncommitted row diffs the working tree against HEAD; a commit row
        // diffs the commit against its parent.
        if self.uncommitted_selected {
            let load_path = path.clone();
            ctx.spawn(
                async move { super::data::load_uncommitted_file_diff(&dir, &load_path).await },
                move |_view, result, ctx| match result {
                    Ok(diff) => {
                        ctx.emit(GitGraphEvent::OpenCommitFileDiff {
                            repo_relative_path: path,
                            short_hash: "working tree".to_string(),
                            base_content: diff.base_content,
                            hunks: diff.hunks,
                            preview: diff.preview,
                        });
                    }
                    Err(err) => {
                        log::warn!("Failed to load uncommitted file diff: {err}");
                    }
                },
            );
            return;
        }

        let Some(commit) = self.selected.and_then(|i| self.commits.get(i)) else {
            return;
        };
        let hash = commit.hash.clone();
        let short_hash = commit.short_hash.clone();
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
                        preview: diff.preview,
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

    /// Rebuilds the detail area's hover mouse-states for a freshly loaded file
    /// set: one per file row (indexed parallel to `files`) plus one per
    /// directory row in the file tree (keyed by full directory path).
    fn rebuild_detail_mouse_states(&mut self, files: &[ChangedFile]) {
        self.detail_file_mouse_states = Arc::new(
            (0..files.len())
                .map(|_| MouseStateHandle::default())
                .collect(),
        );
        self.detail_dir_mouse_states = Arc::new(
            super::file_tree::all_dir_paths(files)
                .into_iter()
                .map(|path| (path, MouseStateHandle::default()))
                .collect(),
        );
    }

    /// Clears the detail area's mouse-states (used when the detail load failed,
    /// so there are no rows to hover).
    fn clear_detail_mouse_states(&mut self) {
        self.detail_file_mouse_states = Arc::new(Vec::new());
        self.detail_dir_mouse_states = Arc::new(HashMap::new());
    }

    /// Toggles a file-tree directory between expanded and collapsed (membership
    /// in [`Self::detail_collapsed_dirs`]).
    fn toggle_dir(&mut self, path: String, ctx: &mut ViewContext<Self>) {
        if !self.detail_collapsed_dirs.remove(&path) {
            self.detail_collapsed_dirs.insert(path);
        }
        ctx.notify();
    }

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
        let uncommitted_count = self.uncommitted_count;
        let uncommitted_selected = self.uncommitted_selected;
        // The synthetic uncommitted row, when present, is layout row 0; commit
        // rows shift down by `offset`.
        let offset = usize::from(uncommitted_count > 0);
        let row_count = layout.rows.len();
        let total = row_count + usize::from(has_more);
        let position_id = self.position_id.clone();

        let list = UniformList::new(self.list_state.clone(), total, move |range, app| {
            let appearance = Appearance::as_ref(app);
            let lane_count = layout.max_lanes;
            let rows: Vec<Box<dyn Element>> = range
                .filter_map(|i| {
                    if i >= row_count {
                        // Last row: load-more indicator (pulse animation;
                        // scrolling here auto-triggers loading).
                        return Some(render_loading_more_row(appearance, shimmer.clone()));
                    }
                    let row = layout.rows.get(i)?;
                    let state = mouse_states.get(i).cloned().unwrap_or_default();
                    // Synthetic "uncommitted changes" row (hollow node); not
                    // selectable yet, but highlights on hover.
                    if offset == 1 && i == 0 {
                        let element =
                            render_uncommitted_row(row, lane_count, uncommitted_count, appearance);
                        let row_position_id = position_id.clone();
                        return Some(
                            Hoverable::new(state, move |mouse_state| {
                                let highlight =
                                    ItemHighlightState::new(uncommitted_selected, mouse_state);
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
                                ctx.dispatch_typed_action(GitGraphAction::SelectUncommitted);
                            })
                            // Right-click opens the working-tree menu (clean
                            // untracked files).
                            .on_right_click(move |ctx, _, position| {
                                let Some(bounds) = ctx.element_position_by_id(&row_position_id)
                                else {
                                    return;
                                };
                                let menu_offset = position - bounds.origin();
                                ctx.dispatch_typed_action(GitGraphAction::OpenMenu {
                                    kind: MenuKind::Uncommitted,
                                    x: menu_offset.x(),
                                    y: menu_offset.y(),
                                });
                            })
                            .finish(),
                        );
                    }
                    let commit_idx = i - offset;
                    let commit = commits.get(commit_idx)?;
                    let element = render_graph_row(
                        row,
                        lane_count,
                        commit,
                        commit_idx,
                        &position_id,
                        appearance,
                    );
                    let is_selected = selected == Some(commit_idx);
                    let row_position_id = position_id.clone();
                    // A stash node carries a stash@{n} label; right-clicking
                    // anywhere on its row opens the stash menu (matching the
                    // badge), not the commit menu whose ops don't apply here.
                    let stash_selector = commit
                        .refs
                        .iter()
                        .find(|r| r.kind == RefKind::Stash)
                        .map(|r| r.name.clone());
                    Some(
                        // Wrap a highlight background on hover/selection (reusing
                        // the left panel list's common [`ItemHighlightState`]:
                        // faint on hover, deeper when selected).
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
                            ctx.dispatch_typed_action(GitGraphAction::SelectCommit(commit_idx));
                        })
                        // Right-click the row (off any ref badge) opens the
                        // commit menu — or the stash menu for a stash row.
                        .on_right_click(move |ctx, _, position| {
                            let Some(bounds) = ctx.element_position_by_id(&row_position_id) else {
                                return;
                            };
                            let menu_offset = position - bounds.origin();
                            let kind = match &stash_selector {
                                Some(name) => MenuKind::Stash {
                                    index: commit_idx,
                                    name: name.clone(),
                                },
                                None => MenuKind::Commit { index: commit_idx },
                            };
                            ctx.dispatch_typed_action(GitGraphAction::OpenMenu {
                                kind,
                                x: menu_offset.x(),
                                y: menu_offset.y(),
                            });
                        })
                        .finish(),
                    )
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
            // The first time the detail area appears, default it to 1/2 of the
            // window height (taking effect on the first frame's layout, with no
            // flicker); afterwards keep the height the user dragged to and don't
            // override it again.
            if !initialized.swap(true, Ordering::Relaxed) {
                if let Ok(mut s) = state.lock() {
                    s.set_size((window_size.y() / 2.0).clamp(min, max));
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
                    &self.detail_selection_handles,
                    self.detail_selected_text.clone(),
                    self.detail_author_mouse_states.clone(),
                    self.detail_committer_mouse_states.clone(),
                    &self.detail_file_mouse_states,
                    &self.detail_dir_mouse_states,
                    &self.detail_collapsed_dirs,
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
                .with_child(text_line(
                    if self.uncommitted_selected {
                        "Uncommitted changes".to_string()
                    } else {
                        "Commit details".to_string()
                    },
                    appearance,
                    true,
                ))
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
                    // Cap the max width + ellipsis so a long summary (a long
                    // branch name, or a comma-joined multi-selection) is
                    // truncated rather than stretching the button (and the
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
                    .with_max_width(200.)
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

    /// Branch filter overlay: a pinned "Filter Branches…" search box atop a
    /// scrollable list whose first row is "Show All" (resets to every branch)
    /// followed by the branch checkboxes narrowed by the search query. Wrapped
    /// in a [`Dismiss`] to close on clicking outside.
    fn render_branch_popup(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();

        // "Show All" is pinned as the first list row and never filtered out (it
        // resets the selection, it isn't a branch). The branch rows below it are
        // narrowed by a case-insensitive substring match on the display name.
        let mut col = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(self.render_show_all_row(appearance));
        for (i, branch) in self.branches.iter().enumerate() {
            if self.branch_filter_query.is_empty()
                || branch
                    .display_name
                    .to_lowercase()
                    .contains(&self.branch_filter_query)
            {
                col = col.with_child(self.render_branch_row(i, branch, appearance));
            }
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

        // The search box is pinned at the top (it doesn't scroll with the list),
        // so it stays reachable however far the branch list is scrolled.
        let body = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(self.render_branch_filter_input(appearance))
            .with_child(
                ConstrainedBox::new(scrollable)
                    .with_max_height(self.branch_popup_max_height)
                    .finish(),
            )
            .finish();

        let panel = Container::new(ConstrainedBox::new(body).with_width(240.).finish())
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

    /// The pinned "Filter Branches…" search box at the top of the overlay
    /// (backed by `branch_filter_input`; its edits drive [`Self::
    /// on_branch_filter_query_changed`]).
    fn render_branch_filter_input(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let input = appearance
            .ui_builder()
            .text_input(self.branch_filter_input.clone())
            .with_style(UiComponentStyles {
                border_width: Some(1.),
                border_color: Some(theme.outline().into()),
                border_radius: Some(CornerRadius::with_all(Radius::Pixels(4.))),
                padding: Some(Coords {
                    top: 5.,
                    bottom: 5.,
                    left: 8.,
                    right: 8.,
                }),
                ..Default::default()
            })
            .build()
            .finish();
        Container::new(input)
            .with_horizontal_padding(6.)
            .with_vertical_padding(4.)
            .finish()
    }

    /// The pinned "Show All" row: checked when no specific branch is selected
    /// (an empty selection = every branch shown); clicking it clears the
    /// selection back to that state.
    fn render_show_all_row(&self, appearance: &Appearance) -> Box<dyn Element> {
        self.render_check_row(
            self.selected_branches.is_empty(),
            "Show All".to_string(),
            false,
            self.branch_show_all_mouse_state.clone(),
            GitGraphAction::ShowAllBranches,
            appearance,
        )
    }

    /// A single branch row in the overlay: delegates to [`Self::render_check_row`]
    /// with the branch's check state, name, and toggle action.
    fn render_branch_row(
        &self,
        index: usize,
        branch: &BranchRef,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let state = self
            .branch_mouse_states
            .get(index)
            .cloned()
            .unwrap_or_default();
        self.render_check_row(
            self.selected_branches.contains(&branch.ref_name),
            branch.display_name.clone(),
            branch.kind == RefKind::RemoteBranch,
            state,
            GitGraphAction::ToggleBranch(branch.ref_name.clone()),
            appearance,
        )
    }

    /// A check row in the overlay: a check mark (✓ when `selected`, an
    /// equally-sized blank placeholder for alignment otherwise) + a label; the
    /// whole row fills the overlay width and is clickable to dispatch `action`.
    /// `is_remote` dims the label to the secondary color. Shared by the "Show
    /// All" row and each branch row.
    fn render_check_row(
        &self,
        selected: bool,
        label: String,
        is_remote: bool,
        state: MouseStateHandle,
        action: GitGraphAction,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
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
            // The label uses Shrinkable to take the remaining width + ellipsis,
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
                            label,
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
            ctx.dispatch_typed_action(action.clone());
        })
        .finish()
    }

    /// Summary text on the branch filter button: "Show All" by default,
    /// otherwise the selected branches' names comma-joined (see
    /// [`branch_summary_text`]).
    fn branch_filter_summary(&self) -> String {
        branch_summary_text(&self.branches, &self.selected_branches)
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
        // A kind with nothing to offer (e.g. the uncommitted row when writing is
        // disabled) opens no menu rather than an empty box.
        if items.is_empty() {
            return;
        }
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

    /// Flips the optional flag on the op in the open confirmation dialog (driven
    /// by the dialog's checkbox). A no-op when no confirm dialog is open or the
    /// op has no such option.
    fn toggle_confirm_option(&mut self, ctx: &mut ViewContext<Self>) {
        if let DialogState::Confirm { op, .. } = &mut self.dialog {
            if let Some(checked) = op.option_state() {
                *op = op.clone().with_option(!checked);
                ctx.notify();
            }
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

    /// Opens the Add-tag dialog at `hash`: clears the shared input editor,
    /// defaults the "Push to remote" checkbox off, and focuses the input.
    fn open_add_tag_dialog(&mut self, hash: String, ctx: &mut ViewContext<Self>) {
        self.open_menu = None;
        self.dialog_input.update(ctx, |editor, ctx| {
            editor.set_buffer_text("", ctx);
        });
        self.dialog = DialogState::AddTag {
            hash,
            push_after: false,
        };
        ctx.focus(&self.dialog_input);
        ctx.notify();
    }

    /// Flips the "Push to remote" checkbox in the open Add-tag dialog. A no-op
    /// when no Add-tag dialog is open.
    fn toggle_add_tag_push(&mut self, ctx: &mut ViewContext<Self>) {
        if let DialogState::AddTag { push_after, .. } = &mut self.dialog {
            *push_after = !*push_after;
            ctx.notify();
        }
    }

    /// Submits the Add-tag dialog: reads the tag name (blank keeps the dialog
    /// open), builds [`GitWriteOp::AddTag`], and — when "Push to remote" is
    /// checked — queues a [`GitWriteOp::PushTag`] follow-up to run after the
    /// tag is created (see [`Self::pending_follow_up`]).
    fn submit_add_tag(&mut self, ctx: &mut ViewContext<Self>) {
        let (hash, push_after) = match &self.dialog {
            DialogState::AddTag { hash, push_after } => (hash.clone(), *push_after),
            _ => return,
        };
        let name = self
            .dialog_input
            .as_ref(ctx)
            .buffer_text(ctx)
            .trim()
            .to_string();
        if name.is_empty() {
            return;
        }
        let add_op = GitWriteOp::AddTag {
            hash,
            name: name.clone(),
            message: None,
        };
        self.pending_follow_up = push_after.then_some(GitWriteOp::PushTag {
            remote: DEFAULT_PUSH_REMOTE.to_string(),
            name,
            force: false,
        });
        self.dialog = DialogState::None;
        ctx.focus_self();
        self.run_op(add_op, ctx);
    }

    /// Opens the stash dialog: clears the shared message editor, defaults the
    /// Include-untracked checkbox on, and focuses the input.
    fn open_stash_dialog(&mut self, ctx: &mut ViewContext<Self>) {
        self.open_menu = None;
        self.dialog_input.update(ctx, |editor, ctx| {
            editor.set_buffer_text("", ctx);
        });
        self.dialog = DialogState::Stash {
            include_untracked: true,
        };
        ctx.focus(&self.dialog_input);
        ctx.notify();
    }

    /// Flips the Include-untracked checkbox in the open stash dialog. A no-op when
    /// no stash dialog is open.
    fn toggle_stash_untracked(&mut self, ctx: &mut ViewContext<Self>) {
        if let DialogState::Stash { include_untracked } = &mut self.dialog {
            *include_untracked = !*include_untracked;
            ctx.notify();
        }
    }

    /// Submits the stash dialog: reads the (optional) message, builds the stash op
    /// with the untracked toggle, and runs it.
    fn submit_stash(&mut self, ctx: &mut ViewContext<Self>) {
        let DialogState::Stash { include_untracked } = self.dialog else {
            return;
        };
        let message = self
            .dialog_input
            .as_ref(ctx)
            .buffer_text(ctx)
            .trim()
            .to_string();
        let op = GitWriteOp::Stash {
            message: (!message.is_empty()).then_some(message),
            include_untracked,
        };
        self.dialog = DialogState::None;
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
                        Ok(op) => {
                            // A queued follow-up (e.g. "push the tag we just
                            // added") wins over the post-op refresh: its own
                            // completion is what eventually reloads the graph,
                            // so we skip this op's reload to avoid a double
                            // load with the second one's state landing on top.
                            if let Some(follow_up) = view.pending_follow_up.take() {
                                view.run_op(follow_up, ctx);
                                return;
                            }
                            match op {
                                // An archive just writes a file; nothing in the graph changed.
                                GitWriteOp::Archive { .. } => ctx.notify(),
                                // Deleting a remote branch: fetch --prune so the dropped
                                // remote-tracking ref disappears from the graph.
                                GitWriteOp::DeleteRemoteBranch { .. } => view.refresh(ctx),
                                // Everything else — including push, which updates (or
                                // creates) the local remote-tracking ref — reloads so the
                                // graph reflects the new ref positions.
                                _ => view.reload(LoadAnchor::Top, ctx),
                            }
                        }
                        Err(err) => {
                            // Drop any queued follow-up: if the local tag
                            // failed, the chained push it queued is moot.
                            view.pending_follow_up = None;
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
                DialogState::AddTag { push_after, .. } => {
                    // Tag-name input on top, "Push to remote" checkbox below.
                    // When the checkbox is checked, the submit chains a
                    // `git push` of the new tag after the local `git tag`.
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
                    let checkbox = appearance
                        .ui_builder()
                        .checkbox(st(2), Some(size))
                        .check(*push_after)
                        .build()
                        .on_click(move |ctx, _, _| {
                            ctx.dispatch_typed_action(GitGraphAction::ToggleAddTagPush);
                        })
                        .finish();
                    let label = Text::new_inline(
                        format!("Push to remote ({})", DEFAULT_PUSH_REMOTE),
                        font,
                        size,
                    )
                    .with_color(theme.foreground().into())
                    .finish();
                    let push_row = Flex::row()
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_child(checkbox)
                        .with_child(Container::new(label).with_padding_left(2.).finish())
                        .finish();
                    let body = Flex::column()
                        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .with_child(input)
                        .with_child(Container::new(push_row).with_margin_top(10.).finish())
                        .finish();
                    (
                        "Add tag".to_string(),
                        body,
                        vec![
                            self.dialog_button(
                                "Cancel".to_string(),
                                GitGraphAction::CancelDialog,
                                st(0),
                                false,
                                appearance,
                            ),
                            self.dialog_button(
                                "Add tag".to_string(),
                                GitGraphAction::SubmitAddTag,
                                st(1),
                                true,
                                appearance,
                            ),
                        ],
                    )
                }
                DialogState::Stash { include_untracked } => {
                    // Message input on top, Include-untracked checkbox below it.
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
                    let checkbox = appearance
                        .ui_builder()
                        .checkbox(st(2), Some(size))
                        .check(*include_untracked)
                        .build()
                        .on_click(move |ctx, _, _| {
                            ctx.dispatch_typed_action(GitGraphAction::ToggleStashUntracked);
                        })
                        .finish();
                    let label = Text::new_inline("Include untracked".to_string(), font, size)
                        .with_color(theme.foreground().into())
                        .finish();
                    let untracked_row = Flex::row()
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_child(checkbox)
                        .with_child(Container::new(label).with_padding_left(2.).finish())
                        .finish();
                    let body = Flex::column()
                        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .with_child(input)
                        .with_child(Container::new(untracked_row).with_margin_top(10.).finish())
                        .finish();
                    (
                        "Stash uncommitted changes".to_string(),
                        body,
                        vec![
                            self.dialog_button(
                                "Cancel".to_string(),
                                GitGraphAction::CancelDialog,
                                st(0),
                                false,
                                appearance,
                            ),
                            self.dialog_button(
                                "Stash".to_string(),
                                GitGraphAction::SubmitStash,
                                st(1),
                                true,
                                appearance,
                            ),
                        ],
                    )
                }
                DialogState::Confirm { op, message } => {
                    // Ops with an optional flag get a checkbox under the message;
                    // toggling it flips that flag on `op` (so the "Confirm" button
                    // below runs the chosen variant).
                    let body = match op.option_state() {
                        Some(checked) => {
                            let checkbox = appearance
                                .ui_builder()
                                .checkbox(st(2), Some(size))
                                .check(checked)
                                .build()
                                .on_click(move |ctx, _, _| {
                                    ctx.dispatch_typed_action(GitGraphAction::ToggleConfirmOption);
                                })
                                .finish();
                            let label = Text::new_inline(op.option_label().to_string(), font, size)
                                .with_color(theme.foreground().into())
                                .finish();
                            let force_row = Flex::row()
                                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                                .with_child(checkbox)
                                .with_child(Container::new(label).with_padding_left(2.).finish())
                                .finish();
                            Flex::column()
                                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                                .with_child(dialog_message(message.clone(), appearance))
                                .with_child(Container::new(force_row).with_margin_top(10.).finish())
                                .finish()
                        }
                        None => dialog_message(message.clone(), appearance),
                    };
                    (
                        "Confirm".to_string(),
                        body,
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
                    )
                }
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
                DialogState::ResetUncommitted => (
                    "Reset uncommitted changes".to_string(),
                    dialog_message(
                        "Reset uncommitted changes to HEAD. Mixed unstages everything but \
                         keeps your edits; Hard discards all uncommitted changes to tracked \
                         files."
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
                            "Mixed".to_string(),
                            GitGraphAction::RunOp(GitWriteOp::Reset {
                                hash: "HEAD".to_string(),
                                mode: ResetMode::Mixed,
                            }),
                            st(1),
                            false,
                            appearance,
                        ),
                        self.dialog_button(
                            "Hard".to_string(),
                            GitGraphAction::RunOp(GitWriteOp::Reset {
                                hash: "HEAD".to_string(),
                                mode: ResetMode::Hard,
                            }),
                            st(2),
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
                button_row =
                    button_row.with_child(Container::new(button).with_padding_left(8.).finish());
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
/// Renders the synthetic "uncommitted changes" row: a hollow graph node plus an
/// "Uncommitted Changes (N)" label.
fn render_uncommitted_row(
    row: &GraphRow,
    lane_count: usize,
    count: usize,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let label = format!("Uncommitted Changes ({count})");
    Flex::row()
        .with_main_axis_size(MainAxisSize::Max)
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(GitGraphRowCanvas::new(row.clone(), lane_count, true).finish())
        .with_child(
            Expanded::new(
                1.0,
                Container::new(
                    Text::new_inline(
                        label,
                        appearance.ui_font_family(),
                        appearance.ui_font_size(),
                    )
                    .with_color(theme.foreground().into())
                    .with_clip(ClipConfig::ellipsis())
                    .finish(),
                )
                .with_padding_left(6.)
                .finish(),
            )
            .finish(),
        )
        .finish()
}

fn render_graph_row(
    row: &GraphRow,
    lane_count: usize,
    commit: &CommitNode,
    index: usize,
    position_id: &str,
    appearance: &Appearance,
) -> Box<dyn Element> {
    // The HEAD "you are here" marker now sits to the left of the current
    // branch's pill (see `render_ref_badge`), so every real commit on the
    // graph draws a plain filled dot — only the synthetic uncommitted row is
    // hollow.
    Flex::row()
        .with_main_axis_size(MainAxisSize::Max)
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(GitGraphRowCanvas::new(row.clone(), lane_count, false).finish())
        .with_child(
            Expanded::new(
                1.0,
                render_commit_text(commit, index, position_id, appearance),
            )
            .finish(),
        )
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

    let mut row = Flex::row()
        .with_main_axis_size(MainAxisSize::Max)
        .with_cross_axis_alignment(CrossAxisAlignment::Center);

    // Stash rows are identified by their `stash@{n}` badge, not a hash, so they
    // omit the hash column. Every real commit shows its short hash, which carries
    // its own right-click menu (copy the 8-char hash), mirroring the ref badges:
    // its handler sits above the row's commit menu so a right-click on the hash
    // copies exactly what's shown rather than the commit menu's full hash.
    if !super::data::is_stash_node(commit) {
        let position_id = position_id.to_string();
        let hash_label = Container::new(
            Text::new_inline(
                commit.short_hash.clone(),
                appearance.monospace_font_family(),
                size,
            )
            .with_color(dim.into())
            .finish(),
        )
        .with_padding_right(8.)
        .finish();
        let short_hash = Hoverable::new(MouseStateHandle::default(), move |_| hash_label)
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
            .finish();
        row = row.with_child(short_hash);
    }

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
        RefKind::Stash => ColorU {
            r: 0x3b,
            g: 0xa5,
            b: 0xf0,
            a: 0xff,
        }, // stash blue
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
        RefKind::Stash => MenuKind::Stash { index, name },
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
    let text = Text::new_inline(
        label.name.clone(),
        appearance.ui_font_family(),
        appearance.ui_font_size(),
    )
    .with_color(text_color.into())
    .finish();
    // Every badge leads with an icon so its kind reads at a glance: a branch
    // glyph for branches (local / remote / the current HEAD), a bookmark for
    // tags, and an inbox for stashes.
    let leading_icon = match label.kind {
        RefKind::Head | RefKind::LocalBranch | RefKind::RemoteBranch => Icon::GitBranch,
        RefKind::Tag => Icon::Bookmark,
        RefKind::Stash => Icon::Inbox,
    };
    let icon = ConstrainedBox::new(leading_icon.to_warpui_icon(text_color.into()).finish())
        .with_width(12.)
        .with_height(12.)
        .finish();
    let content: Box<dyn Element> = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(icon)
        .with_child(Container::new(text).with_padding_left(3.).finish())
        .finish();
    let badge = Container::new(content)
        .with_background_color(bg)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(3.)))
        .with_horizontal_padding(5.)
        .with_vertical_padding(1.)
        .finish();

    // The current branch (HEAD) carries a hollow "you are here" ring just left
    // of its pill — a filled box with a border and no fill draws as a ring once
    // the corner radius rounds it to a circle. It used to mark the commit on the
    // graph lane, but sitting beside the branch name reads more directly.
    let pill: Box<dyn Element> = if is_current {
        let ring = Container::new(
            ConstrainedBox::new(Empty::new().finish())
                .with_width(8.)
                .with_height(8.)
                .finish(),
        )
        .with_border(Border::all(2.).with_border_fill(color))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
        .finish();
        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(Container::new(ring).with_padding_right(5.).finish())
            .with_child(badge)
            .finish()
    } else {
        badge
    };

    let inner = Container::new(pill).with_padding_right(4.).finish();

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

/// Formats a Unix-seconds timestamp for the detail area. Within 24 hours of
/// the system's current time it stays relative (just now / N minutes ago /
/// N hours ago); anything older becomes an absolute `yyyy-MM-dd HH:mm:ss` in
/// the system's local timezone. A negative diff (e.g. from a clock that's
/// been set back) falls back to "just now".
fn detail_time(unix_secs: i64) -> String {
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
        // `from_timestamp` is None only for out-of-range timestamps; fall back
        // to the raw seconds rather than showing nothing.
        _ => chrono::DateTime::from_timestamp(unix_secs, 0)
            .map(|t| {
                t.with_timezone(&chrono::Local)
                    .format("%Y-%m-%d %H:%M:%S")
                    .to_string()
            })
            .unwrap_or_else(|| unix_secs.to_string()),
    }
}

/// Wraps one drag-selectable detail segment (subject / hash + body / the
/// identity card's text) in a [`SelectableArea`] driving `handles[index]` and
/// writing the selected text into `selected_text` for Cmd/Ctrl+C copy.
fn detail_selection_area(
    segment: Box<dyn Element>,
    handles: &[SelectionHandle; 3],
    index: usize,
    selected_text: &Arc<RwLock<Option<(usize, String)>>>,
) -> Box<dyn Element> {
    let selected_text = selected_text.clone();
    let other_handles: Vec<SelectionHandle> = handles
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != index)
        .map(|(_, handle)| handle.clone())
        .collect();
    SelectableArea::new(
        handles[index].clone(),
        move |args, ctx, _| {
            // An empty payload means "no live selection in this segment": it's
            // either a `None` (this area merely saw another segment's mouse
            // event) or the empty `Some("")` a mouse-down / bare click reports
            // before any text is dragged over. Treat both the same — only the
            // owner may clear the shared text, and crucially neither path
            // pulls focus. A mouse-down's empty `Some("")` used to dispatch
            // `FocusPanel`, whose `focus_self` re-renders and rebuilds this
            // `SelectableArea` with a `None` origin; a fast drag then reaches
            // mouse-up before the next paint restores the origin, so the
            // owner can't materialize its text and wipes its own `guard`
            // (highlight visible, Cmd/Ctrl+C copies nothing). Focus is already
            // bootstrapped in `select_commit`, so the empty payload needs no
            // refocus at all.
            let text = args.selection.unwrap_or_default();
            if text.is_empty() {
                if let Ok(mut guard) = selected_text.write() {
                    if guard.as_ref().is_some_and(|(owner, _)| *owner == index) {
                        *guard = None;
                    }
                }
                return;
            }
            // A non-empty selection: this segment owns it. Clear the others
            // (once one area handles a mouse-down the rest no longer see it
            // and would keep a stale highlight) and take over the shared text
            // under this segment's tag — the tag stops another segment's
            // later empty report from wiping it.
            for handle in &other_handles {
                handle.clear();
            }
            let took_ownership = selected_text
                .read()
                .map(|g| !g.as_ref().is_some_and(|(owner, _)| *owner == index))
                .unwrap_or(true);
            if let Ok(mut guard) = selected_text.write() {
                *guard = Some((index, text));
            }
            // Re-bootstrap focus only when this segment newly takes ownership
            // (not on every drag tick), so the re-render its `focus_self`
            // triggers lands after the text is already materialized. This
            // covers the case where focus has since left the panel (e.g.
            // pasting the last copy into an editor) and a fresh drag-select
            // must pull it back for Cmd/Ctrl+C to reach `CopySelection`.
            if took_ownership {
                ctx.dispatch_typed_action(GitGraphAction::FocusPanel);
            }
        },
        segment,
    )
    .finish()
}

/// Renders one dimmed author / committer metadata row (`label`, e.g.
/// `name · time`). The row isn't part of the detail's drag-selectable text;
/// instead, hovering it opens a floating card with the full `identity`
/// (`name <email>`), whose text is itself drag-selectable and copied with
/// Cmd/Ctrl+C like the rest of the detail. The card stays open while the
/// mouse is over it — the row's hover-out delay leaves time to move the mouse
/// into the card — and is an overlay so the detail area's scroll clipping
/// can't cut it off.
fn render_identity_row(
    label: String,
    identity: String,
    mouse_states: (MouseStateHandle, MouseStateHandle),
    selection_handles: &[SelectionHandle; 3],
    selected_text: &Arc<RwLock<Option<(usize, String)>>>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let (row_mouse_state, card_mouse_state) = mouse_states;
    let theme = appearance.theme();
    let dim: ColorU = theme.sub_text_color(theme.background()).into();
    let fg: ColorU = theme.foreground().into();
    let font = appearance.ui_font_family();
    let size = appearance.ui_font_size();

    // The card: the selectable identity text on the panel background, kept
    // visually distinct from the panel by a border. Wrapped in its own
    // Hoverable so the card can keep itself open under the mouse.
    let card_text = Text::new_inline(identity, font, size)
        .with_color(fg)
        .with_selectable(true)
        .finish();
    let card_body = Container::new(detail_selection_area(
        card_text,
        selection_handles,
        2,
        selected_text,
    ))
    .with_background(theme.background())
    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
    .with_border(Border::all(1.).with_border_fill(theme.outline()))
    .with_vertical_padding(4.)
    .with_horizontal_padding(8.)
    .finish();
    let card_state = card_mouse_state.clone();
    let card = Hoverable::new(card_mouse_state, move |_| card_body).finish();

    Hoverable::new(row_mouse_state, move |row_state| {
        let text = Text::new_inline(label, font, size).with_color(dim).finish();
        let mut stack = Stack::new();
        stack.add_child(text);
        // Keep the card mounted while the mouse is over the card itself, not
        // just the row — that's what makes the card's text reachable and
        // drag-selectable at all.
        let card_hovered = card_state
            .lock()
            .map(|state| state.is_hovered())
            .unwrap_or(false);
        if row_state.is_hovered() || card_hovered {
            stack.add_positioned_overlay_child(
                card,
                OffsetPositioning::offset_from_parent(
                    vec2f(0., -4.),
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::TopLeft,
                    ChildAnchor::BottomLeft,
                ),
            );
        }
        stack.finish()
    })
    // The grace period for moving the mouse from the row into the card: the
    // row stays "hovered" long enough to cross the gap without the card
    // unmounting mid-way.
    .with_hover_out_delay(Duration::from_millis(300))
    .finish()
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
/// author rows / full hash / body) + ref badges + changed-file area.
///
/// The metadata is split into drag-selectable [`Text`]s — subject (bold) and
/// hash + body — around the dimmed author / committer rows, which are hover
/// elements instead of selectable text: hovering one opens a floating card
/// with the full `name <email>`, itself drag-selectable (the third selection
/// segment). A [`SelectableArea`] needs a single Text child for its
/// drag-select copy to work, so each segment gets its own; their callbacks
/// clear the other handles so a stale highlight can't linger in another
/// segment. The ref badges and file area sit outside the SelectableAreas and
/// aren't part of the selection.
#[allow(clippy::too_many_arguments)]
fn render_detail_body(
    commit: Option<&CommitNode>,
    detail: &CommitDetail,
    scroll_state: ClippedScrollStateHandle,
    selection_handles: &[SelectionHandle; 3],
    selected_text: Arc<RwLock<Option<(usize, String)>>>,
    author_mouse_states: (MouseStateHandle, MouseStateHandle),
    committer_mouse_states: (MouseStateHandle, MouseStateHandle),
    file_mouse_states: &[MouseStateHandle],
    dir_mouse_states: &HashMap<String, MouseStateHandle>,
    collapsed_dirs: &HashSet<String>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let font = appearance.ui_font_family();
    let size = appearance.ui_font_size();
    let fg: ColorU = theme.foreground().into();
    let dim: ColorU = theme.sub_text_color(theme.background()).into();

    // ---- Segment 1: subject (bold, selectable) ----
    let subject = commit.map(|c| c.subject.clone()).unwrap_or_else(|| {
        detail
            .message
            .lines()
            .next()
            .unwrap_or_default()
            .to_string()
    });
    let subject_text = Text::new(subject, font, size)
        .with_color(fg)
        .with_selectable(true)
        .with_style(Properties::default().weight(Weight::Bold))
        .finish();
    let mut meta_col = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Start)
        .with_child(detail_selection_area(
            subject_text,
            selection_handles,
            0,
            &selected_text,
        ));

    // ---- Author / committer rows (dimmed; hovering opens the identity card
    // with the drag-selectable `name <email>`) ----
    if let Some(c) = commit {
        let author_row = render_identity_row(
            format!("{} · {}", c.author_name, detail_time(c.author_time)),
            format!("{} <{}>", c.author_name, c.author_email),
            author_mouse_states,
            selection_handles,
            &selected_text,
            appearance,
        );
        // The margin stands in for the blank line that separated the subject
        // from the author row when the metadata was one text block.
        meta_col = meta_col.with_child(Container::new(author_row).with_margin_top(size).finish());
        // Add a row only when the committer differs from the author
        // (cherry-pick / rebase / amend, etc.).
        if detail.committer_name != c.author_name {
            meta_col = meta_col.with_child(render_identity_row(
                format!(
                    "committed by {} · {}",
                    detail.committer_name,
                    detail_time(detail.committer_time)
                ),
                format!("{} <{}>", detail.committer_name, detail.committer_email),
                committer_mouse_states,
                selection_handles,
                &selected_text,
                appearance,
            ));
        }
    }

    // ---- Segment 2: full hash (dimmed) + body (selectable) ----
    // Body: the full message with the first line (used as the subject) removed;
    // if empty, append nothing.
    let mut tail_text = commit.map(|c| c.hash.clone()).unwrap_or_default();
    let hash_chars = tail_text.chars().count();
    let body = detail_message_body(&detail.message);
    if !body.is_empty() {
        if !tail_text.is_empty() {
            tail_text.push_str("\n\n");
        }
        tail_text.push_str(&body);
    }
    if !tail_text.is_empty() {
        let mut tail = Text::new(tail_text, font, size)
            .with_color(fg)
            .with_selectable(true);
        if hash_chars > 0 {
            tail = tail.with_single_highlight(
                Highlight::new().with_foreground_color(dim),
                (0..hash_chars).collect(),
            );
        }
        // Without a commit (the uncommitted row) there are no author/hash rows
        // above, so the body needs the blank-line margin under the subject
        // itself.
        let margin_top = if commit.is_some() { 0. } else { size };
        meta_col = meta_col.with_child(
            Container::new(detail_selection_area(
                tail.finish(),
                selection_handles,
                1,
                &selected_text,
            ))
            .with_margin_top(margin_top)
            .finish(),
        );
    }
    let selectable_meta = meta_col.finish();

    // ---- File area: top divider line + summary (N files changed / total
    // additions and deletions) + file rows ----
    let total_add: u32 = detail.files.iter().map(|f| f.additions).sum();
    let total_del: u32 = detail.files.iter().map(|f| f.deletions).sum();
    // Right-align every row's `+adds -dels` into one trailing column by padding
    // the numbers (monospace) to a single shared digit count for the whole
    // detail — the summary row and the file rows. The totals are the sum of all
    // files, so they always have at least as many digits as any single file;
    // padding to the totals' width therefore lines up the summary with every
    // file row.
    let add_width = total_add.to_string().len();
    let del_width = total_del.to_string().len();
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
            total_add, total_del, add_width, del_width, appearance,
        ))
        .finish();

    // Don't virtualize the file list: a single commit has a limited number of
    // files, and putting the info and files in the same scroll region is what
    // lets a long commit message be scrolled through together with the files.
    let mut files_col = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Start)
        .with_child(Container::new(summary).with_vertical_padding(4.).finish());
    // Render the changed files as a collapsible directory tree: directory rows
    // (chevron + name + aggregate counts) toggle expand/collapse; file leaves
    // open a diff. Leaves keep their index into `detail.files`, so the
    // mouse-state lookup and `OpenFileDiff` stay index-based.
    for row in super::file_tree::build_file_rows(&detail.files, collapsed_dirs) {
        let el = match row {
            super::file_tree::FileRow::Dir { path, name, depth } => {
                let mouse_state = dir_mouse_states.get(&path).cloned().unwrap_or_default();
                // Directory: a disclosure chevron shows expand state (down =
                // expanded, right = collapsed); clicking the row toggles it. No
                // `+/-` counts — only files show those.
                let expanded = !collapsed_dirs.contains(&path);
                render_file_tree_row(
                    Some(expanded),
                    Icon::Folder,
                    &name,
                    depth,
                    None,
                    add_width,
                    del_width,
                    mouse_state,
                    GitGraphAction::ToggleDir(path),
                    appearance,
                )
            }
            super::file_tree::FileRow::File { index, name, depth } => {
                // Mouse states are the same length as files; if missing, fall
                // back to a default with no hover highlight (clicking still
                // works).
                let mouse_state = file_mouse_states.get(index).cloned().unwrap_or_default();
                let file = &detail.files[index];
                render_file_tree_row(
                    None,
                    Icon::File,
                    &name,
                    depth,
                    Some((file.additions, file.deletions)),
                    add_width,
                    del_width,
                    mouse_state,
                    GitGraphAction::OpenFileDiff(index),
                    appearance,
                )
            }
        };
        files_col = files_col.with_child(el);
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

/// Renders one clickable row of the detail file tree, in the same form as the
/// Project Explorer: the depth indent + a disclosure chevron column (down =
/// expanded, right = collapsed; empty for a file leaf) + a type icon (folder for
/// directories, file for leaves) + the left-aligned name + (for files only) the
/// red/green `+adds -dels` right-aligned into a trailing column. Chevron, icon,
/// and name share the hover-reactive [`ItemHighlightState::text_and_icon_color`].
/// `action` is dispatched on click (toggle for a directory, open-diff for a
/// file).
///
/// `is_expanded` is `None` for a file leaf (no chevron) and `Some(expanded)` for
/// a directory. `counts` is `Some((adds, dels))` for a file and `None` for a
/// directory (directories don't show diff counts). `add_width` / `del_width` are
/// the commit-wide max digit counts that align the `+`/`-` numbers into a column
/// across file rows.
#[allow(clippy::too_many_arguments)]
fn render_file_tree_row(
    is_expanded: Option<bool>,
    icon: Icon,
    name: &str,
    depth: usize,
    counts: Option<(u32, u32)>,
    add_width: usize,
    del_width: usize,
    mouse_state: MouseStateHandle,
    action: GitGraphAction,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let name = name.to_string();
    // Build the whole row inside the hover callback (like the Project Explorer)
    // so the chevron / icon / text colors track the hover state.
    Hoverable::new(mouse_state, move |mouse_state| {
        let highlight = ItemHighlightState::new(false, mouse_state);
        let color = highlight.text_and_icon_color(appearance);
        let font = appearance.ui_font_family();
        let size = appearance.ui_font_size();

        // Disclosure chevron column; a file leaf leaves it empty so its name
        // still aligns under sibling directories' names.
        let chevron: Box<dyn Element> = match is_expanded {
            Some(true) => Icon::ChevronDown.to_warpui_icon(color.into()).finish(),
            Some(false) => Icon::ChevronRight.to_warpui_icon(color.into()).finish(),
            None => Empty::new().finish(),
        };
        let chevron_col = Container::new(
            ConstrainedBox::new(chevron)
                .with_width(FILE_TREE_ICON_SIZE)
                .with_height(FILE_TREE_ICON_SIZE)
                .finish(),
        )
        .with_margin_right(4.)
        .finish();

        // Type icon column: folder for directories, file for leaves.
        let icon_col = Container::new(
            ConstrainedBox::new(icon.to_warpui_icon(color.into()).finish())
                .with_width(FILE_TREE_ICON_SIZE)
                .with_height(FILE_TREE_ICON_SIZE)
                .finish(),
        )
        .with_margin_right(8.)
        .finish();

        // The name fills the remaining width (left-aligned, ellipsis on
        // overflow), which pushes the counts to the row's right edge so they
        // line up in a column across rows.
        let name_text = Expanded::new(
            1.0,
            Text::new_inline(name.clone(), font, size)
                .with_color(color)
                .with_clip(ClipConfig::ellipsis())
                .finish(),
        )
        .finish();

        // depth indent → chevron → icon → name (fills); the row fills the width
        // so the hover highlight spans it and the whole row is a click target.
        let mut row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center);
        if depth > 0 {
            row = row.with_child(
                ConstrainedBox::new(Empty::new().finish())
                    .with_width(depth as f32 * FILE_TREE_INDENT)
                    .finish(),
            );
        }
        row = row
            .with_child(chevron_col)
            .with_child(icon_col)
            .with_child(name_text);
        // Files show their `+adds -dels` right-aligned into a trailing column
        // (padded to the commit-wide digit counts so `+`/`-` line up across
        // rows); directories show none.
        if let Some((additions, deletions)) = counts {
            row = row.with_child(
                Container::new(render_diff_counts(
                    additions, deletions, add_width, del_width, appearance,
                ))
                .with_padding_left(8.)
                .finish(),
            );
        }
        let row = row.finish();

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
        ctx.dispatch_typed_action(action.clone());
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
            GitGraphAction::SelectUncommitted => self.select_uncommitted(ctx),
            GitGraphAction::SelectRepository(index) => self.select_repository(*index, ctx),
            GitGraphAction::ToggleBranchFilter => {
                self.branch_filter_expanded = !self.branch_filter_expanded;
                if self.branch_filter_expanded {
                    // Re-measure the list's height cap on every open (the
                    // panel may have been resized since the last open).
                    if let Some(height) = self.dropdown_max_height(ctx) {
                        self.branch_popup_max_height = height;
                    }
                    // Focus the search box on open so the user can type to
                    // narrow the list immediately.
                    ctx.focus(&self.branch_filter_input);
                    ctx.notify();
                } else {
                    self.close_branch_filter(ctx);
                }
            }
            GitGraphAction::CloseBranchFilter => self.close_branch_filter(ctx),
            GitGraphAction::ToggleBranch(ref_name) => self.toggle_branch(ref_name, ctx),
            GitGraphAction::ShowAllBranches => self.show_all_branches(ctx),
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
                    .and_then(|guard| guard.as_ref().map(|(_, text)| text.clone()))
                    .filter(|t| !t.is_empty());
                if let Some(text) = text {
                    ctx.clipboard().write(ClipboardContent::plain_text(text));
                }
            }
            GitGraphAction::FocusPanel => ctx.focus_self(),
            GitGraphAction::OpenFileDiff(index) => self.open_file_diff(*index, ctx),
            GitGraphAction::ToggleDir(path) => self.toggle_dir(path.clone(), ctx),
            GitGraphAction::OpenMenu { kind, x, y } => {
                self.open_context_menu(kind.clone(), vec2f(*x, *y), ctx)
            }
            GitGraphAction::CopyToClipboard(text) => {
                ctx.clipboard()
                    .write(ClipboardContent::plain_text(text.clone()));
            }
            GitGraphAction::PromptInput(kind) => self.open_input_dialog(kind.clone(), ctx),
            GitGraphAction::SubmitInput => self.submit_input(ctx),
            GitGraphAction::OpenAddTag { hash } => self.open_add_tag_dialog(hash.clone(), ctx),
            GitGraphAction::ToggleAddTagPush => self.toggle_add_tag_push(ctx),
            GitGraphAction::SubmitAddTag => self.submit_add_tag(ctx),
            GitGraphAction::PromptStash => self.open_stash_dialog(ctx),
            GitGraphAction::ToggleStashUntracked => self.toggle_stash_untracked(ctx),
            GitGraphAction::SubmitStash => self.submit_stash(ctx),
            GitGraphAction::PromptResetMode { hash } => {
                self.open_menu = None;
                self.dialog = DialogState::ResetMode { hash: hash.clone() };
                ctx.notify();
            }
            GitGraphAction::PromptResetUncommitted => {
                self.open_menu = None;
                self.dialog = DialogState::ResetUncommitted;
                ctx.notify();
            }
            GitGraphAction::BeginWriteOp(op) => self.begin_write_op(op.clone(), ctx),
            GitGraphAction::RunOp(op) => self.run_op(op.clone(), ctx),
            GitGraphAction::ToggleConfirmOption => self.toggle_confirm_option(ctx),
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
            // The only case that surfaces the loading placeholder: a first load
            // with no graph on screen yet. A reload that already has commits in
            // hand (repo / branch switch, auto-refresh) keeps the existing graph
            // visible and swaps in place once the new page lands — see the
            // `Loading | Loaded` arms below — so switching never flashes this.
            LoadState::Loading if self.commits.is_empty() => {
                column.with_child(render_centered_placeholder(
                    None,
                    "Loading commit history…".to_string(),
                    None,
                    appearance,
                ))
            }
            LoadState::Error(err) => column.with_child(render_centered_placeholder(
                None,
                "Failed to load git history".to_string(),
                Some(err.clone()),
                appearance,
            )),
            LoadState::Loaded if self.commits.is_empty() => column.with_child(
                render_centered_placeholder(None, "No commits yet".to_string(), None, appearance),
            ),
            // Loaded, or reloading over an existing graph: render the list (plus
            // the open detail, if any). Selection is preserved across an
            // auto-refresh reload and cleared on a repo / branch switch, so the
            // detail only shows while there's still something selected.
            LoadState::Loading | LoadState::Loaded
                if self.selected.is_some() || self.uncommitted_selected =>
            {
                column
                    // The list uses Expanded to fill the remaining space above
                    // (pushing the detail area to the bottom); the detail area's
                    // height is draggable (top drag bar). Expanded rather than
                    // Shrinkable: with few commits, Shrinkable would only shrink to
                    // the content height, leaving the list and detail crammed at the
                    // top with empty space below and the detail's drag misaligned.
                    .with_child(Expanded::new(1.0, self.render_commit_list(appearance)).finish())
                    .with_child(self.render_resizable_detail(appearance))
            }
            LoadState::Loading | LoadState::Loaded => {
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
                    .on_dismiss(|ctx, _app| ctx.dispatch_typed_action(GitGraphAction::CancelDialog))
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
