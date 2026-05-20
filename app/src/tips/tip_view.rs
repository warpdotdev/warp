use pathfinder_geometry::vector::vec2f;
use warpui::elements::{
    Align, ChildAnchor, ClippedScrollStateHandle, ClippedScrollable, DispatchEventResult,
    EventHandler, Hoverable, Icon, MouseStateHandle, OffsetPositioning, PositionedElementAnchor,
    PositionedElementOffsetBounds, Radius, ScrollbarWidth, Stack,
};
use warpui::platform::Cursor;
use warpui::ui_components::button::ButtonVariant;
use warpui::{
    elements::{
        Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Element, Flex,
        ParentElement, Shrinkable,
    },
    fonts::Weight,
    keymap::Keystroke,
    ui_components::components::{UiComponent, UiComponentStyles},
    AppContext, Entity, TypedActionView, View,
};
use warpui::{keymap::FixedBinding, ViewContext};
use warpui::{Action, BlurContext, EntityId, ModelHandle, SingletonEntity, WindowId};

use crate::appearance::Appearance;
use crate::resource_center::{Tip, TipAction, TipsCompleted};
use crate::themes::theme::{Blend, Fill};
use crate::util::bindings::trigger_to_keystroke;

use super::WELCOME_TIP_FEATURE_LENGTH;

const CHECK_MARK_WIDTH: f32 = 20.;
const TIP_VIEW_WIDTH: f32 = 250.;
const CHECK_SVG_PATH: &str = "bundled/svg/check-skinny.svg";

const SKIP_BUTTON_OVERLAY_OPACITY: u8 = 20;

const SCROLLABLE_AREA_HEIGHT: f32 = 390.;
const SKIP_BUTTON_HEIGHT: f32 = 40.;
const MODAL_WIDTH: f32 = 250.;

#[derive(Clone)]
struct TipItem {
    pub title: String,
    pub description: String,
    pub editable_binding_name: String,
    pub shortcut: Option<Keystroke>,
    pub tip_feature: Tip,
}

impl TipItem {
    pub fn new(
        title: String,
        description: String,
        feature: TipAction,
        ctx: &mut AppContext,
    ) -> Self {
        let editable_binding_name = feature.editable_binding_name().to_string();
        let shortcut = feature.keyboard_shortcut(ctx);
        let tip_feature = Tip::Action(feature);

        Self {
            title,
            description,
            editable_binding_name,
            shortcut,
            tip_feature,
        }
    }
}

pub struct TipsView {
    tips_completed: ModelHandle<TipsCompleted>,
    tip_items: Vec<TipItem>,
    button_mouse_states: MouseStateHandles,
    parent_position_id: String,
    action_target: ModelHandle<ActionTarget>,
    clipped_scroll_state: ClippedScrollStateHandle,
}

#[derive(Default)]
struct MouseStateHandles {
    clear_tips: MouseStateHandle,
    tip_handles: Vec<MouseStateHandle>,
    close_tips: MouseStateHandle,
}

#[derive(Debug)]
pub enum TipsAction {
    /// Action taken to close the tips dialog.
    Close,
    /// Action taken to dismiss the tips (and not show them again).
    DismissTips,
    /// Action taken to perform the action associated with a tip
    Click { index: usize },
    /// Keydown on tips view.
    KeyDown { keystroke: Keystroke },
}

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings(vec![FixedBinding::new(
        "escape",
        TipsAction::Close,
        id!("Tips"),
    )]);
}

#[derive(PartialEq, Eq)]
pub enum TipsEvent {
    /// Event fired when the tips dialog should close.
    Close,
    /// Event fired when the tips have been explicitly dismissed by the user.
    TipsDismissed,
}

impl TipsView {
    fn on_tips_model_changed(
        &mut self,
        _: ModelHandle<TipsCompleted>,
        ctx: &mut ViewContext<Self>,
    ) {
        ctx.notify();
    }

    pub fn new(
        tips_completed: ModelHandle<TipsCompleted>,
        parent_position_id: String,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        ctx.observe(&tips_completed, TipsView::on_tips_model_changed);

        let tip_items = vec![
            TipItem::new(
                "Command Palette".to_string(),
                "Easily discover everything you can do in Warp without your hands leaving the keyboard.".to_string(),
                TipAction::CommandPalette,
                ctx,
            ),
            TipItem::new(
                "Split Pane".to_string(),
                "Split tabs into multiple panes to make your ideal layout."
                    .to_string(),
                TipAction::SplitPane,
                ctx,
            ),
            TipItem::new(
                "History Search".to_string(),
                "Find, edit and re-run previously executed commands.".to_string(),
                TipAction::HistorySearch,
                ctx,
            ),
            TipItem::new(
                "AI Command Search".to_string(),
                "Generate shell commands with natural language.".to_string(),
                TipAction::AiCommandSearch,
                ctx,
            ),
            TipItem::new(
                "Theme Picker".to_string(),
                "Make Warp your own by choosing a built-in theme. Or create your own.".to_string(),
                TipAction::ThemePicker,
                ctx,
            ),
        ];

        // Initialize the action target cache. This will be updated when the tip menu is opened
        let action_target = ctx.add_model(|_| ActionTarget::None);

        let button_mouse_states = MouseStateHandles {
            tip_handles: tip_items.iter().map(|_| Default::default()).collect(),
            ..Default::default()
        };
        Self {
            tips_completed,
            tip_items,
            button_mouse_states,
            action_target,
            parent_position_id,
            clipped_scroll_state: Default::default(),
        }
    }

