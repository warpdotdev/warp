use markdown_parser::{
    FormattedText, FormattedTextFragment, FormattedTextLine, FormattedTextStyles, Hyperlink,
};
use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use warp_core::ui::theme::{phenomenon::PhenomenonStyle, Fill};
use warpui::assets::asset_cache::AssetSource;
use warpui::elements::{
    Align, CacheOption, ChildAnchor, ChildView, ConstrainedBox, Container, CornerRadius,
    CrossAxisAlignment, Expanded, Flex, FormattedTextElement, HighlightedHyperlink, Image,
    MainAxisSize, OffsetPositioning, ParentAnchor, ParentElement, ParentOffsetBounds, Radius,
    Stack, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::keymap::FixedBinding;
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::appearance::Appearance;
use crate::ui_components::icons::Icon;
use crate::view_components::action_button::{ActionButton, ActionButtonTheme, ButtonSize};

const MODAL_WIDTH: f32 = 420.;
const HERO_HEIGHT: f32 = 92.;
const HERO_IMAGE_PATH: &str = "async/png/onboarding/openwarp_launch_banner.png";
const REPO_URL: &str = "https://github.com/warpdotdev/warp";
const CONTRIBUTING_URL: &str = "https://github.com/warpdotdev/warp/blob/master/CONTRIBUTING.md";
const OZ_URL: &str = "https://oz.warp.dev";

struct InlineLink {
    text: &'static str,
    url: &'static str,
}

struct FeatureItem {
    icon: Icon,
    title: &'static str,
    description: &'static str,
    /// If set, the first occurrence of `text` in the description is rendered as a hyperlink.
    inline_link: Option<InlineLink>,
}

const FEATURE_ITEMS: &[FeatureItem] = &[
    FeatureItem {
        icon: Icon::HeartHand,
        title: "Contribute",
        description: "Warp's client code is now open source. Get started by using the /feedback skill to open an issue, and follow the contribution guidelines here.",
        inline_link: Some(InlineLink {
            text: "here",
            url: CONTRIBUTING_URL,
        }),
    },
    FeatureItem {
        icon: Icon::Oz,
        title: "Open Automated Development",
        description: "The Warp repo is managed by an agent-first workflow powered by Oz, our cloud agent orchestration platform.",
        inline_link: Some(InlineLink {
            text: "Oz",
            url: OZ_URL,
        }),
    },
    FeatureItem {
        icon: Icon::MessageChatSquare,
        title: "Introducing 'auto (open-weights)'",
        description: "We've added a new auto model that picks the best open weight model for a task, like Kimi or MiniMax.",
        inline_link: None,
    },
];

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings([FixedBinding::new(
        "escape",
        OpenWarpLaunchModalAction::Close,
        id!(OpenWarpLaunchModal::ui_name()),
    )]);
}

#[derive(Clone, Debug)]
pub enum OpenWarpLaunchModalAction {
    Close,
    VisitRepo,
}

#[derive(Clone, Debug)]
pub enum OpenWarpLaunchModalEvent {
    Close,
}

struct CloseButtonTheme;

impl ActionButtonTheme for CloseButtonTheme {
    fn background(&self, hovered: bool, _appearance: &Appearance) -> Option<Fill> {
        if hovered {
            Some(Fill::Solid(PhenomenonStyle::modal_close_button_hover()))
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
        PhenomenonStyle::modal_close_button_text()
    }
}

struct CtaButtonTheme;

impl ActionButtonTheme for CtaButtonTheme {
    fn background(&self, hovered: bool, _appearance: &Appearance) -> Option<Fill> {
        Some(PhenomenonStyle::modal_button_background_fill(hovered))
    }

    fn text_color(
        &self,
        _hovered: bool,
        _background: Option<Fill>,
        _appearance: &Appearance,
    ) -> ColorU {
        PhenomenonStyle::modal_button_text()
    }
}

pub struct OpenWarpLaunchModal {
    close_button: ViewHandle<ActionButton>,
    cta_button: ViewHandle<ActionButton>,
}

impl OpenWarpLaunchModal {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let close_button = ctx.add_view(|_ctx| {
            ActionButton::new("", CloseButtonTheme)
                .with_icon(Icon::X)
                .with_size(ButtonSize::Small)
                .on_click(|ctx| ctx.dispatch_typed_action(OpenWarpLaunchModalAction::Close))
        });

        let cta_button = ctx.add_view(|_ctx| {
            ActionButton::new("Visit the repo", CtaButtonTheme)
                .with_full_width(true)
                .on_click(|ctx| ctx.dispatch_typed_action(OpenWarpLaunchModalAction::VisitRepo))
        });

        Self {
            close_button,
            cta_button,
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
        let text = Text::new_inline("New".to_string(), appearance.ui_font_family(), 14.)
            .with_color(PhenomenonStyle::modal_badge_text())
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
            .with_background(Fill::Solid(PhenomenonStyle::modal_badge_background()))
            .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
            .finish(),
        )
        .with_height(24.)
        .finish()
    }

    fn render_title(appearance: &Appearance) -> Box<dyn Element> {
        Text::new("Warp is now open-source", appearance.ui_font_family(), 20.)
            .with_color(PhenomenonStyle::modal_title_text())
            .with_style(Properties::default().weight(Weight::Semibold))
            .finish()
    }

