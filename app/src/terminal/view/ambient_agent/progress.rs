//! Progress steps UI component for displaying multi-step loading/setup flows.

use std::borrow::Cow;
use std::time::Duration;

use pathfinder_geometry::vector::vec2f;
use warp_core::ui::Icon;
use warp_core::ui::appearance::Appearance;
use warpui::elements::{
    Border, ChildAnchor, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Flex,
    MainAxisAlignment, MainAxisSize, OffsetPositioning, ParentElement, PositionedElementAnchor,
    PositionedElementOffsetBounds, Radius, Rect, SavePosition, Stack, Text,
};
use warpui::{AppContext, Element, WindowId};

/// The state of a progress step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressStepState {
    /// Step has been completed successfully.
    Completed,
    /// Step is currently in progress.
    InProgress,
    /// Step has not yet started.
    Pending,
}

/// Configuration for a single progress step.
#[derive(Debug, Clone)]
pub struct ProgressStep {
    /// The display name of the step.
    pub name: Cow<'static, str>,
    /// The current state of the step.
    pub state: ProgressStepState,
    /// Optional subtext to display below the step name.
    pub subtext: Option<Cow<'static, str>>,
    /// Optional duration to display (typically shown after subtext).
    pub duration: Option<Duration>,
}

/// Props for the progress component.
#[derive(Debug, Clone, Default)]
pub struct ProgressProps {
    /// The list of steps to display.
    pub steps: Vec<ProgressStep>,
    /// Prefix for saved position IDs to support multiple panes with progress UIs.
    pub save_position_prefix: Cow<'static, str>,
}

// UI constants
const INDICATOR_SIZE: f32 = 16.;
const INDICATOR_BORDER_WIDTH: f32 = 2.;
const ICON_SIZE: f32 = 10.;
const TEXT_LEFT_MARGIN: f32 = 12.;
const CONNECTING_LINE_WIDTH: f32 = 1.;
const CONNECTING_LINE_GAP: f32 = 2.;

/// Maximum height of each individual step.
const MAX_STEP_HEIGHT: f32 = 72.;

/// Renders the progress steps UI.
pub fn render_progress(
    props: ProgressProps,
    appearance: &Appearance,
    window_id: WindowId,
    app: &AppContext,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let step_count = props.steps.len();

    let mut steps_column = Flex::column()
        .with_main_axis_alignment(MainAxisAlignment::Center)
        .with_main_axis_size(MainAxisSize::Min)
        .with_cross_axis_alignment(CrossAxisAlignment::Start);

    let save_position_prefix = props.save_position_prefix.clone();
    for (index, step) in props.steps.into_iter().enumerate() {
        steps_column.add_child(render_step(step, index, &save_position_prefix, appearance));
    }

    // Build connecting lines between adjacent indicators. Stack can't position/constrain an element
    // based on *two* other elements, so we emulate this using cached positions from the last frame.
    let mut stack = Stack::new().with_child(steps_column.finish());

    for i in 0..(step_count.saturating_sub(1)) {
        let from_id = indicator_position_id(&save_position_prefix, i);
        let to_id = indicator_position_id(&save_position_prefix, i + 1);

        let from_pos = app.element_position_by_id_at_last_frame(window_id, &from_id);
        let to_pos = app.element_position_by_id_at_last_frame(window_id, &to_id);

        if let (Some(from_rect), Some(to_rect)) = (from_pos, to_pos) {
            // Calculate the vertical distance between indicators, accounting for gaps.
            let line_height = to_rect.origin_y() - from_rect.max_y() - (CONNECTING_LINE_GAP * 2.);

            if line_height > 0.0 {
                let line = ConstrainedBox::new(
                    Rect::new()
                        .with_background(theme.surface_overlay_2())
                        .finish(),
                )
                .with_width(CONNECTING_LINE_WIDTH)
                .with_height(line_height)
                .finish();

                stack.add_positioned_child(
                    line,
                    OffsetPositioning::offset_from_save_position_element(
                        from_id,
                        vec2f(0., CONNECTING_LINE_GAP),
                        PositionedElementOffsetBounds::Unbounded,
                        PositionedElementAnchor::BottomMiddle,
                        ChildAnchor::TopMiddle,
                    ),
                );
            }
        }
    }

    stack.finish()
}

/// Generates a unique position ID for an indicator at the given step index.
fn indicator_position_id(prefix: &str, index: usize) -> String {
    format!("{prefix}_progress_indicator_{index}")
}