    pub fn set_action_target(
        &mut self,
        window_id: WindowId,
        input_id: Option<EntityId>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.action_target.update(ctx, |action_target, ctx| {
            *action_target = ActionTarget::View {
                window_id,
                input_id,
            };
            ctx.notify();
        });
    }

    fn dispatch_tip_action(&self, action: &dyn Action, ctx: &mut ViewContext<Self>) {
        let (window_id, input_id) = match self.action_target.as_ref(ctx) {
            ActionTarget::View {
                window_id,
                input_id,
            } => (*window_id, *input_id),
            ActionTarget::None => return,
        };
        if let Some(input_id) = input_id {
            ctx.dispatch_typed_action_for_view(window_id, input_id, action);
        }
    }

    fn render_check_svg(&self) -> Box<dyn Element> {
        let svg = Icon::new(CHECK_SVG_PATH, Fill::white().into_solid());

        ConstrainedBox::new(svg.finish())
            .with_height(8.)
            .with_width(13.)
            .finish()
    }

    fn render_tip_item(
        &self,
        tip_item: TipItem,
        appearance: &Appearance,
        index: usize,
        is_tip_completed: bool,
    ) -> Box<dyn Element> {
        let ui_builder = appearance.ui_builder();
        let theme = appearance.theme();
        let mut content = Flex::column();

        content.add_child(
            Container::new(
                ui_builder
                    .wrappable_text(tip_item.title, false)
                    .with_style(UiComponentStyles {
                        font_family_id: Some(appearance.ui_font_family()),
                        font_size: Some(appearance.monospace_font_size()),
                        font_weight: Some(Weight::Bold),
                        ..Default::default()
                    })
                    .build()
                    .finish(),
            )
            .with_padding_bottom(10.)
            .finish(),
        );

        content.add_child(
            Container::new(
                ui_builder
                    .wrappable_text(tip_item.description, true)
                    .with_style(UiComponentStyles {
                        font_family_id: Some(appearance.ui_font_family()),
                        font_size: Some(appearance.monospace_font_size() * 0.8),
                        ..Default::default()
                    })
                    .build()
                    .finish(),
            )
            .with_padding_bottom(10.)
            .finish(),
        );

        if let Some(keystroke) = tip_item.shortcut {
            let shortcut = Flex::row()
                .with_child(
                    Container::new(
                        ui_builder
                            .wrappable_text("Shortcut".to_string(), false)
                            .with_style(UiComponentStyles {
                                font_family_id: Some(appearance.ui_font_family()),
                                font_size: Some(appearance.monospace_font_size() * 0.8),
                                ..Default::default()
                            })
                            .build()
                            .finish(),
                    )
                    .with_padding_right(10.)
                    .finish(),
                )
                .with_child(ui_builder.keyboard_shortcut(&keystroke).build().finish())
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .finish();

            content.add_child(Container::new(shortcut).finish());
        }

        let container = if is_tip_completed {
            Container::new(
                Flex::row()
                    .with_child(
                        Shrinkable::new(
                            1.,
                            ConstrainedBox::new(
                                Container::new(content.finish())
                                    .with_padding_left(20.)
                                    .with_padding_top(20.)
                                    .with_padding_bottom(20.)
                                    // Reduce padding here when there's completed check mark rendering on the right.
                                    .with_padding_right(10.)
                                    .with_background(theme.welcome_tips_completion_overlay())
                                    .finish(),
                            )
                            // TODO: Have to hardcode the size here because the flex element
                            // will override its size constraint when its underlying non-flex children
                            // take up a larger size. Once the underlying issue is fixed,
                            // we should no longer needs this hard-coding.
                            .with_width(TIP_VIEW_WIDTH - CHECK_MARK_WIDTH)
                            .finish(),
                        )
                        .finish(),
                    )
                    .with_child(
                        Container::new(
                            ConstrainedBox::new(Align::new(self.render_check_svg()).finish())
                                .with_width(CHECK_MARK_WIDTH)
                                .finish(),
                        )
                        .finish(),
                    )
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .finish(),
            )
        } else {
            Container::new(content.finish()).with_uniform_padding(20.)
        };

        Hoverable::new(
            self.button_mouse_states.tip_handles[index].clone(),
            |state| {
                let background = match (state.is_hovered(), is_tip_completed) {
                    (true, false) => Some(theme.block_selection_color()),
                    (_, true) => Some(Fill::success()),
                    (false, false) => None,
                };

                if let Some(background) = background {
                    container
                        .with_background(background)
                        .with_border(
                            Border::bottom(1.)
                                .with_border_color(theme.split_pane_border_color().into_solid()),
                        )
                        .finish()
                } else {
                    container
                        .with_border(
                            Border::bottom(1.)
                                .with_border_color(theme.split_pane_border_color().into_solid()),
                        )
                        .finish()
                }
            },
        )
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(TipsAction::Click { index });
        })
        .with_cursor(Cursor::PointingHand)
        .finish()
    }

    fn render_body(
        &self,
        appearance: &Appearance,
        tips_completed: &TipsCompleted,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let mut tips = Flex::column();
        let tip_list = Flex::column().with_children(self.tip_items.iter().enumerate().map(
            |(index, tip_item)| {
                self.render_tip_item(
                    tip_item.clone(),
                    appearance,
                    index,
                    tips_completed.features_used.contains(&tip_item.tip_feature),
                )
            },
        ));
        let tip_list_with_background =
            Container::new(tip_list.finish()).with_background(theme.surface_2());
        tips.add_child(
            ConstrainedBox::new(
                ClippedScrollable::vertical(
                    self.clipped_scroll_state.clone(),
                    tip_list_with_background.finish(),
                    ScrollbarWidth::Auto,
                    theme.disabled_text_color(theme.background()).into(),
                    theme.main_text_color(theme.background()).into(),
                    theme.surface_2().into(),
                )
                .finish(),
            )
            .with_max_height(SCROLLABLE_AREA_HEIGHT)
            .finish(),
        );

        tips.add_child(
            ConstrainedBox::new(
                Hoverable::new(self.button_mouse_states.clear_tips.clone(), |state| {
                    Container::new(
                        Align::new(
                            appearance
                                .ui_builder()
                                .paragraph("Skip Welcome Tips".to_string())
                                .build()
                                .finish(),
                        )
                        .finish(),
                    )
                    .with_uniform_padding(10.)
                    .with_background({
                        let overlay = if state.is_hovered() {
                            theme.accent().with_opacity(SKIP_BUTTON_OVERLAY_OPACITY)
                        } else {
                            Fill::black().with_opacity(SKIP_BUTTON_OVERLAY_OPACITY)
                        };

                        theme.surface_2().blend(&overlay)
                    })
                    .with_corner_radius(CornerRadius::with_bottom(Radius::Pixels(8.)))
                    .finish()
                })
                .on_click(|ctx, _, _| ctx.dispatch_typed_action(TipsAction::DismissTips))
                .finish(),
            )
            .with_height(SKIP_BUTTON_HEIGHT)
            .finish(),
        );

        EventHandler::new(
            ConstrainedBox::new(tips.finish())
                .with_width(MODAL_WIDTH)
                .finish(),
        )
        .on_keydown(move |ctx, _, keystroke| {
            ctx.dispatch_typed_action(TipsAction::KeyDown {
                keystroke: keystroke.clone(),
            });
            DispatchEventResult::StopPropagation
        })
        .finish()
    }

    fn render_completed_overlay(&self, appearance: &Appearance) -> Box<dyn Element> {
        let ui_builder = appearance.ui_builder();
        // TODO: We should render this as a SVG.
        let confetti = ui_builder
            .span("🎉")
            .with_style(UiComponentStyles {
                font_size: Some(60.),
                ..Default::default()
            })
            .build()
            .finish();

        let title = ui_builder
            .span("Complete!")
            .with_style(UiComponentStyles {
                font_weight: Some(Weight::Bold),
                // Set to white here as the background has 85% black overlay.
                font_color: Some(Fill::white().into()),
                font_size: Some(appearance.header_font_size()),
                ..Default::default()
            })
            .build()
            .finish();

        let sub_text = ui_builder
            .paragraph("Nice work on finishing the welcome tips!")
            .with_style(UiComponentStyles {
                font_size: Some(12.),
                font_color: Some(Fill::white().into()),
                ..Default::default()
            })
            .build()
            .finish();

        let close_button = ui_builder
            .button(
                ButtonVariant::Accent,
                self.button_mouse_states.close_tips.clone(),
            )
            .with_style(
                UiComponentStyles::default()
                    .set_font_size(12.)
                    .set_width(152.)
                    .set_height(34.),
            )
            .with_centered_text_label("Close Welcome Tips".to_string())
            .build()
            .on_click(|ctx, _, _| ctx.dispatch_typed_action(TipsAction::DismissTips))
            .finish();

        ConstrainedBox::new(
            Container::new(
                Align::new(
                    Container::new(
                        Flex::column()
                            .with_child(Shrinkable::new(1., Align::new(confetti).finish()).finish())
                            .with_child(Shrinkable::new(0.3, Align::new(title).finish()).finish())
                            .with_child(
                                Shrinkable::new(0.3, Align::new(sub_text).finish()).finish(),
                            )
                            .with_child(
                                Shrinkable::new(0.5, Align::new(close_button).finish()).finish(),
                            )
                            .finish(),
                    )
                    .with_padding_top(107.)
                    .with_padding_bottom(97.)
                    .finish(),
                )
                .finish(),
            )
            .with_background(Fill::black().with_opacity(85))
            .with_corner_radius(CornerRadius::with_bottom(Radius::Pixels(8.)))
            .finish(),
        )
        .with_width(MODAL_WIDTH)
        .with_height(SCROLLABLE_AREA_HEIGHT + SKIP_BUTTON_HEIGHT)
        .finish()
    }
}

