use pathfinder_geometry::vector::vec2f;
use warp_core::ui::appearance::Appearance;
use warpui::elements::{
    Border, ChildAnchor, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Empty,
    Expanded, Flex, Hoverable, MainAxisAlignment, MainAxisSize, MouseStateHandle,
    OffsetPositioning, ParentAnchor, ParentElement, ParentOffsetBounds, Radius, Shrinkable, Stack,
    Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::Element;

use crate::settings_view::billing_and_usage::billing_cycle_usage_common::{
    aggregate_segments, cost_type_color, format_cost_cents, format_credits,
    render_breakdown_tooltip, render_section_subheader, BarSegment, BillingUsageMouseStates,
    ROW_BORDER_RADIUS, ROW_BORDER_WIDTH, TOOLTIP_GAP,
};
use crate::ui_components::blended_colors;
use crate::workspaces::workspace::{
    AiCreditsUsageBucket, AiCreditsUsageSource, BillingCycleUsageEntry, UsageVisibility,
    UsageVisibilityGranularity,
};

fn collapse_segments_to_cost_type(segments: Vec<BarSegment>) -> Vec<BarSegment> {
    let mut out: Vec<BarSegment> = Vec::new();
    for seg in segments {
        if let Some(existing) = out.iter_mut().find(|s| s.cost_type == seg.cost_type) {
            existing.credits += seg.credits;
            existing.cost_cents += seg.cost_cents;
        } else {
            out.push(BarSegment {
                cost_type: seg.cost_type,
                usage_bucket: AiCreditsUsageBucket::Aggregate,
                credits: seg.credits,
                cost_cents: seg.cost_cents,
            });
        }
    }
    out
}

/// Pill-shaped bar at the bottom of each team-totals card.
const CARD_BAR_HEIGHT: f32 = 8.;
const CARD_BAR_RADIUS: f32 = CARD_BAR_HEIGHT / 2.;

/// Summary backing a single team-totals card (Overall / Local / Cloud).
#[derive(Debug)]
pub struct TeamTotalCardSummary {
    pub title: &'static str,
    pub card_key: &'static str,
    pub segments: Vec<BarSegment>,
    pub total_credits: i64,
    pub total_cost_cents: i64,
    pub limit_cents: Option<i64>,
}

pub fn build_team_total_card_summaries(
    entries: &[BillingCycleUsageEntry],
    visibility: &UsageVisibility,
) -> Vec<TeamTotalCardSummary> {
    let (overall_segments, overall_credits, overall_cost) = aggregate_segments(entries.iter());
    let mut summaries = vec![TeamTotalCardSummary {
        title: "Overall usage",
        card_key: "__card_overall__",
        segments: overall_segments,
        total_credits: overall_credits,
        total_cost_cents: overall_cost,
        limit_cents: None,
    }];

    let shows_per_source = matches!(
        visibility.granularity,
        UsageVisibilityGranularity::FullBreakdown
    );
    if shows_per_source {
        let (local_segments, local_credits, local_cost) = aggregate_segments(
            entries
                .iter()
                .filter(|e| e.usage_source == AiCreditsUsageSource::Local),
        );
        let (cloud_segments, cloud_credits, cloud_cost) = aggregate_segments(
            entries
                .iter()
                .filter(|e| e.usage_source == AiCreditsUsageSource::Cloud),
        );
        summaries.push(TeamTotalCardSummary {
            title: "Local agent usage",
            card_key: "__card_local__",
            segments: local_segments,
            total_credits: local_credits,
            total_cost_cents: local_cost,
            limit_cents: None,
        });
        summaries.push(TeamTotalCardSummary {
            title: "Cloud agent usage",
            card_key: "__card_cloud__",
            segments: cloud_segments,
            total_credits: cloud_credits,
            total_cost_cents: cloud_cost,
            limit_cents: None,
        });
    }

    // Visibility tiers below FullBreakdown don't expose per-bucket detail,
    // so collapse bucket-dimensioned segments into single per-cost-type lines.
    // Otherwise we get a "Base (AI)" row + a separate bare "Base" row in the team aggregate card.
    if !matches!(
        visibility.granularity,
        UsageVisibilityGranularity::FullBreakdown
    ) {
        for summary in &mut summaries {
            summary.segments =
                collapse_segments_to_cost_type(std::mem::take(&mut summary.segments));
        }
    }

    summaries
}

fn render_card_pill_bar(
    segments: &[BarSegment],
    total_credits: i64,
    total_cost_cents: i64,
    limit_cents: Option<i64>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let track_bg = theme.surface_overlay_1();
    let corner = Radius::Pixels(CARD_BAR_RADIUS);

    if total_credits == 0 || segments.is_empty() {
        return ConstrainedBox::new(
            Container::new(Empty::new().finish())
                .with_background(track_bg)
                .with_corner_radius(CornerRadius::with_all(corner))
                .finish(),
        )
        .with_height(CARD_BAR_HEIGHT)
        .finish();
    }

    let fill_ratio = match limit_cents {
        Some(limit) if limit > 0 => (total_cost_cents as f32 / limit as f32).clamp(0.0, 1.0),
        _ => 1.0,
    };
    let unfill_ratio = 1.0 - fill_ratio;
    let has_unfill = unfill_ratio > 0.0;
    let last_segment_idx = segments.len() - 1;

    let mut filled = Flex::row();
    for (idx, seg) in segments.iter().enumerate() {
        let weight = seg.credits as f32 / total_credits as f32;
        if weight <= 0.0 {
            continue;
        }
        let is_first = idx == 0;
        let is_last_visible = idx == last_segment_idx && !has_unfill;
        let segment_corner = match (is_first, is_last_visible) {
            (true, true) => CornerRadius::with_all(corner),
            (true, false) => CornerRadius::with_left(corner),
            (false, true) => CornerRadius::with_right(corner),
            (false, false) => CornerRadius::default(),
        };
        filled.add_child(
            Expanded::new(
                weight,
                Container::new(Empty::new().finish())
                    .with_background_color(cost_type_color(&seg.cost_type))
                    .with_corner_radius(segment_corner)
                    .finish(),
            )
            .finish(),
        );
    }

    let mut bar = Flex::row();
    bar.add_child(Expanded::new(fill_ratio, filled.finish()).finish());
    if has_unfill {
        bar.add_child(
            Expanded::new(
                unfill_ratio,
                Container::new(Empty::new().finish())
                    .with_background(track_bg)
                    .with_corner_radius(CornerRadius::with_right(corner))
                    .finish(),
            )
            .finish(),
        );
    }

    ConstrainedBox::new(bar.finish())
        .with_height(CARD_BAR_HEIGHT)
        .finish()
}

/// Card body for one team-totals slice. Layout (top to bottom):
///   [title]
///   [$X.XX]                    [Limit: $Y.YY]   (limit optional)
///   [(N credits)]
///   [pill stacked bar]
fn build_team_total_card(
    summary: &TeamTotalCardSummary,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let card_bg = theme.background().into_solid();
    let main = blended_colors::text_main(theme, card_bg);
    let sub = blended_colors::text_sub(theme, card_bg);

    let title_text = Text::new_inline(summary.title.to_string(), appearance.ui_font_family(), 13.)
        .with_color(sub)
        .with_style(Properties::default().weight(Weight::Medium))
        .finish();

    let cost_text = Text::new_inline(
        format_cost_cents(summary.total_cost_cents),
        appearance.ui_font_family(),
        24.,
    )
    .with_color(main)
    .with_style(Properties::default().weight(Weight::Semibold))
    .finish();

    let credits_text = Text::new_inline(
        format!("({} credits)", format_credits(summary.total_credits)),
        appearance.ui_font_family(),
        13.,
    )
    .with_color(sub)
    .finish();

    let totals_col = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Start)
        .with_child(cost_text)
        .with_child(Container::new(credits_text).with_margin_top(2.).finish())
        .finish();

    let totals_row: Box<dyn Element> = match summary.limit_cents {
        Some(limit) => {
            let limit_text = Text::new_inline(
                format!("Limit: {}", format_cost_cents(limit)),
                appearance.ui_font_family(),
                12.,
            )
            .with_color(sub)
            .finish();
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_main_axis_size(MainAxisSize::Max)
                .with_child(Shrinkable::new(1., totals_col).finish())
                .with_child(Container::new(limit_text).with_margin_left(16.).finish())
                .finish()
        }
        None => totals_col,
    };

    let bar = render_card_pill_bar(
        &summary.segments,
        summary.total_credits,
        summary.total_cost_cents,
        summary.limit_cents,
        appearance,
    );

    let body = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_spacing(12.)
        .with_child(
            Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_spacing(6.)
                .with_child(title_text)
                .with_child(totals_row)
                .finish(),
        )
        .with_child(bar)
        .finish();

    Container::new(body)
        .with_background_color(card_bg)
        .with_border(Border::all(ROW_BORDER_WIDTH).with_border_color(theme.outline().into_solid()))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(ROW_BORDER_RADIUS)))
        .with_uniform_padding(16.)
        .finish()
}

