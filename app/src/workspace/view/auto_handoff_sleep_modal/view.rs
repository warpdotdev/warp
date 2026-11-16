use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use warp_core::ui::theme::Fill;
use warpui::assets::asset_cache::AssetSource;
use warpui::elements::{
    Align, CacheOption, ChildAnchor, ChildView, ConstrainedBox, Container, CornerRadius,
    CrossAxisAlignment, Expanded, Flex, Image, MainAxisSize, OffsetPositioning, ParentAnchor,
    ParentElement, ParentOffsetBounds, Radius, Stack, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::keymap::FixedBinding;
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::appearance::Appearance;
use crate::ui_components::icons::Icon;
use crate::view_components::action_button::{
    ActionButton, ActionButtonTheme, ButtonSize, PrimaryTheme, SecondaryTheme,
};

const MODAL_WIDTH: f32 = 420.;
const HERO_HEIGHT: f32 = 92.;
const HERO_IMAGE_PATH: &str = "async/png/onboarding/auto_handoff_sleep_banner.png";

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings([FixedBinding::new(
        "escape",
        AutoHandoffSleepModalAction::Dismiss,
        id!(AutoHandoffSleepModal::ui_name()),
    )]);
}

#[derive(Clone, Debug)]
pub enum AutoHandoffSleepModalAction {
    Enable,
    Dismiss,
}

#[derive(Clone, Debug)]
pub enum AutoHandoffSleepModalEvent {
    /// User clicked "Enable" — turn on auto-handoff-on-sleep and close.
    Enable,
    /// User dismissed the modal without enabling.
    Dismiss,
}

struct CloseButtonTheme;

impl ActionButtonTheme for CloseButtonTheme {
    fn background(&self, hovered: bool, appearance: &Appearance) -> Option<Fill> {
        if hovered {
            Some(appearance.theme().surface_overlay_1())
        } else {
            None
        }
    }

    fn text_color(
        &self,
        _hovered: bool,
        _background: Option<Fill>,
        _appearance: &Appearance,
    ) -> ColorU {
        ColorU::white()
    }
}

pub struct AutoHandoffSleepModal {
    close_button: ViewHandle<ActionButton>,
    enable_button: ViewHandle<ActionButton>,
    dismiss_button: ViewHandle<ActionButton>,
}

impl AutoHandoffSleepModal {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let close_button = ctx.add_view(|_ctx| {
            ActionButton::new("", CloseButtonTheme)
                .with_icon(Icon::X)
                .with_size(ButtonSize::Small)
                .on_click(|ctx| ctx.dispatch_typed_action(AutoHandoffSleepModalAction::Dismiss))
        });

        let enable_button = ctx.add_view(|_ctx| {
            ActionButton::new("Enable", PrimaryTheme)
                .with_full_width(true)
                .with_size(ButtonSize::Default)
                .on_click(|ctx| ctx.dispatch_typed_action(AutoHandoffSleepModalAction::Enable))
        });

        let dismiss_button = ctx.add_view(|_ctx| {
            ActionButton::new("Dismiss", SecondaryTheme)
                .with_full_width(true)
                .with_size(ButtonSize::Default)
                .on_click(|ctx| ctx.dispatch_typed_action(AutoHandoffSleepModalAction::Dismiss))
        });

