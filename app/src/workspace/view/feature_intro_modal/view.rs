use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use warp_core::features::FeatureFlag;
use warp_core::ui::theme::Fill;
use warpui::assets::asset_cache::AssetSource;
use warpui::elements::{
    CacheOption, ChildAnchor, ChildView, Clipped, ConstrainedBox, Container, CornerRadius,
    CrossAxisAlignment, Empty, Flex, Image, MainAxisSize, OffsetPositioning, ParentAnchor,
    ParentElement, ParentOffsetBounds, Radius, Stack, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::keymap::FixedBinding;
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::appearance::Appearance;
use crate::ui_components::icons::Icon;
use crate::view_components::action_button::{ActionButton, ButtonSize, NakedTheme, PrimaryTheme};

const MODAL_WIDTH: f32 = 420.;
const HERO_HEIGHT: f32 = 92.;

/// Identifies a single feature announced through the reusable feature-intro
/// popover. The string form ([`FeatureIntroId::as_key`]) is the persisted
/// "seen" key, so it must remain stable across releases.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeatureIntroId {
    CustomModelRouter,
}

impl FeatureIntroId {
    /// The stable key used to record that this feature intro has been seen.
    pub fn as_key(self) -> &'static str {
        match self {
            FeatureIntroId::CustomModelRouter => "custom_model_router",
        }
    }
}

/// A data-driven description of a single feature-intro popover. New feature
/// announcements are added by appending an entry to [`FEATURE_INTROS`]; no new
/// view, model, settings, or workspace wiring is required.
pub struct FeatureIntro {
    /// Stable identifier; also the persisted "seen" key.
    pub id: FeatureIntroId,
    /// Per-feature rollout gate. The popover only triggers when this is enabled.
    pub flag: FeatureFlag,
    /// Bundled hero image shown at the top of the card.
    pub hero_image_path: &'static str,
    /// Optional pill rendered above the title (e.g. "New").
    pub badge: Option<&'static str>,
    pub title: &'static str,
    pub description: &'static str,
    /// Label for the primary call-to-action button.
    pub cta_label: &'static str,
    /// URL opened when the user clicks the call-to-action. `None` simply
    /// dismisses the popover.
    pub cta_url: Option<&'static str>,
}

/// The registry of feature-intro popovers, in priority order. On startup the
/// first entry whose [`FeatureIntro::flag`] is enabled and whose id has not yet
/// been seen is shown.
pub const FEATURE_INTROS: &[FeatureIntro] = &[FeatureIntro {
    id: FeatureIntroId::CustomModelRouter,
    flag: FeatureFlag::CustomModelRouterIntro,
    hero_image_path: "async/png/onboarding/custom_model_router_intro_banner.png",
    badge: Some("New"),
    title: "Build a custom model router for the Warp Agent",
    description: "Custom routers can be complexity-based, where tasks are routed based on how difficult they are, or rule-based, where they are routed based on a set of natural language prompts.",
    cta_label: "Get started",
    cta_url: Some("https://docs.warp.dev/agents/using-agents/custom-model-routers"),
}];

/// Looks up a feature-intro descriptor by its id.
pub fn feature_intro_by_id(id: FeatureIntroId) -> Option<&'static FeatureIntro> {
    FEATURE_INTROS.iter().find(|intro| intro.id == id)
}

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
        FeatureIntroModalAction::Close,
        id!(FeatureIntroModal::ui_name()),
    )]);
}

#[derive(Clone, Debug)]
pub enum FeatureIntroModalAction {
    Close,
    GetStarted,
}

#[derive(Clone, Debug)]
pub enum FeatureIntroModalEvent {
    /// The user dismissed the popover (close button or escape).
    Close(FeatureIntroId),
    /// The user clicked the primary call-to-action.
    GetStarted(FeatureIntroId),
}

/// A single, reusable popover for introducing new features. The popover is a
/// non-blocking bottom-right overlay (no scrim, does not grab focus); the
/// content is driven entirely by the [`FeatureIntro`] descriptor set via
/// [`FeatureIntroModal::set_feature`].
pub struct FeatureIntroModal {
    close_button: ViewHandle<ActionButton>,
    cta_button: ViewHandle<ActionButton>,
    /// The feature currently being shown, if any.
    current: Option<&'static FeatureIntro>,
}