/// A model for tracking where the events from the tip view should be dispatched
///
/// Similar to command palette - we need a model to cache the information of where
/// we should send the actions from the welcome tips. When the tip view is opened,
/// we cache the current active window ID as well as the input ID of the active
/// tab/pane. By sending all the actions to the input view, we ensure that
/// they propagate correctly. This propagation assumes that each welcome tip action
/// must be in the responder chain. If an action is not in the responder chain
/// (such as a block navigation action) then it won't propagate correctly.
enum ActionTarget {
    None,
    View {
        window_id: WindowId,
        input_id: Option<EntityId>,
    },
}

impl Entity for ActionTarget {
    type Event = ();
}

impl Entity for TipsView {
    type Event = TipsEvent;
}

impl TypedActionView for TipsView {
    type Action = TipsAction;

    fn handle_action(&mut self, action: &TipsAction, ctx: &mut ViewContext<Self>) {
        match action {
            TipsAction::Close => {
                ctx.emit(TipsEvent::Close);
            }
            TipsAction::DismissTips => {
                ctx.emit(TipsEvent::TipsDismissed);
            }
            TipsAction::Click { index } => {
                let action = ctx
                    .editable_bindings()
                    .find(|action| action.name == self.tip_items[*index].editable_binding_name)
                    .map(|action| action.action.clone());
                if let Some(action) = action {
                    self.dispatch_tip_action(action.as_ref(), ctx);
                }
            }
            TipsAction::KeyDown { keystroke } => {
                let action = ctx
                    .editable_bindings()
                    .find(|action| trigger_to_keystroke(action.trigger) == Some(keystroke.clone()))
                    .map(|action| action.action.clone());
                if let Some(action) = action {
                    self.dispatch_tip_action(action.as_ref(), ctx);
                }
            }
        }
    }
}

