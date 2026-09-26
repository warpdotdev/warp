use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use warp_core::ui::theme::Fill;
use warpui::assets::asset_cache::AssetSource;
use warpui::elements::{
    Align, CacheOption, ChildAnchor, ChildView, Clipped, ConstrainedBox, Container, CornerRadius,
    CrossAxisAlignment, Dismiss, Expanded, Flex, Highlight, Image, MainAxisSize, OffsetPositioning,
    ParentAnchor, ParentElement, ParentOffsetBounds, Radius, Stack, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::keymap::FixedBinding;
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::appearance::Appearance;
use crate::ui_components::icons::Icon;
use crate::view_components::action_button::{
    ActionButton, ActionButtonTheme, ButtonSize, PrimaryTheme,
};

const MODAL_WIDTH: f32 = 420.;
const HERO_HEIGHT: f32 = 92.;
const HERO_IMAGE_PATH: &str = "async/png/onboarding/factories_launch_banner.png";
const OFFER_TEXT: &str =
    "Get hands-on implementation support and up to $10K in Factory usage during Early Access.";
const OFFER_EMPHASIS: &str = "up to $10K";

/// Persisted and globally synced; changing this key would reshow the modal.
pub const FACTORIES_LAUNCH_SEEN_KEY: &str = "factories_launch";

struct HowItWorksItem {
    icon: Icon,
    title: &'static str,
    description: &'static str,
}

const HOW_IT_WORKS: &[HowItWorksItem] = &[
    HowItWorksItem {
        icon: Icon::Code1,
        title: "Factories-as-code",
        description: "Define your software factory declaratively and version it alongside your codebase.",
    },
    HowItWorksItem {
        icon: Icon::Atom,
        title: "Any model or harness",
        description: "Mix and match models and agent harnesses for each stage of the factory.",
    },
    HowItWorksItem {
        icon: Icon::Cognition,
        title: "Evals & self-improvement",
        description: "Built-in evals continuously tune your factory's agents over time.",
    },
];

fn modal_background(appearance: &Appearance) -> Fill {
    appearance.theme().surface_3()
}

fn modal_text_main(appearance: &Appearance) -> ColorU {
    appearance
        .theme()
        .main_text_color(modal_background(appearance))
        .into_solid()
}

fn modal_text_sub(appearance: &Appearance) -> ColorU {
    appearance
        .theme()
        .sub_text_color(modal_background(appearance))
        .into_solid()
}

fn modal_terminal_magenta(appearance: &Appearance) -> ColorU {
    appearance.theme().terminal_colors().normal.magenta.into()
}

fn modal_terminal_magenta_overlay_1(appearance: &Appearance) -> ColorU {
    let magenta = appearance.theme().terminal_colors().normal.magenta;
    appearance.theme().ansi_overlay_1(magenta)
}

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings([FixedBinding::new(
        "escape",
        FactoriesLaunchModalAction::Close,
        id!(FactoriesLaunchModal::ui_name()),
    )]);
}

#[derive(Clone, Debug)]
pub enum FactoriesLaunchModalAction {
    Close,
    GetEarlyAccess,
}

#[derive(Clone, Debug)]
pub enum FactoriesLaunchModalEvent {
    /// The user dismissed the modal (close button, Escape, or clicking the scrim).
    Close,
    /// The user clicked the primary call-to-action.
    GetEarlyAccess,
}

struct CloseButtonTheme;

impl ActionButtonTheme for CloseButtonTheme {
    fn background(&self, hovered: bool, _appearance: &Appearance) -> Option<Fill> {
        // A fixed dark scrim keeps the icon legible over arbitrary hero artwork.
        let opacity = if hovered { 50 } else { 30 };
        Some(Fill::Solid(ColorU::black()).with_opacity(opacity))
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

/// The centered, blocking launch modal for Factories.
pub struct FactoriesLaunchModal {
    close_button: ViewHandle<ActionButton>,
    cta_button: ViewHandle<ActionButton>,
}

impl FactoriesLaunchModal {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let close_button = ctx.add_view(|_ctx| {
            ActionButton::new("", CloseButtonTheme)
                .with_icon(Icon::X)
                .with_size(ButtonSize::Small)
                .on_click(|ctx| ctx.dispatch_typed_action(FactoriesLaunchModalAction::Close))
        });

        let cta_button = ctx.add_view(|_ctx| {
            ActionButton::new("Get Early Access", PrimaryTheme)
                .with_full_width(true)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(FactoriesLaunchModalAction::GetEarlyAccess)
                })
        });

