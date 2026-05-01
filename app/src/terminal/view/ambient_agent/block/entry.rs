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
    prelude::{CornerRadius, Radius},
    text_layout::ClipConfig,
    Element, Entity, ModelHandle, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
    WeakModelHandle,
};

use crate::ai::blocklist::agent_view::{render_block_container, AgentViewEntryOrigin};
use crate::terminal::view::ambient_agent::AmbientAgentViewModel;
use crate::{
    pane_group::pane::{PaneConfiguration, PaneConfigurationEvent, PaneStack},
    terminal::{BlockListSettings, TerminalManager, TerminalView},
    ui_components::blended_colors,
};

use super::super::{AmbientAgentViewModelEvent, Status};
use crate::ai::ambient_agents::telemetry::{CloudAgentTelemetryEvent, CloudModeEntryPoint};

/// Icon size for the status indicator
const STATUS_ICON_SIZE: f32 = 16.;

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

        Self {
            terminal_view,
            terminal_manager,
            pane_stack,
            state_handles: Default::default(),
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
            | AmbientAgentViewModelEvent::Cancelled => ctx.notify(),
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

    /// Gets the title to display for this ambient agent block.
    /// Derives the title from the terminal view's selected conversation or a default.
    fn get_title(&self, app: &AppContext) -> String {
        let terminal_view = self.terminal_view.as_ref(app);
        let ai_context_model = terminal_view.ai_context_model().as_ref(app);

        ai_context_model
            .selected_conversation(app)
            .and_then(|c| c.title())
            .unwrap_or_else(|| "New cloud agent".to_owned())
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

    /// Renders the status icon based on the ambient agent status.
    fn render_status_icon(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn warpui::Element> {
        let theme = appearance.theme();

        let Some(view_model) = self.ambient_agent_view_model(app) else {
            return Empty::new().finish();
        };
        let (icon, color) = if view_model.is_failed() {
            (Icon::AlertTriangle, theme.ui_error_color())
        } else if view_model.is_needs_github_auth() {
            (Icon::Info, blended_colors::accent(theme).into_solid())
        } else if view_model.is_cancelled() {
            (
                Icon::Cancelled,
                theme.disabled_text_color(theme.background()).into_solid(),
            )
        } else if view_model.is_waiting_for_session() {
            (Icon::ClockLoader, theme.ansi_fg_magenta())
        } else {
            (
                Icon::OzCloud,
                theme.main_text_color(theme.background()).into_solid(),
            )
        };

        Container::new(
            ConstrainedBox::new(icon.to_warpui_icon(color.into()).finish())
                .with_width(STATUS_ICON_SIZE)
                .with_height(STATUS_ICON_SIZE)
                .finish(),
        )
        .with_uniform_padding(2.)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
        .with_margin_right(8.)
        .finish()
    }
}

impl View for AmbientAgentEntryBlock {
    fn ui_name() -> &'static str {
        "AmbientAgentEntryBlock"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let title = self.get_title(app);

        let mut title_row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(8.);
        title_row.add_child(
            Shrinkable::new(
                1.,
                Text::new(
                    title,
                    appearance.ui_font_family(),
                    appearance.monospace_font_size(),
                )
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

        let row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(self.render_status_icon(appearance, app))
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