impl View for TipsView {
    fn ui_name() -> &'static str {
        "Tips"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let mut stack = Stack::new();
        let tips_completed = self.tips_completed.as_ref(app);

        // Not great but we have to do this as a workaround for now. On the parent level,
        // add_positioned_child sets the bound on the z-index the view is going to be
        // rendered on. But then stack creates a new layer on top of it, which nullifies
        // the original position.
        stack.add_positioned_child(
            self.render_body(appearance, tips_completed),
            OffsetPositioning::offset_from_save_position_element(
                self.parent_position_id.as_str(),
                vec2f(0., 10.),
                PositionedElementOffsetBounds::Unbounded,
                PositionedElementAnchor::BottomRight,
                ChildAnchor::TopRight,
            ),
        );

        if tips_completed.completed_count() == WELCOME_TIP_FEATURE_LENGTH {
            stack.add_positioned_child(
                self.render_completed_overlay(appearance),
                OffsetPositioning::offset_from_save_position_element(
                    self.parent_position_id.as_str(),
                    vec2f(0., 10.),
                    PositionedElementOffsetBounds::Unbounded,
                    PositionedElementAnchor::BottomRight,
                    ChildAnchor::TopRight,
                ),
            );
        }

        stack.finish()
    }

    fn on_blur(&mut self, blur_ctx: &BlurContext, ctx: &mut ViewContext<Self>) {
        if blur_ctx.is_self_blurred() {
            ctx.emit(TipsEvent::Close);
        }
    }
}

#[cfg(test)]
#[path = "tip_view_tests.rs"]
mod tests;
