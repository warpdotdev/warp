//! Unified dialog for git operations (commit / push / create PR).
//!
//! `GitDialog` is a single view with multiple modes — each mode owns its own
//! state, body renderer, and async op in its own submodule. The outer view
//! owns everything shared: chrome (title, close/cancel/confirm buttons,
//! overlay), the loading lifecycle, ESC keybinding, and dispatch.
//!
//! To add a new mode, add a submodule with a `State` + `new_*` + `render_body`
//! + confirm async, extend `GitDialogMode`, add the per-mode action and
//! outcome variant, and wire up dispatch.

use pathfinder_geometry::vector::vec2f;
use warp_core::features::FeatureFlag;
use warp_core::send_telemetry_from_ctx;
use warp_core::ui::appearance::Appearance;
use warpui::elements::{
    Align, Border, ChildAnchor, ChildView, ClippedScrollStateHandle, ClippedScrollable,
    ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Element, Flex, Hoverable,
    Icon as IconElement, MainAxisAlignment, MainAxisSize, MouseStateHandle, OffsetPositioning,
    ParentAnchor, ParentElement, ParentOffsetBounds, Radius, ScrollbarWidth, Stack, Text,
};
use warpui::keymap::{self, FixedBinding};
use warpui::platform::Cursor;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{
    AppContext, Entity, FocusContext, ModelHandle, SingletonEntity, TypedActionView, View,
    ViewContext, ViewHandle,
};

use crate::code::buffer_location::LocalOrRemotePath;
use crate::code::editor::{add_color, remove_color};
use crate::code_review::diff_state::{
    CommitChainMode, DiffStateModel, DiffStateModelEvent, GitOpResult,
};
use crate::code_review::telemetry_event::{
    CodeReviewTelemetryEvent, GitDialogStatus, GitOperationKind,
};
use crate::settings::AISettings;
use crate::ui_components::dialog::{Dialog, dialog_styles};
use crate::ui_components::icons::Icon;
use crate::util::git::{Commit, FileChangeEntry};
use crate::view_components::DismissibleToast;
use crate::view_components::action_button::{ActionButton, ButtonSize, NakedTheme, SecondaryTheme};
use crate::workspace::ToastStack;
use crate::workspaces::user_workspaces::UserWorkspaces;

pub(crate) mod commit;
pub(crate) mod pr;
pub(crate) mod push;

pub use commit::{CommitState, CommitSubAction};
pub use pr::{PrState, PrSubAction};
pub use push::{PushState, PushSubAction};

/// Describes which kind of `GitDialog` to open. Passed to
/// `CodeReviewView::open_git_dialog` so the open path can be fully shared
/// across modes.
#[derive(Clone, Copy, Debug)]
pub enum GitDialogKind {
    Commit,
    Push { publish: bool },
    CreatePr,
}

pub fn init(ctx: &mut AppContext) {
    ctx.register_fixed_bindings(vec![FixedBinding::new(
        "escape",
        GitDialogAction::Cancel,
        warpui::id!("GitDialog"),
    )]);
}

/// Top-level action dispatched to `GitDialog`.
///
/// `Cancel` / `Confirm` are shared across modes; mode-specific actions are
/// carried in per-mode sub-action enums.
#[derive(Clone, Debug, PartialEq)]
pub enum GitDialogAction {
    Cancel,
    Confirm,
    Commit(CommitSubAction),
    Push(PushSubAction),
    Pr(PrSubAction),
}

/// Events emitted to the parent view. Each mode handles its own success /
/// failure toasts internally; the parent only needs to know whether the
/// dialog completed (close + refresh state) or was cancelled (just close).
#[derive(Clone, Debug)]
pub enum GitDialogEvent {
    /// The dialog's async op ran and emitted its own toast. Parent should
    /// close the dialog and refresh repo/PR metadata.
    Completed,
    /// The user cancelled (ESC / close button / cancel button). Parent
    /// should close the dialog; no refresh needed.
    Cancelled,
}

/// Shows an ephemeral toast for a git-dialog outcome. Submodules call this
/// directly from their success/failure paths.
fn show_toast(msg: impl Into<String>, ctx: &mut ViewContext<GitDialog>) {
    let window_id = ctx.window_id();
    let msg = msg.into();
    ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
        let toast = DismissibleToast::default(msg);
        toast_stack.add_ephemeral_toast(toast, window_id, ctx);
    });
}