        Self {
            close_button,
            enable_button,
            dismiss_button,
        }
    }

    fn render_hero(&self) -> Box<dyn Element> {
        let hero = ConstrainedBox::new(
            Image::new(
                AssetSource::Bundled {
                    path: HERO_IMAGE_PATH,
                },
                CacheOption::Original,
            )
            .with_corner_radius(CornerRadius::with_top(Radius::Pixels(8.)))
            .cover()
            .top_aligned()
            .finish(),
        )
        .with_width(MODAL_WIDTH)
        .with_height(HERO_HEIGHT)
        .finish();

        let close_el = Container::new(ChildView::new(&self.close_button).finish())
            .with_uniform_padding(4.)
            .with_padding_right(2.)
            .finish();

        let mut hero_stack = Stack::new();
        hero_stack.add_child(hero);
        hero_stack.add_positioned_child(
            close_el,
            OffsetPositioning::offset_from_parent(
                vec2f(-4., 0.),
                ParentOffsetBounds::ParentByPosition,
                ParentAnchor::TopRight,
                ChildAnchor::TopRight,
            ),
        );
        hero_stack.finish()
    }

    fn render_badge(appearance: &Appearance) -> Box<dyn Element> {
        let red = appearance.theme().terminal_colors().normal.red;
        let text_color: ColorU = red.into();
        let background_color = appearance.theme().ansi_overlay_2(red);
        let text = Text::new_inline(
            "Run Connection Lost".to_string(),
            appearance.ui_font_family(),
            14.,
        )
        .with_color(text_color)
        .finish();
        ConstrainedBox::new(
            Container::new(
                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_main_axis_size(MainAxisSize::Min)
                    .with_child(text)
                    .finish(),
            )
            .with_horizontal_padding(8.)
            .with_background(Fill::Solid(background_color))
            .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
            .finish(),
        )
        .with_height(24.)
        .finish()
    }

    fn render_title(appearance: &Appearance) -> Box<dyn Element> {
        Text::new("Enable auto-handoff?", appearance.ui_font_family(), 20.)
            .with_color(
                appearance
                    .theme()
                    .main_text_color(appearance.theme().surface_3())
                    .into_solid(),
            )
            .with_style(Properties::default().weight(Weight::Semibold))
            .finish()
    }

    fn render_description(appearance: &Appearance) -> Box<dyn Element> {
        Text::new(
            "Give Warp the option to automatically move active local agents to the cloud when \
             your computer sleeps.",
            appearance.ui_font_family(),
            14.,
        )
        .with_color(
            appearance
                .theme()
                .sub_text_color(appearance.theme().surface_3())
                .into_solid(),
        )
        .finish()
    }

    fn render_body(&self, appearance: &Appearance) -> Box<dyn Element> {
        let footer = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(12.)
            .with_child(Expanded::new(1., ChildView::new(&self.enable_button).finish()).finish())
            .with_child(Expanded::new(1., ChildView::new(&self.dismiss_button).finish()).finish())
            .finish();

        Container::new(
            Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_spacing(12.)
                .with_child(
                    Flex::column()
                        .with_cross_axis_alignment(CrossAxisAlignment::Start)
                        .with_spacing(8.)
                        .with_child(Self::render_badge(appearance))
                        .with_child(Self::render_title(appearance))
                        .finish(),
                )
                .with_child(Self::render_description(appearance))
                .with_child(footer)
                .finish(),
        )
        .with_horizontal_padding(32.)
        .with_vertical_padding(32.)
        .with_background(appearance.theme().surface_3())
        .with_corner_radius(CornerRadius::with_bottom(Radius::Pixels(8.)))
        .finish()
    }
}

impl Entity for AutoHandoffSleepModal {
    type Event = AutoHandoffSleepModalEvent;
}

impl View for AutoHandoffSleepModal {
    fn ui_name() -> &'static str {
        "AutoHandoffSleepModal"
    }

    fn on_focus(&mut self, _focus_ctx: &warpui::FocusContext, ctx: &mut ViewContext<Self>) {
        ctx.focus_self();
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        let card = ConstrainedBox::new(
            Container::new(
                Flex::column()
                    .with_main_axis_size(MainAxisSize::Min)
                    .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .with_child(self.render_hero())
                    .with_child(self.render_body(appearance))
                    .finish(),
            )
            .with_background(appearance.theme().surface_3())
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
            .finish(),
        )
        .with_width(MODAL_WIDTH)
        .finish();

        Container::new(Align::new(card).finish())
            .with_background(Fill::Solid(ColorU::new(97, 97, 97, 255)).with_opacity(50))
            .finish()
    }
}

impl TypedActionView for AutoHandoffSleepModal {
    type Action = AutoHandoffSleepModalAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            AutoHandoffSleepModalAction::Enable => {
                ctx.emit(AutoHandoffSleepModalEvent::Enable);
            }
            AutoHandoffSleepModalAction::Dismiss => {
                ctx.emit(AutoHandoffSleepModalEvent::Dismiss);
            }
        }
    }
}
