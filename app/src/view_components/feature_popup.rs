use warpui::elements::{
    ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Flex, Hoverable, MouseStateHandle,
    ParentElement, Radius, Text,
};
use warpui::platform::Cursor;
use warpui::{AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext};

use crate::appearance::Appearance;
use crate::ui_components::icons::Icon;

pub enum NewFeaturePopupLabel {
    /// A static label.
    FromString(String),
    /// A label that is computed on demand.
    FromCallable(Box<dyn Fn(&AppContext) -> String>),
}

pub enum FeaturePopupBadge {
    // Displays "NEW" badge prior to the label
    New,
    // Displays an alert icon prior to the label
    AlertIcon,
}

/// A dismissable popup that displays a label indicating that a new feature is available.
pub struct FeaturePopup {
    dismiss_mouse_state: MouseStateHandle,
    label: NewFeaturePopupLabel,
    badge: FeaturePopupBadge,
}

#[derive(Debug, Clone)]
pub enum NewFeaturePopupAction {
    Dismiss,
}

impl FeaturePopup {
    pub fn new_feature(label: NewFeaturePopupLabel) -> Self {
        Self {
            dismiss_mouse_state: Default::default(),
            label,
            badge: FeaturePopupBadge::New,
        }
    }

    pub fn alert_icon(label: NewFeaturePopupLabel) -> Self {
        Self {
            dismiss_mouse_state: Default::default(),
            label,
            badge: FeaturePopupBadge::AlertIcon,
        }
    }

    fn render_badge(&self, appearance: &Appearance) -> Box<dyn Element> {
        let background = appearance.theme().background();
        match self.badge {
            FeaturePopupBadge::New => Container::new(
                Text::new(
                    "NEW",
                    appearance.ui_font_family(),
                    appearance.ui_font_size(),
                )
                .with_color(appearance.theme().main_text_color(background).into())
                .finish(),
            )
            .with_vertical_padding(2.)
            .with_horizontal_padding(4.)
            .with_background_color(
                appearance
                    .theme()
                    .ansi_bg(appearance.theme().terminal_colors().normal.green),
            )
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(2.)))
            .finish(),
            FeaturePopupBadge::AlertIcon => Container::new(
                ConstrainedBox::new(
                    Icon::AlertCircle
                        .to_warpui_icon(appearance.theme().main_text_color(
                            appearance.theme().terminal_colors().normal.green.into(),
                        ))
                        .finish(),
                )
                .with_height(16.)
                .with_width(16.)
                .finish(),
            )
            .finish(),
        }
    }
}

impl View for FeaturePopup {
    fn ui_name() -> &'static str {
        "FeaturePopup"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let background = appearance.theme().background();
        let new_badge = self.render_badge(appearance);

        let label = match &self.label {
            NewFeaturePopupLabel::FromString(label) => label.clone(),
            NewFeaturePopupLabel::FromCallable(callable) => callable(app),
        };
        ConstrainedBox::new(
            Container::new(
                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(Container::new(new_badge).with_margin_right(4.).finish())
                    .with_child(
                        Container::new(
                            Text::new(
                                label,
                                appearance.ui_font_family(),
                                appearance.ui_font_size(),
                            )
                            .with_color(background.into())
                            .finish(),
                        )
                        .with_horizontal_padding(4.)
                        .finish(),
                    )
                    .with_child(
                        Hoverable::new(self.dismiss_mouse_state.clone(), |_| {
                            ConstrainedBox::new(
                                Icon::X
                                    .to_warpui_icon(appearance.theme().sub_text_color(
                                        appearance.theme().main_text_color(background),
                                    ))
                                    .finish(),
                            )
                            .with_height(16.)
                            .with_width(16.)
                            .finish()
                        })
                        .on_click(|ctx, _, _| {
                            ctx.dispatch_typed_action(NewFeaturePopupAction::Dismiss)
                        })
                        .with_cursor(Cursor::PointingHand)
                        .finish(),
                    )
                    .finish(),
            )
            .with_horizontal_padding(4.)
            .with_vertical_padding(4.)
            .with_background(appearance.theme().main_text_color(background))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
            .finish(),
        )
        .finish()
    }
}

impl TypedActionView for FeaturePopup {
    type Action = NewFeaturePopupAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            NewFeaturePopupAction::Dismiss => {
                ctx.emit(NewFeaturePopupEvent::Dismissed);
            }
        }
    }
}

#[derive(Debug, Clone)]
pub enum NewFeaturePopupEvent {
    Dismissed,
}

impl Entity for FeaturePopup {
    type Event = NewFeaturePopupEvent;
}