fn render_team_total_card(
    summary: &TeamTotalCardSummary,
    tooltip_mouse_state: MouseStateHandle,
    appearance: &Appearance,
) -> Box<dyn Element> {
    if summary.segments.is_empty() {
        return build_team_total_card(summary, appearance);
    }

    Hoverable::new(tooltip_mouse_state, move |state| {
        let mut stack = Stack::new();
        stack.add_child(build_team_total_card(summary, appearance));

        if state.is_hovered() {
            stack.add_positioned_overlay_child(
                render_breakdown_tooltip(
                    &summary.segments,
                    summary.total_credits,
                    summary.total_cost_cents,
                    appearance,
                ),
                OffsetPositioning::offset_from_parent(
                    vec2f(0., -TOOLTIP_GAP),
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::TopMiddle,
                    ChildAnchor::BottomMiddle,
                ),
            );
        }

        stack.finish()
    })
    .finish()
}

/// Horizontal row of team-totals cards (Overall + Local + Cloud).
fn render_team_totals_section(
    entries: &[BillingCycleUsageEntry],
    visibility: &UsageVisibility,
    mouse_states: &BillingUsageMouseStates,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let summaries = build_team_total_card_summaries(entries, visibility);
    let mut row = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_main_axis_size(MainAxisSize::Max)
        .with_spacing(12.);
    for summary in &summaries {
        let tooltip_state = mouse_states.tooltip_mouse_state(summary.card_key);
        row.add_child(
            Expanded::new(
                1.,
                render_team_total_card(summary, tooltip_state, appearance),
            )
            .finish(),
        );
    }
    row.finish()
}

/// "Team" subheader + cards
pub fn render_team_totals_block(
    entries: &[BillingCycleUsageEntry],
    visibility: &UsageVisibility,
    mouse_states: &BillingUsageMouseStates,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let mut column = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
    column.add_child(
        Container::new(render_section_subheader("Team", appearance))
            .with_margin_bottom(8.)
            .finish(),
    );
    column.add_child(render_team_totals_section(
        entries,
        visibility,
        mouse_states,
        appearance,
    ));
    column.finish()
}

#[cfg(test)]
#[path = "billing_cycle_usage_team_totals_tests.rs"]
mod tests;
