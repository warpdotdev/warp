//! Create-PR mode for [`GitDialog`].
//!
//! Renders the branch's PR diff (what would be included in the pull request)
//! with expandable per-file stats. On confirm, spawns `create_pr` and shows
//! a toast with a clickable "Open PR" link.

use warp_core::send_telemetry_from_ctx;
use warp_core::ui::appearance::Appearance;
use warp_errors::report_error;
use warpui::elements::{
    ClippedScrollStateHandle, Container, Element, Flex, MouseStateHandle, ParentElement, Text,
};
use warpui::{SingletonEntity, ViewContext};

use crate::code_review::git_dialog::{
    GitDialog, GitDialogAction, GitDialogEvent, GitDialogMode, render_branch_section,
    render_file_changes_box, should_send_git_ops_ai_request, show_toast, user_facing_git_error,
};
use crate::code_review::telemetry_event::{
    CodeReviewTelemetryEvent, GitDialogStatus, GitOperationKind,
};
use crate::ui_components::icons::Icon;
use crate::util::git::{FileChangeEntry, PrInfo};
use crate::view_components::{DismissibleToast, ToastLink};
use crate::workspace::ToastStack;

/// PR-mode sub-actions, dispatched wrapped in `GitDialogAction::Pr`.
#[derive(Clone, Debug, PartialEq)]
pub enum PrSubAction {
    ToggleChangesExpanded,
}

pub struct PrState {
    base_branch_name: Option<String>,
    file_changes: Vec<FileChangeEntry>,
    changes_expanded: bool,
    summary_mouse_state: MouseStateHandle,
    changes_scroll_state: ClippedScrollStateHandle,
}

pub(super) fn confirm_label_for() -> &'static str {
    "Create PR"
}

pub(super) fn confirm_icon_for() -> Icon {
    Icon::Github
}

fn loading_label_for() -> &'static str {
    "Creating\u{2026}"
}

/// PR mode has no prerequisites beyond a branch with commits; confirm is
/// always enabled when not loading.
pub(super) fn is_ready_to_confirm(_state: &PrState) -> bool {
    true
}

pub(super) fn new_state(base_branch_name: Option<String>) -> PrState {
    PrState {
        base_branch_name: base_branch_name.map(|name| {
            let name = name.trim();
            name.strip_prefix("origin/").unwrap_or(name).to_string()
        }),
        file_changes: Vec::new(),
        changes_expanded: false,
        summary_mouse_state: MouseStateHandle::default(),
        changes_scroll_state: ClippedScrollStateHandle::default(),
    }
}

/// Kicks off an on-demand fetch of the committed branch diff
/// (`merge_base(HEAD, main)..HEAD`) for the create-PR Changes box. The result
/// arrives via `DiffStateModelEvent::BranchCommittedFilesReceived` and is
/// applied by [`apply_committed_file_changes`]. Unlike the working-tree-based
/// `against_base_branch` metadata, this is committed-only, so the box previews
/// exactly what `gh pr create` will include — not uncommitted or untracked
/// changes. Called on dialog open; local computes it off-thread, remote fetches
/// it via RPC.
pub(super) fn fetch_committed_file_changes(me: &mut GitDialog, ctx: &mut ViewContext<GitDialog>) {
    me.diff_state_model().update(ctx, |model, ctx| {
        model.fetch_committed_branch_files(ctx);
    });
}

/// Applies the committed branch files delivered via
/// `DiffStateModelEvent::BranchCommittedFilesReceived` to the create-PR
/// Changes box. No-op when the dialog isn't in create-PR mode.
pub(super) fn apply_committed_file_changes(
    me: &mut GitDialog,
    files: Vec<FileChangeEntry>,
    ctx: &mut ViewContext<GitDialog>,
) {
    {
        let GitDialogMode::CreatePr(state) = me.mode_mut() else {
            return;
        };
        state.file_changes = files;
    }
    ctx.notify();
}

