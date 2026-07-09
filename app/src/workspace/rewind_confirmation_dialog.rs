use pathfinder_geometry::vector::vec2f;
use warp_core::ui::color::coloru_with_opacity;
use warp_core::ui::theme::Fill;
use warp_errors::report_error;
use warpui::elements::{
    Align, ChildAnchor, ConstrainedBox, Container, CrossAxisAlignment, Flex, Hoverable,
    MouseStateHandle, OffsetPositioning, ParentAnchor, ParentElement, ParentOffsetBounds, Stack,
    Text,
};
use warpui::fonts::Weight;
use warpui::keymap::{FixedBinding, Keystroke};
use warpui::platform::Cursor;
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{
    AppContext, Element, Entity, EntityId, SingletonEntity, TypedActionView, View, ViewContext,
};

use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::AIAgentExchangeId;
use crate::appearance::Appearance;
use crate::ui_components::dialog::{dialog_styles, Dialog};
use crate::ui_components::icons::Icon;

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings([
        FixedBinding::new(
            "escape",
            RewindConfirmationAction::Cancel,
            id!(RewindConfirmationDialog::ui_name()),
        ),
        FixedBinding::new(
            "enter",
            RewindConfirmationAction::Confirm,
            id!(RewindConfirmationDialog::ui_name()),
        ),
    ]);
}

const DIALOG_WIDTH: f32 = 460.;

/// Data needed to perform the rewind action after confirmation
#[derive(Clone)]
pub struct RewindDialogSource {
    pub ai_block_view_id: EntityId,
    pub exchange_id: AIAgentExchangeId,
    pub conversation_id: AIConversationId,
}

pub struct RewindConfirmationDialog {
    cancel_mouse_state: MouseStateHandle,
    confirm_mouse_state: MouseStateHandle,
    /// Source will be None if dialog was never opened
    rewind_source: Option<RewindDialogSource>,
}

impl Default for RewindConfirmationDialog {
    fn default() -> Self {
        Self::new()
    }
}

impl RewindConfirmationDialog {
    pub fn new() -> Self {
        Self {
            cancel_mouse_state: Default::default(),
            confirm_mouse_state: Default::default(),
            rewind_source: None,
        }
    }

    pub fn set_rewind_source(&mut self, source: RewindDialogSource) {
        self.rewind_source = Some(source);
    }
}

impl Entity for RewindConfirmationDialog {
    type Event = RewindConfirmationEvent;
}

