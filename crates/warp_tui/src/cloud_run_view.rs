use instant::Instant;
use warp::tui_export::{BlocklistAIHistoryModel, ConversationStatus};
use warp_errors::report_error;
use warpui::SingletonEntity as _;
use warpui_core::r#async::Timer;
use warpui_core::elements::CrossAxisAlignment;
use warpui_core::elements::tui::{
    TuiChildView, TuiContainer, TuiElement, TuiEventHandler, TuiFlex, TuiText,
};
use warpui_core::keymap::macros::*;
use warpui_core::keymap::{self, EditableBinding, FixedBinding};
use warpui_core::platform::TerminationMode;
use warpui_core::{
    AppContext, Entity, EntityId, ModelHandle, TuiView, TypedActionView, ViewContext, ViewHandle,
};

use crate::agent_message::{conversation_status_glyph, conversation_status_glyph_style};
use crate::cloud_run::{TuiCloudRunStartup, TuiCloudRunState};
use crate::exit_confirmation::{CTRL_C_EXIT_WINDOW, ExitConfirmation};
use crate::keybindings::TUI_BINDING_GROUP;
use crate::link::TuiLink;
use crate::orchestration_model::{TuiOrchestrationModel, TuiOrchestrationSnapshot};
use crate::orchestration_tabs::{
    ORCHESTRATION_TAB_BAR_FOCUSED_FLAG, orchestration_tab_bar_config,
    render_cloud_orchestration_tab_footer,
};
use crate::session_registry::TuiSessions;
use crate::tab_bar::{
    TuiTabBarConfig, TuiTabBarEvent, TuiTabBarNavigationDirection, TuiTabBarSecondaryEdge,
    TuiTabBarView,
};
use crate::tui_builder::TuiUiBuilder;
use crate::ui::centered_in_viewport;

#[derive(Debug, Clone)]
pub(crate) enum TuiCloudRunAction {
    Interrupt,
    OpenUrl(String),
    OpenPrimaryUrl,
    FocusOrchestrationTabs,
    SelectPreviousOrchestrationTab,
    SelectNextOrchestrationTab,
    SelectFirstOrchestrationChild,
    SelectLastOrchestrationChild,
}

struct CloudRunDisplayState {
    status: ConversationStatus,
    status_label: String,
    detail: Option<String>,
    link_label: Option<&'static str>,
    link_url: Option<String>,
}

pub(crate) struct TuiCloudRunView {
    state: ModelHandle<TuiCloudRunState>,
    link: TuiLink,
    orchestration_tab_bar: ViewHandle<TuiTabBarView>,
    orchestration_tabs_focused: bool,
    exit_confirmation: ExitConfirmation,
    surface_id: EntityId,
}

pub(crate) fn init(app: &mut AppContext) {
    app.register_fixed_bindings([FixedBinding::new(
        "ctrl-c",
        TuiCloudRunAction::Interrupt,
        id!(TuiCloudRunView::ui_name()),
    )
    .with_group(TUI_BINDING_GROUP)]);

    let view_context = id!(TuiCloudRunView::ui_name());
    let tab_context = view_context.clone() & id!(ORCHESTRATION_TAB_BAR_FOCUSED_FLAG);
    app.register_editable_bindings([
        EditableBinding::new(
            "tui:cloud_session:open_url",
            "Open the cloud run link",
            TuiCloudRunAction::OpenPrimaryUrl,
        )
        .with_context_predicate(view_context.clone())
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("enter"),
        EditableBinding::new(
            "tui:cloud_session:focus_orchestration_tabs",
            "Focus the orchestration tab bar",
            TuiCloudRunAction::FocusOrchestrationTabs,
        )
        .with_context_predicate(view_context)
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("shift-up"),
        EditableBinding::new(
            "tui:orchestration_tabs:previous",
            "Select the previous orchestration tab",
            TuiCloudRunAction::SelectPreviousOrchestrationTab,
        )
        .with_context_predicate(tab_context.clone())
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("left"),
        EditableBinding::new(
            "tui:orchestration_tabs:previous",
            "Select the previous orchestration tab",
            TuiCloudRunAction::SelectPreviousOrchestrationTab,
        )
        .with_context_predicate(tab_context.clone())
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("shift-tab"),
        EditableBinding::new(
            "tui:orchestration_tabs:next",
            "Select the next orchestration tab",
            TuiCloudRunAction::SelectNextOrchestrationTab,
        )
        .with_context_predicate(tab_context.clone())
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("right"),
        EditableBinding::new(
            "tui:orchestration_tabs:next",
            "Select the next orchestration tab",
            TuiCloudRunAction::SelectNextOrchestrationTab,
        )
        .with_context_predicate(tab_context.clone())
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("tab"),
        EditableBinding::new(
            "tui:orchestration_tabs:first_child",
            "Select the first child agent",
            TuiCloudRunAction::SelectFirstOrchestrationChild,
        )
        .with_context_predicate(tab_context.clone())
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("shift-left"),
        EditableBinding::new(
            "tui:orchestration_tabs:last_child",
            "Select the last child agent",
            TuiCloudRunAction::SelectLastOrchestrationChild,
        )
        .with_context_predicate(tab_context)
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("shift-right"),
    ]);
}

