//! Configurable terminal-session statusline formatting, rendering, and metadata subscriptions.

use std::time::Duration;

use chrono::{Local, NaiveDateTime};
use vim::vim::{MotionType, VimMode};
use warp::settings::{AISettings, TuiStatuslineConfig, TuiStatuslineItem};
use warp::tui_export::{ConversationUsageTotals, GitRepoModels, GitStatusMetadata, LLMPreferences};
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui::SingletonEntity;
use warpui_core::elements::tui::{
    Modifier, TuiAnimated, TuiElement, TuiFlex, TuiHoverable, TuiStyle, TuiText,
};
use warpui_core::{AppContext, ViewContext};

use super::{
    CTRL_C_EXIT_HINT, CTRL_C_KILL_CHILD_HINT, ConversationRestoreState, LOADING_CONVERSATION_HINT,
    RUNNING_COMMAND_DETACH_HINT, SHELL_MODE_HINT, TuiConversationRestoreOrigin,
    TuiTerminalSessionAction, TuiTerminalSessionView, render_mcp_install_footer,
    render_mcp_menu_footer,
};
use crate::transient_hint::TransientHintTone;
use crate::tui_builder::TuiUiBuilder;
use crate::ui::compact_footer_path;
#[cfg(feature = "voice_input")]
use crate::voice_input::TuiVoiceInputState;

const STATUSLINE_DATETIME_REPAINT_INTERVAL: Duration = Duration::from_secs(60);

struct FooterHint<'a> {
    text: &'a str,
    style: FooterHintStyle,
}

enum FooterHintStyle {
    Muted,
    Success,
    Error,
    #[cfg(feature = "voice_input")]
    VoiceInput,
}

impl<'a> FooterHint<'a> {
    fn muted(text: &'a str) -> Self {
        Self {
            text,
            style: FooterHintStyle::Muted,
        }
    }

    #[cfg(feature = "voice_input")]
    fn voice_input(text: &'a str) -> Self {
        Self {
            text,
            style: FooterHintStyle::VoiceInput,
        }
    }