/// Renders a single progress step row.
fn render_step(
    step: ProgressStep,
    index: usize,
    save_position_prefix: &str,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let indicator = render_indicator(step.state, appearance);
    let text_content = render_step_text(step, appearance);

    let indicator_with_position = SavePosition::new(
        indicator,
        &indicator_position_id(save_position_prefix, index),
    )
    .finish();

    ConstrainedBox::new(
        Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(
                Flex::row()
                    .with_main_axis_alignment(MainAxisAlignment::Start)
                    .with_cross_axis_alignment(CrossAxisAlignment::Start)
                    .with_child(indicator_with_position)
                    .with_child(
                        Container::new(text_content)
                            .with_margin_left(TEXT_LEFT_MARGIN)
                            .finish(),
                    )
                    .finish(),
            )
            .finish(),
    )
    .with_max_height(MAX_STEP_HEIGHT)
    .finish()
}

/// Renders the state indicator.
fn render_indicator(state: ProgressStepState, appearance: &Appearance) -> Box<dyn Element> {
    let theme = appearance.theme();
    let background = theme.background();

    let content_size = INDICATOR_SIZE - INDICATOR_BORDER_WIDTH * 2.;
    let circle_radius = CornerRadius::with_all(Radius::Percentage(50.0));

    let content = match state {
        ProgressStepState::Completed => {
            let check_icon = ConstrainedBox::new(
                Icon::Check
                    .to_warpui_icon(theme.main_text_color(background))
                    .finish(),
            )
            .with_width(ICON_SIZE)
            .with_height(ICON_SIZE)
            .finish();

            ConstrainedBox::new(
                Container::new(
                    Flex::row()
                        .with_child(check_icon)
                        .with_main_axis_alignment(MainAxisAlignment::Center)
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_main_axis_size(MainAxisSize::Max)
                        .finish(),
                )
                .with_corner_radius(circle_radius)
                .with_background(theme.ui_green_color())
                .finish(),
            )
            .with_width(content_size)
            .with_height(content_size)
            .finish()
        }
        ProgressStepState::InProgress => ConstrainedBox::new(
            Rect::new()
                .with_background(theme.main_text_color(background))
                .with_corner_radius(circle_radius)
                .finish(),
        )
        .with_width(content_size)
        .with_height(content_size)
        .finish(),
        ProgressStepState::Pending => ConstrainedBox::new(
            Rect::new()
                .with_background(theme.disabled_text_color(background))
                .with_corner_radius(circle_radius)
                .finish(),
        )
        .with_width(content_size)
        .with_height(content_size)
        .finish(),
    };

    let border = match state {
        ProgressStepState::Completed => theme.green_overlay_2(),
        ProgressStepState::InProgress | ProgressStepState::Pending => theme.surface_overlay_2(),
    };

    Container::new(content)
        .with_corner_radius(circle_radius)
        .with_border(Border::all(INDICATOR_BORDER_WIDTH).with_border_fill(border))
        .finish()
}

/// Renders the text content for a step.
fn render_step_text(step: ProgressStep, appearance: &Appearance) -> Box<dyn Element> {
    let theme = appearance.theme();
    let background = theme.background();

    let name_text_color = match step.state {
        ProgressStepState::InProgress => theme.main_text_color(background),
        ProgressStepState::Completed => theme.sub_text_color(background),
        ProgressStepState::Pending => theme.disabled_text_color(background),
    };

    let name_text = Text::new(
        step.name,
        appearance.ui_font_family(),
        appearance.ui_font_size() + 2.,
    )
    .with_color(name_text_color.into_solid())
    .finish();

    let detail_element = if step.subtext.is_some() || step.duration.is_some() {
        let mut row = Flex::row();

        let has_detail = step.subtext.is_some();
        if let Some(s) = step.subtext {
            let detail_text_color = match step.state {
                ProgressStepState::Completed | ProgressStepState::InProgress => {
                    theme.sub_text_color(background)
                }
                ProgressStepState::Pending => theme.disabled_text_color(background),
            };
            row.add_child(
                Text::new(s, appearance.ui_font_family(), appearance.ui_font_size())
                    .with_color(detail_text_color.into_solid())
                    .finish(),
            );
        }

        if let Some(d) = step.duration {
            if has_detail {
                row.add_child(
                    Text::new(
                        " • ",
                        appearance.ui_font_family(),
                        appearance.ui_font_size(),
                    )
                    .with_color(theme.sub_text_color(background).into_solid())
                    .finish(),
                );
            }

            let duration_str = format_duration(d);
            row.add_child(
                Text::new(
                    duration_str,
                    appearance.ui_font_family(),
                    appearance.ui_font_size(),
                )
                .with_color(theme.disabled_text_color(background).into_solid())
                .finish(),
            );
        }

        Some(Container::new(row.finish()).with_margin_top(4.).finish())
    } else {
        None
    };

    Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Start)
        .with_child(name_text)
        .with_children(detail_element)
        .finish()
}

/// Formats a duration as a human-readable string (e.g., "10.5s").
fn format_duration(duration: Duration) -> String {
    let total_millis = duration.as_millis();
    let seconds = total_millis / 1000;
    let tenths = (total_millis % 1000) / 100;
    format!("{seconds}.{tenths}s")
}
