use pathfinder_geometry::vector::vec2f;
use warp_core::ui::theme::phenomenon::PhenomenonStyle;
use warp_core::ui::theme::Fill;
use warpui::assets::asset_cache::AssetSource;
use warpui::elements::{
    CacheOption, ChildAnchor, ChildView, ConstrainedBox, Container, CornerRadius,
    CrossAxisAlignment, Expanded, Flex, Image, MainAxisSize, OffsetPositioning, ParentAnchor,
    ParentElement, ParentOffsetBounds, Radius, Stack, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::{Element, ViewHandle};

use crate::appearance::Appearance;
use crate::ui_components::icons::Icon;
use crate::view_components::action_button::ActionButton;

const BANNER_WIDTH: f32 = 420.;
const HERO_HEIGHT: f32 = 92.;
const HERO_IMAGE_PATH: &str = "async/png/onboarding/hoa_welcome_banner.png";

struct FeatureItem {
    icon: Icon,
    title_key: &'static str,
    description_key: &'static str,
}

const FEATURE_ITEMS: &[FeatureItem] = &[
    FeatureItem {
        icon: Icon::LayoutAlt01,
        title_key: "workspace.hoa.feature.vertical_tabs.title",
        description_key: "workspace.hoa.feature.vertical_tabs.description",
    },
    FeatureItem {
        icon: Icon::Sliders,
        title_key: "workspace.hoa.feature.tab_configs.title",
        description_key: "workspace.hoa.feature.tab_configs.description",
    },
    FeatureItem {
        icon: Icon::Inbox,
        title_key: "workspace.hoa.feature.agent_inbox.title",
        description_key: "workspace.hoa.feature.agent_inbox.description",
    },
    FeatureItem {
        icon: Icon::MessageCheckSquare,
        title_key: "workspace.hoa.feature.native_code_review.title",
        description_key: "workspace.hoa.feature.native_code_review.description",
    },
];

pub fn render_welcome_banner(
    close_button: &ViewHandle<ActionButton>,
    cta_button: &ViewHandle<ActionButton>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    // Hero image with close button overlay
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
    .with_width(BANNER_WIDTH)
    .with_height(HERO_HEIGHT)
    .finish();

    let close_el = Container::new(ChildView::new(close_button).finish()).finish();

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

    // "New" badge
    let text = Text::new_inline(i18n::t("common.new"), appearance.ui_font_family(), 14.)
        .with_color(PhenomenonStyle::modal_badge_text())
        .finish();
    let badge = ConstrainedBox::new(
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
    .finish();

    // Title
    let title = Text::new(
        i18n::t("workspace.hoa.welcome_title"),
        appearance.ui_font_family(),
        20.,
    )
    .with_color(PhenomenonStyle::modal_title_text())
    .with_style(Properties::default().weight(Weight::Semibold))
    .finish();

    // Feature list
    let mut features_col = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Start)
        .with_spacing(12.);

    for item in FEATURE_ITEMS {
        let icon_el = ConstrainedBox::new(
            item.icon
                .to_warpui_icon(Fill::Solid(PhenomenonStyle::modal_feature_title_text()))
                .finish(),
        )
        .with_width(16.)
        .with_height(16.)
        .finish();

        let text_col = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_spacing(2.)
            .with_child(
                Text::new_inline(i18n::t(item.title_key), appearance.ui_font_family(), 14.)
                    .with_color(PhenomenonStyle::modal_feature_title_text())
                    .finish(),
            )
            .with_child(
                Text::new(
                    i18n::t(item.description_key),
                    appearance.ui_font_family(),
                    14.,
                )
                .with_color(PhenomenonStyle::modal_feature_description_text())
                .finish(),
            )
            .finish();

        let row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_spacing(10.)
            .with_child(icon_el)
            .with_child(Expanded::new(1., text_col).finish())
            .finish();

        features_col.add_child(row);
    }

    // CTA button
    let cta = ChildView::new(cta_button).finish();

    // Body content
    let body = Container::new(
        Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(
                Flex::column()
                    .with_cross_axis_alignment(CrossAxisAlignment::Start)
                    .with_spacing(8.)
                    .with_child(badge)
                    .with_child(title)
                    .finish(),
            )
            .with_child(
                Container::new(features_col.finish())
                    .with_margin_top(12.)
                    .finish(),
            )
            .with_child(Container::new(cta).with_margin_top(32.).finish())
            .finish(),
    )
    .with_horizontal_padding(32.)
    .with_vertical_padding(32.)
    .with_background(Fill::Solid(PhenomenonStyle::modal_background()))
    .with_corner_radius(CornerRadius::with_bottom(Radius::Pixels(8.)))
    .finish();

    // Full banner
    ConstrainedBox::new(
        Container::new(
            Flex::column()
                .with_main_axis_size(MainAxisSize::Min)
                .with_child(hero_stack.finish())
                .with_child(body)
                .finish(),
        )
        .with_background(Fill::Solid(PhenomenonStyle::modal_background()))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
        .finish(),
    )
    .with_width(BANNER_WIDTH)
    .finish()
}
