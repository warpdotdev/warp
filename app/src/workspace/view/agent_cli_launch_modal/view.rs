use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use warp_core::ui::theme::Fill;
use warpui::assets::asset_cache::AssetSource;
use warpui::elements::{
    Align, CacheOption, ChildAnchor, ChildView, Clipped, ConstrainedBox, Container, CornerRadius,
    CrossAxisAlignment, Empty, Expanded, Flex, Highlight, Image, MainAxisSize, OffsetPositioning,
    ParentAnchor, ParentElement, ParentOffsetBounds, Radius, Stack, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::keymap::FixedBinding;
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::appearance::Appearance;
use crate::ui_components::icons::Icon;
use crate::view_components::action_button::{ActionButton, ActionButtonTheme, ButtonSize};

const MODAL_WIDTH: f32 = 720.;
const MODAL_HEIGHT: f32 = 440.;
/// The card is split into two equal columns: copy on the left, hero on the right.
const COLUMN_WIDTH: f32 = MODAL_WIDTH / 2.;
const COLUMN_HORIZONTAL_PADDING: f32 = 32.;
const COLUMN_VERTICAL_PADDING: f32 = 40.;
const CONTENT_WIDTH: f32 = COLUMN_WIDTH - COLUMN_HORIZONTAL_PADDING * 2.;
const CARD_RADIUS: f32 = 8.;
const FEATURE_FONT_SIZE: f32 = 12.;
const HERO_IMAGE_PATH: &str = "async/png/onboarding/agent_cli_launch_banner.png";
const GET_STARTED_URL: &str = "https://docs.warp.dev/cli/quickstart/";

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

fn modal_overlay_1(appearance: &Appearance) -> Fill {
    appearance.theme().surface_overlay_1()
}

fn modal_terminal_magenta(appearance: &Appearance) -> ColorU {
    appearance.theme().terminal_colors().normal.magenta.into()
}

fn modal_terminal_magenta_overlay_1(appearance: &Appearance) -> ColorU {
    let magenta = appearance.theme().terminal_colors().normal.magenta;
    appearance.theme().ansi_overlay_1(magenta)
}

struct FeatureItem {
    icon: Icon,
    label: &'static str,
    title: &'static str,
    description: &'static str,
}

const FEATURE_ITEMS: &[FeatureItem] = &[
    FeatureItem {
        icon: Icon::LayoutAlt01,
        label: "What's new:",
        title: "Use the Warp Agent anywhere",
        description: "Warp's state of the art agent is now available in any terminal through the CLI.",
    },
    FeatureItem {
        icon: Icon::Inbox,
        label: "What's special:",
        title: "Built-in terminal multiplexer",
        description: "Each Warp Agent creates its own PTY, enabling better behavior for REPLS, ssh, directory switching, and more.",
    },
];

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings([FixedBinding::new(
        "escape",
        AgentCliLaunchModalAction::Close,
        id!(AgentCliLaunchModal::ui_name()),
    )]);
}

#[derive(Clone, Debug)]
pub enum AgentCliLaunchModalAction {
    Close,
    GetStarted,
}

#[derive(Clone, Debug)]
pub enum AgentCliLaunchModalEvent {
    Close,
}

struct CloseButtonTheme;

