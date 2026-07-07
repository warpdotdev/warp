use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use warp_core::ui::theme::color::internal_colors;
use warpui::elements::{
    Border, ChildAnchor, ConstrainedBox, Container, CornerRadius, Expanded, Flex, Hoverable,
    Icon as WarpUiIcon, MainAxisAlignment, MainAxisSize, MouseStateHandle, OffsetPositioning,
    ParentAnchor, ParentElement as _, ParentOffsetBounds, Percentage, Radius, Rect, Stack, Text,
};
use warpui::prelude::{Align, CrossAxisAlignment};
use warpui::text_layout::ClipConfig;
use warpui::ui_components::components::UiComponent;
use warpui::{AppContext, Element, SingletonEntity as _};

use crate::ai::llms::LLMSpec;
use crate::appearance::Appearance;
use crate::terminal::input::inline_menu::styles as inline_styles;

const CORNER_RADIUS: f32 = 4.0;
const ROW_SPACING: f32 = 12.0;

pub const MODEL_SPECS_TITLE: &str = "Model Specs";
pub const MODEL_SPECS_DESCRIPTION: &str = "Warp's benchmarks for how well a model performs in our harness, the rate at which it consumes credits, and task speed.";

pub const REASONING_LEVEL_TITLE: &str = "Reasoning level";
pub const REASONING_LEVEL_DESCRIPTION: &str = "Increased reasoning levels consume more credits and have higher latency, but higher performance for complicated tasks.";

pub const CUSTOM_MODEL_ROUTER_TITLE: &str = "Custom Model Router";
pub const CUSTOM_MODEL_ROUTER_DESCRIPTION: &str = "Routes each request to a concrete model based on your routing rules, rather than using a single fixed model.";

pub enum CostRow {
    Bar {
        value: Option<f32>,
    },
    BilledToProvider {
        label: &'static str,
        tooltip: Option<CostRowTooltip>,
        manage_button: Box<dyn Element>,
    },
}
pub struct CostRowTooltip {
    pub text: &'static str,
    pub mouse_state: MouseStateHandle,
}

pub struct ModelSpecScoresLayout {
    pub bg_bar_color: ColorU,
}

pub fn render_model_spec_scores(
    spec: Option<&LLMSpec>,
    cost_row: CostRow,
    layout: ModelSpecScoresLayout,
    app: &AppContext,
) -> Box<dyn Element> {
    let mut rows = vec![render_score_row(
        "Intelligence",
        ScoreRowKind::Bar {
            value: spec.as_ref().map(|spec| spec.quality),
        },
        None,
        layout.bg_bar_color,
        app,
    )];

    rows.push(render_score_row(
        "Speed",
        ScoreRowKind::Bar {
            value: spec.as_ref().map(|spec| spec.speed),
        },
        None,
        layout.bg_bar_color,
        app,
    ));

    match cost_row {
        CostRow::Bar { value } => {
            rows.push(render_score_row(
                "Cost",
                ScoreRowKind::Bar { value },
                None,
                layout.bg_bar_color,
                app,
            ));
        }
        CostRow::BilledToProvider {
            label,
            tooltip,
            manage_button,
        } => {
            rows.push(render_score_row(
                "Cost",
                ScoreRowKind::BilledToProvider {
                    label,
                    manage_button,
                },
                tooltip,
                layout.bg_bar_color,
                app,
            ));
        }
    }

    Flex::column()
        .with_spacing(ROW_SPACING)
        .with_children(rows)
        .finish()
}

enum ScoreRowKind {
    Bar {
        value: Option<f32>,
    },
    BilledToProvider {
        label: &'static str,
        manage_button: Box<dyn Element>,
    },
}

