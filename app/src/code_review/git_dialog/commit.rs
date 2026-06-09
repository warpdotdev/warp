//! Commit mode for [`GitDialog`]. Drafts a commit message via AI on open,
//! then on confirm runs `run_commit` and optionally chains `run_push` /
//! `create_pr` per the selected intent.

use std::path::Path;

use warp_core::send_telemetry_from_ctx;
use warp_core::ui::appearance::Appearance;
use warpui::elements::{
    ChildView, ClippedScrollStateHandle, Container, CornerRadius, CrossAxisAlignment, Element,
    Flex, MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement, Radius, Text,
};
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::ui_components::switch::SwitchStateHandle;
use warpui::{AppContext, SingletonEntity, ViewContext, ViewHandle};

use crate::code_review::diff_state::CommitChainMode;
use crate::code_review::git_dialog::pr::show_pr_created_toast;
use crate::code_review::git_dialog::{
    render_branch_section, render_file_changes_box, should_send_git_ops_ai_request, show_toast,
    user_facing_git_error, GitDialog, GitDialogAction, GitDialogEvent, GitDialogMode,
};
use crate::code_review::telemetry_event::{
    CodeReviewTelemetryEvent, GitDialogStatus, GitOperationKind,
};
use crate::editor::{
    EditorOptions, EditorView, Event as EditorEvent, InteractionState,
    PropagateAndNoOpNavigationKeys, TextOptions,
};
use crate::ui_components::icons::Icon;
use crate::util::git::{get_file_change_entries, FileChangeEntry, PrInfo};
use crate::view_components::action_button::{ActionButton, ButtonSize, SecondaryTheme};

/// Commit-specific sub-actions, dispatched wrapped in `GitDialogAction::Commit`.
#[derive(Clone, Debug, PartialEq)]
pub enum CommitSubAction {
    SetIntent(CommitChainMode),
    ToggleIncludeUnstaged,
    ToggleChangesExpanded,
}

const EDITOR_FONT_SIZE: f32 = 12.;
const EDITOR_MIN_HEIGHT: f32 = 72.;
/// Placeholder shown while the open-time AI commit-message autogen is in
/// flight.
const GENERATING_PLACEHOLDER_TEXT: &str = "Generating commit message\u{2026}";
/// Placeholder shown once the open-time autogen resolves — either as a
/// nudge if the user later clears the generated draft, or as guidance when
/// autogen failed and the editor is blank. Also used when autogen is off.
const FALLBACK_PLACEHOLDER_TEXT: &str = "Type a commit message";
/// Loading-state label while the commit / chain runs. Static regardless of
/// which chain is in flight — the success toast communicates what actually
/// ran.
const LOADING_LABEL: &str = "Committing\u{2026}";

pub struct CommitState {
    pub(super) intent: CommitChainMode,
    include_unstaged: bool,
    file_changes: Vec<FileChangeEntry>,
    changes_expanded: bool,
    switch_state: SwitchStateHandle,
    summary_mouse_state: MouseStateHandle,
    changes_scroll_state: ClippedScrollStateHandle,
    pub(super) message_editor: ViewHandle<EditorView>,
    commit_button: ViewHandle<ActionButton>,
    commit_and_push_button: ViewHandle<ActionButton>,
    /// `None` when creating a PR doesn't make sense for this branch —
    /// either a PR already exists or we're on the repo's main branch.
    /// The intent is hidden entirely in either case; an existing PR is
    /// still reachable via the git operations menu in the header.
    commit_and_create_pr_button: Option<ViewHandle<ActionButton>>,
}

