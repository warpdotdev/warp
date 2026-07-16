use instant::Instant;
use warp::tui_export::{BlocklistAIHistoryModel, ConversationStatus, TerminalSurfaceInit};
use warpui::SingletonEntity;
use warpui_core::r#async::Timer;
use warpui_core::elements::CrossAxisAlignment;
use warpui_core::elements::tui::{
    TuiChildView, TuiContainer, TuiElement, TuiEventHandler, TuiFlex, TuiText,
};
use warpui_core::platform::TerminationMode;
use warpui_core::{AppContext, ModelHandle, ViewContext};

use super::{TuiTerminalSessionAction, TuiTerminalSessionMode, TuiTerminalSessionView};
use crate::agent_message::{conversation_status_glyph, conversation_status_glyph_style};
use crate::cloud_run::{TuiCloudRunStartup, TuiCloudRunState};
use crate::exit_confirmation::CTRL_C_EXIT_WINDOW;
use crate::orchestration_model::TuiOrchestrationSnapshot;
use crate::resume::TuiExitSummaryHandle;
use crate::tui_builder::TuiUiBuilder;
use crate::ui::centered_in_viewport;

/// Derived display state for a cloud session's status and primary link.
struct CloudSessionDisplayState {
    status: ConversationStatus,
    status_label: String,
    detail: Option<String>,
    link_label: Option<&'static str>,
    link_url: Option<String>,
}

impl TuiTerminalSessionView {
    /// Builds a read-only deferred cloud session over the normal terminal surface plumbing.
    pub(crate) fn new_cloud(
        surface_init: TerminalSurfaceInit,
        cloud_run_state: ModelHandle<TuiCloudRunState>,
        exit_summary: TuiExitSummaryHandle,
        keyboard_enhancement_supported: bool,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(&cloud_run_state, |_, _, _, ctx| ctx.notify());
        let mut view = Self::new(
            surface_init,
            exit_summary,
            keyboard_enhancement_supported,
            ctx,
        );
        view.mode = TuiTerminalSessionMode::Cloud(cloud_run_state);
        view
    }

    /// Whether this is a read-only cloud session rather than a local PTY session.
    pub(super) fn is_cloud_session(&self) -> bool {
        matches!(self.mode, TuiTerminalSessionMode::Cloud(_))
    }

    /// Derives cloud-session display state from startup state and conversation history.
    fn cloud_session_display_state(&self, ctx: &AppContext) -> Option<CloudSessionDisplayState> {
        let TuiTerminalSessionMode::Cloud(state) = &self.mode else {
            return None;
        };
        let state = state.as_ref(ctx);
        Some(match state.startup() {
            TuiCloudRunStartup::Dispatching => CloudSessionDisplayState {
                status: ConversationStatus::InProgress,
                status_label: "Starting cloud run…".to_string(),
                detail: None,
                link_label: None,
                link_url: None,
            },
            TuiCloudRunStartup::Blocked(blocker) => CloudSessionDisplayState {
                status: ConversationStatus::Blocked {
                    blocked_action: blocker.message().to_owned(),
                },
                status_label: "GitHub authentication required".to_string(),
                detail: Some(format!(
                    "{} Authenticate, then run the orchestration request again.",
                    blocker.message()
                )),
                link_label: Some("Click the link or hit Enter to authenticate:"),
                link_url: Some(blocker.primary_url().to_string()),
            },
            TuiCloudRunStartup::Failed(failure) => CloudSessionDisplayState {
                status: ConversationStatus::Error,
                status_label: "Cloud run failed to start".to_string(),
                detail: Some(failure.message().to_string()),
                link_label: None,
                link_url: None,
            },
            TuiCloudRunStartup::Spawned => {
                let status = state
                    .conversation_id()
                    .and_then(|conversation_id| {
                        BlocklistAIHistoryModel::as_ref(ctx)
                            .conversation(&conversation_id)
                            .map(|conversation| conversation.status())
                    })
                    .unwrap_or(&ConversationStatus::InProgress);
                let status_label = match status {
                    ConversationStatus::InProgress
                    | ConversationStatus::TransientError
                    | ConversationStatus::WaitingForEvents => "Cloud run in progress",
                    ConversationStatus::Blocked { .. } => "Cloud run blocked",
                    ConversationStatus::Success => "Cloud run succeeded",
                    ConversationStatus::Error => "Cloud run failed",
                    ConversationStatus::Cancelled => "Cloud run cancelled",
                };
                CloudSessionDisplayState {
                    status: status.clone(),
                    status_label: status_label.to_string(),
                    detail: None,
                    link_label: Some("Click the link or hit Enter to view cloud run here:"),
                    link_url: state.run_url().map(str::to_string),
                }
            }
        })
    }