/// Whether the git-operations AI autogen flow should send an AI request.
///
/// Folds the parent feature flag, the user's dedicated per-feature AI toggle
/// (which itself requires active AI / auth / remote-session org policy to
/// allow AI), and the current team's Git Operations AI tier policy.
///
/// When this returns `false`, call sites skip AI entirely: commit.rs opens
/// with the manual-type placeholder and pr.rs goes straight to
/// `gh pr create --fill`.
fn should_send_git_ops_ai_request(app: &AppContext) -> bool {
    FeatureFlag::GitOperationsInCodeReview.is_enabled()
        && AISettings::as_ref(app).is_git_operations_autogen_enabled(app)
        && UserWorkspaces::as_ref(app).is_git_operations_ai_enabled()
}

/// Maps a raw git error string to a user-friendly toast message. Known
/// failure modes get dedicated copy; anything else falls back to a generic
/// message (the raw error is always logged separately at the call site).
fn user_facing_git_error(raw: &str) -> &'static str {
    let lower = raw.to_lowercase();
    if lower.contains("no changes added to commit") {
        // Distinct from a clean tree: changes exist but nothing is staged
        // (e.g. "include unstaged" off with an empty index).
        "No staged changes to commit."
    } else if lower.contains("nothing to commit") {
        "No changes to commit."
    } else if lower.contains("please tell me who you are")
        || lower.contains("author identity unknown")
    {
        "Git identity not configured. Set user.name and user.email."
    } else if lower.contains("updates were rejected")
        || lower.contains("non-fast-forward")
        || lower.contains("fetch first")
    {
        "Remote has new changes \u{2014} pull before pushing."
    } else if lower.contains("does not appear to be a git repository")
        || lower.contains("no configured push destination")
        || lower.contains("no such remote")
    {
        "No remote configured for this branch."
    } else if lower.contains("authentication failed")
        || lower.contains("permission denied (publickey)")
    {
        "Authentication failed. Check your Git credentials."
    } else if lower.contains("could not resolve host")
        || lower.contains("network is unreachable")
        || lower.contains("connection timed out")
    {
        "Network error. Check your connection."
    } else if lower.contains("repository not found") {
        "Remote repository not found."
    } else if lower.contains("failed to execute gh command") {
        // `run_gh_command` wraps spawn failures with this prefix, which is
        // the reliable "gh binary missing" signal.
        "GitHub CLI (gh) not installed. See https://cli.github.com/."
    } else if lower.contains("not logged in")
        || lower.contains("authentication required")
        || lower.contains("gh auth login")
    {
        // Phrases mirror `context_chips::current_prompt::is_gh_auth_error`,
        // which has been vetted against real `gh` failure output.
        "GitHub CLI not authenticated. Run `gh auth login`."
    } else if lower.contains("another git operation is in progress") {
        // Daemon-side guard for a repo mid-merge/rebase/cherry-pick or with a
        // held index lock (see `git_operation_in_progress`).
        "Another git operation is in progress. Finish or abort it first."
    } else {
        "Git operation failed."
    }
}

// ── Shared rendering helpers ─────────────────────────────────────────
//
// These helpers are used by per-mode body renderers (`commit::render_body`,
// `push::render_body`, etc.) and are kept here so the whole dialog lives in
// one module.

/// Renders a "Branch" label with git-branch icon and branch name.
fn render_branch_section(
    branch_name: impl Into<String>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let branch_name = branch_name.into();
    let theme = appearance.theme();
    let main_color = theme.main_text_color(theme.surface_1()).into_solid();
    let sub_color = theme.sub_text_color(theme.surface_1()).into_solid();

    let label = Text::new(
        "Branch",
        appearance.ui_font_family(),
        appearance.ui_font_size(),
    )
    .with_color(main_color)
    .finish();

    let icon = ConstrainedBox::new(
        IconElement::new(
            <Icon as Into<&'static str>>::into(Icon::GitBranch),
            sub_color,
        )
        .finish(),
    )
    .with_width(16.)
    .with_height(16.)
    .finish();

    let branch_text = Text::new(
        branch_name,
        appearance.ui_font_family(),
        appearance.ui_font_size(),
    )
    .with_color(sub_color)
    .finish();

    let branch_row = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(icon)
        .with_child(Container::new(branch_text).with_margin_left(4.).finish())
        .finish();

    Flex::column()
        .with_child(Container::new(label).with_margin_bottom(4.).finish())
        .with_child(branch_row)
        .finish()
}