pub(super) fn new_state(
    local_repo_path: Option<&Path>,
    allow_create_pr: bool,
    has_upstream: bool,
    ctx: &mut ViewContext<GitDialog>,
) -> CommitState {
    // Dialog always opens with the plain commit intent; the user picks
    // something else via the segmented intent selector inside the dialog.
    let intent = CommitChainMode::CommitOnly;
    // `CommitAndPush` always runs `git push --set-upstream`, so it works
    // whether or not the branch already has an upstream — but the label
    // and icon flip to communicate the user-visible difference.
    let (push_label, push_icon) = if has_upstream {
        ("Commit and push", Icon::ArrowUp)
    } else {
        ("Commit and publish", Icon::UploadCloud)
    };
    // If AI autogen is on, the dialog opens with "Generating\u{2026}" and a
    // background request fills the editor when it resolves. Otherwise, we
    // land on the manual-type prompt immediately.
    let ai_autogen_enabled = should_send_git_ops_ai_request(ctx);
    let initial_placeholder = if ai_autogen_enabled {
        GENERATING_PLACEHOLDER_TEXT
    } else {
        FALLBACK_PLACEHOLDER_TEXT
    };
    let message_editor = ctx.add_typed_action_view(|ctx| {
        let appearance = Appearance::as_ref(ctx);
        let options = EditorOptions {
            text: TextOptions {
                font_size_override: Some(EDITOR_FONT_SIZE),
                font_family_override: Some(appearance.ui_font_family()),
                ..Default::default()
            },
            soft_wrap: true,
            autogrow: true,
            propagate_and_no_op_vertical_navigation_keys: PropagateAndNoOpNavigationKeys::Always,
            supports_vim_mode: false,
            single_line: false,
            ..Default::default()
        };

        let mut editor = EditorView::new(options, ctx);
        editor.set_placeholder_text(initial_placeholder, ctx);
        editor
    });

    ctx.subscribe_to_view(&message_editor, |me, _, event, ctx| {
        handle_editor_event(me, event, ctx);
    });

    let commit_button = ctx.add_typed_action_view(|_ctx| {
        ActionButton::new("Commit", SecondaryTheme)
            .with_size(ButtonSize::XSmall)
            .with_height(32.)
            .with_icon(Icon::GitCommit)
            .on_click(|ctx| {
                ctx.dispatch_typed_action(GitDialogAction::Commit(CommitSubAction::SetIntent(
                    CommitChainMode::CommitOnly,
                )))
            })
    });
    let commit_and_push_button = ctx.add_typed_action_view(move |_ctx| {
        ActionButton::new(push_label, SecondaryTheme)
            .with_size(ButtonSize::XSmall)
            .with_height(32.)
            .with_icon(push_icon)
            .on_click(|ctx| {
                ctx.dispatch_typed_action(GitDialogAction::Commit(CommitSubAction::SetIntent(
                    CommitChainMode::CommitAndPush,
                )))
            })
    });

    let commit_and_create_pr_button = if allow_create_pr {
        Some(ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("Commit and create PR", SecondaryTheme)
                .with_size(ButtonSize::XSmall)
                .with_height(32.)
                .with_icon(Icon::Github)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(GitDialogAction::Commit(CommitSubAction::SetIntent(
                        CommitChainMode::CommitAndCreatePr,
                    )))
                })
        }))
    } else {
        None
    };

    let include_unstaged = true;
    // Local repos load the changes list from the working tree here; remote
    // repos source the Changes box from synced metadata.
    if let Some(repo_path) = local_repo_path {
        let repo_path_for_load = repo_path.to_path_buf();
        ctx.spawn(
            async move { get_file_change_entries(&repo_path_for_load, include_unstaged).await },
            move |me, result, ctx| {
                let GitDialogMode::Commit(state) = &mut me.mode else {
                    return;
                };
                match result {
                    Ok(entries) => state.file_changes = entries,
                    Err(err) => log::warn!("Failed to load file changes: {err}"),
                }
                me.refresh_confirm_enabled(ctx);
                ctx.notify();
            },
        );
    }

    let state = CommitState {
        intent,
        include_unstaged,
        file_changes: Vec::new(),
        changes_expanded: true,
        switch_state: SwitchStateHandle::default(),
        summary_mouse_state: MouseStateHandle::default(),
        changes_scroll_state: ClippedScrollStateHandle::default(),
        message_editor,
        commit_button,
        commit_and_push_button,
        commit_and_create_pr_button,
    };
    apply_intent_selector(&state, ctx);
    state
}

