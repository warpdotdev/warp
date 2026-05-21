//! Create-PR mode for [`GitDialog`].
//!
//! Renders the branch's PR diff (what would be included in the pull request)
//! with expandable per-file stats. On confirm, spawns `create_pr` and shows
//! a toast with a clickable "Open PR" link.

use std::path::Path;

use warp_core::ui::appearance::Appearance;
use warpui::{
    elements::{
        ClippedScrollStateHandle, Container, Element, Flex, MouseStateHandle, ParentElement, Text,
    },
    SingletonEntity, ViewContext,
};

use crate::{
    ai::generate_code_review_content::api::{GenerateCodeReviewContentRequest, OutputType},
    code_review::{
        git_dialog::{
            interactive_path_future, render_branch_section, render_file_changes_box,
            should_send_git_ops_ai_request, show_toast, user_facing_git_error, GitDialog,
            GitDialogAction, GitDialogEvent, GitDialogMode,
        },
        telemetry_event::{CodeReviewTelemetryEvent, GitDialogStatus, GitOperationKind},
    },
    server::server_api::{ai::AIClient, ServerApiProvider},
    ui_components::icons::Icon,
    util::git::{
        create_pr, get_branch_commit_messages, get_branch_diff_entries, get_diff_for_pr,
        FileChangeEntry, PrInfo,
    },
    view_components::{DismissibleToast, ToastLink},
    workspace::ToastStack,
};
use warp_core::send_telemetry_from_ctx;

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

pub(super) fn new_state(
    repo_path: &Path,
    base_branch_name: Option<String>,
    ctx: &mut ViewContext<GitDialog>,
) -> PrState {
    let diff_repo_path = repo_path.to_path_buf();
    ctx.spawn(
        async move { get_branch_diff_entries(&diff_repo_path).await },
        |me, result, ctx| {
            if let GitDialogMode::CreatePr(state) = &mut me.mode {
                match result {
                    Ok(entries) => {
                        state.file_changes = entries;
                        ctx.notify();
                    }
                    Err(err) => {
                        log::error!("Failed to load branch diff entries: {err}");
                    }
                }
            }
        },
    );

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
    let repo_path = me.repo_path().clone();
    let branch_name = me.branch_name().to_string();

    me.set_loading(loading_label_for(), ctx);

    let code_review_ai = if should_send_git_ops_ai_request(ctx) {
        Some(ServerApiProvider::handle(ctx).read(ctx, |p, _| p.get_ai_client()))
    } else {
        None
    };
    let path_future = interactive_path_future(ctx);

    ctx.spawn(
        async move {
            let path_env = path_future.await;
            if let Some(code_review_ai) = code_review_ai.as_ref() {
                create_pr_with_ai_content(
                    &repo_path,
                    &branch_name,
                    code_review_ai.as_ref(),
                    path_env.as_deref(),
                )
                .await
            } else {
                create_pr(&repo_path, None, None, path_env.as_deref()).await
            }
        },
        move |_me, result, ctx| {
            let (status, error) = match &result {
                Ok(_) => (GitDialogStatus::Succeeded, None),
                Err(err) => (GitDialogStatus::Failed, Some(err.to_string())),
            };
            match result {
                Ok(pr_info) => {
                    show_pr_created_toast(&pr_info, ctx);
                }
                Err(err) => {
                    log::error!("Failed to create PR: {err}");
                    show_toast(user_facing_git_error(&err.to_string()), ctx);
                }
            }
            send_telemetry_from_ctx!(
                CodeReviewTelemetryEvent::GitDialogCompleted {
                    operation: GitOperationKind::CreatePr,
                    status,
                    error,
                },
                ctx
            );
            ctx.emit(GitDialogEvent::Completed);
        },
    );
}

/// Generates PR title and body via AI (in parallel) and creates the PR.
/// Falls back to `gh pr create --fill` if AI generation fails or returns
/// empty content.
pub(super) async fn create_pr_with_ai_content(
    repo_path: &Path,
    branch_name: &str,
    code_review_ai: &dyn AIClient,
    path_env: Option<&str>,
) -> anyhow::Result<PrInfo> {
    let diff = get_diff_for_pr(repo_path).await?;
    let commit_messages = get_branch_commit_messages(repo_path)
        .await
        .unwrap_or_default();

    let title_req = GenerateCodeReviewContentRequest {
        output_type: OutputType::PrTitle,
        diff: diff.clone(),
        branch_name: branch_name.to_string(),
        commit_messages: commit_messages.clone(),
    };
    let body_req = GenerateCodeReviewContentRequest {
        output_type: OutputType::PrDescription,
        diff,
        branch_name: branch_name.to_string(),
        commit_messages,
    };

    match futures::try_join!(
        code_review_ai.generate_code_review_content(title_req),
        code_review_ai.generate_code_review_content(body_req),
    ) {
        Ok((title_resp, body_resp))
            if !title_resp.content.trim().is_empty() && !body_resp.content.trim().is_empty() =>
        {
            create_pr(
                repo_path,
                Some(&title_resp.content),
                Some(&body_resp.content),
                path_env,
            )
            .await
        }
        Ok(_) => {
            // Empty title/body would make `gh pr create` fail; fall back to --fill.
            log::warn!(
                "AI PR content generation returned empty title/body, falling back to --fill"
            );
            create_pr(repo_path, None, None, path_env).await
        }
        Err(err) => {
            log::warn!("AI PR content generation failed, falling back to --fill: {err}");
            create_pr(repo_path, None, None, path_env).await
        }
    }
}

/// Shows a toast announcing PR creation with a clickable "Open PR" link.
pub(super) fn show_pr_created_toast(pr_info: &PrInfo, ctx: &mut ViewContext<GitDialog>) {
    let window_id = ctx.window_id();
    let url = pr_info.url.clone();
    ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
        let link =
            ToastLink::new(crate::i18n::tr_static(ctx, "Open PR").to_string()).with_href(url);
        let toast = DismissibleToast::default(
            crate::i18n::tr_static(ctx, "PR successfully created.").to_string(),
        )
        .with_link(link);
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
