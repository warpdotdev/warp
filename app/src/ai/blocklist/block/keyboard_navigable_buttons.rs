use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors;
use warpui::elements::{
    ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Empty, Flex, MainAxisAlignment,
    MainAxisSize, MouseStateHandle, ParentElement, Radius, Shrinkable, Text,
};
use warpui::keymap::FixedBinding;
use warpui::ui_components::button::{Button, ButtonVariant};
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::{AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext};

use super::numbered_button::render_recommended_badge;
use crate::ui_components::icons::Icon;

const MARGIN_BETWEEN_BUTTONS: f32 = 4.;
const HAS_OPTIONS: &str = "HasOptions";

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings([
        FixedBinding::new(
            "enter",
            KeyboardNavigableButtonsAction::Enter,
            id!(KeyboardNavigableButtons::ui_name()) & id!(HAS_OPTIONS),
        ),
        FixedBinding::new(
            "numpadenter",
            KeyboardNavigableButtonsAction::Enter,
            id!(KeyboardNavigableButtons::ui_name()) & id!(HAS_OPTIONS),
        ),
        FixedBinding::new(
            "up",
            KeyboardNavigableButtonsAction::ArrowUp,
            id!(KeyboardNavigableButtons::ui_name()) & id!(HAS_OPTIONS),
        ),
        FixedBinding::new(
            "down",
            KeyboardNavigableButtonsAction::ArrowDown,
            id!(KeyboardNavigableButtons::ui_name()) & id!(HAS_OPTIONS),
        ),
    ]);
}

#[derive(Debug, Clone)]
pub enum KeyboardNavigableButtonsAction {
    HoveredIn(usize),
    ButtonClicked(usize),

    ArrowUp,
    ArrowDown,
    Enter,
}

pub enum KeyboardNavigableButtonsEvent {}

pub type ButtonBuilder = Box<dyn Fn(bool, &warpui::AppContext) -> Button>;
pub type OnButtonClickFn = Box<dyn Fn(&mut ViewContext<KeyboardNavigableButtons>)>;

pub struct KeyboardNavigableButtonBuilder {
    button_builder: ButtonBuilder,
    /// Called when the button is selected through click or enter.
    on_click: OnButtonClickFn,
}

impl KeyboardNavigableButtonBuilder {
    pub fn new(
        button_builder: impl Fn(bool, &warpui::AppContext) -> Button + 'static,
        on_selected: impl Fn(&mut ViewContext<KeyboardNavigableButtons>) + 'static,
    ) -> Self {
        Self {
            button_builder: Box::new(button_builder),
            on_click: Box::new(on_selected),
        }
    }
}

/// Creates a simple navigation button with standard styling.
/// This is a convenience function for the common case of a text-only button
/// that dispatches an action when clicked.
pub fn simple_navigation_button<A: warpui::Action + Clone + 'static>(
    text_label: String,
    mouse_state: MouseStateHandle,
    action: A,
    disabled: bool,
) -> KeyboardNavigableButtonBuilder {
    KeyboardNavigableButtonBuilder::new(
        move |is_selected, app| {
            let appearance = Appearance::as_ref(app);
            let mut button = appearance
                .ui_builder()
                .button(ButtonVariant::Secondary, mouse_state.clone())
                .with_style(UiComponentStyles {
                    font_size: Some(appearance.monospace_font_size()),
                    ..UiComponentStyles::default()
                })
                .with_hovered_styles(UiComponentStyles {
                    font_size: Some(appearance.monospace_font_size()),
                    ..UiComponentStyles::default()
                });
            if disabled {
                button = button.disabled();
            } else if is_selected {
                button = button.with_style(UiComponentStyles {
                    border_color: Some(appearance.theme().accent().into()),
                    border_width: Some(1.0),
                    background: Some(appearance.theme().surface_2().into()),
                    ..UiComponentStyles::default()
                });
            }
            button.with_text_label(text_label.clone())
        },
        move |ctx: &mut ViewContext<KeyboardNavigableButtons>| {
            if !disabled {
                ctx.dispatch_typed_action(&action);
            }
        },
    )
}