    /// Renders the cloud session's status, primary link, and optional orchestration tabs.
    pub(super) fn render_cloud_session(
        &self,
        orchestration_tabs: Option<&TuiOrchestrationSnapshot>,
        builder: &TuiUiBuilder,
        ctx: &AppContext,
    ) -> Box<dyn TuiElement> {
        let Some(display_state) = self.cloud_session_display_state(ctx) else {
            return TuiFlex::column().finish();
        };
        let mut content = TuiFlex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .child(
                TuiText::from_spans([
                    (
                        format!("{} ", conversation_status_glyph(&display_state.status)),
                        conversation_status_glyph_style(&display_state.status, builder),
                    ),
                    (display_state.status_label, builder.primary_text_style()),
                ])
                .finish(),
            );
        if let Some(detail) = display_state.detail {
            content = content.child(
                TuiText::new(detail)
                    .with_style(builder.muted_text_style())
                    .finish(),
            );
        }
        if let (Some(label), Some(url)) = (display_state.link_label, display_state.link_url.clone())
        {
            let click_url = url.clone();
            content = content
                .child(
                    TuiText::new(label)
                        .with_style(builder.muted_text_style())
                        .finish(),
                )
                .child(self.cloud_link.render(url, ctx, move |event_ctx, _| {
                    event_ctx.dispatch_typed_action(TuiTerminalSessionAction::OpenCloudRunUrl(
                        click_url.clone(),
                    ));
                }));
        }
        let body = centered_in_viewport(content.finish());
        let body = if let Some(url) = display_state.link_url {
            TuiEventHandler::new(body)
                .on_key("enter", move |_, event_ctx, _| {
                    event_ctx.dispatch_typed_action(TuiTerminalSessionAction::OpenCloudRunUrl(
                        url.clone(),
                    ));
                })
                .finish()
        } else {
            body
        };
        if orchestration_tabs.is_some() {
            let footer = if self.orchestration_tabs_focused {
                self.render_cloud_orchestration_tab_footer(builder)
            } else {
                TuiText::new("Shift + ↑ sub-agents")
                    .with_style(builder.muted_text_style())
                    .truncate()
                    .finish()
            };
            let session = TuiFlex::column()
                .flex_child(body)
                .child(TuiContainer::new(footer).with_padding_x(2).finish())
                .finish();
            TuiFlex::column()
                .child(TuiChildView::new(&self.orchestration_tab_bar).finish())
                .flex_child(session)
                .finish()
        } else {
            body
        }
    }

    pub(super) fn primary_cloud_run_url(&self, ctx: &AppContext) -> Option<String> {
        self.cloud_session_display_state(ctx)
            .and_then(|display_state| display_state.link_url)
    }

    /// Handles ctrl-c for a read-only cloud session, which has no local input,
    /// conversation, or PTY to interrupt. The first press arms the standard exit
    /// confirmation; a second press within the confirmation window exits the TUI.
    pub(super) fn handle_cloud_session_interrupt(&mut self, ctx: &mut ViewContext<Self>) {
        let now = Instant::now();
        if self.exit_confirmation.should_exit(now) {
            ctx.terminate_app(TerminationMode::ForceTerminate, None);
            return;
        }
        let window_expires_at = self.exit_confirmation.arm(now);
        ctx.spawn(Timer::after(CTRL_C_EXIT_WINDOW), move |view, _, ctx| {
            if view.exit_confirmation.disarm_expired(window_expires_at) {
                ctx.notify();
            }
        });
        ctx.notify();
    }
}