fn split_file_path(path: &str) -> (&str, &str) {
    match path.rfind('/') {
        Some(idx) => (&path[idx + 1..], &path[..idx + 1]),
        None => (path, ""),
    }
}

/// Renders a chevron icon (ChevronDown when expanded, ChevronRight when collapsed).
fn render_chevron_icon(expanded: bool, appearance: &Appearance) -> Box<dyn Element> {
    let icon = if expanded {
        Icon::ChevronDown
    } else {
        Icon::ChevronRight
    };
    let icon_color = appearance
        .theme()
        .sub_text_color(appearance.theme().surface_1())
        .into_solid();
    ConstrainedBox::new(
        IconElement::new(<Icon as Into<&'static str>>::into(icon), icon_color).finish(),
    )
    .with_width(16.)
    .with_height(16.)
    .finish()
}

/// Renders the bordered, collapsible "Changes" box shared by the commit
/// and create-PR modes: a clickable summary row showing totals (files /
/// +adds / -dels) with a chevron, and an expandable scrollable file list
/// below it. The caller supplies the action to dispatch when the summary
/// is clicked, and stacks their own header above the box.
fn render_file_changes_box(
    file_changes: &[FileChangeEntry],
    expanded: bool,
    summary_mouse_state: &MouseStateHandle,
    scroll_state: &ClippedScrollStateHandle,
    on_toggle: GitDialogAction,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let main_color = theme.main_text_color(theme.surface_1()).into_solid();

    let total_files = file_changes.len();
    let total_additions: usize = file_changes.iter().map(|f| f.additions).sum();
    let total_deletions: usize = file_changes.iter().map(|f| f.deletions).sum();

    let files_text = Text::new(
        format!(
            "{total_files} {}",
            if total_files == 1 { "file" } else { "files" }
        ),
        appearance.ui_font_family(),
        appearance.ui_font_size(),
    )
    .with_color(main_color)
    .finish();

    let additions_text = Container::new(
        Text::new(
            format!("+{total_additions}"),
            appearance.ui_font_family(),
            appearance.ui_font_size(),
        )
        .with_color(add_color(appearance))
        .finish(),
    )
    .with_margin_left(8.)
    .finish();

    let deletions_text = Container::new(
        Text::new(
            format!("-{total_deletions}"),
            appearance.ui_font_family(),
            appearance.ui_font_size(),
        )
        .with_color(remove_color(appearance))
        .finish(),
    )
    .with_margin_left(4.)
    .finish();

    let summary_left = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(files_text)
        .with_child(additions_text)
        .with_child(deletions_text)
        .finish();

    let summary_row = Flex::row()
        .with_main_axis_size(MainAxisSize::Max)
        .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(summary_left)
        .with_child(render_chevron_icon(expanded, appearance))
        .finish();

    let summary_container = Hoverable::new(summary_mouse_state.clone(), |_| {
        Container::new(summary_row)
            .with_padding_top(8.)
            .with_padding_bottom(8.)
            .with_padding_left(12.)
            .with_padding_right(8.)
            .finish()
    })
    .on_click(move |ctx, _, _| {
        ctx.dispatch_typed_action(on_toggle.clone());
    })
    .with_cursor(Cursor::PointingHand)
    .finish();

    let mut content = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_child(summary_container);

    if expanded && !file_changes.is_empty() {
        let file_list = render_file_list(file_changes, appearance);
        let scrollable_file_list = ConstrainedBox::new(
            ClippedScrollable::vertical(
                scroll_state.clone(),
                file_list,
                ScrollbarWidth::Auto,
                theme.nonactive_ui_detail().into(),
                theme.active_ui_detail().into(),
                warpui::elements::Fill::None,
            )
            .finish(),
        )
        .with_max_height(130.)
        .finish();
        content.add_child(scrollable_file_list);
    }

    Container::new(content.finish())
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
        .with_border(Border::all(1.).with_border_fill(theme.surface_3()))
        .finish()
}

/// Renders a file list with per-file name, directory, and +/- stats.
fn render_file_list(files: &[FileChangeEntry], appearance: &Appearance) -> Box<dyn Element> {
    let theme = appearance.theme();
    let main_color = theme.main_text_color(theme.surface_1()).into_solid();
    let sub_color = theme.sub_text_color(theme.surface_1()).into_solid();

    let mut list = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

    for entry in files {
        let (filename, directory) = split_file_path(&entry.path);

        let mut name_row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Text::new(
                    filename.to_string(),
                    appearance.ui_font_family(),
                    appearance.ui_font_size(),
                )
                .with_color(main_color)
                .soft_wrap(false)
                .finish(),
            );

        if !directory.is_empty() {
            name_row.add_child(
                Container::new(
                    Text::new(
                        directory.to_string(),
                        appearance.ui_font_family(),
                        appearance.ui_font_size(),
                    )
                    .with_color(sub_color)
                    .finish(),
                )
                .with_margin_left(4.)
                .finish(),
            );
        }

        let mut stats = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
        stats.add_child(
            Container::new(
                Text::new(
                    format!("+{}", entry.additions),
                    appearance.ui_font_family(),
                    appearance.ui_font_size(),
                )
                .with_color(add_color(appearance))
                .finish(),
            )
            .with_margin_right(4.)
            .finish(),
        );
        stats.add_child(
            Text::new(
                format!("-{}", entry.deletions),
                appearance.ui_font_family(),
                appearance.ui_font_size(),
            )
            .with_color(remove_color(appearance))
            .finish(),
        );

        let row = Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(name_row.finish())
                .with_child(stats.finish())
                .finish(),
        )
        .with_padding_top(4.)
        .with_padding_bottom(4.)
        .with_padding_left(12.)
        .with_padding_right(12.)
        .finish();

        list.add_child(row);
    }

    Container::new(list.finish())
        .with_padding_bottom(4.)
        .finish()
}