pub(super) fn on_focus(state: &CommitState, ctx: &mut ViewContext<GitDialog>) {
    ctx.focus(&state.message_editor);
}

pub(super) fn is_ready_to_confirm(state: &CommitState, app: &AppContext) -> bool {
    // Confirm requires committable changes and a non-empty commit message.
    // While open-time autogen is in flight the editor is still empty, so this
    // keeps the button disabled until the draft lands (or the user types
    // something).
    has_committable_changes(state) && commit_message(state, app).is_some()
}

/// Whether there's at least one change to commit — the guard that keeps
/// Confirm disabled when there's nothing to commit.
///
/// Gates on `file_changes`, which already reflects the active "include
/// unstaged" scope: local re-reads the working tree on toggle, while remote
/// shows the full synced set (it can't re-scope client-side). The daemon-side
/// `run_commit` is the authoritative backstop that rejects an empty commit —
/// e.g. "exclude unstaged" with nothing staged — surfacing it as an error
/// toast rather than a phantom success.
fn has_committable_changes(state: &CommitState) -> bool {
    !state.file_changes.is_empty()
}

/// Returns a tooltip to show on the disabled Confirm button when the
/// user needs to take action, or `None` when no tooltip is needed.
pub(super) fn confirm_tooltip(state: &CommitState, app: &AppContext) -> Option<&'static str> {
    // Only nudge for a missing message; an empty Changes box is self-evident,
    // and gating a tooltip on it would also flash during the open-time load.
    if has_committable_changes(state) && commit_message(state, app).is_none() {
        return Some("Enter a commit message");
    }
    None
}

/// Populates the commit message editor from an AI-generated message. Shared
/// by both backends, whose open-time autogen arrives via the
/// `CommitMessageGenerated` model event, so both behave identically: on
/// success, fill the editor unless the user already typed; on failure, swap
/// to the manual-type placeholder (no toast — the empty editor tells the
/// story and the failure isn't retryable).
pub(super) fn apply_generated_commit_message(
    me: &mut GitDialog,
    result: Result<String, String>,
    ctx: &mut ViewContext<GitDialog>,
) {
    let editor_handle = match me.mode() {
        GitDialogMode::Commit(state) => state.message_editor.clone(),
        _ => return,
    };
    match result {
        Ok(generated) => {
            let user_typed = !editor_handle.as_ref(ctx).buffer_text(ctx).trim().is_empty();
            editor_handle.update(ctx, |editor, ctx| {
                // Swap "Generating\u{2026}" for the manual-type prompt so it
                // shows if the user later clears the generated draft.
                editor.set_placeholder_text(FALLBACK_PLACEHOLDER_TEXT, ctx);
                // User input wins — don't clobber their text.
                if !user_typed {
                    editor.system_reset_buffer_text(generated.trim(), ctx);
                }
            });
            me.refresh_confirm_enabled(ctx);
            ctx.notify();
        }
        Err(err) => {
            log::warn!("Failed to autogenerate commit message: {err}");
            editor_handle.update(ctx, |editor, ctx| {
                editor.set_placeholder_text(FALLBACK_PLACEHOLDER_TEXT, ctx);
            });
            me.refresh_confirm_enabled(ctx);
            ctx.notify();
        }
    }
}