    fn render_description(appearance: &Appearance) -> Box<dyn Element> {
        Text::new(
            "You, our community, can participate in building Warp using an agent-first workflow.",
            appearance.ui_font_family(),
            14.,
        )
        .with_color(PhenomenonStyle::modal_feature_description_text())
        .finish()
    }

    /// Splits a plain text string on occurrences of `/feedback`, emitting
    /// `inline_code` fragments for each match and plain fragments for the rest.
    fn split_inline_code_fragments(text: &str) -> Vec<FormattedTextFragment> {
        const CODE_TOKEN: &str = "/feedback";
        let mut fragments = Vec::new();
        let mut remaining = text;
        while let Some(pos) = remaining.find(CODE_TOKEN) {
            if pos > 0 {
                fragments.push(FormattedTextFragment::plain_text(&remaining[..pos]));
            }
            fragments.push(FormattedTextFragment {
                text: CODE_TOKEN.into(),
                styles: FormattedTextStyles {
                    inline_code: true,
                    ..Default::default()
                },
            });
            remaining = &remaining[pos + CODE_TOKEN.len()..];
        }
        if !remaining.is_empty() {
            fragments.push(FormattedTextFragment::plain_text(remaining));
        }
        fragments
    }

    fn render_feature_description(item: &FeatureItem, appearance: &Appearance) -> Box<dyn Element> {
        let Some(link) = &item.inline_link else {
            return Text::new(item.description, appearance.ui_font_family(), 14.)
                .with_color(PhenomenonStyle::modal_feature_description_text())
                .finish();
        };

        // Build a formatted description with an inline hyperlink and inline code.
        let (before, after) = item
            .description
            .split_once(link.text)
            .unwrap_or((item.description, ""));

        let link_fragment = FormattedTextFragment {
            text: link.text.into(),
            styles: FormattedTextStyles {
                underline: true,
                hyperlink: Some(Hyperlink::Url(link.url.into())),
                ..Default::default()
            },
        };

        let mut fragments = Self::split_inline_code_fragments(before);
        fragments.push(link_fragment);
        if !after.is_empty() {
            fragments.extend(Self::split_inline_code_fragments(after));
        }

        let formatted = FormattedText::new([FormattedTextLine::Line(fragments)]);

        FormattedTextElement::new(
            formatted,
            14.,
            appearance.ui_font_family(),
            appearance.monospace_font_family(),
            PhenomenonStyle::modal_feature_description_text(),
            HighlightedHyperlink::default(),
        )
        .with_line_height_ratio(1.2)
        // Render the inline link in the same color as the description text so it
        // blends in; the underline (applied via FormattedTextStyles) still signals it's a link.
        .with_hyperlink_font_color(PhenomenonStyle::modal_feature_description_text())
        .register_default_click_handlers(|link, _ctx, app| {
            app.open_url(&link.url);
        })
        .finish()
    }

    fn render_feature_row(item: &FeatureItem, appearance: &Appearance) -> Box<dyn Element> {
        let icon_el = ConstrainedBox::new(
            item.icon
                .to_warpui_icon(Fill::Solid(
                    PhenomenonStyle::modal_feature_description_text(),
                ))
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
                    .with_color(PhenomenonStyle::modal_feature_title_text())
                    .finish(),
            )
            .with_child(Self::render_feature_description(item, appearance))
            .finish();

        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_spacing(10.)
            .with_child(icon_el)
            .with_child(Expanded::new(1., text_col).finish())
            .finish()
    }

    fn render_body(&self, appearance: &Appearance) -> Box<dyn Element> {
        let mut features_col = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_spacing(12.);
        for item in FEATURE_ITEMS {
            features_col.add_child(Self::render_feature_row(item, appearance));
        }

        let cta = ChildView::new(&self.cta_button).finish();

        Container::new(
            Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_child(
                    Flex::column()
                        .with_cross_axis_alignment(CrossAxisAlignment::Start)
                        .with_spacing(8.)
                        .with_child(Self::render_badge(appearance))
                        .with_child(Self::render_title(appearance))
                        .with_child(Self::render_description(appearance))
                        .finish(),
                )
                .with_child(
                    Container::new(features_col.finish())
                        .with_margin_top(16.)
                        .finish(),
                )
                .with_child(Container::new(cta).with_margin_top(32.).finish())
                .finish(),
        )
        .with_horizontal_padding(32.)
        .with_vertical_padding(32.)
        .with_background(Fill::Solid(PhenomenonStyle::modal_background()))
        .with_corner_radius(CornerRadius::with_bottom(Radius::Pixels(8.)))
        .finish()
    }
}

impl Entity for OpenWarpLaunchModal {
    type Event = OpenWarpLaunchModalEvent;
}

impl View for OpenWarpLaunchModal {
    fn ui_name() -> &'static str {
        "OpenWarpLaunchModal"
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
            .with_background(Fill::Solid(PhenomenonStyle::modal_background()))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
            .finish(),
        )
        .with_width(MODAL_WIDTH)
        .finish();

        Container::new(Align::new(card).finish())
            .with_background_color(ColorU::new(18, 18, 18, 128))
            .finish()
    }
}

impl TypedActionView for OpenWarpLaunchModal {
    type Action = OpenWarpLaunchModalAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            OpenWarpLaunchModalAction::Close => {
                ctx.emit(OpenWarpLaunchModalEvent::Close);
            }
            OpenWarpLaunchModalAction::VisitRepo => {
                ctx.open_url(REPO_URL);
                ctx.emit(OpenWarpLaunchModalEvent::Close);
            }
        }
    }
}