        Self {
            close_button,
            cta_button,
        }
    }

    fn render_hero(&self) -> Box<dyn Element> {
        let hero = Clipped::new(
            ConstrainedBox::new(
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
            .finish(),
        )
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
        let text_color = modal_terminal_magenta(appearance);
        let background_color = modal_terminal_magenta_overlay_1(appearance);
        let text = Text::new_inline("New".to_string(), appearance.ui_font_family(), 14.)
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
        Text::new(
            "Build your software factory on Warp",
            appearance.ui_font_family(),
            20.,
        )
        .with_color(modal_text_main(appearance))
        .with_style(Properties::default().weight(Weight::Semibold))
        .finish()
    }

    fn render_description(appearance: &Appearance) -> Box<dyn Element> {
        Text::new(
            "Open, flexible infrastructure for building cloud software factories around your team. Factories-as-code, any model or harness, with evals and self-improvement built in.",
            appearance.ui_font_family(),
            14.,
        )
        .with_color(modal_text_sub(appearance))
        .finish()
    }

    fn render_how_it_works_row(item: &HowItWorksItem, appearance: &Appearance) -> Box<dyn Element> {
        let icon_el = ConstrainedBox::new(
            item.icon
                .to_warpui_icon(Fill::Solid(modal_text_sub(appearance)))
                .finish(),
        )
        .with_width(16.)
        .with_height(16.)
        .finish();

        let text_col = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_spacing(2.)
            .with_child(
                Text::new_inline(item.title.to_string(), appearance.ui_font_family(), 14.)
                    .with_color(modal_text_main(appearance))
                    .with_style(Properties::default().weight(Weight::Medium))
                    .finish(),
            )
            .with_child(
                Text::new(item.description, appearance.ui_font_family(), 13.)
                    .with_color(modal_text_sub(appearance))
                    .finish(),
            )
            .finish();

        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_spacing(10.)
            .with_child(icon_el)
            .with_child(Expanded::new(1., text_col).finish())
            .finish()
    }

    fn render_how_it_works(appearance: &Appearance) -> Box<dyn Element> {
        let mut col = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_spacing(14.);
        for item in HOW_IT_WORKS {
            col.add_child(Self::render_how_it_works_row(item, appearance));
        }
        col.finish()
    }

    fn render_offer(appearance: &Appearance) -> Box<dyn Element> {
        let mut text = Text::new(OFFER_TEXT, appearance.ui_font_family(), 14.)
            .with_color(modal_text_main(appearance))
            .with_line_height_ratio(1.4);
        if let Some(byte_start) = OFFER_TEXT.find(OFFER_EMPHASIS) {
            let char_start = OFFER_TEXT[..byte_start].chars().count();
            let char_count = OFFER_EMPHASIS.chars().count();
            text = text.with_single_highlight(
                Highlight::new()
                    .with_properties(Properties::default().weight(Weight::Bold))
                    .with_foreground_color(appearance.theme().accent().into_solid()),
                (char_start..char_start + char_count).collect(),
            );
        }

        Container::new(text.finish())
            .with_uniform_padding(12.)
            .with_background(appearance.theme().accent_overlay())
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
            .finish()
    }

    fn render_body(&self, appearance: &Appearance) -> Box<dyn Element> {
        Container::new(
            Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_spacing(16.)
                .with_child(
                    Flex::column()
                        .with_cross_axis_alignment(CrossAxisAlignment::Start)
                        .with_spacing(8.)
                        .with_child(Self::render_badge(appearance))
                        .with_child(Self::render_title(appearance))
                        .with_child(Self::render_description(appearance))
                        .finish(),
                )
                .with_child(Self::render_how_it_works(appearance))
                .with_child(Self::render_offer(appearance))
                .with_child(ChildView::new(&self.cta_button).finish())
                .finish(),
        )
        .with_horizontal_padding(32.)
        .with_vertical_padding(32.)
        .with_background(modal_background(appearance))
        .with_corner_radius(CornerRadius::with_bottom(Radius::Pixels(8.)))
        .finish()
    }
}

impl Entity for FactoriesLaunchModal {
    type Event = FactoriesLaunchModalEvent;
}

impl View for FactoriesLaunchModal {
    fn ui_name() -> &'static str {
        "FactoriesLaunchModal"
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
            .with_background(modal_background(appearance))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
            .finish(),
        )
        .with_width(MODAL_WIDTH)
        .finish();

        Container::new(
            Dismiss::new(Align::new(card).finish())
                .prevent_interaction_with_other_elements()
                .on_dismiss(|ctx, _app| {
                    ctx.dispatch_typed_action(FactoriesLaunchModalAction::Close)
                })
                .finish(),
        )
        .with_background(Fill::Solid(ColorU::new(97, 97, 97, 255)).with_opacity(50))
        .finish()
    }
}

impl TypedActionView for FactoriesLaunchModal {
    type Action = FactoriesLaunchModalAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            FactoriesLaunchModalAction::Close => {
                ctx.emit(FactoriesLaunchModalEvent::Close);
            }
            FactoriesLaunchModalAction::GetEarlyAccess => {
                ctx.emit(FactoriesLaunchModalEvent::GetEarlyAccess);
            }
        }
    }
}
