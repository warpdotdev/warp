use settings::Setting;
use warp_core::send_telemetry_from_ctx;
use warp_core::ui::{appearance::Appearance, Icon};
use warpui::prelude::Empty;
use warpui::AppContext;
use warpui::{
    elements::{
        ConstrainedBox, Container, CrossAxisAlignment, Flex, Hoverable, MainAxisSize,
        MouseStateHandle, ParentElement, Shrinkable, Text,
    },
    fonts::Properties,
    platform::Cursor,
    text_layout::ClipConfig,
    Element, Entity, ModelHandle, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
    WeakModelHandle,
};

use crate::ai::agent::conversation::ConversationStatus;
use crate::ai::agent_conversations_model::{AgentConversationsModel, AgentConversationsModelEvent};
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::blocklist::agent_view::{render_block_container, AgentViewEntryOrigin};
use crate::pane_group::focus_state::PaneGroupFocusEvent;
use crate::terminal::view::ambient_agent::AmbientAgentViewModel;
use crate::{
    pane_group::pane::{PaneConfiguration, PaneConfigurationEvent, PaneStack},
    terminal::{BlockListSettings, TerminalManager, TerminalView},
    ui_components::{
        agent_icon::terminal_view_agent_icon_variant,
        blended_colors,
        icon_with_status::{render_icon_with_status, IconWithStatusVariant},
    },
};

use super::super::{AmbientAgentViewModelEvent, Status};
use crate::ai::ambient_agents::telemetry::{CloudAgentTelemetryEvent, CloudModeEntryPoint};
const DEFAULT_CLOUD_AGENT_TITLE: &str = "New cloud agent";

#[derive(Default)]
struct StateHandles {
    block: MouseStateHandle,
}

/// Rich content block rendered in the terminal mode blocklist to represent an ambient agent run.
pub struct AmbientAgentEntryBlock {
    terminal_view: ViewHandle<TerminalView>,
    terminal_manager: ModelHandle<Box<dyn TerminalManager>>,
    pane_stack: WeakModelHandle<PaneStack<TerminalView>>,
    state_handles: StateHandles,
    fetched_task_id: Option<AmbientAgentTaskId>,
}

impl AmbientAgentEntryBlock {
    pub fn new(
        terminal_view: ViewHandle<TerminalView>,
        terminal_manager: ModelHandle<Box<dyn TerminalManager>>,
        pane_stack: WeakModelHandle<PaneStack<TerminalView>>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        if let Some(view_model) = terminal_view
            .as_ref(ctx)
            .ambient_agent_view_model()
            .cloned()
        {
            ctx.subscribe_to_model(&view_model, Self::handle_ambient_agent_view_model_event);
        } else {
            log::warn!("AmbientAgentEntryBlock created without an ambient agent view model");
        }

        let pane_configuration = terminal_view.as_ref(ctx).pane_configuration().clone();
        ctx.subscribe_to_model(&pane_configuration, Self::handle_pane_configuration_event);
        let agent_conversations_model = AgentConversationsModel::handle(ctx);
        ctx.subscribe_to_model(&agent_conversations_model, |_, _, event, ctx| match event {
            AgentConversationsModelEvent::ConversationsLoaded
            | AgentConversationsModelEvent::NewTasksReceived
            | AgentConversationsModelEvent::TasksUpdated
            | AgentConversationsModelEvent::ConversationUpdated { .. } => ctx.notify(),
            AgentConversationsModelEvent::ConversationArtifactsUpdated { .. } => {}
        });

        if let Some(focus_handle) = terminal_view.as_ref(ctx).focus_handle().cloned() {
            let focus_state = focus_handle.focus_state_handle().clone();
            ctx.subscribe_to_model(&focus_state, move |_, _, event, ctx| {
                if matches!(event, PaneGroupFocusEvent::FontSizeOverrideChanged { .. })
                    && focus_handle.is_affected(event)
                {
                    ctx.notify();
                }
            });
        }

        Self {
            terminal_view,
            terminal_manager,
            pane_stack,
            state_handles: Default::default(),
            fetched_task_id: None,
        }
    }
}