impl TuiCloudRunView {
    pub(crate) fn new(state: ModelHandle<TuiCloudRunState>, ctx: &mut ViewContext<Self>) -> Self {
        let orchestration_tab_bar = ctx.add_typed_action_tui_view(|_| TuiTabBarView::empty());
        ctx.subscribe_to_model(&state, |view, _, _, ctx| {
            view.refresh_orchestration_tab_state(ctx);
            ctx.notify();
        });
        ctx.subscribe_to_model(&BlocklistAIHistoryModel::handle(ctx), |_, _, _, ctx| {
            ctx.notify();
        });
        ctx.subscribe_to_view(&orchestration_tab_bar, |view, _, event, ctx| match event {
            TuiTabBarEvent::SelectTab(conversation_id) => {
                view.switch_to_orchestration_tab(Some(conversation_id.clone()), ctx);
            }
            TuiTabBarEvent::PageChanged(page_anchor) => {
                let Ok(page_anchor) = page_anchor.clone().try_into() else {
                    return;
                };
                TuiOrchestrationModel::handle(ctx).update(ctx, |model, ctx| {
                    model.set_explicit_page(page_anchor, ctx);
                });
            }
        });
        Self {
            state,
            link: TuiLink::default(),
            orchestration_tab_bar,
            orchestration_tabs_focused: false,
            exit_confirmation: ExitConfirmation::default(),
            surface_id: ctx.view_id(),
        }
    }

    pub(crate) fn activate(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.focus_self();
        ctx.notify();
    }

    pub(crate) fn refresh_orchestration_tab_state(&mut self, ctx: &mut ViewContext<Self>) {
        let snapshot = self.compute_orchestration_tab_snapshot(ctx);
        let config = snapshot
            .as_ref()
            .map(|snapshot| {
                orchestration_tab_bar_config(
                    snapshot,
                    self.orchestration_tabs_focused,
                    &TuiUiBuilder::from_app(ctx),
                )
            })
            .unwrap_or_else(|| TuiTabBarConfig::new(Vec::new()));
        self.set_orchestration_tab_bar_config(config, ctx);
        if !self.orchestration_tab_bar.as_ref(ctx).has_tabs() {
            self.orchestration_tabs_focused = false;
        }
        ctx.notify();
    }

    pub(crate) fn set_orchestration_tab_focus(
        &mut self,
        focused: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        self.orchestration_tabs_focused =
            focused && self.orchestration_tab_bar.as_ref(ctx).has_tabs();
        self.refresh_orchestration_tab_state(ctx);
        ctx.focus_self();
    }

    fn compute_orchestration_tab_snapshot(
        &self,
        ctx: &AppContext,
    ) -> Option<TuiOrchestrationSnapshot> {
        if !ctx.has_singleton_model::<TuiOrchestrationModel>()
            || !ctx.has_singleton_model::<TuiSessions>()
        {
            return None;
        }
        let conversation_id = self.state.as_ref(ctx).conversation_id()?;
        TuiOrchestrationModel::as_ref(ctx).snapshot(conversation_id, ctx)
    }

    fn set_orchestration_tab_bar_config(
        &self,
        config: TuiTabBarConfig,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Err(error) = self
            .orchestration_tab_bar
            .update(ctx, |tab_bar, ctx| tab_bar.set_config(config, ctx))
        {
            report_error!(
                anyhow::Error::new(error)
                    .context("Failed to update cloud orchestration tab bar configuration"),
                warp_errors::ReportErrorLogMode::OncePerRun
            );
        }
    }

    fn switch_to_orchestration_tab(&mut self, key: Option<String>, ctx: &mut ViewContext<Self>) {
        let Some(conversation_id) = key.and_then(|key| key.try_into().ok()) else {
            return;
        };
        let session_id = TuiOrchestrationModel::handle(ctx).update(ctx, |model, ctx| {
            model.focus_conversation_session(conversation_id, ctx)
        });
        let Some(session_id) = session_id else {
            return;
        };
        if session_id.surface_id() == self.surface_id {
            self.set_orchestration_tab_focus(true, ctx);
            return;
        }
        self.orchestration_tabs_focused = false;
        ctx.notify();
        TuiSessions::set_orchestration_tab_focus(session_id, true, ctx);
    }