    fn render(self, builder: &TuiUiBuilder) -> TuiFlex {
        let style = match self.style {
            FooterHintStyle::Muted => builder.muted_text_style(),
            FooterHintStyle::Success => builder.success_glyph_style(),
            FooterHintStyle::Error => builder.error_text_style(),
            #[cfg(feature = "voice_input")]
            FooterHintStyle::VoiceInput => builder.voice_input_status_style(),
        };
        TuiFlex::row().child(
            TuiText::new(self.text)
                .with_style(style)
                .truncate()
                .finish(),
        )
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct ContextWindowUsage {
    pub(super) bar: String,
    pub(super) percentage_remaining: u8,
    pub(super) warning: bool,
}
pub(super) fn format_context_window_usage(usage: f32) -> ContextWindowUsage {
    const BAR_WIDTH: usize = 4;

    let remaining = (1.0 - usage).clamp(0.0, 1.0);
    let percentage_remaining = (remaining * 100.0).round() as u8;
    let filled = ((remaining * BAR_WIDTH as f32) + f32::EPSILON).floor() as usize;
    ContextWindowUsage {
        bar: format!("{}{}", "█".repeat(filled), "░".repeat(BAR_WIDTH - filled)),
        percentage_remaining,
        warning: percentage_remaining <= 25,
    }
}
pub(super) fn format_statusline_date(now: NaiveDateTime) -> String {
    now.format("%B %-d, %Y").to_string()
}
pub(super) fn format_statusline_time_12_hour(now: NaiveDateTime) -> String {
    now.format("%-I:%M%P").to_string()
}
pub(super) fn format_statusline_time_24_hour(now: NaiveDateTime) -> String {
    now.format("%H:%M").to_string()
}
pub(super) fn render_statusline_datetime(
    formatter: fn(NaiveDateTime) -> String,
    style: TuiStyle,
) -> Box<dyn TuiElement> {
    TuiAnimated::new(STATUSLINE_DATETIME_REPAINT_INTERVAL, move || {
        TuiText::new(formatter(Local::now().naive_local()))
            .with_style(style)
            .truncate()
            .finish()
    })
    .finish()
}
pub(super) fn format_todo_progress(completed: usize, total: usize, finished: bool) -> String {
    let marker = if finished { "✓" } else { "❒" };
    format!("{marker} {completed}/{total}")
}

/// One resolved item in the footer's configured presentation order.
pub(super) enum FooterSegment {
    ShellMode,
    AutoApproveIndicator(Box<dyn TuiElement>),
    VimIndicator(&'static str),
    Model(Box<dyn TuiElement>),
    WorkingDirectory(String),
    GitBranch(String),
    CreditUsage(Box<dyn TuiElement>),
    ContextWindowUsage(ContextWindowUsage),
    GitDiff {
        files_changed: usize,
        additions: usize,
        deletions: usize,
    },
    GitBranchStatus(Box<dyn TuiElement>),
    GitHubPullRequest(Box<dyn TuiElement>),
    DateTime(Box<dyn TuiElement>),
    AgentTodoList(Box<dyn TuiElement>),
    #[cfg(feature = "voice_input")]
    VoiceInput(Box<dyn TuiElement>),
}

impl FooterSegment {
    fn separator_to(&self, next: &Self) -> &'static str {
        match (self, next) {
            (Self::ShellMode | Self::VimIndicator(_), Self::WorkingDirectory(_)) => " ",
            (Self::WorkingDirectory(_), Self::GitBranch(_) | Self::GitBranchStatus(_)) => " ",
            (
                Self::WorkingDirectory(_) | Self::GitBranch(_),
                Self::WorkingDirectory(_) | Self::GitBranch(_),
            )
            | (Self::DateTime(_), Self::DateTime(_))
            | (Self::ShellMode, _)
            | (_, Self::ShellMode) => " • ",
            #[cfg(feature = "voice_input")]
            (Self::VoiceInput(_), _) | (_, Self::VoiceInput(_)) => " | ",
            (
                Self::AutoApproveIndicator(_)
                | Self::VimIndicator(_)
                | Self::Model(_)
                | Self::WorkingDirectory(_)
                | Self::GitBranch(_)
                | Self::CreditUsage(_)
                | Self::ContextWindowUsage(_)
                | Self::GitDiff { .. }
                | Self::GitBranchStatus(_)
                | Self::GitHubPullRequest(_)
                | Self::DateTime(_)
                | Self::AgentTodoList(_),
                Self::AutoApproveIndicator(_)
                | Self::VimIndicator(_)
                | Self::Model(_)
                | Self::WorkingDirectory(_)
                | Self::GitBranch(_)
                | Self::CreditUsage(_)
                | Self::ContextWindowUsage(_)
                | Self::GitDiff { .. }
                | Self::GitBranchStatus(_)
                | Self::GitHubPullRequest(_)
                | Self::DateTime(_)
                | Self::AgentTodoList(_),
            ) => " | ",
        }
    }
}

/// Resolved segments for the footer's left-aligned status row.
pub(super) struct FooterSegments {
    pub(super) ordered: Vec<FooterSegment>,
}
/// Builds the status row from resolved segments. Working directory follows a
/// leading shell-mode label with a plain space; a branch or composite branch
/// status owns its `⊢` glyph and follows working directory with one space.
/// Items in different Figma groups use ` | `; other adjacent pairs use ` • `.
/// The first item never receives a separator.
pub(super) fn render_status_footer_row(
    segments: FooterSegments,
    builder: &TuiUiBuilder,
) -> TuiFlex {
    let muted = builder.muted_text_style();
    let mut row = TuiFlex::row();
    let mut segments = segments.ordered.into_iter().peekable();
    while let Some(segment) = segments.next() {
        let separator = segments.peek().map(|next| segment.separator_to(next));
        match segment {
            FooterSegment::ShellMode => {
                row = row.child(
                    TuiText::new(SHELL_MODE_HINT)
                        .with_style(builder.shell_command_accent_style())
                        .truncate()
                        .finish(),
                );
            }
            FooterSegment::VimIndicator(label) => {
                row = row.child(
                    TuiText::new(label)
                        .with_style(builder.accent_border_style())
                        .truncate()
                        .finish(),
                );
            }
            FooterSegment::AutoApproveIndicator(element)
            | FooterSegment::Model(element)
            | FooterSegment::CreditUsage(element)
            | FooterSegment::GitBranchStatus(element)
            | FooterSegment::GitHubPullRequest(element)
            | FooterSegment::DateTime(element)
            | FooterSegment::AgentTodoList(element) => {
                row = row.child(element);
            }
            #[cfg(feature = "voice_input")]
            FooterSegment::VoiceInput(element) => {
                row = row.child(element);
            }
            FooterSegment::WorkingDirectory(cwd) => {
                row = row.child(TuiText::new(cwd).with_style(muted).truncate().finish());
            }
            FooterSegment::GitBranch(branch) => {
                row = row.child(
                    TuiText::new(format!("⊢ {branch}"))
                        .with_style(muted)
                        .truncate()
                        .finish(),
                );
            }
            FooterSegment::ContextWindowUsage(usage) => {
                let value_style = if usage.warning {
                    builder.attention_glyph_style()
                } else {
                    builder.primary_text_style()
                };
                row = row.child(
                    TuiText::from_spans([
                        (
                            format!("{} {}% ", usage.bar, usage.percentage_remaining),
                            value_style,
                        ),
                        ("context remaining".to_owned(), muted),
                    ])
                    .truncate()
                    .finish(),
                );
            }
            FooterSegment::GitDiff {
                files_changed,
                additions,
                deletions,
            } => {
                let mut spans = vec![(format!("☰ {files_changed}"), muted)];
                if additions > 0 || deletions > 0 {
                    spans.push((" •".to_owned(), muted));
                }
                if additions > 0 {
                    spans.push((format!(" +{additions}"), builder.diff_added_style()));
                }
                if deletions > 0 {
                    spans.push((" ".to_owned(), muted));
                    spans.push((format!("-{deletions}"), builder.diff_removed_style()));
                }
                row = row.child(TuiText::from_spans(spans).truncate().finish());
            }
        }
        if let Some(separator) = separator {
            row = row.child(
                TuiText::new(separator)
                    .with_style(muted)
                    .truncate()
                    .finish(),
            );
        }
    }

    row
}

pub(super) fn render_git_branch_status(
    branch: &str,
    rebased: bool,
    ahead: Option<String>,
    behind: Option<String>,
    builder: &TuiUiBuilder,
) -> Box<dyn TuiElement> {
    let muted = builder.muted_text_style();
    let accent = builder.accent_text_style();
    let has_ahead = ahead.is_some();
    let has_behind = behind.is_some();
    let mut spans = vec![(format!("⊢ {branch}"), muted)];
    if rebased || has_ahead || has_behind {
        spans.push((" • ".to_owned(), muted));
    }
    if rebased {
        spans.push(("⇅".to_owned(), accent));
    } else {
        if let Some(ahead) = ahead {
            spans.push(("↑".to_owned(), accent));
            spans.push((
                format!("{ahead}{}", if has_behind { " " } else { "" }),
                muted,
            ));
        }
        if let Some(behind) = behind {
            spans.push(("↓".to_owned(), accent));
            spans.push((behind, muted));
        }
    }
    TuiText::from_spans(spans).truncate().finish()
}

pub(super) fn should_render_plain_git_branch(config: &TuiStatuslineConfig) -> bool {
    config.is_enabled(TuiStatuslineItem::GitBranch)
        && !config.is_enabled(TuiStatuslineItem::GitBranchStatus)
}

impl TuiTerminalSessionView {
    /// Selects the single message that replaces the normal footer, preserving
    /// the priority order between competing session states.
    fn footer_hint(
        &self,
        voice_statusline_visible: bool,
        ctx: &AppContext,
    ) -> Option<FooterHint<'_>> {
        if self.exit_confirmation.is_armed() {
            // When the kill-child window is armed, show the child-specific hint
            // so the user knows the next ctrl-c will kill the child agent rather
            // than exiting the whole TUI.
            if self.child_kill_armed_conversation.is_some() {
                return Some(FooterHint::muted(CTRL_C_KILL_CHILD_HINT));
            }
            return Some(FooterHint::muted(CTRL_C_EXIT_HINT));
        }
        if matches!(
            &self.conversation_restore_state,
            ConversationRestoreState::Loading {
                origin: TuiConversationRestoreOrigin::ConversationList,
                ..
            }
        ) {
            return Some(FooterHint::muted(LOADING_CONVERSATION_HINT));
        }
        if let Some((text, tone)) = self.transient_hint.current() {
            let style = match tone {
                TransientHintTone::Muted => FooterHintStyle::Muted,
                TransientHintTone::Success => FooterHintStyle::Success,
                TransientHintTone::Error => FooterHintStyle::Error,
            };
            return Some(FooterHint { text, style });
        }
        if self
            .session_state(ctx)
            .is_ok_and(|state| state.agent_is_tagged_in())
        {
            return Some(FooterHint::muted(RUNNING_COMMAND_DETACH_HINT));
        }
        #[cfg(feature = "voice_input")]
        {
            if voice_statusline_visible {
                return None;
            }
            match self.input_view.as_ref(ctx).voice_state(ctx) {
                TuiVoiceInputState::Listening => {
                    let hint = if self.input_view.as_ref(ctx).voice_hold_key(ctx).is_some() {
                        "listening to voice input... · release key to stop"
                    } else {
                        "listening to voice input... · esc or enter to stop"
                    };
                    return Some(FooterHint::voice_input(hint));
                }
                TuiVoiceInputState::Transcribing => {
                    return Some(FooterHint::voice_input("Transcribing... · esc to cancel"));
                }
                TuiVoiceInputState::Idle => {}
            }
        }
        #[cfg(not(feature = "voice_input"))]
        let _ = voice_statusline_visible;
        None
    }

    /// Builds the configured statusline under the input box. Normal mode uses
    /// the persisted item order and visibility; shell mode always leads with
    /// its mode label and resolves configured shell-relevant metadata. A
    /// replacing hint — the ctrl-c exit confirmation while armed, the
    /// conversation-list loading hint, an active transient notice, or the
    /// interrupt hint for a manually attached running command — occupies the
    /// whole row instead. An open MCP install flow or management menu similarly
    /// replaces the statusline with its controls. An empty resolved configuration
    /// consumes no row.
    pub(super) fn render_footer(&self, ctx: &AppContext) -> TuiFlex {
        let builder = TuiUiBuilder::from_app(ctx);
        let shell_mode = self.is_shell_mode(ctx);
        let config = AISettings::as_ref(ctx).tui_statusline.normalized();
        let voice_statusline_visible = config.is_enabled(TuiStatuslineItem::VoiceInput)
            && self.voice_statusline_is_available(shell_mode, ctx);
        if let Some(hint) = self.footer_hint(voice_statusline_visible, ctx) {
            return hint.render(&builder);
        }
        if self.mcp_install_flow.as_ref(ctx).is_open(ctx) {
            return render_mcp_install_footer(
                &builder,
                self.mcp_install_flow.as_ref(ctx).primary_action_hint(),
            );
        }
        if self.mcp_menu.as_ref(ctx).is_open(ctx) {
            let menu = self.mcp_menu.as_ref(ctx);
            return render_mcp_menu_footer(
                &builder,
                menu.selected_primary_action(ctx),
                menu.can_log_out_selected(ctx),
            );
        }
        let git_metadata = self.git_status_metadata(ctx);
        let mut ordered = Vec::new();
        if shell_mode {
            ordered.push(FooterSegment::ShellMode);
        }
        for item in config.order.iter().copied() {
            if !config.is_enabled(item) {
                continue;
            }
            let segment = match item {
                TuiStatuslineItem::AutoApprove => (!shell_mode).then(|| {
                    FooterSegment::AutoApproveIndicator(
                        self.render_auto_approve_statusline(&builder, ctx),
                    )
                }),
                TuiStatuslineItem::VimModeIndicator => self
                    .vim_mode_indicator(ctx)
                    .map(FooterSegment::VimIndicator),
                TuiStatuslineItem::Model => (!shell_mode).then(|| {
                    let model_name = LLMPreferences::as_ref(ctx)
                        .get_active_base_model(ctx, Some(self.terminal_surface_id))
                        .display_name
                        .clone();
                    let model_label_hovered = self
                        .model_label_hover
                        .lock()
                        .is_ok_and(|state| state.is_hovered());
                    let model_label_style = if model_label_hovered {
                        builder.primary_text_style()
                    } else {
                        builder.muted_text_style()
                    };
                    FooterSegment::Model(
                        TuiHoverable::new(
                            self.model_label_hover.clone(),
                            TuiText::new(model_name)
                                .with_style(model_label_style)
                                .truncate()
                                .finish(),
                        )
                        .on_click(|event_ctx, _| {
                            event_ctx
                                .dispatch_typed_action(TuiTerminalSessionAction::ToggleModelMenu);
                        })
                        .finish(),
                    )
                }),
                TuiStatuslineItem::GitHubPullRequest => (!shell_mode)
                    .then_some(self.github_repo.as_ref())
                    .flatten()
                    .and_then(|repo| repo.as_ref(ctx).pr_info(ctx))
                    .map(|pr| {
                        let url = pr.url.clone();
                        FooterSegment::GitHubPullRequest(self.github_pr_link.render(
                            format!("PR #{}", pr.number),
                            builder.muted_text_style(),
                            move |event_ctx, _| {
                                event_ctx.dispatch_typed_action(TuiTerminalSessionAction::OpenUrl(
                                    url.clone(),
                                ));
                            },
                        ))
                    }),
                TuiStatuslineItem::WorkingDirectory => self
                    .current_working_directory(ctx)
                    .map(|cwd| FooterSegment::WorkingDirectory(compact_footer_path(&cwd))),
                TuiStatuslineItem::GitBranch => should_render_plain_git_branch(&config)
                    .then(|| {
                        git_metadata.map(|metadata| {
                            FooterSegment::GitBranch(metadata.current_branch_name.clone())
                        })
                    })
                    .flatten(),
                TuiStatuslineItem::GitBranchStatus => git_metadata.map(|metadata| {
                    let tracking = &metadata.branch_tracking_status;
                    FooterSegment::GitBranchStatus(render_git_branch_status(
                        &metadata.current_branch_name,
                        tracking.is_rebased(),
                        tracking.ahead_display_count(),
                        tracking.behind_display_count(),
                        &builder,
                    ))
                }),
                TuiStatuslineItem::GitDiffStatus => git_metadata.and_then(|metadata| {
                    let stats = metadata.stats_against_head;
                    (stats.files_changed > 0).then_some(FooterSegment::GitDiff {
                        files_changed: stats.files_changed,
                        additions: stats.total_additions,
                        deletions: stats.total_deletions,
                    })
                }),
                TuiStatuslineItem::CreditUsage => (!shell_mode)
                    .then(|| self.selected_conversation_usage_totals(ctx))
                    .flatten()
                    .map(|totals| {
                        let mode = AISettings::as_ref(ctx).usage_display_mode;
                        FooterSegment::CreditUsage(self.usage_toggle.render_entry(
                            mode,
                            totals,
                            ctx,
                            |event_ctx, _| {
                                event_ctx.dispatch_typed_action(
                                    TuiTerminalSessionAction::ToggleUsageDisplay,
                                );
                            },
                        ))
                    }),
                TuiStatuslineItem::ContextWindowUsage => (!shell_mode)
                    .then(|| {
                        self.conversation_selection
                            .as_ref(ctx)
                            .selected_conversation(ctx)
                    })
                    .flatten()
                    .map(|conversation| {
                        FooterSegment::ContextWindowUsage(format_context_window_usage(
                            conversation.context_window_usage(),
                        ))
                    }),
                TuiStatuslineItem::Date => Some(FooterSegment::DateTime(
                    render_statusline_datetime(format_statusline_date, builder.muted_text_style()),
                )),
                TuiStatuslineItem::Time12Hour => {
                    Some(FooterSegment::DateTime(render_statusline_datetime(
                        format_statusline_time_12_hour,
                        builder.muted_text_style(),
                    )))
                }
                TuiStatuslineItem::Time24Hour => {
                    Some(FooterSegment::DateTime(render_statusline_datetime(
                        format_statusline_time_24_hour,
                        builder.muted_text_style(),
                    )))
                }
                TuiStatuslineItem::AgentTodoList => (!shell_mode)
                    .then(|| {
                        self.conversation_selection
                            .as_ref(ctx)
                            .selected_conversation(ctx)
                    })
                    .flatten()
                    .and_then(|conversation| conversation.active_todo_list())
                    .filter(|todo_list| !todo_list.is_empty())
                    .map(|todo_list| {
                        let hovered = self
                            .todo_list_mouse
                            .lock()
                            .is_ok_and(|state| state.is_hovered());
                        let style = if hovered {
                            builder.primary_text_style()
                        } else {
                            builder.muted_text_style()
                        };
                        let progress = format_todo_progress(
                            todo_list.completed_items().len(),
                            todo_list.len(),
                            todo_list.is_finished(),
                        );
                        FooterSegment::AgentTodoList(
                            TuiHoverable::new(
                                self.todo_list_mouse.clone(),
                                TuiText::new(progress).with_style(style).truncate().finish(),
                            )
                            .on_click(|event_ctx, _| {
                                event_ctx.dispatch_typed_action(
                                    TuiTerminalSessionAction::ToggleTodoMenu,
                                );
                            })
                            .finish(),
                        )
                    }),
                TuiStatuslineItem::VoiceInput => {
                    #[cfg(feature = "voice_input")]
                    {
                        voice_statusline_visible.then(|| {
                            FooterSegment::VoiceInput(self.render_voice_statusline(&builder, ctx))
                        })
                    }
                    #[cfg(not(feature = "voice_input"))]
                    {
                        None
                    }
                }
            };
            if let Some(segment) = segment {
                ordered.push(segment);
            }
        }
        render_status_footer_row(FooterSegments { ordered }, &builder)
    }
    /// Returns a brief Vim mode label for the footer when Vim mode is enabled.
    pub(super) fn vim_mode_indicator(&self, ctx: &AppContext) -> Option<&'static str> {
        let mode = self.input_view.as_ref(ctx).vim_mode(ctx)?;
        match mode {
            VimMode::Normal => Some("NOR"),
            VimMode::Visual(MotionType::Charwise) => Some("VIS"),
            VimMode::Visual(MotionType::Linewise) => Some("V-L"),
            VimMode::Replace => Some("REP"),
            VimMode::Insert => Some("INS"),
        }
    }