impl AmbientAgentEntryBlock {
    fn handle_ambient_agent_view_model_event(
        &mut self,
        _: ModelHandle<AmbientAgentViewModel>,
        event: &AmbientAgentViewModelEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            AmbientAgentViewModelEvent::DispatchedAgent
            | AmbientAgentViewModelEvent::ProgressUpdated
            | AmbientAgentViewModelEvent::SessionReady { .. }
            | AmbientAgentViewModelEvent::Failed { .. }
            | AmbientAgentViewModelEvent::NeedsGithubAuth
            | AmbientAgentViewModelEvent::Cancelled => {
                self.maybe_fetch_task_data(ctx);
                ctx.notify();
            }
            _ => (),
        }
    }

    fn handle_pane_configuration_event(
        &mut self,
        _: ModelHandle<PaneConfiguration>,
        event: &PaneConfigurationEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        // Re-render when we get a new title from the agent conversation.
        // Subscribing via the pane configuration ensures we catch all updates.
        if matches!(event, PaneConfigurationEvent::TitleUpdated) {
            ctx.notify();
        }
    }

    fn maybe_fetch_task_data(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(task_id) = self
            .ambient_agent_view_model(ctx)
            .and_then(AmbientAgentViewModel::task_id)
        else {
            return;
        };

        if self.fetched_task_id == Some(task_id) {
            return;
        }
        self.fetched_task_id = Some(task_id);

        AgentConversationsModel::handle(ctx).update(ctx, |model, ctx| {
            model.get_or_async_fetch_task_data(&task_id, ctx);
        });
    }

    fn meaningful_title(title: &str) -> Option<String> {
        let title = title.trim();
        (!title.is_empty() && !title.eq_ignore_ascii_case(DEFAULT_CLOUD_AGENT_TITLE))
            .then(|| title.to_owned())
    }

    fn title_from_task_data(&self, app: &AppContext) -> Option<String> {
        let task_id = self.ambient_agent_view_model(app)?.task_id()?;
        let task = AgentConversationsModel::as_ref(app).get_task_data(&task_id)?;

        Self::meaningful_title(&task.title).or_else(|| Self::meaningful_title(&task.prompt))
    }

    fn title_from_spawn_request(&self, app: &AppContext) -> Option<String> {
        let request = self.ambient_agent_view_model(app)?.request()?;
        request
            .title
            .as_deref()
            .and_then(Self::meaningful_title)
            .or_else(|| Self::meaningful_title(&request.prompt))
    }
    fn get_title(&self, app: &AppContext) -> String {
        let terminal_view = self.terminal_view.as_ref(app);
        let ai_context_model = terminal_view.ai_context_model().as_ref(app);

        ai_context_model
            .selected_conversation(app)
            .and_then(|conversation| conversation.title())
            .and_then(|title| Self::meaningful_title(&title))
            .or_else(|| self.title_from_task_data(app))
            .or_else(|| self.title_from_spawn_request(app))
            .unwrap_or_else(|| DEFAULT_CLOUD_AGENT_TITLE.to_owned())
    }

    fn ambient_agent_view_model<'a>(
        &self,
        app: &'a AppContext,
    ) -> Option<&'a AmbientAgentViewModel> {
        self.terminal_view
            .as_ref(app)
            .ambient_agent_view_model()
            .map(|model| model.as_ref(app))
    }

    /// Gets the detail text to display based on the ambient agent status.
    fn detail_text(&self, app: &AppContext) -> Option<&'static str> {
        match self.ambient_agent_view_model(app)?.status() {
            Status::Setup | Status::Composing => None,
            Status::WaitingForSession { .. } => Some("Starting environment..."),
            Status::AgentRunning => Some("Agent is working on task"),
            Status::Failed { .. } => Some("Agent failed"),
            Status::NeedsGithubAuth { .. } => Some("Authentication required"),
            Status::Cancelled { .. } => Some("Cancelled"),
        }
    }

    fn ambient_status_for_icon(&self, app: &AppContext) -> Option<ConversationStatus> {
        match self.ambient_agent_view_model(app)?.status() {
            Status::Setup | Status::Composing => None,
            Status::WaitingForSession { .. } | Status::AgentRunning => {
                Some(ConversationStatus::InProgress)
            }
            Status::Failed { .. } => Some(ConversationStatus::Error),
            Status::NeedsGithubAuth { .. } => Some(ConversationStatus::Blocked {
                blocked_action: "GitHub authentication required".to_owned(),
            }),
            Status::Cancelled { .. } => Some(ConversationStatus::Cancelled),
        }
    }

    fn icon_variant(&self, app: &AppContext) -> IconWithStatusVariant {
        let fallback_status = self.ambient_status_for_icon(app);
        let terminal_view = self.terminal_view.as_ref(app);

        match terminal_view_agent_icon_variant(terminal_view, app) {
            Some(IconWithStatusVariant::OzAgent {
                status: None,
                is_ambient,
            }) => IconWithStatusVariant::OzAgent {
                status: fallback_status,
                is_ambient,
            },
            Some(IconWithStatusVariant::CLIAgent {
                agent,
                status: None,
                is_ambient,
            }) => IconWithStatusVariant::CLIAgent {
                agent,
                status: fallback_status,
                is_ambient,
            },
            Some(variant) => variant,
            None => IconWithStatusVariant::OzAgent {
                status: fallback_status,
                is_ambient: true,
            },
        }
    }
}