impl View for RewindConfirmationDialog {
    fn ui_name() -> &'static str {
        "RewindConfirmationDialog"
    }

    fn on_focus(&mut self, _focus_ctx: &warpui::FocusContext, ctx: &mut ViewContext<Self>) {
        ctx.focus_self();
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let button_style = UiComponentStyles {
            font_size: Some(14.),
            font_weight: Some(Weight::Bold),
            height: Some(40.),
            ..Default::default()
        };

        // Build rewind button label with Enter keyboard shortcut indicator
        let enter_keystroke = Keystroke::parse("enter").expect("Valid keystroke");
        let text_color = theme.main_text_color(theme.accent()).into_solid();
        let rewind_button_label = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Text::new_inline("Rewind", appearance.ui_font_family(), 14.)
                    .with_color(text_color)
                    .finish(),
            )
            .with_child(
                Container::new(
                    appearance
                        .ui_builder()
                        .keyboard_shortcut(&enter_keystroke)
                        .with_style(UiComponentStyles {
                            font_size: Some(10.),
                            height: Some(16.),
                            padding: Some(Coords::uniform(1.)),
                            border_width: Some(1.),
                            border_color: Some(coloru_with_opacity(text_color, 60).into()),
                            font_color: Some(text_color),
                            font_family_id: Some(appearance.ui_font_family()),
                            ..Default::default()
                        })
                        .with_line_height_ratio(1.0)
                        .build()
                        .finish(),
                )
                .with_margin_left(8.)
                .finish(),
            )
            .finish();

        let rewind_button = appearance
            .ui_builder()
            .button(ButtonVariant::Accent, self.confirm_mouse_state.clone())
            .with_custom_label(rewind_button_label)
            .with_style(UiComponentStyles {
                padding: Some(Coords {
                    left: 16.,
                    right: 16.,
                    top: 0.,
                    bottom: 0.,
                }),
                ..button_style
            })
            .build()
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| ctx.dispatch_typed_action(RewindConfirmationAction::Confirm))
            .finish();

        let cancel_text_color = theme.sub_text_color(theme.surface_2());
        let cancel_button = Container::new(
            Hoverable::new(self.cancel_mouse_state.clone(), move |mouse_state| {
                let color = if mouse_state.is_mouse_over_element() {
                    theme.main_text_color(theme.surface_2())
                } else {
                    cancel_text_color
                };
                Text::new_inline("Cancel", appearance.ui_font_family(), 14.)
                    .with_color(color.into_solid())
                    .finish()
            })
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| ctx.dispatch_typed_action(RewindConfirmationAction::Cancel))
            .finish(),
        )
        .with_margin_right(16.)
        .finish();

        // Info text with icon
        let info_color = theme.sub_text_color(theme.surface_2());
        let info_row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Container::new(
                    ConstrainedBox::new(Icon::Info.to_warpui_icon(info_color).finish())
                        .with_height(14.)
                        .with_width(14.)
                        .finish(),
                )
                .with_margin_right(6.)
                .finish(),
            )
            .with_child(
                Text::new_inline(
                    "Rewinding does not affect files edited manually or via shell commands.",
                    appearance.ui_font_family(),
                    12.,
                )
                .with_color(info_color.into_solid())
                .finish(),
            )
            .finish();

        let dialog = Container::new(
            Dialog::new(
                "Rewind".into(),
                Some(
                    "Are you sure you want to rewind? This will restore your code and conversation to before this point, and cancel any commands the agent is currently running. A copy of the original conversation will be saved in your conversation history."
                        .into(),
                ),
                UiComponentStyles {
                    width: Some(DIALOG_WIDTH),
                    padding: Some(Coords::uniform(24.)),
                    ..dialog_styles(appearance)
                },
            )
            .with_child(info_row)
            .with_bottom_row_child(cancel_button)
            .with_bottom_row_child(rewind_button)
            .build()
            .finish(),
        )
        .with_margin_top(35.)
        .finish();

        // Stack needed so that dialog can get bounds information
        let mut stack = Stack::new();
        stack.add_positioned_child(
            dialog,
            OffsetPositioning::offset_from_parent(
                vec2f(0., 0.),
                ParentOffsetBounds::WindowByPosition,
                ParentAnchor::Center,
                ChildAnchor::Center,
            ),
        );

        // This blurs the background and makes it uninteractable
        Container::new(Align::new(stack.finish()).finish())
            .with_background_color(Fill::blur().into())
            .with_corner_radius(app.windows().window_corner_radius())
            .finish()
    }
}

pub enum RewindConfirmationEvent {
    Confirm { rewind_source: RewindDialogSource },
    Cancel,
}

#[derive(Debug)]
pub enum RewindConfirmationAction {
    Confirm,
    Cancel,
}

impl TypedActionView for RewindConfirmationDialog {
    type Action = RewindConfirmationAction;

    fn handle_action(&mut self, action: &RewindConfirmationAction, ctx: &mut ViewContext<Self>) {
        match action {
            RewindConfirmationAction::Confirm => {
                let Some(rewind_source) = self.rewind_source.clone() else {
                    report_error!("Rewind confirm button pressed with no rewind source");
                    return;
                };
                ctx.emit(RewindConfirmationEvent::Confirm { rewind_source });
            }
            RewindConfirmationAction::Cancel => {
                ctx.emit(RewindConfirmationEvent::Cancel);
            }
        }
    }
}