    pub(super) fn render_auto_approve_statusline(
        &self,
        builder: &TuiUiBuilder,
        ctx: &AppContext,
    ) -> Box<dyn TuiElement> {
        let enabled = self
            .conversation_selection
            .as_ref(ctx)
            .pending_query_autoexecute_override(ctx)
            .is_autoexecute_any_action();
        let hovered = self
            .footer_auto_approve_mouse
            .lock()
            .is_ok_and(|state| state.is_hovered());
        let mut style = if enabled {
            builder.success_glyph_style()
        } else {
            builder.muted_text_style()
        };
        if hovered {
            style = style.add_modifier(Modifier::BOLD);
        }
        TuiHoverable::new(
            self.footer_auto_approve_mouse.clone(),
            TuiText::new("▶▶").with_style(style).truncate().finish(),
        )
        .on_click(|event_ctx, _| {
            event_ctx.dispatch_typed_action(TuiTerminalSessionAction::ToggleAutoApprove {
                show_feedback: false,
            });
        })
        .finish()
    }

    #[cfg(feature = "voice_input")]
    pub(super) fn voice_statusline_is_available(&self, shell_mode: bool, ctx: &AppContext) -> bool {
        !shell_mode && AISettings::as_ref(ctx).is_voice_input_enabled(ctx)
    }

