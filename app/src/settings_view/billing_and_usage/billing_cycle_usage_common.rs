use std::cell::RefCell;
use std::collections::HashMap;

use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use thousands::Separable;
use warp_core::ui::appearance::Appearance;
use warpui::elements::{
    Align, Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, DropShadow, Empty,
    Flex, MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement, Radius, Shrinkable,
    Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::Element;

use crate::settings_view::billing_and_usage_page_v2::{
    AGGREGATE_CREDITS_DOT_COLOR, AMBIENT_CREDITS_DOT_COLOR, BASE_CREDITS_DOT_COLOR,
    BONUS_CREDITS_DOT_COLOR, PAYG_CREDITS_DOT_COLOR,
};
use crate::ui_components::blended_colors;
use crate::workspaces::workspace::{
    AiCreditsUsageAndCostSubjectType, AiCreditsUsageAndCostType, AiCreditsUsageBucket,
    BillingCycleUsageEntry,
};

// for a bunch of this (min fill ratio, cost type order, ... )
// you will find analogous ts code in warp-server
pub const ROW_BORDER_RADIUS: f32 = 8.;
pub const ROW_BORDER_WIDTH: f32 = 1.;
pub const TOOLTIP_GAP: f32 = 6.;

const COST_TYPE_ORDER: &[AiCreditsUsageAndCostType] = &[
    AiCreditsUsageAndCostType::BaseLimit,
    AiCreditsUsageAndCostType::BonusGrant,
    AiCreditsUsageAndCostType::Payg,
    AiCreditsUsageAndCostType::AmbientBonusGrant,
];
const BUCKET_ORDER: &[AiCreditsUsageBucket] = &[
    AiCreditsUsageBucket::Ai,
    AiCreditsUsageBucket::Compute,
    AiCreditsUsageBucket::Platform,
];

/// One colored slice of the stacked bar. `cost_type` drives color; `usage_bucket`
/// drives the tooltip breakdown.
#[derive(Clone, Debug)]
pub struct BarSegment {
    pub cost_type: AiCreditsUsageAndCostType,
    pub usage_bucket: AiCreditsUsageBucket,
    pub credits: i64,
    pub cost_cents: i64,
}

/// Shared mouse-state bag for the billing-and-usage section: the
/// All/Local/Cloud filter pills plus a tooltip handle for every interactive
/// element keyed by string id (per-member rows, team-totals cards, ...).
pub struct BillingUsageMouseStates {
    pub filter_all: MouseStateHandle,
    pub filter_local: MouseStateHandle,
    pub filter_cloud: MouseStateHandle,
    tooltip_by_subject: RefCell<HashMap<String, MouseStateHandle>>,
}

impl Default for BillingUsageMouseStates {
    fn default() -> Self {
        Self {
            filter_all: MouseStateHandle::default(),
            filter_local: MouseStateHandle::default(),
            filter_cloud: MouseStateHandle::default(),
            tooltip_by_subject: RefCell::new(HashMap::new()),
        }
    }
}

impl BillingUsageMouseStates {
    pub fn tooltip_mouse_state(&self, key: &str) -> MouseStateHandle {
        let mut map = self.tooltip_by_subject.borrow_mut();
        map.entry(key.to_string()).or_default().clone()
    }
}

/// Swatch color for one cost-type bucket, mirroring the legend palette.
pub fn cost_type_color(cost_type: &AiCreditsUsageAndCostType) -> ColorU {
    match cost_type {
        AiCreditsUsageAndCostType::BaseLimit => BASE_CREDITS_DOT_COLOR,
        AiCreditsUsageAndCostType::BonusGrant => BONUS_CREDITS_DOT_COLOR,
        AiCreditsUsageAndCostType::Payg => PAYG_CREDITS_DOT_COLOR,
        AiCreditsUsageAndCostType::AmbientBonusGrant => AMBIENT_CREDITS_DOT_COLOR,
        AiCreditsUsageAndCostType::Aggregate => AGGREGATE_CREDITS_DOT_COLOR,
        AiCreditsUsageAndCostType::Other(_) => BASE_CREDITS_DOT_COLOR,
    }
}

fn cost_type_label(cost_type: &AiCreditsUsageAndCostType) -> &'static str {
    match cost_type {
        AiCreditsUsageAndCostType::BaseLimit => "Base",
        AiCreditsUsageAndCostType::BonusGrant => "Add-ons",
        AiCreditsUsageAndCostType::Payg => "Pay-as-you-go",
        AiCreditsUsageAndCostType::AmbientBonusGrant => "Cloud-only",
        AiCreditsUsageAndCostType::Aggregate => "Combined",
        AiCreditsUsageAndCostType::Other(_) => "Other",
    }
}