/// Mode-specific state. Outer chrome lives on `GitDialog` itself.
pub enum GitDialogMode {
    Commit(CommitState),
    Push(PushState),
    CreatePr(PrState),
}

pub struct GitDialog {
    repo_location: LocalOrRemotePath,
    diff_state_model: ModelHandle<DiffStateModel>,
    branch_name: String,
    mode: GitDialogMode,
    loading: bool,
    confirm_button: ViewHandle<ActionButton>,
    cancel_button: ViewHandle<ActionButton>,
    close_button: ViewHandle<ActionButton>,
}

impl GitDialog {
    pub fn new_for_commit(
        repo_location: LocalOrRemotePath,
        diff_state_model: ModelHandle<DiffStateModel>,
        branch_name: String,
        allow_create_pr: bool,
        has_upstream: bool,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        // Commit's confirm button is a static "Confirm" with no icon; the
        // segmented intent selector inside the dialog is the sole UI that
        // communicates which of commit / commit-and-push / commit-and-create-PR
        // will actually run on click.
        let (confirm_button, cancel_button, close_button) =
            Self::build_dialog_buttons("Confirm", None, ctx);
        ctx.subscribe_to_model(&diff_state_model, Self::handle_diff_state_event);
        let state = commit::new_state(
            repo_location.to_local_path(),
            allow_create_pr,
            has_upstream,
            ctx,
        );
        let mut this = Self {
            repo_location,
            diff_state_model,
            branch_name,
            mode: GitDialogMode::Commit(state),
            loading: false,
            confirm_button,
            cancel_button,
            close_button,
        };
        // Open-time AI commit-message autogen runs for both backends; the model
        // generates it (local in-process, remote on the daemon) and the result
        // returns via the diff-state subscription wired up just above.
        commit::maybe_start_commit_message_autogen(&this, ctx);
        // Remote repos source the Changes box from synced metadata (the local
        // path loads it from the working tree in `commit::new_state`).
        commit::refresh_remote_file_changes(&mut this, ctx);
        this.refresh_confirm_enabled(ctx);
        this
    }