    fn display_state(&self, ctx: &AppContext) -> CloudRunDisplayState {
        let state = self.state.as_ref(ctx);
        match state.startup() {
            TuiCloudRunStartup::Dispatching => CloudRunDisplayState {
                status: ConversationStatus::InProgress,
                status_label: "Starting cloud run…".to_string(),
                detail: None,
                link_label: None,
                link_url: None,
            },
            TuiCloudRunStartup::Blocked(blocker) => CloudRunDisplayState {
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
            TuiCloudRunStartup::Failed(failure) => CloudRunDisplayState {
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
                CloudRunDisplayState {
                    status: status.clone(),
                    status_label: status_label.to_string(),
                    detail: None,
                    link_label: Some("Click the link or hit Enter to view cloud run here:"),
                    link_url: state.run_url().map(str::to_string),
                }
            }
        }
    }

    fn primary_url(&self, ctx: &AppContext) -> Option<String> {
        self.display_state(ctx).link_url
    }

    fn handle_interrupt(&mut self, ctx: &mut ViewContext<Self>) {
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

#[cfg(test)]
#[path = "cloud_run_view_tests.rs"]
mod tests;

impl Entity for TuiCloudRunView {
    type Event = ();
}

impl TuiView for TuiCloudRunView {
    fn ui_name() -> &'static str {
        "TuiCloudRunView"
    }

    fn child_view_ids(&self, _ctx: &AppContext) -> Vec<EntityId> {
        vec![self.orchestration_tab_bar.id()]
    }

    fn keymap_context(&self, _ctx: &AppContext) -> keymap::Context {
        let mut context = Self::default_keymap_context();
        if self.orchestration_tabs_focused {
            context.set.insert(ORCHESTRATION_TAB_BAR_FOCUSED_FLAG);
        }
        context
    }

    fn render(&self, ctx: &AppContext) -> Box<dyn TuiElement> {
        let builder = TuiUiBuilder::from_app(ctx);
        let display_state = self.display_state(ctx);
        let mut content = TuiFlex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .child(
                TuiText::from_spans([
                    (
                        format!("{} ", conversation_status_glyph(&display_state.status)),
                        conversation_status_glyph_style(&display_state.status, &builder),
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
                .child(self.link.render(url, ctx, move |event_ctx, _| {
                    event_ctx.dispatch_typed_action(TuiCloudRunAction::OpenUrl(click_url.clone()));
                }));
        }
        let body = centered_in_viewport(content.finish());
        let body = if let Some(url) = display_state.link_url {
            TuiEventHandler::new(body)
                .on_key("enter", move |_, event_ctx, _| {
                    event_ctx.dispatch_typed_action(TuiCloudRunAction::OpenUrl(url.clone()));
                })
                .finish()
        } else {
            body
        };
        if self.orchestration_tab_bar.as_ref(ctx).has_tabs() {
            let footer = if self.orchestration_tabs_focused {
                render_cloud_orchestration_tab_footer(&builder)
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
}

impl TypedActionView for TuiCloudRunView {
    type Action = TuiCloudRunAction;

    fn handle_action(&mut self, action: &TuiCloudRunAction, ctx: &mut ViewContext<Self>) {
        match action {
            TuiCloudRunAction::Interrupt => self.handle_interrupt(ctx),
            TuiCloudRunAction::OpenUrl(url) => ctx.open_url(url),
            TuiCloudRunAction::OpenPrimaryUrl => {
                if let Some(url) = self.primary_url(ctx) {
                    ctx.open_url(&url);
                }
            }
            TuiCloudRunAction::FocusOrchestrationTabs => {
                self.set_orchestration_tab_focus(true, ctx);
            }
            TuiCloudRunAction::SelectPreviousOrchestrationTab => {
                let key = self
                    .orchestration_tab_bar
                    .as_ref(ctx)
                    .navigation_target(TuiTabBarNavigationDirection::Previous);
                self.switch_to_orchestration_tab(key, ctx);
            }
            TuiCloudRunAction::SelectNextOrchestrationTab => {
                let key = self
                    .orchestration_tab_bar
                    .as_ref(ctx)
                    .navigation_target(TuiTabBarNavigationDirection::Next);
                self.switch_to_orchestration_tab(key, ctx);
            }
            TuiCloudRunAction::SelectFirstOrchestrationChild => {
                let key = self
                    .orchestration_tab_bar
                    .as_ref(ctx)
                    .secondary_edge_target(TuiTabBarSecondaryEdge::First);
                self.switch_to_orchestration_tab(key, ctx);
            }
            TuiCloudRunAction::SelectLastOrchestrationChild => {
                let key = self
                    .orchestration_tab_bar
                    .as_ref(ctx)
                    .secondary_edge_target(TuiTabBarSecondaryEdge::Last);
                self.switch_to_orchestration_tab(key, ctx);
            }
        }
    }
}