fn bucket_label(bucket: &AiCreditsUsageBucket) -> &'static str {
    match bucket {
        AiCreditsUsageBucket::Ai => "AI",
        AiCreditsUsageBucket::Compute => "Compute",
        AiCreditsUsageBucket::Platform => "Platform",
        AiCreditsUsageBucket::SuggestedCodeDiffs => "Suggested code diffs",
        AiCreditsUsageBucket::Voice => "Voice",
        AiCreditsUsageBucket::Aggregate => "Total",
        AiCreditsUsageBucket::Other(_) => "Other",
    }
}

fn cost_type_rank(cost_type: &AiCreditsUsageAndCostType) -> usize {
    COST_TYPE_ORDER
        .iter()
        .position(|c| c == cost_type)
        .unwrap_or(COST_TYPE_ORDER.len())
}

fn bucket_rank(bucket: &AiCreditsUsageBucket) -> usize {
    BUCKET_ORDER
        .iter()
        .position(|b| b == bucket)
        .unwrap_or(BUCKET_ORDER.len())
}

fn segment_sort_key(segment: &BarSegment) -> (usize, usize) {
    (
        cost_type_rank(&segment.cost_type),
        bucket_rank(&segment.usage_bucket),
    )
}

/// Group `entries` by `(cost_type, usage_bucket)` into [`BarSegment`]s; returns
/// sorted segments plus row totals. Linear Vec lookup since cynic enums don't
/// impl Hash and per-row entry counts are small.
pub fn aggregate_segments<'a>(
    entries: impl IntoIterator<Item = &'a BillingCycleUsageEntry>,
) -> (Vec<BarSegment>, i64, i64) {
    let mut segments: Vec<BarSegment> = Vec::new();

    for entry in entries {
        if let Some(existing) = segments
            .iter_mut()
            .find(|s| s.cost_type == entry.cost_type && s.usage_bucket == entry.usage_bucket)
        {
            existing.credits += entry.credits_used as i64;
            existing.cost_cents += entry.cost_cents as i64;
        } else {
            segments.push(BarSegment {
                cost_type: entry.cost_type.clone(),
                usage_bucket: entry.usage_bucket.clone(),
                credits: entry.credits_used as i64,
                cost_cents: entry.cost_cents as i64,
            });
        }
    }

    segments.retain(|s| s.credits > 0);
    segments.sort_by_key(segment_sort_key);

    let total_credits = segments.iter().map(|s| s.credits).sum();
    let total_cost_cents = segments.iter().map(|s| s.cost_cents).sum();

    (segments, total_credits, total_cost_cents)
}

/// Drops Voice / SuggestedCodeDiffs entries from the usage view.
///
/// These buckets are tracked server-side against their own dedicated per-cycle
/// limits (`VoiceRequestLimit` / `SuggestedCodeDiffsLimit`) rather than the
/// AI/Compute base credit pool — see
/// `model/sql/ai_credits_usage_and_cost/get_base_limits_usage.sql` and
/// `isBaseLimitExhaustedForBucket` in warp-server. Records are written with
/// `cost_type = BASE_LIMIT` and `cost_cents = 0`, so surfacing them here
/// would inflate the per-row `total_credits` and skew the `used / limit`
/// math without contributing to anything the user is actually billed for.
///
/// TODO: this also hides the rare case where a user blows past their
/// dedicated Voice or SuggestedCodeDiffs limit and the resolver falls
/// through to bonus grants — those entries would have real `cost_cents`
/// and *do* draw down add-on credits. In practice ~nobody (maybe ZL?) hits
/// those limits, so we filter unconditionally for now; revisit if usage of
/// those features ever grows enough that the overflow matters.
pub fn filter_legacy_buckets(entries: &[BillingCycleUsageEntry]) -> Vec<BillingCycleUsageEntry> {
    entries
        .iter()
        .filter(|e| {
            e.usage_bucket != AiCreditsUsageBucket::Voice
                && e.usage_bucket != AiCreditsUsageBucket::SuggestedCodeDiffs
        })
        .cloned()
        .collect()
}

/// "Is there any data in `entries` that's not my own?"
pub fn has_non_viewer_data(entries: &[BillingCycleUsageEntry], viewer_uid: Option<&str>) -> bool {
    entries.iter().any(|e| match &e.subject_type {
        AiCreditsUsageAndCostSubjectType::Team => true,
        _ => match (e.subject_uid.as_deref(), viewer_uid) {
            (Some(uid), Some(viewer)) => uid != viewer,
            // Unknown subject — conservatively treat as non-viewer.
            _ => true,
        },
    })
}

pub fn format_credits(credits: i64) -> String {
    credits.separate_with_commas()
}

pub fn format_cost_cents(cents: i64) -> String {
    let dollars = cents / 100;
    let remainder = (cents.abs() % 100) as u8;
    if dollars < 0 {
        format!(
            "-${}.{remainder:02}",
            dollars.unsigned_abs().separate_with_commas()
        )
    } else {
        format!("${}.{remainder:02}", dollars.separate_with_commas())
    }
}

