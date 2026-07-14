use std::collections::HashMap;

use warp_core::ui::appearance::Appearance;
use warp_core::ui::Icon;
use warpui::elements::ParentElement;
use warpui::prelude::{
    ConstrainedBox, Container, CrossAxisAlignment, Cursor, Flex, Hoverable, MouseStateHandle, Text,
};
use warpui::{
    AppContext, Element, Entity, ModelHandle, SingletonEntity, TypedActionView, View, ViewContext,
};

use crate::ai::blocklist::agent_view::AgentViewController;
use crate::ai::blocklist::inline_action::inline_action_icons;
use crate::ai::blocklist::{BlocklistAIHistoryEvent, BlocklistAIHistoryModel};
use crate::terminal::view::ambient_agent::{AmbientAgentViewModel, AmbientAgentViewModelEvent};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SetupCommandGroupId(u64);

impl SetupCommandGroupId {
    fn initial() -> Self {
        Self(0)
    }
}

#[derive(Debug, Clone)]
pub struct SetupCommandState {
    did_execute_a_setup_command: bool,
    current_group_id: SetupCommandGroupId,
    next_group_id: u64,
    expanded_groups: HashMap<SetupCommandGroupId, bool>,
    running_group_id: Option<SetupCommandGroupId>,
}

impl Default for SetupCommandState {
    fn default() -> Self {
        let current_group_id = SetupCommandGroupId::initial();
        let mut expanded_groups = HashMap::new();
        expanded_groups.insert(current_group_id, true);
        Self {
            did_execute_a_setup_command: false,
            current_group_id,
            next_group_id: 1,
            expanded_groups,
            running_group_id: Some(current_group_id),
        }
    }
}

impl SetupCommandState {
    pub fn current_group_id(&self) -> SetupCommandGroupId {
        self.current_group_id
    }
    pub fn did_execute_a_setup_command(&self) -> bool {
        self.did_execute_a_setup_command
    }

    pub fn set_did_execute_a_setup_command(&mut self, value: bool) {
        self.did_execute_a_setup_command = value;
    }

    pub fn should_expand(&self, group_id: SetupCommandGroupId) -> bool {
        self.expanded_groups.get(&group_id).copied().unwrap_or(true)
    }

    pub fn set_should_expand(&mut self, group_id: SetupCommandGroupId, value: bool) {
        self.expanded_groups.insert(group_id, value);
    }

    pub fn is_running(&self, group_id: SetupCommandGroupId) -> bool {
        self.running_group_id == Some(group_id)
    }

    pub fn should_suppress_input_sync_for_current_group(&self) -> bool {
        self.current_group_id != SetupCommandGroupId::initial()
            && self.is_running(self.current_group_id)
    }

    pub fn start_new_group(&mut self) -> SetupCommandGroupId {
        let group_id = SetupCommandGroupId(self.next_group_id);
        self.next_group_id += 1;
        self.current_group_id = group_id;
        self.did_execute_a_setup_command = false;
        self.expanded_groups.insert(group_id, true);
        self.running_group_id = Some(group_id);
        group_id
    }

    pub fn finish_group(&mut self, group_id: SetupCommandGroupId) {
        if self.running_group_id == Some(group_id) {
            self.running_group_id = None;
        }
    }
}

pub struct CloudModeSetupTextBlock {
    group_id: SetupCommandGroupId,
    ambient_agent_view_model: ModelHandle<AmbientAgentViewModel>,
    mouse_state: MouseStateHandle,
}

impl CloudModeSetupTextBlock {
    pub fn new(
        group_id: SetupCommandGroupId,
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
                            me.ambient_agent_view_model.update(ctx, |model, ctx| {
                                model.finish_setup_command_group(me.group_id, ctx);
                                model.set_setup_command_group_visibility(me.group_id, false, ctx);
                            });
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
            group_id,
            ambient_agent_view_model,
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
        let chevron_icon = if self
            .ambient_agent_view_model
            .as_ref(app)
            .setup_command_state()
            .should_expand(self.group_id)
        {
            Icon::ChevronDown
        } else {
            Icon::ChevronRight
        };

        let appearance = Appearance::as_ref(app);
        let icon_size = inline_action_icons::icon_size(app);
        let text_color = appearance
            .theme()
            .disabled_text_color(appearance.theme().background())
            .into_solid();
        let expandable = Hoverable::new(self.mouse_state.clone(), move |_is_hovered| {
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(
                    Text::new(
                        if self
                            .ambient_agent_view_model
                            .as_ref(app)
                            .setup_command_state()
                            .is_running(self.group_id)
                        {
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

        super::cloud_mode_setup_text_row_spacing(
            expandable.finish(),
            &self.ambient_agent_view_model,
            app,
        )
        .with_margin_top(8.)
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
                    model.set_setup_command_group_visibility(
                        self.group_id,
                        !model.setup_command_state().should_expand(self.group_id),
                        ctx,
                    );
                });
            }
        }
    }
}

#[cfg(test)]
#[path = "setup_command_text_tests.rs"]
mod tests;