/// Builds the label for a [`rich_navigation_button`]: a title row (with
/// optional `Recommended` badge) above an optional muted sub-label, plus
/// an enter-key indicator centered vertically within the full label area.
fn build_rich_navigation_label(
    text_label: &str,
    sub_label: Option<&str>,
    recommended: bool,
    show_enter_indicator: bool,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let font_size = appearance.monospace_font_size();

    let title = Text::new(
        text_label.to_string(),
        appearance.ui_font_family(),
        font_size,
    )
    .soft_wrap(true)
    .with_color(theme.foreground().into())
    .finish();

    // Title row: title text + optional recommended badge (enter indicator is
    // handled separately so it can be centered within the full button height).
    let title_row: Box<dyn Element> = if recommended {
        Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(title)
            .with_child(
                Container::new(render_recommended_badge(appearance))
                    .with_margin_left(8.)
                    .finish(),
            )
            .finish()
    } else {
        title
    };

    // Build the text column (title + optional sublabel).
    let text_column: Box<dyn Element> = if let Some(sub_label) = sub_label {
        let sub_label_element = Text::new(
            sub_label.to_string(),
            appearance.ui_font_family(),
            appearance.monospace_font_size() - 2.,
        )
        .soft_wrap(true)
        .with_color(internal_colors::neutral_5(theme))
        .finish();

        Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(title_row)
            .with_child(
                Container::new(sub_label_element)
                    .with_margin_top(4.)
                    .finish(),
            )
            .finish()
    } else {
        title_row
    };

    // Always reserve the same horizontal space for the enter indicator so
    // the text column width stays constant and the sub-label doesn't rewrap
    // on hover.
    const ENTER_KEY_PADDING: f32 = 4.;
    let enter_indicator_size = font_size + 2. * ENTER_KEY_PADDING;
    let right_element: Box<dyn Element> = if show_enter_indicator {
        let enter_icon = ConstrainedBox::new(
            Icon::CornerDownLeft
                .to_warpui_icon(theme.foreground())
                .finish(),
        )
        .with_width(font_size)
        .with_height(font_size)
        .finish();

        Container::new(enter_icon)
            .with_uniform_padding(ENTER_KEY_PADDING)
            .with_background(internal_colors::fg_overlay_1(theme))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
            .finish()
    } else {
        // Invisible spacer matching the enter indicator dimensions.
        ConstrainedBox::new(Empty::new().finish())
            .with_width(enter_indicator_size)
            .with_height(enter_indicator_size)
            .finish()
    };

    Flex::row()
        .with_main_axis_size(MainAxisSize::Max)
        .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(Shrinkable::new(1., text_column).finish())
        .with_child(Container::new(right_element).with_margin_left(12.).finish())
        .finish()
}

/// Creates a keyboard-navigable button with a rich two-line label: a title
/// (with an optional trailing "Recommended" badge) plus an optional muted
/// sub-label underneath.
pub fn rich_navigation_button<A: warpui::Action + Clone + 'static>(
    text_label: String,
    sub_label: Option<String>,
    recommended: bool,
    mouse_state: MouseStateHandle,
    action: A,
) -> KeyboardNavigableButtonBuilder {
    KeyboardNavigableButtonBuilder::new(
        move |is_selected, app| {
            let appearance = Appearance::as_ref(app);
            let theme = appearance.theme();
            let font_size = appearance.monospace_font_size();

            let base_style = UiComponentStyles {
                font_size: Some(font_size),
                border_radius: Some(CornerRadius::with_all(Radius::Pixels(4.))),
                ..UiComponentStyles::default()
            };

            let style = if is_selected {
                UiComponentStyles {
                    border_color: Some(theme.accent().into()),
                    border_width: Some(1.0),
                    background: Some(internal_colors::fg_overlay_2(theme).into()),
                    ..base_style
                }
            } else {
                base_style
            };

            let label = build_rich_navigation_label(
                &text_label,
                sub_label.as_deref(),
                recommended,
                is_selected,
                appearance,
            );

            appearance
                .ui_builder()
                .button(ButtonVariant::Secondary, mouse_state.clone())
                .with_style(style)
                .with_hovered_styles(base_style)
                .with_custom_label(label)
        },
        move |ctx: &mut ViewContext<KeyboardNavigableButtons>| {
            ctx.dispatch_typed_action(&action);
        },
    )
}