fn render_score_row(
    name: &str,
    kind: ScoreRowKind,
    label_tooltip: Option<CostRowTooltip>,
    bg_bar_color: ColorU,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();

    // Approximate the required width for the longest label ("Intelligence"), and use this as a
    // consistent width for the labels so the labels and bars are vertically aligned.
    //
    // 8 ems is enough space for Intelligence with some right margin to spare.
    let label_width = app.font_cache().em_width(
        appearance.ui_font_family(),
        appearance.monospace_font_size(),
    ) * 8.;
    let label = ConstrainedBox::new(render_row_label(name, label_tooltip, appearance, app))
        .with_width(label_width)
        .finish();

    let bar_height = app.font_cache().line_height(
        appearance.monospace_font_size(),
        appearance.line_height_ratio(),
    );

    let row_content: Box<dyn Element> = match kind {
        ScoreRowKind::Bar { value: Some(value) } => {
            let background_bar = Rect::new()
                .with_background_color(bg_bar_color)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(CORNER_RADIUS)))
                .finish();

            let filled_bar = Rect::new()
                .with_background_color(internal_colors::neutral_6(theme))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(CORNER_RADIUS)))
                .finish();

            Expanded::new(
                1.,
                ConstrainedBox::new(
                    Stack::new()
                        .with_child(background_bar)
                        .with_child(Percentage::width(value, filled_bar).finish())
                        .finish(),
                )
                .with_height(bar_height)
                .finish(),
            )
            .finish()
        }
        ScoreRowKind::Bar { value: None } => {
            let background_bar = Rect::new()
                .with_background_color(bg_bar_color)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(CORNER_RADIUS)))
                .finish();

            Expanded::new(
                1.,
                ConstrainedBox::new(
                    Stack::new()
                        .with_child(background_bar)
                        .with_child(
                            Align::new(
                                Text::new(
                                    "?",
                                    appearance.ui_font_family(),
                                    inline_styles::font_size(appearance),
                                )
                                .with_color(
                                    appearance
                                        .theme()
                                        .disabled_text_color(bg_bar_color.into())
                                        .into_solid(),
                                )
                                .finish(),
                            )
                            .finish(),
                        )
                        .finish(),
                )
                .with_height(bar_height)
                .finish(),
            )
            .finish()
        }
        ScoreRowKind::BilledToProvider {
            label,
            manage_button,
        } => Expanded::new(
            1.,
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(render_provider_label(label, appearance))
                .with_child(manage_button)
                .finish(),
        )
        .finish(),
    };

    Flex::row()
        .with_main_axis_size(MainAxisSize::Max)
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(label)
        .with_child(Expanded::new(1., row_content).finish())
        .finish()
}

fn render_row_label(
    label: &str,
    tooltip: Option<CostRowTooltip>,
    appearance: &Appearance,
    app: &AppContext,
) -> Box<dyn Element> {
    let label = Text::new(
        label.to_string(),
        appearance.ui_font_family(),
        appearance.monospace_font_size(),
    )
    .with_color(
        inline_styles::primary_text_color(
            appearance.theme(),
            inline_styles::menu_background_color(app).into(),
        )
        .into_solid(),
    )
    .finish();

    let Some(tooltip) = tooltip else {
        return label;
    };

    Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(label)
        .with_child(
            Container::new(render_info_tooltip(tooltip, appearance))
                .with_margin_left(4.)
                .finish(),
        )
        .finish()
}

fn render_provider_label(label: &'static str, appearance: &Appearance) -> Box<dyn Element> {
    Container::new(
        Text::new(label.to_string(), appearance.ui_font_family(), 14.)
            .with_color(appearance.theme().disabled_ui_text_color().into())
            .finish(),
    )
    .finish()
}

fn render_info_tooltip(tooltip: CostRowTooltip, appearance: &Appearance) -> Box<dyn Element> {
    let icon_color = appearance.theme().disabled_ui_text_color();
    let ui_builder = appearance.ui_builder();
    let tooltip_text = tooltip.text.to_string();
    Hoverable::new(tooltip.mouse_state, move |state| {
        let info_icon = Container::new(
            ConstrainedBox::new(WarpUiIcon::new("bundled/svg/info.svg", icon_color).finish())
                .with_width(13.)
                .with_height(13.)
                .finish(),
        )
        .finish();

        let mut stack = Stack::new().with_child(info_icon);
        if state.is_hovered() {
            let tooltip = ui_builder.tool_tip(tooltip_text.clone()).build();
            stack.add_positioned_child(
                tooltip.finish(),
                OffsetPositioning::offset_from_parent(
                    vec2f(0., -3.),
                    ParentOffsetBounds::Unbounded,
                    ParentAnchor::TopMiddle,
                    ChildAnchor::BottomMiddle,
                ),
            );
        }
        stack.finish()
    })
    .finish()
}

pub fn render_model_spec_header(
    title: &str,
    description: &str,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();

    let title = Text::new(
        title.to_string(),
        appearance.ui_font_family(),
        appearance.monospace_font_size(),
    )
    .with_color(
        inline_styles::primary_text_color(theme, inline_styles::menu_background_color(app).into())
            .into_solid(),
    )
    .with_clip(ClipConfig::ellipsis())
    .finish();

    let description = Text::new(
        description.to_string(),
        appearance.ui_font_family(),
        inline_styles::font_size(appearance),
    )
    .with_color(theme.disabled_ui_text_color().into())
    .finish();

    Container::new(
        Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(title)
            .with_child(Container::new(description).with_margin_top(4.).finish())
            .finish(),
    )
    .with_padding_bottom(12.)
    .with_border(Border::bottom(1.).with_border_fill(theme.outline()))
    .finish()
}
