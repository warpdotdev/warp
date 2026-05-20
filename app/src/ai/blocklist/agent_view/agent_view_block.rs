use pathfinder_color::ColorU;
use pathfinder_geometry::vector::Vector2F;
use settings::Setting;
use warp_core::ui::{appearance::Appearance, Icon};
use warpui::{
    elements::{
        ConstrainedBox, Container, CrossAxisAlignment, Empty, Expanded, Flex, Hoverable,
        MainAxisSize, MouseStateHandle, ParentElement, SavePosition, Shrinkable, Text,
    },
    fonts::{Properties, Style, Weight::Bold},
    platform::Cursor,
    prelude::{Border, CornerRadius, Radius},
    text_layout::ClipConfig,
    AppContext, Element, Entity, EntityId, EventContext, ModelHandle, SingletonEntity,
    TypedActionView, View, ViewContext,
};

use crate::{
    ai::{
        active_agent_views_model::ActiveAgentViewsModel, agent::conversation::AIConversationId,
        blocklist::BlocklistAIHistoryEvent,
    },
    terminal::BlockListSettings,
    ui_components::{
        blended_colors,
        icon_with_status::{render_icon_with_status, IconWithStatusVariant},
    },
    view_components::DismissibleToast,
    workspace::{ToastStack, WorkspaceAction},
    BlocklistAIHistoryModel,
};

use super::{AgentViewController, AgentViewEntryOrigin};

#[derive(Default)]
struct StateHandles {
    block: MouseStateHandle,
    fork_button: MouseStateHandle,
    open_conversation_button: MouseStateHandle,
}

pub struct AgentViewEntryBlockParams {
    pub conversation_id: AIConversationId,
    pub is_new: bool,
    pub is_restored: bool,
    pub origin: AgentViewEntryOrigin,
    pub agent_view_controller: ModelHandle<AgentViewController>,
}

/// Rich content block rendered in the terminal mode blocklist to represent an Agent View entry for
/// a given conversation.
pub struct AgentViewEntryBlock {
    conversation_id: AIConversationId,
    agent_view_controller: ModelHandle<AgentViewController>,
    is_new: bool,
    is_restored: bool,
    origin: AgentViewEntryOrigin,
    /// Cached title for rendering when conversation no longer exists (i.e. after deletion).
    cached_title: Option<String>,
    state_handles: StateHandles,
    /// UI instance anchor for positioning the entry context menu.
    view_id: EntityId,
}

impl AgentViewEntryBlock {
    pub fn new(params: AgentViewEntryBlockParams, ctx: &mut ViewContext<Self>) -> Self {
        let AgentViewEntryBlockParams {
            conversation_id,
            is_new,
            is_restored,
            origin,
            agent_view_controller,
        } = params;
        let history_model = BlocklistAIHistoryModel::handle(ctx);
        ctx.subscribe_to_model(&history_model, |me, _, event, ctx| match event {
            BlocklistAIHistoryEvent::UpdatedStreamingExchange {
                conversation_id, ..
            }
            | BlocklistAIHistoryEvent::UpdatedConversationStatus {
                conversation_id, ..
            } if *conversation_id == me.conversation_id => {
                ctx.notify();
            }
            BlocklistAIHistoryEvent::DeletedConversation {
                conversation_id,
                conversation_title,
                ..
            } if *conversation_id == me.conversation_id => {
                me.cached_title = conversation_title.clone();
                ctx.notify();
            }
            _ => (),
        });
        ctx.subscribe_to_model(&agent_view_controller, |_, _, _, ctx| ctx.notify());

        let active_agent_views_model = ActiveAgentViewsModel::handle(ctx);
        ctx.subscribe_to_model(&active_agent_views_model, |_, _, _, ctx| ctx.notify());

        Self {
            conversation_id,
            agent_view_controller,
            is_new,
            is_restored,
            origin,
            cached_title: Default::default(),
            state_handles: Default::default(),
            view_id: ctx.view_id(),
        }
    }
}