/// A view that wraps buttons to make them keyboard navigable.
/// Mouse hover and keyboard navigation both update the same selection index.
/// When hovering stops, the selection remains on the last selected button.
/// Note that this view must be focused for keyboard shortcuts to work -
/// the parent view likely needs to focus this view manually.
pub struct KeyboardNavigableButtons {
    button_builders: Vec<KeyboardNavigableButtonBuilder>,
    selected_button_index: usize,
}

impl KeyboardNavigableButtons {
    pub fn new(button_builders: Vec<KeyboardNavigableButtonBuilder>) -> Self {
        Self {
            button_builders,
            selected_button_index: 0,
        }
    }

    fn selected_button_index(&self) -> usize {
        self.selected_button_index
    }
}

impl View for KeyboardNavigableButtons {
    fn ui_name() -> &'static str {
        "KeyboardNavigableButtons"
    }

    fn render(&self, app: &warpui::AppContext) -> Box<dyn warpui::Element> {
        let mut content = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
        for (index, button_builder) in self.button_builders.iter().enumerate() {
            let is_selected = index == self.selected_button_index();
            let button = (button_builder.button_builder)(is_selected, app);
            let mut hoverable = button.build();

            hoverable = hoverable
                .additional_on_hover(move |is_hovered, ctx, _app, _pos| {
                    if is_hovered {
                        ctx.dispatch_typed_action(KeyboardNavigableButtonsAction::HoveredIn(index));
                    }
                })
                .on_click(move |ctx, _app, _pos| {
                    ctx.dispatch_typed_action(KeyboardNavigableButtonsAction::ButtonClicked(index));
                });
            let margin_bottom = if index == self.button_builders.len() - 1 {
                0.
            } else {
                MARGIN_BETWEEN_BUTTONS
            };
            content.add_child(
                Container::new(hoverable.finish())
                    .with_margin_bottom(margin_bottom)
                    .finish(),
            );
        }
        content.finish()
    }

    fn keymap_context(&self, _app: &AppContext) -> warpui::keymap::Context {
        let mut context = Self::default_keymap_context();
        if !self.button_builders.is_empty() {
            context.set.insert(HAS_OPTIONS);
        }
        context
    }
}

impl TypedActionView for KeyboardNavigableButtons {
    type Action = KeyboardNavigableButtonsAction;

    fn handle_action(
        &mut self,
        action: &KeyboardNavigableButtonsAction,
        ctx: &mut ViewContext<Self>,
    ) {
        match action {
            KeyboardNavigableButtonsAction::HoveredIn(index) => {
                self.selected_button_index = *index;
            }
            KeyboardNavigableButtonsAction::ButtonClicked(index) => {
                if let Some(builder) = self.button_builders.get(*index) {
                    (builder.on_click)(ctx);
                }
            }
            KeyboardNavigableButtonsAction::ArrowUp => {
                self.selected_button_index =
                    (self.selected_button_index + self.button_builders.len() - 1)
                        % self.button_builders.len();
            }
            KeyboardNavigableButtonsAction::ArrowDown => {
                self.selected_button_index =
                    (self.selected_button_index + 1) % self.button_builders.len();
            }
            KeyboardNavigableButtonsAction::Enter => {
                if let Some(builder) = self.button_builders.get(self.selected_button_index()) {
                    (builder.on_click)(ctx);
                }
            }
        };
        ctx.notify();
    }
}

impl Entity for KeyboardNavigableButtons {
    type Event = KeyboardNavigableButtonsEvent;
}