    #[cfg(not(feature = "voice_input"))]
    pub(super) fn voice_statusline_is_available(
        &self,
        _shell_mode: bool,
        _ctx: &AppContext,
    ) -> bool {
        false
    }

    #[cfg(feature = "voice_input")]
    fn render_voice_statusline(
        &self,
        builder: &TuiUiBuilder,
        ctx: &AppContext,
    ) -> Box<dyn TuiElement> {
        let state = self.input_view.as_ref(ctx).voice_state(ctx);
        let hovered = self
            .voice_input_mouse
            .lock()
            .is_ok_and(|state| state.is_hovered());
        let (label, style) = match state {
            TuiVoiceInputState::Idle => (
                "◉ Voice",
                if hovered {
                    builder.primary_text_style().add_modifier(Modifier::BOLD)
                } else {
                    builder.primary_text_style()
                },
            ),
            TuiVoiceInputState::Listening => ("◉ Voice", builder.success_glyph_style()),
            TuiVoiceInputState::Transcribing => {
                return TuiText::new("… Transcribing")
                    .with_style(builder.voice_input_status_style())
                    .truncate()
                    .finish();
            }
        };
        TuiHoverable::new(
            self.voice_input_mouse.clone(),
            TuiText::new(label).with_style(style).truncate().finish(),
        )
        .on_click(|event_ctx, _| {
            event_ctx
                .dispatch_typed_action(TuiTerminalSessionAction::ToggleVoiceInputFromStatusline);
        })
        .finish()
    }

