use std::sync::Arc;

use parking_lot::FairMutex;
use warp_core::ui::appearance::Appearance;
use warp_terminal::model::BlockId;
use warpui::{
    prelude::{Container, Empty, MouseStateHandle},
    scene::{CornerRadius, Radius},
    AppContext, Element, Entity, ModelHandle, SingletonEntity, TypedActionView, View, ViewContext,
};

use crate::{
    ai::{
        agent::icons::{failed_icon, yellow_running_icon},
        blocklist::inline_action::{
            inline_action_header::{ExpandedConfig, HeaderConfig, InteractionMode},
            inline_action_icons::green_check_icon,
            requested_command::VIEWING_COMMAND_DETAIL_MESSAGE,
        },
    },
    terminal::{
        event::BlockCompletedEvent,
        model_events::{ModelEvent, ModelEventDispatcher},
        view::ambient_agent::{
            AmbientAgentViewModel, AmbientAgentViewModelEvent, SetupCommandGroupId,
        },
        TerminalModel,
    },
};

enum Status {
    Running,
    Completed { is_success: bool },
}

pub struct CloudModeSetupCommandBlock {
    group_id: SetupCommandGroupId,
    block_id: BlockId,
    command: String,
    status: Status,
    ambient_agent_view_model: ModelHandle<AmbientAgentViewModel>,
    terminal_model: Arc<FairMutex<TerminalModel>>,
    is_expanded: bool,
    mouse_state: MouseStateHandle,
}

impl CloudModeSetupCommandBlock {
    pub fn new(
        group_id: SetupCommandGroupId,
        block_id: BlockId,
        ambient_agent_view_model: ModelHandle<AmbientAgentViewModel>,
        model_events: &ModelHandle<ModelEventDispatcher>,
        terminal_model: Arc<FairMutex<TerminalModel>>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(&ambient_agent_view_model, |me, model, event, ctx| {
            if let AmbientAgentViewModelEvent::UpdatedSetupCommandVisibility = event {
                if !model
                    .as_ref(ctx)
                    .setup_command_state()
                    .should_expand(me.group_id)
                    && !me
                        .terminal_model
                        .lock()
                        .block_list()
                        .block_with_id(&me.block_id)
                        .is_some_and(|block| block.is_hidden())
                {
                    ctx.emit(CloudModeSetupCommandBlockEvent::ToggleBlockVisibility(
                        me.block_id.clone(),
                    ));
                }
                ctx.notify();
            }
        });

        ctx.subscribe_to_model(model_events, |me, model_events, event, ctx| {
            if let ModelEvent::BlockCompleted(BlockCompletedEvent { block_id, .. }) = event {
                if *block_id == me.block_id {
                    if me
                        .terminal_model
                        .lock()
                        .block_list()
                        .block_with_id(block_id)
                        .is_some_and(|block| block.exit_code().was_successful())
                    {
                        me.status = Status::Completed { is_success: true };
                    } else {
                        me.status = Status::Completed { is_success: false };
                    }
                    ctx.unsubscribe_to_model(&model_events);
                    ctx.notify();
                }
            }
        });

        let command = terminal_model
            .lock()
            .block_list()
            .block_with_id(&block_id)
            .map(|block| block.command_to_string())
            .unwrap_or_default();
        Self {
            group_id,
            block_id,
            command,
            ambient_agent_view_model,
            terminal_model,
            is_expanded: false,
            status: Status::Running,
            mouse_state: Default::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum CloudModeSetupCommandBlockEvent {
    ToggleBlockVisibility(BlockId),
}

impl Entity for CloudModeSetupCommandBlock {
    type Event = CloudModeSetupCommandBlockEvent;
}

impl View for CloudModeSetupCommandBlock {
    fn ui_name() -> &'static str {
        "CloudModeSetupCommandBlock"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        if !self
            .ambient_agent_view_model
            .as_ref(app)
            .setup_command_state()
            .should_expand(self.group_id)
        {
            return Empty::new().finish();
        }

        let appearance = Appearance::as_ref(app);
        let mut config = HeaderConfig::new(
            if self.is_expanded {
                VIEWING_COMMAND_DETAIL_MESSAGE.to_owned()
            } else {
                self.command.clone()
            },
            app,
        )
        .with_interaction_mode(InteractionMode::ManuallyExpandable(
            ExpandedConfig::new(self.is_expanded, self.mouse_state.clone()).with_toggle_callback(
                |ctx| {
                    ctx.dispatch_typed_action(
                        CloudModeSetupCommandBlockAction::ToggleBlockVisibility,
                    )
                },
            ),
        ))
        .with_icon(match self.status {
            Status::Running => yellow_running_icon(appearance),
            Status::Completed { is_success } => {
                if is_success {
                    green_check_icon(appearance)
                } else {
                    failed_icon(appearance)
                }
            }
        })
        .with_font_family(if self.is_expanded {
            appearance.ui_font_family()
        } else {
            appearance.monospace_font_family()
        });

        if self.is_expanded {
            config = config.with_corner_radius_override(CornerRadius::with_top(Radius::Pixels(8.)))
        }

        let mut container = Container::new(config.render(app));

        if !self.is_expanded {
            container = super::cloud_mode_setup_row_spacing(
                container.finish(),
                &self.ambient_agent_view_model,
                app,
            );
        }

        container.finish()
    }
}

#[derive(Debug, Clone)]
pub enum CloudModeSetupCommandBlockAction {
    ToggleBlockVisibility,
}

impl TypedActionView for CloudModeSetupCommandBlock {
    type Action = CloudModeSetupCommandBlockAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            CloudModeSetupCommandBlockAction::ToggleBlockVisibility => {
                self.is_expanded = !self.is_expanded;
                ctx.emit(CloudModeSetupCommandBlockEvent::ToggleBlockVisibility(
                    self.block_id.clone(),
                ));
                ctx.notify();
            }
        }
    }
}