    pub fn new_for_push(
        repo_location: LocalOrRemotePath,
        diff_state_model: ModelHandle<DiffStateModel>,
        branch_name: String,
        publish: bool,
        commits: Vec<Commit>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let (confirm_button, cancel_button, close_button) = Self::build_dialog_buttons(
            push::confirm_label(publish),
            Some(push::confirm_icon(publish)),
            ctx,
        );
        ctx.subscribe_to_model(&diff_state_model, Self::handle_diff_state_event);
        let state = push::new_state(publish, commits);
        Self {
            repo_location,
            diff_state_model,
            branch_name,
            mode: GitDialogMode::Push(state),
            loading: false,
            confirm_button,
            cancel_button,
            close_button,
        }
    }

    pub fn new_for_pr(
        repo_location: LocalOrRemotePath,
        diff_state_model: ModelHandle<DiffStateModel>,
        branch_name: String,
        base_branch_name: Option<String>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let (confirm_button, cancel_button, close_button) =
            Self::build_dialog_buttons(pr::confirm_label_for(), Some(pr::confirm_icon_for()), ctx);
        ctx.subscribe_to_model(&diff_state_model, Self::handle_diff_state_event);
        let state = pr::new_state(base_branch_name);
        let mut this = Self {
            repo_location,
            diff_state_model,
            branch_name,
            mode: GitDialogMode::CreatePr(state),
            loading: false,
            confirm_button,
            cancel_button,
            close_button,
        };
        // Fetch the committed branch diff on open (committed-only, so the
        // Changes box previews exactly what the PR will contain). Both backends
        // deliver the result via `BranchCommittedFilesReceived`, applied in
        // `handle_diff_state_event`.
        pr::fetch_committed_file_changes(&mut this, ctx);
        this
    }