pub fn get_agent_view_entry_block_position_id(view_id: EntityId) -> String {
    format!("agent_view_entry_block_{view_id}")
}

pub fn render_block_container(
    origin: AgentViewEntryOrigin,
    content: Box<dyn Element>,
    background: ColorU,
    appearance: &Appearance,
    are_block_dividers_enabled: bool,
) -> Box<dyn Element> {
    let border = if are_block_dividers_enabled {
        Border::top(1.).with_border_fill(appearance.theme().outline())
    } else {
        Border::new(1.)
            .with_sides(true, false, true, false)
            .with_border_fill(appearance.theme().outline())
    };

    let mut container = Container::new(content).with_background(background);

    if matches!(origin, AgentViewEntryOrigin::LongRunningCommand) {
        container = container
            .with_uniform_padding(12.)
            .with_horizontal_margin(16.)
            .with_margin_bottom(16.)
            .with_margin_top(8.)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)));
    } else {
        container = container
            .with_horizontal_padding(20.)
            .with_vertical_padding(18.)
            .with_border(border);
    }

    container.finish()
}

fn render_subtext(text: String, appearance: &Appearance) -> Box<dyn Element> {
    Container::new(
        Text::new(
            text,
            appearance.ui_font_family(),
            appearance.monospace_font_size() - 2.,
        )
        .with_color(blended_colors::text_disabled(
            appearance.theme(),
            appearance.theme().background(),
        ))
        .with_style(Properties {
            style: Style::Italic,
            ..Default::default()
        })
        .finish(),
    )
    .with_margin_left(8.)
    .finish()
}

/// Renders a custom button with icon and hover/click states for the Agent View Block.
///
/// This button component is currently used for the "fork" and "open agent conversation" actions
/// within the Agent View. Future actions re-using this pattern should use this component for
/// visually consistent icon-based interaction.
fn render_agent_view_block_button<F>(
    icon: Icon,
    icon_color: ColorU,
    mouse_state: MouseStateHandle,
    appearance: &Appearance,
    on_click: F,
) -> Box<dyn Element>
where
    F: 'static + FnMut(&mut EventContext, &AppContext, Vector2F),
{
    Hoverable::new(mouse_state, move |state| {
        let container = Container::new(
            ConstrainedBox::new(icon.to_warpui_icon(icon_color.into()).finish())
                .with_height(20.)
                .with_width(20.)
                .finish(),
        )
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(5.)));

        let container = if state.is_clicked() {
            container.with_background(blended_colors::fg_overlay_4(appearance.theme()))
        } else if state.is_hovered() {
            container.with_background(blended_colors::fg_overlay_3(appearance.theme()))
        } else {
            container
        };

        container.finish()
    })
    .with_cursor(Cursor::PointingHand)
    .on_click(on_click)
    .finish()
}

fn render_deleted_state(
    origin: AgentViewEntryOrigin,
    cached_title: Option<String>,
    appearance: &Appearance,
    are_block_dividers_enabled: bool,
) -> Box<dyn Element> {
    let disabled_color =
        blended_colors::text_disabled(appearance.theme(), appearance.theme().background());

    let row = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_main_axis_size(MainAxisSize::Max)
        .with_child(
            Text::new(
                cached_title.unwrap_or_else(|| "Deleted conversation".to_string()),
                appearance.ui_font_family(),
                appearance.monospace_font_size(),
            )
            .with_color(disabled_color)
            .with_style(Properties {
                weight: Bold,
                ..Default::default()
            })
            .finish(),
        )
        .with_child(render_subtext("Deleted".to_string(), appearance))
        .finish();

    render_block_container(
        origin,
        row,
        blended_colors::fg_overlay_1(appearance.theme()).into(),
        appearance,
        are_block_dividers_enabled,
    )
}