pub(super) fn handle_sub_action(
    me: &mut GitDialog,
    action: &PrSubAction,
    ctx: &mut ViewContext<GitDialog>,
) {
    match action {
        PrSubAction::ToggleChangesExpanded => {
            if let GitDialogMode::CreatePr(state) = me.mode_mut() {
                state.changes_expanded = !state.changes_expanded;
            }
            ctx.notify();
        }
    }
}

pub(super) fn start_confirm(me: &mut GitDialog, ctx: &mut ViewContext<GitDialog>) {
    let GitDialogMode::CreatePr(_) = me.mode() else {
        return;
    };
    let branch_name = me.branch_name().to_string();
    // AI-generate the PR title/body when the user has it enabled; falls back to
    // `gh pr create --fill`.
    let autogenerate_content = should_send_git_ops_ai_request(ctx);

    me.set_loading(loading_label_for(), ctx);

    me.diff_state_model().update(ctx, |m, ctx| {
        m.create_pr(branch_name, autogenerate_content, ctx);
    });
}

/// Shared create-PR completion: toast (with Open PR link) + telemetry +
/// close.
pub(super) fn finish_create_pr(
    me: &GitDialog,
    result: anyhow::Result<PrInfo>,
    ctx: &mut ViewContext<GitDialog>,
) {
    let (status, error) = match &result {
        Ok(_) => (GitDialogStatus::Succeeded, None),
        Err(err) => (GitDialogStatus::Failed, Some(err.to_string())),
    };
    match &result {
        Ok(pr_info) => show_pr_created_toast(pr_info, ctx),
        Err(err) => {
            report_error!(err);
            show_toast(user_facing_git_error(&err.to_string()), ctx);
        }
    }
    send_telemetry_from_ctx!(
        CodeReviewTelemetryEvent::GitDialogCompleted {
            is_local: Some(!me.repo_location().is_remote()),
            operation: GitOperationKind::CreatePr,
            status,
            error,
        },
        ctx
    );
    ctx.emit(GitDialogEvent::Completed);
}

/// Shows a toast announcing PR creation with a clickable "Open PR" link.
pub(super) fn show_pr_created_toast(pr_info: &PrInfo, ctx: &mut ViewContext<GitDialog>) {
    let window_id = ctx.window_id();
    let url = pr_info.url.clone();
    ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
        let link = ToastLink::new("Open PR".to_string()).with_href(url);
        let toast =
            DismissibleToast::default("PR successfully created.".to_string()).with_link(link);
        toast_stack.add_ephemeral_toast(toast, window_id, ctx);
    });
}

pub(super) fn render_body(
    state: &PrState,
    branch_name: &str,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let base_branch = state
        .base_branch_name
        .as_deref()
        .unwrap_or("default branch");
    let branch_name = format!("{branch_name} \u{2192} {base_branch}");
    Flex::column()
        .with_child(
            Container::new(render_branch_section(branch_name, appearance))
                .with_margin_bottom(16.)
                .finish(),
        )
        .with_child(render_changes_section(state, appearance))
        .finish()
}

fn render_changes_section(state: &PrState, appearance: &Appearance) -> Box<dyn Element> {
    let theme = appearance.theme();
    let main_color = theme.main_text_color(theme.surface_1()).into_solid();

    let label = Text::new(
        "Changes",
        appearance.ui_font_family(),
        appearance.ui_font_size(),
    )
    .with_color(main_color)
    .finish();

    let changes_box = render_file_changes_box(
        &state.file_changes,
        state.changes_expanded,
        &state.summary_mouse_state,
        &state.changes_scroll_state,
        GitDialogAction::Pr(PrSubAction::ToggleChangesExpanded),
        appearance,
    );

    Flex::column()
        .with_child(Container::new(label).with_margin_bottom(8.).finish())
        .with_child(changes_box)
        .finish()
}