impl View for AmbientAgentEntryBlock {
    fn ui_name() -> &'static str {
        "AmbientAgentEntryBlock"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let font_size = self
            .terminal_view
            .as_ref(app)
            .effective_monospace_font_size(app);

        let title = self.get_title(app);

        let mut title_row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(8.);
        title_row.add_child(
            Shrinkable::new(
                1.,
                Text::new(title, appearance.ui_font_family(), font_size)
                    .with_color(theme.main_text_color(theme.background()).into_solid())
                    .with_style(Properties {
                        weight: warpui::fonts::Weight::Bold,
                        ..Default::default()
                    })
                    .soft_wrap(false)
                    .with_clip(ClipConfig::ellipsis())
                    .finish(),
            )
            .finish(),
        );

        if let Some(detail_text) = self.detail_text(app) {
            title_row.add_child(
                Shrinkable::new(
                    1.,
                    Text::new(
                        detail_text,
                        appearance.ui_font_family(),
                        appearance.ui_font_size(),
                    )
                    .with_color(blended_colors::text_sub(theme, theme.background()))
                    .soft_wrap(false)
                    .with_clip(ClipConfig::ellipsis())
                    .finish(),
                )
                .finish(),
            );
        }

        let icon_variant = self.icon_variant(app);
        let agent_icon = render_icon_with_status(icon_variant, 24., 0., theme, theme.background());

        let row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(Container::new(agent_icon).with_margin_right(8.).finish())
            .with_child(Shrinkable::new(1., title_row.finish()).finish())
            .with_child(
                Container::new(Empty::new().finish())
                    .with_margin_right(8.)
                    .finish(),
            )
            .with_child(
                ConstrainedBox::new(
                    Icon::ChevronRight
                        .to_warpui_icon(theme.sub_text_color(theme.background()))
                        .finish(),
                )
                .with_height(20.)
                .with_width(20.)
                .finish(),
            );

        let are_block_dividers_enabled =
            *BlockListSettings::as_ref(app).show_block_dividers.value();

        Hoverable::new(self.state_handles.block.clone(), move |hoverable_state| {
            let background = if hoverable_state.is_hovered() {
                blended_colors::fg_overlay_2(appearance.theme())
            } else {
                blended_colors::fg_overlay_1(appearance.theme())
            };
            render_block_container(
                AgentViewEntryOrigin::CloudAgent,
                row.finish(),
                background.into(),
                appearance,
                are_block_dividers_enabled,
            )
        })
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(AmbientAgentEntryBlockAction::OpenAmbientAgent);
        })
        .with_cursor(Cursor::PointingHand)
        .finish()
    }
}

/// Action dispatched when the ambient agent task block is clicked.
#[derive(Clone, Copy, Debug)]
pub enum AmbientAgentEntryBlockAction {
    /// Navigate to the ambient agent view.
    OpenAmbientAgent,
}

impl Entity for AmbientAgentEntryBlock {
    type Event = ();
}

impl TypedActionView for AmbientAgentEntryBlock {
    type Action = AmbientAgentEntryBlockAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            AmbientAgentEntryBlockAction::OpenAmbientAgent => {
                send_telemetry_from_ctx!(
                    CloudAgentTelemetryEvent::EnteredCloudMode {
                        entry_point: CloudModeEntryPoint::EntryBlock,
                    },
                    ctx
                );
                if let Some(stack) = self.pane_stack.upgrade(ctx) {
                    stack.update(ctx, |stack, ctx| {
                        stack.push(
                            self.terminal_manager.clone(),
                            self.terminal_view.clone(),
                            ctx,
                        );
                    });
                }
            }
        }
    }
}