/// Kicks off AI commit-message autogen request.
/// The model runs the generation (local in-process, remote on the daemon) and  
/// the result returns via `DiffStateModelEvent::CommitMessageGenerated`, applied by `apply_generated_commit_message`.
pub(super) fn maybe_start_commit_message_autogen(me: &GitDialog, ctx: &mut ViewContext<GitDialog>) {
    if !should_send_git_ops_ai_request(ctx) {
        return;
    }
    // Generate from the same scope that will be committed (the "include
    // unstaged" toggle), so the message describes what `run_commit` stages
    // rather than always assuming the full working set.
    let include_unstaged = match me.mode() {
        GitDialogMode::Commit(state) => state.include_unstaged,
        _ => return,
    };
    let branch_name = me.branch_name().to_string();
    me.diff_state_model().update(ctx, |m, ctx| {
        m.generate_commit_message(include_unstaged, branch_name, ctx);
    });
}

/// Sources the commit Changes box from synced metadata (`against_head.files`).
/// Remote repos can't read the working tree, so the list comes from metadata
/// instead of `get_file_change_entries`. No-op for local repos, which load it
/// from the working tree in `new_state` (and re-scope it on the unstaged
/// toggle). Safe to call on open and on every metadata refresh.
pub(super) fn refresh_remote_file_changes(me: &mut GitDialog, ctx: &mut ViewContext<GitDialog>) {
    if !me.repo_location().is_remote() {
        return;
    }
    let entries = me.diff_state_model().read(ctx, |model, ctx| {
        model.uncommitted_file_entries(ctx).to_vec()
    });
    {
        let GitDialogMode::Commit(state) = me.mode_mut() else {
            return;
        };
        state.file_changes = entries;
    }
    me.refresh_confirm_enabled(ctx);
    ctx.notify();
}

pub(super) fn handle_sub_action(
    me: &mut GitDialog,
    action: &CommitSubAction,
    ctx: &mut ViewContext<GitDialog>,
) {
    if me.loading() {
        return;
    }
    match action {
        CommitSubAction::SetIntent(new_intent) => {
            if let GitDialogMode::Commit(state) = me.mode_mut() {
                state.intent = *new_intent;
            }
            // Re-highlight the selected segment. The confirm button's
            // label is static ("Confirm"), so it doesn't need to update.
            if let GitDialogMode::Commit(state) = me.mode() {
                apply_intent_selector(state, ctx);
            }
        }
        CommitSubAction::ToggleIncludeUnstaged => {
            if let GitDialogMode::Commit(state) = me.mode_mut() {
                state.include_unstaged = !state.include_unstaged;
            }
            // Local re-reads the working tree scoped to the new toggle (its
            // spawn callback re-evaluates Confirm when it lands). Remote can't
            // re-scope its synced list, so it keeps showing the full set; the
            // daemon-side commit is the backstop that rejects an empty staged
            // set when unstaged is excluded.
            reload_file_changes(me, ctx);
            me.refresh_confirm_enabled(ctx);
            ctx.notify();
        }
        CommitSubAction::ToggleChangesExpanded => {
            if let GitDialogMode::Commit(state) = me.mode_mut() {
                state.changes_expanded = !state.changes_expanded;
            }
            ctx.notify();
        }
    }
}

pub(super) fn start_confirm(me: &mut GitDialog, ctx: &mut ViewContext<GitDialog>) {
    let GitDialogMode::Commit(state) = me.mode() else {
        return;
    };
    // `is_ready_to_confirm` already guarantees a non-empty message, but
    // guard against dispatch paths that could bypass the disabled state
    // (e.g. keyboard shortcut).
    let Some(message) = commit_message(state, ctx) else {
        return;
    };
    let intent = state.intent;
    let include_unstaged = state.include_unstaged;
    let message_editor = state.message_editor.clone();
    let branch_name = me.branch_name().to_string();
    // When the chain includes create-PR, AI-generate the PR title/body when the
    // user has it enabled (ignored for commit-only / commit-and-push).
    let autogenerate_pr_content = should_send_git_ops_ai_request(ctx);

    me.set_loading(LOADING_LABEL, ctx);

    // Lock the commit message editor while the async op is in flight.
    message_editor.update(ctx, |editor, ctx| {
        editor.set_interaction_state(InteractionState::Disabled, ctx);
    });

    me.diff_state_model().update(ctx, |m, ctx| {
        m.git_commit_chain(
            intent,
            message,
            include_unstaged,
            branch_name,
            autogenerate_pr_content,
            ctx,
        );
    });
}