/// Section subheader (e.g. "Team totals", "Member usage"). One step below
/// the v2 page's bold section title.
pub fn render_section_subheader(label: &str, appearance: &Appearance) -> Box<dyn Element> {
    Text::new_inline(label.to_string(), appearance.ui_font_family(), 14.)
        .with_color(appearance.theme().active_ui_text_color().into())
        .with_style(Properties::default().weight(Weight::Medium))
        .finish()
}

/// Per-cost-type breakdown card. Parameterized by raw segments and totals so
/// it can back team-totals card hovers as well as per-member row hovers.
pub fn render_breakdown_tooltip(
    segments: &[BarSegment],
    total_credits: i64,
    total_cost_cents: i64,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let font_family = appearance.ui_font_family();
    let bg = theme.background().into_solid();
    let main = blended_colors::text_main(theme, bg);
    let sub = blended_colors::text_sub(theme, bg);

    let mut column = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_spacing(6.);

    for line in segments {
        let label = if matches!(line.usage_bucket, AiCreditsUsageBucket::Aggregate) {
            cost_type_label(&line.cost_type).to_string()
        } else {
            format!(
                "{} ({})",
                cost_type_label(&line.cost_type),
                bucket_label(&line.usage_bucket)
            )
        };

        column.add_child(render_tooltip_row(
            Some(cost_type_color(&line.cost_type)),
            label,
            line.credits,
            line.cost_cents,
            sub,
            main,
            font_family,
            /* bold */ false,
        ));
    }

    // Divider before the total row.
    column.add_child(
        Container::new(Empty::new().finish())
            .with_padding_top(1.)
            .with_background_color(theme.outline().into_solid())
            .finish(),
    );

    column.add_child(render_tooltip_row(
        /* no swatch on the total row */ None,
        "Total usage".to_string(),
        total_credits,
        total_cost_cents,
        main,
        main,
        font_family,
        /* bold */ true,
    ));

    ConstrainedBox::new(
        Container::new(column.finish())
            .with_background_color(bg)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
            .with_border(Border::all(1.).with_border_color(theme.outline().into_solid()))
            .with_uniform_padding(10.)
            .with_drop_shadow(
                DropShadow::new_with_standard_offset_and_spread(ColorU::new(0, 0, 0, 48))
                    .with_offset(vec2f(0., 4.)),
            )
            .finish(),
    )
    .with_min_width(200.)
    .with_max_width(320.)
    .finish()
}

/// Single tooltip row: `[swatch + label] [spacer] [credits / cost]` with
/// fixed-width right-aligned number columns.
#[allow(clippy::too_many_arguments)]
fn render_tooltip_row(
    swatch_color: Option<ColorU>,
    label: String,
    credits: i64,
    cost_cents: i64,
    label_color: ColorU,
    value_color: ColorU,
    font_family: warpui::fonts::FamilyId,
    bold: bool,
) -> Box<dyn Element> {
    let style = if bold {
        Properties::default().weight(Weight::Semibold)
    } else {
        Properties::default()
    };

    let label_text = Text::new_inline(label, font_family, 12.)
        .with_color(label_color)
        .with_style(style)
        .finish();

    let mut left = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
    if let Some(color) = swatch_color {
        left.add_child(
            ConstrainedBox::new(
                Container::new(Empty::new().finish())
                    .with_background_color(color)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(2.)))
                    .finish(),
            )
            .with_width(10.)
            .with_height(10.)
            .finish(),
        );
        left.add_child(Container::new(label_text).with_margin_left(8.).finish());
    } else {
        left.add_child(label_text);
    }

    let credits_text = Text::new_inline(format_credits(credits), font_family, 12.)
        .with_color(value_color)
        .with_style(style)
        .finish();
    let cost_text = Text::new_inline(format_cost_cents(cost_cents), font_family, 12.)
        .with_color(value_color)
        .with_style(style)
        .finish();
    let divider = Text::new_inline("/".to_string(), font_family, 12.)
        .with_color(label_color)
        .with_style(style)
        .finish();

    let credits_col = ConstrainedBox::new(Align::new(credits_text).right().finish())
        .with_width(60.)
        .finish();
    let cost_col = ConstrainedBox::new(Align::new(cost_text).right().finish())
        .with_width(64.)
        .finish();

    let right = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(credits_col)
        .with_child(Container::new(divider).with_horizontal_margin(3.).finish())
        .with_child(cost_col)
        .finish();

    Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
        .with_main_axis_size(MainAxisSize::Max)
        .with_child(Shrinkable::new(1., left.finish()).finish())
        .with_child(right)
        .finish()
}

#[cfg(test)]
#[path = "billing_cycle_usage_common_tests.rs"]
mod tests;