    /// Updates the watcher-backed git subscription after repository detection
    /// completes for the active working directory. GitHub metadata is retained
    /// only while its statusline item is enabled.
    pub(super) fn update_git_status_subscription(
        &mut self,
        repo_path: Option<LocalOrRemotePath>,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.current_repo_path == repo_path && self.git_repo_status.is_some() {
            self.update_github_status_subscription(ctx);
            return;
        }
        self.current_repo_path = repo_path.clone();
        self.git_repo_status = None;
        self.github_repo = None;

        let Some(repo_path) = repo_path else {
            ctx.notify();
            return;
        };
        match GitRepoModels::handle(ctx)
            .update(ctx, |models, ctx| models.subscribe(&repo_path, ctx))
        {
            Ok(handle) => {
                ctx.subscribe_to_model(&handle, |_, _, _, ctx| ctx.notify());
                self.git_repo_status = Some(handle);
            }
            Err(error) => {
                log::warn!("Unable to subscribe TUI footer to git status: {error}");
            }
        }
        self.update_github_status_subscription(ctx);
        ctx.notify();
    }

    pub(super) fn update_github_status_subscription(&mut self, ctx: &mut ViewContext<Self>) {
        let enabled = AISettings::as_ref(ctx)
            .tui_statusline
            .normalized()
            .is_enabled(TuiStatuslineItem::GitHubPullRequest);
        if !enabled {
            self.github_repo = None;
            ctx.notify();
            return;
        }
        if self.github_repo.is_some() {
            return;
        }
        let Some(repo_path) = self.current_repo_path.clone() else {
            return;
        };
        match GitRepoModels::handle(ctx).update(ctx, |models, ctx| {
            models.subscribe_github_repo(&repo_path, ctx)
        }) {
            Ok(handle) => {
                ctx.subscribe_to_model(&handle, |_, _, _, ctx| ctx.notify());
                self.github_repo = Some(handle);
            }
            Err(error) => {
                log::warn!("Unable to subscribe TUI footer to GitHub status: {error}");
            }
        }
        ctx.notify();
    }

    fn git_status_metadata<'a>(&self, ctx: &'a AppContext) -> Option<&'a GitStatusMetadata> {
        self.git_repo_status.as_ref()?.as_ref(ctx).metadata(ctx)
    }

    /// The selected conversation's accumulated usage totals, or `None` (entry
    /// hidden) until any usage has been reported.
    pub(super) fn selected_conversation_usage_totals(
        &self,
        ctx: &AppContext,
    ) -> Option<ConversationUsageTotals> {
        let totals = self
            .conversation_selection
            .as_ref(ctx)
            .selected_conversation(ctx)?
            .usage_totals();
        totals.has_usage.then_some(totals)
    }
}