    fn build_dialog_buttons(
        confirm_label: &'static str,
        confirm_icon: Option<Icon>,
        ctx: &mut ViewContext<Self>,
    ) -> (
        ViewHandle<ActionButton>,
        ViewHandle<ActionButton>,
        ViewHandle<ActionButton>,
    ) {
        let confirm_button = ctx.add_typed_action_view(move |_ctx| {
            let mut button = ActionButton::new(confirm_label, SecondaryTheme)
                .with_size(ButtonSize::Small)
                .with_height(32.);
            if let Some(icon) = confirm_icon {
                button = button.with_icon(icon);
            }
            button.on_click(|ctx| ctx.dispatch_typed_action(GitDialogAction::Confirm))
        });
        let cancel_button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("Cancel", NakedTheme)
                .with_size(ButtonSize::Small)
                .with_height(32.)
                .on_click(|ctx| ctx.dispatch_typed_action(GitDialogAction::Cancel))
        });
        let close_button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("", NakedTheme)
                .with_icon(Icon::X)
                .with_size(ButtonSize::Small)
                .with_tooltip("ESC")
                .on_click(|ctx| ctx.dispatch_typed_action(GitDialogAction::Cancel))
        });
        (confirm_button, cancel_button, close_button)
    }

    fn repo_location(&self) -> &LocalOrRemotePath {
        &self.repo_location
    }

    fn diff_state_model(&self) -> &ModelHandle<DiffStateModel> {
        &self.diff_state_model
    }

    // ── Model event handling ─────────────────────────────────────────

    fn handle_diff_state_event(
        &mut self,
        _model: ModelHandle<DiffStateModel>,
        event: &DiffStateModelEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        // Commit-message autogen arrives at dialog open (before any op is
        // initiated), so it's handled outside the `loading` gate the
        // op-completion events use below.
        if let DiffStateModelEvent::CommitMessageGenerated(result) = event {
            commit::apply_generated_commit_message(self, result.clone(), ctx);
            return;
        }
        // Commit mode (remote) sources its Changes box from synced metadata, so
        // refresh it whenever metadata lands. Arrives independently of any
        // in-flight op, so it's handled outside the `loading` gate below.
        if let DiffStateModelEvent::MetadataRefreshed(_) = event {
            commit::refresh_remote_file_changes(self, ctx);
            return;
        }
        // The create-PR dialog fetches its committed file list on open
        // (committed-only, so it matches what the PR will contain); the result
        // arrives here and populates the Changes box.
        if let DiffStateModelEvent::BranchCommittedFilesReceived(files) = event {
            pr::apply_committed_file_changes(self, files.clone(), ctx);
            return;
        }
        let DiffStateModelEvent::GitOpCompleted(result) = event else {
            return;
        };
        // Only act when we're in a loading state (i.e. we initiated the op).
        if !self.loading {
            return;
        }
        match result {
            GitOpResult::CommitChainCompleted(result) => {
                let intent = match &self.mode {
                    GitDialogMode::Commit(state) => state.intent,
                    _ => return,
                };
                // Unified completion path (toast + telemetry + close) for both
                // backends; the model already applied the delta / PR info to
                // metadata before emitting this event.
                commit::finish_commit_chain(self, intent, result.clone(), ctx);
            }
            GitOpResult::PushCompleted(result) => {
                let publish = match &self.mode {
                    GitDialogMode::Push(state) => state.publish,
                    _ => return,
                };
                push::finish_push(
                    self,
                    publish,
                    result.clone().map_err(|e| anyhow::anyhow!(e)),
                    ctx,
                );
            }
            GitOpResult::PrCreated(result) => {
                pr::finish_create_pr(self, result.clone().map_err(|e| anyhow::anyhow!(e)), ctx);
            }
        }
    }

    fn branch_name(&self) -> &str {
        &self.branch_name
    }

    fn mode(&self) -> &GitDialogMode {
        &self.mode
    }

    fn mode_mut(&mut self) -> &mut GitDialogMode {
        &mut self.mode
    }

    fn loading(&self) -> bool {
        self.loading
    }

    /// Disables cancel/confirm/close and swaps the confirm label while the
    /// async op is running.
    fn set_loading(&mut self, loading_label: &'static str, ctx: &mut ViewContext<Self>) {
        self.loading = true;
        self.confirm_button.update(ctx, |b, ctx| {
            b.set_label(loading_label, ctx);
            b.set_disabled(true, ctx);
        });
        self.cancel_button.update(ctx, |b, ctx| {
            b.set_disabled(true, ctx);
        });
        self.close_button.update(ctx, |b, ctx| {
            b.set_disabled(true, ctx);
        });
        ctx.notify();
    }

    /// Re-evaluates the confirm button's disabled state based on mode-specific
    /// inputs (e.g. commit requires a message and some files). Push mode has
    /// no prerequisites, so it's always enabled when not loading.
    fn refresh_confirm_enabled(&self, ctx: &mut ViewContext<Self>) {
        if self.loading {
            return;
        }
        let (disabled, tooltip) = match &self.mode {
            GitDialogMode::Commit(state) => (
                !commit::is_ready_to_confirm(state, ctx),
                commit::confirm_tooltip(state, ctx),
            ),
            GitDialogMode::Push(_) => (false, None),
            GitDialogMode::CreatePr(state) => (!pr::is_ready_to_confirm(state), None),
        };
        self.confirm_button.update(ctx, |b, ctx| {
            b.set_disabled(disabled, ctx);
            b.set_tooltip(tooltip, ctx);
        });
    }

    fn title(&self) -> &'static str {
        match &self.mode {
            GitDialogMode::Commit(_) => "Commit your changes",
            GitDialogMode::Push(state) => {
                if state.publish {
                    "Publish branch"
                } else {
                    "Push changes"
                }
            }
            GitDialogMode::CreatePr(_) => "Create pull request",
        }
    }

    fn header_icon(&self) -> Icon {
        match &self.mode {
            GitDialogMode::Commit(_) => Icon::GitCommit,
            GitDialogMode::Push(state) => {
                if state.publish {
                    Icon::UploadCloud
                } else {
                    Icon::ArrowUp
                }
            }
            GitDialogMode::CreatePr(_) => Icon::Github,
        }
    }

    fn render_body(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        match &self.mode {
            GitDialogMode::Commit(state) => commit::render_body(state, &self.branch_name, app),
            GitDialogMode::Push(state) => push::render_body(state, &self.branch_name, appearance),
            GitDialogMode::CreatePr(state) => pr::render_body(state, &self.branch_name, appearance),
        }
    }

    /// Builds the `Dialog` component (title, body, bottom buttons) and wraps
    /// it in centered overlay chrome with a blurred background.
    fn render_dialog(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let close = ChildView::new(&self.close_button).finish();
        let cancel = ChildView::new(&self.cancel_button).finish();
        let confirm = Container::new(ChildView::new(&self.confirm_button).finish())
            .with_margin_left(8.)
            .finish();

        let body = self.render_body(app);

        let surface2 = theme.surface_2();
        let icon_color = theme.main_text_color(surface2).into_solid();
        let header_icon = Container::new(
            ConstrainedBox::new(
                IconElement::new(
                    <Icon as Into<&'static str>>::into(self.header_icon()),
                    icon_color,
                )
                .finish(),
            )
            .with_width(16.)
            .with_height(16.)
            .finish(),
        )
        .with_uniform_padding(8.)
        .with_background(surface2)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
        .finish();

        let dialog = Dialog::new(
            self.title().to_string(),
            None,
            UiComponentStyles {
                width: Some(460.),
                padding: Some(Coords::uniform(24.).bottom(12.)),
                ..dialog_styles(appearance)
            },
        )
        .with_header_icon(header_icon)
        .with_close_button(close)
        .with_child(body)
        .with_separator()
        .with_bottom_row_child(cancel)
        .with_bottom_row_child(confirm)
        .build()
        .finish();

        let dialog = Container::new(dialog).with_margin_top(35.).finish();

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

        Container::new(Align::new(stack.finish()).finish())
            .with_background_color(appearance.theme().blurred_background_overlay().into())
            .with_corner_radius(app.windows().window_corner_radius())
            .finish()
    }
}