impl ActionButtonTheme for CloseButtonTheme {
    fn background(&self, hovered: bool, appearance: &Appearance) -> Option<Fill> {
        if hovered {
            Some(modal_overlay_1(appearance))
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

struct CtaButtonTheme;

impl ActionButtonTheme for CtaButtonTheme {
    fn background(&self, _hovered: bool, appearance: &Appearance) -> Option<Fill> {
        Some(Fill::Solid(appearance.theme().foreground().into_solid()))
    }

    fn text_color(
        &self,
        _hovered: bool,
        _background: Option<Fill>,
        appearance: &Appearance,
    ) -> ColorU {
        appearance.theme().background().into_solid()
    }
}

pub struct AgentCliLaunchModal {
    close_button: ViewHandle<ActionButton>,
    get_started_button: ViewHandle<ActionButton>,
}

impl AgentCliLaunchModal {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let close_button = ctx.add_view(|_ctx| {
            ActionButton::new("", CloseButtonTheme)
                .with_icon(Icon::X)
                .with_size(ButtonSize::Small)
                .on_click(|ctx| ctx.dispatch_typed_action(AgentCliLaunchModalAction::Close))
        });

        let get_started_button = ctx.add_view(|_ctx| {
            ActionButton::new("Get started", CtaButtonTheme)
                .with_full_width(true)
                .on_click(|ctx| ctx.dispatch_typed_action(AgentCliLaunchModalAction::GetStarted))
        });

        Self {
            close_button,
            get_started_button,
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
                .with_corner_radius(CornerRadius::with_right(Radius::Pixels(CARD_RADIUS)))
                .cover()
                .finish(),
            )
            .with_width(COLUMN_WIDTH)
            .with_height(MODAL_HEIGHT)
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
            "Introducing the Warp Agent CLI: A coding agent that does what others can't",
            appearance.ui_font_family(),
            20.,
        )
        .with_color(modal_text_main(appearance))
        .with_style(Properties::default().weight(Weight::Semibold))
        .finish()
    }

    fn render_feature_row(item: &FeatureItem, appearance: &Appearance) -> Box<dyn Element> {
        let icon_el = ConstrainedBox::new(
            item.icon
                .to_warpui_icon(Fill::Solid(modal_text_sub(appearance)))
                .finish(),
        )
        .with_width(16.)
        .with_height(16.)
        .finish();

        let title = format!("{} {}", item.label, item.title);
        let label_char_count = item.label.chars().count();
        let title_row = Text::new(title, appearance.ui_font_family(), FEATURE_FONT_SIZE)
            .with_color(modal_text_main(appearance))
            .with_single_highlight(
                Highlight::new().with_properties(Properties::default().weight(Weight::Bold)),
                (0..label_char_count).collect(),
            )
            .finish();

        let text_col = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_spacing(2.)
            .with_child(title_row)
            .with_child(
                Text::new(
                    item.description,
                    appearance.ui_font_family(),
                    FEATURE_FONT_SIZE,
                )
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

    fn render_content_column(&self, appearance: &Appearance) -> Box<dyn Element> {
        let mut features_col = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_spacing(16.);
        for item in FEATURE_ITEMS {
            features_col.add_child(Self::render_feature_row(item, appearance));
        }

        let column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(Self::render_badge(appearance))
            .with_child(
                Container::new(Self::render_title(appearance))
                    .with_margin_top(8.)
                    .finish(),
            )
            .with_child(
                Container::new(features_col.finish())
                    .with_margin_top(16.)
                    .finish(),
            )
            .with_child(Expanded::new(1., Empty::new().finish()).finish())
            .with_child(
                ConstrainedBox::new(ChildView::new(&self.get_started_button).finish())
                    .with_width(CONTENT_WIDTH)
                    .finish(),
            )
            .finish();

        ConstrainedBox::new(
            Container::new(column)
                .with_vertical_padding(COLUMN_VERTICAL_PADDING)
                .with_horizontal_padding(COLUMN_HORIZONTAL_PADDING)
                .finish(),
        )
        .with_width(COLUMN_WIDTH)
        .with_height(MODAL_HEIGHT)
        .finish()
    }
}

impl Entity for AgentCliLaunchModal {
    type Event = AgentCliLaunchModalEvent;
}

impl View for AgentCliLaunchModal {
    fn ui_name() -> &'static str {
        "AgentCliLaunchModal"
    }

    fn on_focus(&mut self, _focus_ctx: &warpui::FocusContext, ctx: &mut ViewContext<Self>) {
        ctx.focus_self();
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        let card = ConstrainedBox::new(
            Container::new(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Min)
                    .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .with_child(self.render_content_column(appearance))
                    .with_child(self.render_hero())
                    .finish(),
            )
            .with_background(modal_background(appearance))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(CARD_RADIUS)))
            .finish(),
        )
        .with_width(MODAL_WIDTH)
        .with_height(MODAL_HEIGHT)
        .finish();

        Container::new(Align::new(card).finish())
            .with_background(Fill::Solid(ColorU::new(97, 97, 97, 255)).with_opacity(50))
            .finish()
    }
}

impl TypedActionView for AgentCliLaunchModal {
    type Action = AgentCliLaunchModalAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            AgentCliLaunchModalAction::Close => {
                ctx.emit(AgentCliLaunchModalEvent::Close);
            }
            AgentCliLaunchModalAction::GetStarted => {
                ctx.open_url(GET_STARTED_URL);
                ctx.emit(AgentCliLaunchModalEvent::Close);
            }
        }
    }
}