/// Shared commit-chain completion for both backends: toast + telemetry + close.
/// `Ok(Some)` means create-PR ran; `Ok(None)` is a plain commit / commit-and-push.
pub(super) fn finish_commit_chain(
    me: &GitDialog,
    intent: CommitChainMode,
    result: Result<Option<PrInfo>, String>,
    ctx: &mut ViewContext<GitDialog>,
) {
    let operation = match intent {
        CommitChainMode::CommitOnly => GitOperationKind::CommitOnly,
        CommitChainMode::CommitAndPush => GitOperationKind::CommitAndPush,
        CommitChainMode::CommitAndCreatePr => GitOperationKind::CommitAndCreatePr,
    };
    let (status, error) = match &result {
        Ok(_) => (GitDialogStatus::Succeeded, None),
        Err(err) => (GitDialogStatus::Failed, Some(err.clone())),
    };
    match &result {
        Ok(Some(pr)) => show_pr_created_toast(pr, ctx),
        Ok(None) => {
            let msg = if matches!(intent, CommitChainMode::CommitOnly) {
                "Changes successfully committed."
            } else {
                "Changes committed and pushed."
            };
            show_toast(msg, ctx);
        }
        Err(err) => {
            log::error!("Commit failed: {err}");
            show_toast(user_facing_git_error(err), ctx);
        }
    }
    send_telemetry_from_ctx!(
        CodeReviewTelemetryEvent::GitDialogCompleted {
            is_local: Some(!me.repo_location().is_remote()),
            operation,
            status,
            error,
        },
        ctx
    );
    ctx.emit(GitDialogEvent::Completed);
}

fn handle_editor_event(me: &mut GitDialog, event: &EditorEvent, ctx: &mut ViewContext<GitDialog>) {
    match event {
        EditorEvent::Escape => {
            if !me.loading() {
                ctx.emit(GitDialogEvent::Cancelled);
            }
        }
        EditorEvent::Edited(_) => {
            me.refresh_confirm_enabled(ctx);
            ctx.notify();
        }
        _ => {}
    }
}

fn apply_intent_selector(state: &CommitState, ctx: &mut ViewContext<GitDialog>) {
    state.commit_button.update(ctx, |b, ctx| {
        b.set_active(state.intent == CommitChainMode::CommitOnly, ctx);
    });
    state.commit_and_push_button.update(ctx, |b, ctx| {
        b.set_active(state.intent == CommitChainMode::CommitAndPush, ctx);
    });
    if let Some(button) = &state.commit_and_create_pr_button {
        button.update(ctx, |b, ctx| {
            b.set_active(state.intent == CommitChainMode::CommitAndCreatePr, ctx);
        });
    }
}

fn reload_file_changes(me: &mut GitDialog, ctx: &mut ViewContext<GitDialog>) {
    let Some(repo_path) = me.repo_location().to_local_path().map(Path::to_path_buf) else {
        return;
    };
    let include_unstaged = match me.mode() {
        GitDialogMode::Commit(state) => state.include_unstaged,
        _ => return,
    };
    ctx.spawn(
        async move { get_file_change_entries(&repo_path, include_unstaged).await },
        |me, result, ctx| {
            if let GitDialogMode::Commit(state) = &mut me.mode {
                match result {
                    Ok(entries) => {
                        state.file_changes = entries;
                        me.refresh_confirm_enabled(ctx);
                        ctx.notify();
                    }
                    Err(err) => log::warn!("Failed to reload file changes: {err}"),
                }
            }
        },
    );
}