impl Entity for GitDialog {
    type Event = GitDialogEvent;
}

impl View for GitDialog {
    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        self.render_dialog(app)
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if !focus_ctx.is_self_focused() {
            return;
        }
        match &self.mode {
            GitDialogMode::Commit(state) => commit::on_focus(state, ctx),
            GitDialogMode::Push(_) | GitDialogMode::CreatePr(_) => {}
        }
    }

    fn keymap_context(&self, _: &AppContext) -> keymap::Context {
        let mut ctx = keymap::Context::default();
        ctx.set.insert(Self::ui_name());
        ctx
    }

    fn ui_name() -> &'static str {
        "GitDialog"
    }
}

impl TypedActionView for GitDialog {
    type Action = GitDialogAction;

    fn handle_action(&mut self, action: &GitDialogAction, ctx: &mut ViewContext<Self>) {
        match action {
            GitDialogAction::Cancel => {
                if !self.loading {
                    let operation = match &self.mode {
                        GitDialogMode::Commit(state) => match state.intent {
                            CommitChainMode::CommitOnly => GitOperationKind::CommitOnly,
                            CommitChainMode::CommitAndPush => GitOperationKind::CommitAndPush,
                            CommitChainMode::CommitAndCreatePr => {
                                GitOperationKind::CommitAndCreatePr
                            }
                        },
                        GitDialogMode::Push(state) => {
                            if state.publish {
                                GitOperationKind::Publish
                            } else {
                                GitOperationKind::Push
                            }
                        }
                        GitDialogMode::CreatePr(_) => GitOperationKind::CreatePr,
                    };
                    // Derive the real local/remote value rather than hardcoding
                    // it, so cancel telemetry matches the repo the dialog acts
                    // on (the completion paths report the same value).
                    let is_local = !self.repo_location.is_remote();
                    send_telemetry_from_ctx!(
                        CodeReviewTelemetryEvent::GitDialogCompleted {
                            is_local: Some(is_local),
                            operation,
                            status: GitDialogStatus::Cancelled,
                            error: None,
                        },
                        ctx
                    );
                    ctx.emit(GitDialogEvent::Cancelled);
                }
            }
            GitDialogAction::Confirm => {
                if self.loading {
                    return;
                }
                match &self.mode {
                    GitDialogMode::Commit(_) => commit::start_confirm(self, ctx),
                    GitDialogMode::Push(_) => push::start_confirm(self, ctx),
                    GitDialogMode::CreatePr(_) => pr::start_confirm(self, ctx),
                }
            }
            GitDialogAction::Commit(sub) => commit::handle_sub_action(self, sub, ctx),
            GitDialogAction::Push(sub) => push::handle_sub_action(self, sub, ctx),
            GitDialogAction::Pr(sub) => pr::handle_sub_action(self, sub, ctx),
        }
    }
}