impl FeatureIntroModal {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let close_button = ctx.add_view(|_ctx| {
            ActionButton::new("", NakedTheme)
                .with_icon(Icon::X)
                .with_size(ButtonSize::Small)
                .on_click(|ctx| ctx.dispatch_typed_action(FeatureIntroModalAction::Close))
        });

        let cta_button = ctx.add_view(|_ctx| {
            ActionButton::new("Get started", PrimaryTheme)
                .with_full_width(true)
                .on_click(|ctx| ctx.dispatch_typed_action(FeatureIntroModalAction::GetStarted))
        });

        Self {
            close_button,
            cta_button,
            current: None,
        }
    }

    /// Sets the feature descriptor that the popover should render. Passing
    /// `None` leaves the popover empty (the workspace simply stops rendering it).
    pub fn set_feature(
        &mut self,
        intro: Option<&'static FeatureIntro>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.current = intro;
        if let Some(intro) = intro {
            self.cta_button.update(ctx, |button, ctx| {
                button.set_label(intro.cta_label, ctx);
            });
        }
        ctx.notify();
    }

    fn render_hero(&self, intro: &FeatureIntro) -> Box<dyn Element> {
        let hero = Clipped::new(
            ConstrainedBox::new(
                Image::new(
                    AssetSource::Bundled {
                        path: intro.hero_image_path,
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

    fn render_badge(label: &'static str, appearance: &Appearance) -> Box<dyn Element> {
        let text_color = modal_terminal_magenta(appearance);
        let background_color = modal_terminal_magenta_overlay_1(appearance);
        let text = Text::new_inline(label.to_string(), appearance.ui_font_family(), 14.)
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

    fn render_title(title: &'static str, appearance: &Appearance) -> Box<dyn Element> {
        Text::new(title, appearance.ui_font_family(), 20.)
            .with_color(modal_text_main(appearance))
            .with_style(Properties::default().weight(Weight::Semibold))
            .finish()
    }

    fn render_description(description: &'static str, appearance: &Appearance) -> Box<dyn Element> {
        Text::new(description, appearance.ui_font_family(), 14.)
            .with_color(modal_text_sub(appearance))
            .finish()
    }

    fn render_body(&self, intro: &FeatureIntro, appearance: &Appearance) -> Box<dyn Element> {
        let mut header = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_spacing(8.);
        if let Some(badge) = intro.badge {
            header.add_child(Self::render_badge(badge, appearance));
        }
        header.add_child(Self::render_title(intro.title, appearance));
        header.add_child(Self::render_description(intro.description, appearance));

        Container::new(
            Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_child(header.finish())
                .with_child(
                    Container::new(ChildView::new(&self.cta_button).finish())
                        .with_margin_top(24.)
                        .finish(),
                )
                .finish(),
        )
        .with_horizontal_padding(20.)
        .with_vertical_padding(20.)
        .with_background(modal_background(appearance))
        .with_corner_radius(CornerRadius::with_bottom(Radius::Pixels(8.)))
        .finish()
    }
}

impl Entity for FeatureIntroModal {
    type Event = FeatureIntroModalEvent;
}

impl View for FeatureIntroModal {
    fn ui_name() -> &'static str {
        "FeatureIntroModal"
    }

    // NOTE: intentionally no `on_focus` override. The popover is non-blocking and
    // must not steal focus from the terminal/input; its buttons work on click.

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let Some(intro) = self.current else {
            return Empty::new().finish();
        };

        ConstrainedBox::new(
            Container::new(
                Flex::column()
                    .with_main_axis_size(MainAxisSize::Min)
                    .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .with_child(self.render_hero(intro))
                    .with_child(self.render_body(intro, appearance))
                    .finish(),
            )
            .with_background(modal_background(appearance))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
            .with_foreground_border(appearance.theme().outline().into_solid())
            .finish(),
        )
        .with_width(MODAL_WIDTH)
        .finish()
    }
}

impl TypedActionView for FeatureIntroModal {
    type Action = FeatureIntroModalAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        let Some(intro) = self.current else {
            return;
        };
        match action {
            FeatureIntroModalAction::Close => {
                ctx.emit(FeatureIntroModalEvent::Close(intro.id));
            }
            FeatureIntroModalAction::GetStarted => {
                ctx.emit(FeatureIntroModalEvent::GetStarted(intro.id));
            }
        }
    }
}