impl View for AgentViewEntryBlock {
    fn ui_name() -> &'static str {
        "EnterAgentBlock"
    }

    fn render(&self, app: &warpui::AppContext) -> Box<dyn warpui::Element> {
        if self.agent_view_controller.as_ref(app).is_fullscreen() {
            return Empty::new().finish();
        }

        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let are_block_dividers_enabled =
            *BlockListSettings::as_ref(app).show_block_dividers.value();

        let history_model = BlocklistAIHistoryModel::as_ref(app);
        let Some(conversation) = history_model.conversation(&self.conversation_id) else {
            // If the agent_view_block's conversation no longer exists,
            // we assume that it has been deleted.
            return render_deleted_state(
                self.origin,
                self.cached_title.clone(),
                appearance,
                are_block_dividers_enabled,
            );
        };

        if conversation.is_entirely_passive() {
            return Empty::new().finish();
        }

        let agent_icon = render_icon_with_status(
            IconWithStatusVariant::OzAgent {
                status: Some(conversation.status().clone()),
                is_ambient: false,
            },
            24.,
            0.,
            theme,
            theme.background(),
        );

        let is_active =
            ActiveAgentViewsModel::as_ref(app).is_conversation_open(self.conversation_id, app);
        let is_active_in_this_pane = self
            .agent_view_controller
            .as_ref(app)
            .agent_view_state()
            .active_conversation_id()
            == Some(self.conversation_id);
        let is_open_elsewhere = is_active && !is_active_in_this_pane;

        let subtext = if is_open_elsewhere {
            Some("Open in different pane")
        } else if self.is_restored {
            Some("Restored")
        } else if !self.is_new
            && !matches!(
                self.origin,
                AgentViewEntryOrigin::LongRunningCommand
                    | AgentViewEntryOrigin::AgentRequestedNewConversation
            )
        {
            Some("Continued")
        } else {
            None
        };

        // Build the title+subtext section.
        let mut title_section = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Shrinkable::new(
                    1.,
                    Text::new(
                        conversation
                            .title()
                            .unwrap_or("Untitled conversation".to_string()),
                        appearance.ui_font_family(),
                        appearance.monospace_font_size(),
                    )
                    .with_color(blended_colors::text_main(
                        appearance.theme(),
                        appearance.theme().background(),
                    ))
                    .with_style(Properties {
                        weight: Bold,
                        ..Default::default()
                    })
                    .soft_wrap(false)
                    .with_clip(ClipConfig::ellipsis())
                    .finish(),
                )
                .finish(),
            );
        if let Some(subtext) = subtext {
            title_section.add_child(render_subtext(subtext.to_string(), appearance));
        }

        let conversation_id = self.conversation_id;
        let icon_color = blended_colors::text_sub(theme, theme.background());
        let fork_button = ConstrainedBox::new(render_agent_view_block_button(
            Icon::ArrowSplit,
            icon_color,
            self.state_handles.fork_button.clone(),
            appearance,
            move |ctx, _, _| {
                ctx.dispatch_typed_action(EnterAgentBlockAction::ForkConversation {
                    conversation_id,
                });
            },
        ))
        .with_height(20.)
        .with_width(20.)
        .finish();

        let open_conversation_button = ConstrainedBox::new(render_agent_view_block_button(
            Icon::ChevronRight,
            icon_color,
            self.state_handles.open_conversation_button.clone(),
            appearance,
            move |ctx, _, _| {
                ctx.dispatch_typed_action(EnterAgentBlockAction::EnterAgentMode {
                    conversation_id,
                });
            },
        ))
        .with_height(20.)
        .with_width(20.)
        .finish();

        let row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(Container::new(agent_icon).with_margin_right(8.).finish())
            // Expanded fills all remaining space, pushing buttons to the right.
            .with_child(Expanded::new(1., title_section.finish()).finish())
            .with_child(Container::new(fork_button).with_margin_left(8.).finish())
            .with_child(open_conversation_button);

        let origin = self.origin;
        let entry_block_id = self.view_id;
        let entry_block_position_id = get_agent_view_entry_block_position_id(entry_block_id);
        SavePosition::new(
            Hoverable::new(self.state_handles.block.clone(), move |hoverable_state| {
                let background = if hoverable_state.is_hovered() {
                    blended_colors::fg_overlay_2(appearance.theme())
                } else {
                    blended_colors::fg_overlay_1(appearance.theme())
                };
                render_block_container(
                    origin,
                    row.finish(),
                    background.into(),
                    appearance,
                    are_block_dividers_enabled,
                )
            })
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(EnterAgentBlockAction::EnterAgentMode {
                    conversation_id,
                });
            })
            .on_right_click(move |ctx, _, position| {
                let Some(entry_bounds) = ctx.element_position_by_id(&entry_block_position_id)
                else {
                    log::warn!(
                        "Could not retrieve the position of the agent view entry block for context menu display."
                    );
                    return;
                };
                // Subtract the block's origin so the position is expressed relative
                // to the block element. OffsetPositioning::offset_from_save_position_element
                // then offsets the menu from the saved block position, placing it at
                // the exact mouse cursor location.
                ctx.dispatch_typed_action(EnterAgentBlockAction::OpenConversationContextMenu {
                    conversation_id,
                    agent_view_entry_block_id: entry_block_id,
                    position: position - entry_bounds.origin(),
                });
            })
            .with_defer_events_to_children()
            .finish(),
            &get_agent_view_entry_block_position_id(entry_block_id),
        )
        .finish()
    }
}