fn commit_message(state: &CommitState, app: &AppContext) -> Option<String> {
    let text = state.message_editor.as_ref(app).buffer_text(app);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

pub(super) fn render_body(
    state: &CommitState,
    branch_name: &str,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);

    let branch_section = render_branch_section(branch_name, appearance);
    let changes_section = render_changes_section(state, appearance);
    let message_section = render_message_editor(state, appearance, app);
    let intent_section = render_intent_buttons(state);

    Flex::column()
        .with_child(
            Container::new(branch_section)
                .with_margin_bottom(16.)
                .finish(),
        )
        .with_child(
            Container::new(changes_section)
                .with_margin_bottom(16.)
                .finish(),
        )
        .with_child(
            Container::new(message_section)
                .with_margin_bottom(16.)
                .finish(),
        )
        .with_child(intent_section)
        .finish()
}

fn render_changes_section(state: &CommitState, appearance: &Appearance) -> Box<dyn Element> {
    let theme = appearance.theme();
    let main_color = theme.main_text_color(theme.surface_1()).into_solid();
    let sub_color = theme.sub_text_color(theme.surface_1()).into_solid();

    let changes_label = Text::new(
        "Changes",
        appearance.ui_font_family(),
        appearance.ui_font_size(),
    )
    .with_color(main_color)
    .finish();

    let include_label = Text::new(
        "Include unstaged",
        appearance.ui_font_family(),
        appearance.ui_font_size(),
    )
    .with_color(sub_color)
    .finish();

    let switch = appearance
        .ui_builder()
        .switch(state.switch_state.clone())
        .check(state.include_unstaged)
        .build()
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(GitDialogAction::Commit(
                CommitSubAction::ToggleIncludeUnstaged,
            ));
        })
        .finish();

    let toggle_row = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(include_label)
        .with_child(Container::new(switch).with_margin_left(4.).finish())
        .finish();

    let header_row = Flex::row()
        .with_main_axis_size(MainAxisSize::Max)
        .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(changes_label)
        .with_child(toggle_row)
        .finish();

    let changes_box = render_file_changes_box(
        &state.file_changes,
        state.changes_expanded,
        &state.summary_mouse_state,
        &state.changes_scroll_state,
        GitDialogAction::Commit(CommitSubAction::ToggleChangesExpanded),
        appearance,
    );

    Flex::column()
        .with_child(Container::new(header_row).with_margin_bottom(8.).finish())
        .with_child(changes_box)
        .finish()
}

fn render_message_editor(
    state: &CommitState,
    appearance: &Appearance,
    app: &AppContext,
) -> Box<dyn Element> {
    let label = Text::new(
        "Commit message",
        appearance.ui_font_family(),
        appearance.ui_font_size(),
    )
    .with_color(
        appearance
            .theme()
            .main_text_color(appearance.theme().surface_1())
            .into_solid(),
    )
    .finish();

    let line_height = state
        .message_editor
        .as_ref(app)
        .line_height(app.font_cache(), appearance);

    let editor_element = appearance
        .ui_builder()
        .text_input(state.message_editor.clone())
        .with_style(UiComponentStyles {
            border_color: Some(appearance.theme().surface_3().into()),
            border_radius: Some(CornerRadius::with_all(Radius::Pixels(6.))),
            height: Some(EDITOR_MIN_HEIGHT.max(line_height * 3.)),
            ..Default::default()
        })
        .build()
        .finish();

    Flex::column()
        .with_child(Container::new(label).with_margin_bottom(8.).finish())
        .with_child(editor_element)
        .finish()
}

fn render_intent_buttons(state: &CommitState) -> Box<dyn Element> {
    let mut column = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_child(ChildView::new(&state.commit_button).finish())
        .with_child(
            Container::new(ChildView::new(&state.commit_and_push_button).finish())
                .with_margin_top(4.)
                .finish(),
        );
    if let Some(button) = &state.commit_and_create_pr_button {
        column.add_child(
            Container::new(ChildView::new(button).finish())
                .with_margin_top(4.)
                .finish(),
        );
    }
    column.finish()
}
