use warp_core::ui::{appearance::Appearance, Icon};
use warpui::{
    elements::ParentElement,
    prelude::{
        ConstrainedBox, Container, CrossAxisAlignment, Cursor, Empty, Flex, Hoverable,
        MouseStateHandle, Text,
    },
    AppContext, Element, Entity, ModelHandle, SingletonEntity, TypedActionView, View, ViewContext,
};

use crate::{
    ai::blocklist::{
        agent_view::{agent_view_bg_color, AgentViewController},
        inline_action::inline_action_icons,
        BlocklistAIHistoryEvent, BlocklistAIHistoryModel,
    },
    terminal::view::ambient_agent::{
        is_cloud_agent_pre_first_exchange, AmbientAgentViewModel, AmbientAgentViewModelEvent,
    },
};

#[derive(Debug, Clone, Copy)]
pub struct SetupCommandState {
    did_execute_a_setup_command: bool,
    should_expand_setup_commands: bool,
}

impl Default for SetupCommandState {
    fn default() -> Self {
        Self {
            did_execute_a_setup_command: false,
            should_expand_setup_commands: true,
        }
    }
}

impl SetupCommandState {
    pub fn did_execute_a_setup_command(&self) -> bool {
        self.did_execute_a_setup_command
    }

    pub fn set_did_execute_a_setup_command(&mut self, value: bool) {
        self.did_execute_a_setup_command = value;
    }

    pub fn should_expand(&self) -> bool {
        self.should_expand_setup_commands
    }

    pub fn set_should_expand(&mut self, value: bool) {
        self.should_expand_setup_commands = value;
    }
}

pub struct CloudModeSetupTextBlock {
    ambient_agent_view_model: ModelHandle<AmbientAgentViewModel>,
    agent_view_controller: ModelHandle<AgentViewController>,
    mouse_state: MouseStateHandle,
}

impl CloudModeSetupTextBlock {
    pub fn new(
        ambient_agent_view_model: ModelHandle<AmbientAgentViewModel>,
        agent_view_controller: ModelHandle<AgentViewController>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        if let Some(conversation_id) = agent_view_controller
            .as_ref(ctx)
            .agent_view_state()
            .active_conversation_id()
        {
            ctx.subscribe_to_model(
                &BlocklistAIHistoryModel::handle(ctx),
                move |me, history_model, event, ctx| {
                    if let BlocklistAIHistoryEvent::AppendedExchange {
                        conversation_id: updated_conversation_id,
                        ..
                    } = event
                    {
                        if *updated_conversation_id == conversation_id {
                            if me
                                .ambient_agent_view_model
                                .as_ref(ctx)
                                .setup_command_state()
                                .should_expand()
                            {
                                me.ambient_agent_view_model.update(ctx, |model, ctx| {
                                    model.set_setup_command_visibility(false, ctx);
                                });
                            }
                            ctx.unsubscribe_to_model(&history_model);
                        }
                    }
                },
            );
        }

        ctx.subscribe_to_model(&ambient_agent_view_model, |_, _, event, ctx| {
            if let AmbientAgentViewModelEvent::UpdatedSetupCommandVisibility = event {
                ctx.notify();
            }
        });

        Self {
            ambient_agent_view_model,
            agent_view_controller,
            mouse_state: Default::default(),
        }
    }
}

impl Entity for CloudModeSetupTextBlock {
    type Event = ();
}

impl View for CloudModeSetupTextBlock {
    fn ui_name() -> &'static str {
        "CloudModeSetupTextBlock"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        if !self.ambient_agent_view_model.as_ref(app).is_agent_running() {
            return Empty::new().finish();
        }

        let chevron_icon = if self
            .ambient_agent_view_model
            .as_ref(app)
            .setup_command_state()
            .should_expand()
        {
            Icon::ChevronDown
        } else {
            Icon::ChevronRight
        };

        let appearance = Appearance::as_ref(app);
        let icon_size = inline_action_icons::icon_size(app);
        let text_color = appearance
            .theme()
            .disabled_text_color(agent_view_bg_color(app).into())
            .into_solid();
        let expandable = Hoverable::new(self.mouse_state.clone(), move |_is_hovered| {
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(
                    Text::new(
                        if is_cloud_agent_pre_first_exchange(
                            &self.ambient_agent_view_model,
                            &self.agent_view_controller,
                            app,
                        ) {
                            "Running setup commands..."
                        } else {
                            "Ran setup commands"
                        },
                        appearance.ai_font_family(),
                        appearance.monospace_font_size(),
                    )
                    .with_color(text_color)
                    .with_selectable(false)
                    .finish(),
                )
                .with_child(
                    Container::new(
                        ConstrainedBox::new(
                            chevron_icon.to_warpui_icon(text_color.into()).finish(),
                        )
                        .with_width(icon_size)
                        .with_height(icon_size)
                        .finish(),
                    )
                    .with_margin_right(8.)
                    .finish(),
                )
                .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(CloudModeSetupTextBlockAction::ToggleSetupCommandVisibility);
        });

        super::cloud_mode_setup_row_spacing(
            expandable.finish(),
            &self.ambient_agent_view_model,
            app,
        )
        .finish()
    }
}

#[derive(Debug, Clone)]
pub enum CloudModeSetupTextBlockAction {
    ToggleSetupCommandVisibility,
}

impl TypedActionView for CloudModeSetupTextBlock {
    type Action = CloudModeSetupTextBlockAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            CloudModeSetupTextBlockAction::ToggleSetupCommandVisibility => {
                self.ambient_agent_view_model.update(ctx, |model, ctx| {
                    model.set_setup_command_visibility(
                        !model.setup_command_state().should_expand(),
                        ctx,
                    );
                });
            }
        }
    }
}