#[derive(Debug, Clone)]
pub enum AgentViewEntryBlockEvent {
    EnterAgentView {
        conversation_id: AIConversationId,
    },
    OpenConversationContextMenu {
        conversation_id: AIConversationId,
        agent_view_entry_block_id: EntityId,
        position: Vector2F,
    },
    ForkConversation {
        conversation_id: AIConversationId,
    },
}

impl Entity for AgentViewEntryBlock {
    type Event = AgentViewEntryBlockEvent;
}

#[derive(Debug, Clone)]
pub enum EnterAgentBlockAction {
    EnterAgentMode {
        conversation_id: AIConversationId,
    },
    OpenConversationContextMenu {
        conversation_id: AIConversationId,
        agent_view_entry_block_id: EntityId,
        position: Vector2F,
    },
    ForkConversation {
        conversation_id: AIConversationId,
    },
}

impl TypedActionView for AgentViewEntryBlock {
    type Action = EnterAgentBlockAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            EnterAgentBlockAction::EnterAgentMode { conversation_id } => {
                let is_active =
                    ActiveAgentViewsModel::as_ref(ctx).is_conversation_open(*conversation_id, ctx);
                let is_active_in_this_pane = self
                    .agent_view_controller
                    .as_ref(ctx)
                    .agent_view_state()
                    .active_conversation_id()
                    == Some(*conversation_id);

                if is_active && !is_active_in_this_pane {
                    let Some(target_terminal_view_id) = ActiveAgentViewsModel::as_ref(ctx)
                        .terminal_view_id_for_conversation(*conversation_id, ctx)
                    else {
                        let window_id = ctx.window_id();
                        ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                            toast_stack.add_ephemeral_toast(
                                DismissibleToast::error(
                                    "Couldn't navigate to conversation.".to_string(),
                                ),
                                window_id,
                                ctx,
                            );
                        });
                        return;
                    };

                    ctx.dispatch_typed_action_deferred(
                        WorkspaceAction::FocusTerminalViewInWorkspace {
                            terminal_view_id: target_terminal_view_id,
                        },
                    );
                } else {
                    ctx.emit(AgentViewEntryBlockEvent::EnterAgentView {
                        conversation_id: *conversation_id,
                    });
                }
            }
            EnterAgentBlockAction::OpenConversationContextMenu {
                conversation_id,
                agent_view_entry_block_id,
                position,
            } => {
                ctx.emit(AgentViewEntryBlockEvent::OpenConversationContextMenu {
                    conversation_id: *conversation_id,
                    agent_view_entry_block_id: *agent_view_entry_block_id,
                    position: *position,
                });
            }
            EnterAgentBlockAction::ForkConversation { conversation_id } => {
                ctx.emit(AgentViewEntryBlockEvent::ForkConversation {
                    conversation_id: *conversation_id,
                });
            }
        }
    }
}
