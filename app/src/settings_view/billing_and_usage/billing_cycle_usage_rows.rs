use std::cell::RefCell;
use std::collections::HashMap;

use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use thousands::Separable;
use warp_core::ui::appearance::Appearance;
use warpui::{
    elements::{
        Align, Border, ChildAnchor, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment,
        DropShadow, Empty, Expanded, Flex, Hoverable, MainAxisAlignment, MainAxisSize,
        MouseStateHandle, OffsetPositioning, ParentAnchor, ParentElement, ParentOffsetBounds,
        Radius, Shrinkable, Stack, Text,
    },
    fonts::{Properties, Weight},
    platform::Cursor,
    AppContext, Element, EventContext, SingletonEntity,
};

use crate::{
    auth::AuthStateProvider,
    settings_view::billing_and_usage_page_v2::{
        AGGREGATE_CREDITS_DOT_COLOR, AMBIENT_CREDITS_DOT_COLOR, BASE_CREDITS_DOT_COLOR,
        BONUS_CREDITS_DOT_COLOR, PAYG_CREDITS_DOT_COLOR,
    },
    ui_components::{blended_colors, icons::Icon},
    workspaces::workspace::{
        AiCreditsUsageAndCostSubjectType, AiCreditsUsageAndCostType, AiCreditsUsageBucket,
        AiCreditsUsageSource, BillingCycleUsageEntry, UsageVisibilityGranularity, Workspace,
        WorkspaceMember,
    },
};

// for a bunch of this (min fill ratio, cost type order, ... )
// you will find analogous ts code in warp-server
const BAR_HEIGHT: f32 = 8.;
const MIN_FILL_RATIO: f32 = 0.05;
const ROW_BORDER_RADIUS: f32 = 8.;
const ROW_BORDER_WIDTH: f32 = 1.;
/// Size of the leading icons in the row credit cluster (coin + credit-card).
const ROW_ICON_SIZE: f32 = 12.;
/// Inner radius so the bar's curve sits flush against the card's inner border.
const BAR_CORNER_RADIUS: f32 = ROW_BORDER_RADIUS - ROW_BORDER_WIDTH;
const ROW_PADDING: f32 = 12.;
const TOOLTIP_GAP: f32 = 6.;
/// Padding inside each team-totals card. Larger than ROW_PADDING since the
/// card has more vertical breathing room (title + big number + bar).
const CARD_PADDING: f32 = 16.;
/// Horizontal gap between team-totals cards.
const CARD_GAP: f32 = 12.;
/// Pill-shaped bar at the bottom of each team-totals card.
const CARD_BAR_HEIGHT: f32 = 8.;
const CARD_BAR_RADIUS: f32 = CARD_BAR_HEIGHT / 2.;

const CARD_OVERALL_KEY: &str = "__card_overall__";
const CARD_LOCAL_KEY: &str = "__card_local__";
const CARD_CLOUD_KEY: &str = "__card_cloud__";
const OTHER_MEMBERS_KEY: &str = "__other_members__";
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SourceFilter {
    #[default]
    All,
    Local,
    Cloud,
}

impl SourceFilter {
    pub fn label(self) -> &'static str {
        match self {
            SourceFilter::All => "All",
            SourceFilter::Local => "Local",
            SourceFilter::Cloud => "Cloud",
        }
    }

    fn matches(self, source: &AiCreditsUsageSource) -> bool {
        match self {
            SourceFilter::All => true,
            SourceFilter::Local => *source == AiCreditsUsageSource::Local,
            SourceFilter::Cloud => *source == AiCreditsUsageSource::Cloud,
        }
    }
}

pub struct RowMouseStates {
    pub filter_all: MouseStateHandle,
    pub filter_local: MouseStateHandle,
    pub filter_cloud: MouseStateHandle,
    tooltip_by_subject: RefCell<HashMap<String, MouseStateHandle>>,
}

const SELF_OWN_KEY: &str = "__self_own__";

impl Default for RowMouseStates {
    fn default() -> Self {
        Self {
            filter_all: MouseStateHandle::default(),
            filter_local: MouseStateHandle::default(),
            filter_cloud: MouseStateHandle::default(),
            tooltip_by_subject: RefCell::new(HashMap::new()),
        }
    }
}

impl RowMouseStates {
    fn tooltip_mouse_state(&self, key: &str) -> MouseStateHandle {
        let mut map = self.tooltip_by_subject.borrow_mut();
        map.entry(key.to_string()).or_default().clone()
    }
}

/// One colored slice of the stacked bar. `cost_type` drives color; `usage_bucket`
/// drives the tooltip breakdown.
#[derive(Clone, Debug)]
pub struct BarSegment {
    pub cost_type: AiCreditsUsageAndCostType,
    pub usage_bucket: AiCreditsUsageBucket,
    pub credits: i64,
    pub cost_cents: i64,
}

/// Aggregated usage for one subject (or the synthetic team aggregate).
#[derive(Debug)]
pub struct MemberUsageRow {
    pub subject_type: AiCreditsUsageAndCostSubjectType,
    pub subject_key: String,
    pub display_name: String,
    pub total_credits: i64,
    pub total_cost_cents: i64,
    /// Per-user base credit limit, rendered as `used / limit`. None for service
    /// accounts, team-aggregate rows, and unlimited members.
    pub base_limit: Option<i64>,
    /// Sorted by [`COST_TYPE_ORDER`] then [`BUCKET_ORDER`]; zero-credit entries dropped.
    pub segments: Vec<BarSegment>,
}

fn member_base_limit(member: &WorkspaceMember) -> Option<i64> {
    if member.usage_info.is_unlimited {
        None
    } else {
        Some(member.usage_info.request_limit as i64)
    }
}

/// Swatch color for one cost-type bucket, mirroring the legend palette.
fn cost_type_color(cost_type: &AiCreditsUsageAndCostType) -> ColorU {
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
        AiCreditsUsageAndCostType::Aggregate => "All sources",
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
fn aggregate_segments<'a>(
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

/// Single row for `OwnOnly` viewers — the viewer's own aggregated usage.
pub fn build_own_usage_row(
    entries: &[BillingCycleUsageEntry],
    viewer_uid: Option<&str>,
    viewer_display_name: String,
    viewer_base_limit: Option<i64>,
    source_filter: SourceFilter,
) -> MemberUsageRow {
    let viewer_entries: Vec<&BillingCycleUsageEntry> = entries
        .iter()
        .filter(|e| e.subject_type != AiCreditsUsageAndCostSubjectType::Team)
        .filter(|e| source_filter.matches(&e.usage_source))
        // Defensive: filter to viewer-only even though OwnOnly should already be redacted.
        .filter(|e| match (viewer_uid, e.subject_uid.as_deref()) {
            (Some(uid), Some(entry_uid)) => uid == entry_uid,
            (_, None) => true,
            (None, _) => true,
        })
        .collect();

    let (segments, total_credits, total_cost_cents) =
        aggregate_segments(viewer_entries.iter().copied());

    MemberUsageRow {
        subject_type: AiCreditsUsageAndCostSubjectType::User,
        subject_key: SELF_OWN_KEY.to_string(),
        display_name: viewer_display_name,
        total_credits,
        total_cost_cents,
        base_limit: viewer_base_limit,
        segments,
    }
}

fn build_other_members_usage_row(entries: &[BillingCycleUsageEntry]) -> MemberUsageRow {
    let team_entries = entries
        .iter()
        .filter(|e| e.subject_type == AiCreditsUsageAndCostSubjectType::Team);
    let (segments, total_credits, total_cost_cents) = aggregate_segments(team_entries);

    MemberUsageRow {
        subject_type: AiCreditsUsageAndCostSubjectType::Team,
        subject_key: OTHER_MEMBERS_KEY.to_string(),
        display_name: "Other members".to_string(),
        total_credits,
        total_cost_cents,
        base_limit: None,
        segments,
    }
}

/// Summary backing a single team-totals card (Overall / Local / Cloud).
/// Mirrors the admin panel's `AgentSpendingLimitItem` shape so we can swap in
/// per-source spend limits later without restructuring the renderer.
#[derive(Debug)]
pub struct TeamTotalCardSummary {
    pub title: &'static str,
    pub card_key: &'static str,
    pub segments: Vec<BarSegment>,
    pub total_credits: i64,
    pub total_cost_cents: i64,
    /// Monthly spend limit driving the bar fill and threshold border colors.
    /// `None` for the rendering scaffold — bars fill 100% and the border stays
    /// at the default outline. Plumbed through once the per-source limits land
    /// on the client `Workspace`.
    pub limit_cents: Option<i64>,
}

/// Builds the three team-totals card summaries (Overall + Local + Cloud).
/// Cards always reflect every renderable entry for their slice and ignore the
/// source filter toggle, which only scopes the per-member rows below.
pub fn build_team_total_card_summaries(
    entries: &[BillingCycleUsageEntry],
) -> Vec<TeamTotalCardSummary> {
    let (overall_segments, overall_credits, overall_cost) = aggregate_segments(entries.iter());
    let mut summaries = vec![TeamTotalCardSummary {
        title: "Overall usage",
        card_key: CARD_OVERALL_KEY,
        segments: overall_segments,
        total_credits: overall_credits,
        total_cost_cents: overall_cost,
        limit_cents: None,
    }];

    let has_per_source_data = !entries.is_empty()
        && entries
            .iter()
            .all(|e| e.usage_source != AiCreditsUsageSource::Aggregate);
    if has_per_source_data {
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
            card_key: CARD_LOCAL_KEY,
            segments: local_segments,
            total_credits: local_credits,
            total_cost_cents: local_cost,
            limit_cents: None,
        });
        summaries.push(TeamTotalCardSummary {
            title: "Cloud agent usage",
            card_key: CARD_CLOUD_KEY,
            segments: cloud_segments,
            total_credits: cloud_credits,
            total_cost_cents: cloud_cost,
            limit_cents: None,
        });
    }

    summaries
}

/// Per-member rows for `PerUserTotals` viewers. Iterates the workspace member
/// list so zero-usage members still get a row. Service accounts and other
/// non-member subjects surface as extra rows at the bottom.
pub fn build_member_usage_rows(
    entries: &[BillingCycleUsageEntry],
    members: &[WorkspaceMember],
    source_filter: SourceFilter,
) -> Vec<MemberUsageRow> {
    // Group entries by subject for joining against the member list below.
    let mut grouped: HashMap<
        String,
        (
            AiCreditsUsageAndCostSubjectType,
            String,
            Vec<BillingCycleUsageEntry>,
        ),
    > = HashMap::new();
    let mut unknown_counter = 0usize;

    for entry in entries
        .iter()
        .filter(|e| e.subject_type != AiCreditsUsageAndCostSubjectType::Team)
    {
        if !source_filter.matches(&entry.usage_source) {
            continue;
        }

        let key = match entry.subject_uid.as_deref() {
            Some(uid) => format!("{:?}:{uid}", entry.subject_type),
            None => {
                unknown_counter += 1;
                format!("{:?}:unknown-{unknown_counter}", entry.subject_type)
            }
        };
        let group = grouped.entry(key).or_insert_with(|| {
            (
                entry.subject_type.clone(),
                entry
                    .subject_display_name
                    .clone()
                    .unwrap_or_else(|| "Unknown".to_string()),
                Vec::new(),
            )
        });
        group.2.push(entry.clone());
    }

    let mut rows: Vec<MemberUsageRow> = Vec::with_capacity(members.len());

    // One row per workspace member, including zero-usage members.
    let mut seen_keys: std::collections::HashSet<String> = Default::default();
    for member in members {
        let key = format!(
            "{:?}:{}",
            AiCreditsUsageAndCostSubjectType::User,
            member.uid.as_str()
        );
        seen_keys.insert(key.clone());

        let (segments, total_credits, total_cost_cents) = match grouped.remove(&key) {
            Some((_, _, entries)) => aggregate_segments(entries.iter()),
            None => (Vec::new(), 0, 0),
        };

        rows.push(MemberUsageRow {
            subject_type: AiCreditsUsageAndCostSubjectType::User,
            subject_key: key,
            display_name: member.email.clone(),
            total_credits,
            total_cost_cents,
            base_limit: member_base_limit(member),
            segments,
        });
    }

    // Subjects not in the member list (typically service accounts) render after.
    for (key, (subject_type, display_name, entries)) in grouped {
        if seen_keys.contains(&key) {
            continue;
        }
        let (segments, total_credits, total_cost_cents) = aggregate_segments(entries.iter());
        rows.push(MemberUsageRow {
            subject_type,
            subject_key: key,
            display_name,
            total_credits,
            total_cost_cents,
            base_limit: None,
            segments,
        });
    }

    // Sort by total credits desc, stable by subject_key.
    rows.sort_by(|a, b| {
        b.total_credits
            .cmp(&a.total_credits)
            .then_with(|| a.subject_key.cmp(&b.subject_key))
    });

    rows
}

/// True if any entry is cloud-sourced; gates the source filter toggle.
pub fn has_cloud_usage(entries: &[BillingCycleUsageEntry]) -> bool {
    entries
        .iter()
        .any(|e| e.usage_source == AiCreditsUsageSource::Cloud)
}

fn format_credits(credits: i64) -> String {
    credits.separate_with_commas()
}

fn format_cost_cents(cents: i64) -> String {
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

fn render_stacked_bar(
    segments: &[BarSegment],
    total_credits: i64,
    team_max_credits: i64,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let track_bg = theme.surface_overlay_1();
    let corner = Radius::Pixels(BAR_CORNER_RADIUS);

    if team_max_credits == 0 || total_credits == 0 || segments.is_empty() {
        // Empty track, top-rounded on both ends.
        return ConstrainedBox::new(
            Container::new(Empty::new().finish())
                .with_background(track_bg)
                .with_corner_radius(CornerRadius::with_top(corner))
                .finish(),
        )
        .with_height(BAR_HEIGHT)
        .finish();
    }

    let fill_ratio = (total_credits as f32 / team_max_credits as f32).clamp(MIN_FILL_RATIO, 1.0);
    let unfill_ratio = 1.0 - fill_ratio;
    let has_unfill = unfill_ratio > 0.0;
    let last_segment_idx = segments.len() - 1;

    // One Expanded per segment, weighted by share of total_credits. First/last
    // segment get rounded top corners (last only if no muted tail).
    let mut filled = Flex::row();
    for (idx, seg) in segments.iter().enumerate() {
        let weight = seg.credits as f32 / total_credits as f32;
        if weight <= 0.0 {
            continue;
        }
        let is_first = idx == 0;
        let is_last_visible = idx == last_segment_idx && !has_unfill;
        let segment_corner = match (is_first, is_last_visible) {
            (true, true) => CornerRadius::with_top(corner),
            (true, false) => CornerRadius::with_top_left(corner),
            (false, true) => CornerRadius::with_top_right(corner),
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
                    .with_corner_radius(CornerRadius::with_top_right(corner))
                    .finish(),
            )
            .finish(),
        );
    }

    ConstrainedBox::new(bar.finish())
        .with_height(BAR_HEIGHT)
        .finish()
}

/// Per-cost-type tooltip breakdown with a "Total usage" footer.
fn render_usage_tooltip_content(row: &MemberUsageRow, appearance: &Appearance) -> Box<dyn Element> {
    render_breakdown_tooltip(
        &row.segments,
        row.total_credits,
        row.total_cost_cents,
        appearance,
    )
}

/// Same per-cost-type breakdown card, but parameterized by raw segments and
/// totals so it can back team-totals card hovers as well as per-member row
/// hovers.
fn render_breakdown_tooltip(
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
    .with_min_width(240.)
    .with_max_width(360.)
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
        .with_child(Container::new(divider).with_horizontal_margin(6.).finish())
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

/// Builds one row card (stacked bar + name/totals).
fn build_row_card(
    row: &MemberUsageRow,
    team_max_credits: i64,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let card_bg = theme.background().into_solid();
    let main = blended_colors::text_main(theme, card_bg);
    let sub = blended_colors::text_sub(theme, card_bg);

    let bar = render_stacked_bar(
        &row.segments,
        row.total_credits,
        team_max_credits,
        appearance,
    );

    let display_name_text = Text::new_inline(
        row.display_name.clone(),
        appearance.ui_font_family(),
        appearance.ui_font_size(),
    )
    .with_color(main)
    .finish();

    let mut name_row = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(display_name_text);

    if matches!(
        row.subject_type,
        AiCreditsUsageAndCostSubjectType::ServiceAccount
    ) {
        name_row.add_child(
            Container::new(
                Text::new_inline(
                    "(agent)".to_string(),
                    appearance.ui_font_family(),
                    appearance.ui_font_size(),
                )
                .with_color(sub)
                .finish(),
            )
            .with_margin_left(6.)
            .finish(),
        );
    }

    // Credit + cost cluster: `[coin] X[/limit]   [card] $cost`.
    let credits_str = match row.base_limit {
        Some(limit) => format!(
            "{}/{}",
            format_credits(row.total_credits),
            format_credits(limit)
        ),
        None => format_credits(row.total_credits),
    };
    let credits_text = Text::new_inline(
        credits_str,
        appearance.ui_font_family(),
        appearance.ui_font_size(),
    )
    .with_color(main)
    .finish();
    let cost_text = Text::new_inline(
        format_cost_cents(row.total_cost_cents),
        appearance.ui_font_family(),
        appearance.ui_font_size(),
    )
    .with_color(main)
    .finish();
    let icon_color = theme.sub_text_color(theme.background());
    let coin_icon = ConstrainedBox::new(Icon::Credits.to_warpui_icon(icon_color).finish())
        .with_width(ROW_ICON_SIZE)
        .with_height(ROW_ICON_SIZE)
        .finish();
    let card_icon = ConstrainedBox::new(Icon::CreditCard.to_warpui_icon(icon_color).finish())
        .with_width(ROW_ICON_SIZE)
        .with_height(ROW_ICON_SIZE)
        .finish();
    let credits_cluster = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(coin_icon)
        .with_child(Container::new(credits_text).with_margin_left(4.).finish())
        .finish();
    let cost_cluster = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(card_icon)
        .with_child(Container::new(cost_text).with_margin_left(4.).finish())
        .finish();

    let credits_and_cost = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(credits_cluster)
        .with_child(Container::new(cost_cluster).with_margin_left(6.).finish())
        .finish();

    let body = Container::new(
        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(Shrinkable::new(1., name_row.finish()).finish())
            .with_child(
                Container::new(credits_and_cost)
                    .with_margin_left(16.)
                    .finish(),
            )
            .finish(),
    )
    .with_uniform_padding(ROW_PADDING)
    .finish();

    Container::new(
        Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(bar)
            .with_child(body)
            .finish(),
    )
    .with_background_color(card_bg)
    .with_border(Border::all(ROW_BORDER_WIDTH).with_border_color(theme.outline().into_solid()))
    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(ROW_BORDER_RADIUS)))
    .finish()
}

/// Row card wrapped in a Hoverable that opens the breakdown tooltip.
fn render_member_row(
    row: &MemberUsageRow,
    team_max_credits: i64,
    tooltip_mouse_state: MouseStateHandle,
    appearance: &Appearance,
) -> Box<dyn Element> {
    // No segments => no tooltip needed.
    if row.segments.is_empty() {
        return build_row_card(row, team_max_credits, appearance);
    }

    Hoverable::new(tooltip_mouse_state, move |state| {
        let mut stack = Stack::new();
        stack.add_child(build_row_card(row, team_max_credits, appearance));

        if state.is_hovered() {
            stack.add_positioned_overlay_child(
                render_usage_tooltip_content(row, appearance),
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

/// Pill-shaped stacked progress bar for team-totals cards. Mirrors the admin
/// panel's `StackedProgressBar`: fills to `total_cost_cents / limit_cents`
/// when a limit is set (capped at 100%), otherwise fills 100% when there is
/// any usage. Empty slots render as a muted track.
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
        .with_uniform_padding(CARD_PADDING)
        .finish()
}

/// Card wrapped in a Hoverable that opens the breakdown tooltip when the card
/// has any segments.
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

/// Section subheader (e.g. "Team totals", "Member usage"). One step below
/// the v2 page's bold section title.
fn render_section_subheader(label: &str, appearance: &Appearance) -> Box<dyn Element> {
    Text::new_inline(label.to_string(), appearance.ui_font_family(), 14.)
        .with_color(appearance.theme().active_ui_text_color().into())
        .with_style(Properties::default().weight(Weight::Medium))
        .finish()
}

/// Horizontal row of team-totals cards (Overall + Local + Cloud).
fn render_team_totals_section(
    entries: &[BillingCycleUsageEntry],
    mouse_states: &RowMouseStates,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let summaries = build_team_total_card_summaries(entries);
    let mut row = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_main_axis_size(MainAxisSize::Max)
        .with_spacing(CARD_GAP);
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

/// Team-totals block (subheader + cards) for visibilities that surface team-
/// level usage. Skipped for `OwnOnly`; callers should gate on visibility
/// before invoking. `PerUserTotals` gets a "Team totals" subheader above
/// the cards to match the admin panel; `TeamAggregate` shows just the cards.
pub fn render_team_totals_block(
    entries: &[BillingCycleUsageEntry],
    visibility: &crate::workspaces::workspace::UsageVisibility,
    mouse_states: &RowMouseStates,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let mut column = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
    if visibility.granularity == UsageVisibilityGranularity::PerUserTotals {
        column.add_child(
            Container::new(render_section_subheader("Team totals", appearance))
                .with_margin_bottom(8.)
                .finish(),
        );
    }
    column.add_child(render_team_totals_section(
        entries,
        mouse_states,
        appearance,
    ));
    column.finish()
}

/// Drops legacy buckets we no longer charge for (Voice / SuggestedCodeDiffs).
/// Centralized here so the section view can filter once before fanning the
/// entries out to the team-totals block and the per-row renderer below.
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

pub type FilterChangeFn = std::sync::Arc<dyn Fn(SourceFilter, &mut EventContext) + 'static>;

/// All / Local / Cloud pill toggle.
fn render_source_filter_toggle(
    current: SourceFilter,
    mouse_states: &RowMouseStates,
    appearance: &Appearance,
    on_change: FilterChangeFn,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let bg = theme.surface_1();
    let main = blended_colors::text_main(theme, bg);
    let sub = blended_colors::text_sub(theme, bg);

    let options: [(SourceFilter, MouseStateHandle); 3] = [
        (SourceFilter::All, mouse_states.filter_all.clone()),
        (SourceFilter::Local, mouse_states.filter_local.clone()),
        (SourceFilter::Cloud, mouse_states.filter_cloud.clone()),
    ];

    let mut row = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_main_axis_size(MainAxisSize::Min);

    for (filter, mouse_state) in options {
        let label = filter.label();
        let is_selected = filter == current;
        let fg = if is_selected { main } else { sub };
        let font_family = appearance.ui_font_family();
        let on_change = on_change.clone();

        let cell = Hoverable::new(mouse_state, move |_state| {
            let mut cell = Container::new(
                Text::new_inline(label, font_family, 11.)
                    .with_color(fg)
                    .finish(),
            )
            .with_horizontal_padding(10.)
            .with_vertical_padding(4.);
            if is_selected {
                cell = cell.with_background(theme.surface_overlay_1());
            }
            cell.finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            on_change(filter, ctx);
        })
        .finish();

        row.add_child(cell);
    }

    Container::new(row.finish())
        .with_border(Border::all(1.).with_border_color(theme.surface_3().into_solid()))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
        .finish()
}

/// Resolves the current viewer's own usage row from the auth state, picking
/// up their display name and base credit limit from the workspace member list.
fn build_viewer_own_usage_row(
    workspace: &Workspace,
    entries: &[BillingCycleUsageEntry],
    app: &AppContext,
    source_filter: SourceFilter,
) -> MemberUsageRow {
    let auth_state = AuthStateProvider::as_ref(app).get();
    let viewer_uid = auth_state.user_id().map(|uid| uid.as_string());
    let display_name = auth_state
        .display_name()
        .or_else(|| auth_state.username_for_display())
        .or_else(|| auth_state.user_email())
        .unwrap_or_else(|| "Your usage".to_string());
    // Surface the viewer's own base limit so they see `used / limit`.
    let viewer_base_limit = viewer_uid.as_deref().and_then(|uid| {
        workspace
            .members
            .iter()
            .find(|m| m.uid.as_str() == uid)
            .and_then(member_base_limit)
    });
    build_own_usage_row(
        entries,
        viewer_uid.as_deref(),
        display_name,
        viewer_base_limit,
        source_filter,
    )
}

/// Top-level row dispatcher. Caller is expected to have already filtered
/// legacy buckets via [`filter_legacy_buckets`] and rendered the team-totals
/// block (if any) above. `on_filter_change` is only consumed by the
/// `PerUserTotals` branch.
#[allow(clippy::too_many_arguments)]
pub fn render_rows(
    workspace: &Workspace,
    entries: &[BillingCycleUsageEntry],
    visibility: &crate::workspaces::workspace::UsageVisibility,
    source_filter: SourceFilter,
    mouse_states: &RowMouseStates,
    appearance: &Appearance,
    app: &AppContext,
    on_filter_change: FilterChangeFn,
) -> Box<dyn Element> {
    let mut column = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_spacing(8.);

    match visibility.granularity {
        UsageVisibilityGranularity::OwnOnly => {
            let row = build_viewer_own_usage_row(workspace, entries, app, source_filter);
            let tooltip_state = mouse_states.tooltip_mouse_state(&row.subject_key);
            let max_credits = row.total_credits.max(1);
            column.add_child(render_member_row(
                &row,
                max_credits,
                tooltip_state,
                appearance,
            ));
        }
        UsageVisibilityGranularity::TeamAggregate => {
            let mut rows = vec![build_viewer_own_usage_row(
                workspace,
                entries,
                app,
                SourceFilter::All,
            )];
            if workspace.members.len() > 1
                || entries
                    .iter()
                    .any(|e| e.subject_type == AiCreditsUsageAndCostSubjectType::Team)
            {
                rows.push(build_other_members_usage_row(entries));
            }
            let max_credits = rows
                .iter()
                .map(|row| row.total_credits)
                .max()
                .unwrap_or(0)
                .max(1);
            for row in &rows {
                let tooltip_state = mouse_states.tooltip_mouse_state(&row.subject_key);
                column.add_child(render_member_row(
                    row,
                    max_credits,
                    tooltip_state,
                    appearance,
                ));
            }
        }
        UsageVisibilityGranularity::PerUserTotals => {
            let member_rows = build_member_usage_rows(entries, &workspace.members, source_filter);

            // Member usage subheader; source filter (if any cloud usage) lives
            // on the right of the same row.
            let mut member_header_row = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_main_axis_size(MainAxisSize::Max)
                .with_child(render_section_subheader("Member usage", appearance));
            if has_cloud_usage(entries) {
                member_header_row.add_child(render_source_filter_toggle(
                    source_filter,
                    mouse_states,
                    appearance,
                    on_filter_change,
                ));
            }
            column.add_child(
                Container::new(member_header_row.finish())
                    .with_margin_bottom(8.)
                    .finish(),
            );

            if member_rows.iter().all(|r| r.total_credits == 0) {
                column.add_child(render_empty_filter_state(source_filter, appearance));
            } else {
                // Member rows are scaled against the top individual member so
                // the heaviest user fills the bar and everyone else reads as a
                // fraction of that user. Team totals live in the cards above.
                let top_member_max = member_rows
                    .iter()
                    .map(|r| r.total_credits)
                    .max()
                    .unwrap_or(0)
                    .max(1);
                for row in &member_rows {
                    let tooltip_state = mouse_states.tooltip_mouse_state(&row.subject_key);
                    column.add_child(render_member_row(
                        row,
                        top_member_max,
                        tooltip_state,
                        appearance,
                    ));
                }
            }
        }
        UsageVisibilityGranularity::FullBreakdown => {
            // Enterprise admins short-circuit to the admin-panel CTA before
            // render_rows runs, so this branch is unreachable in practice.
            column.add_child(Empty::new().finish());
        }
    }

    column.finish()
}

fn render_empty_filter_state(
    source_filter: SourceFilter,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let bg = theme.background().into_solid();
    let text = match source_filter {
        SourceFilter::All => "No usage this period",
        SourceFilter::Local => "No local usage this period",
        SourceFilter::Cloud => "No cloud usage this period",
    };
    Container::new(
        Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::Center)
            .with_child(
                Text::new_inline(
                    text.to_string(),
                    appearance.ui_font_family(),
                    appearance.ui_font_size(),
                )
                .with_color(blended_colors::text_sub(theme, bg))
                .finish(),
            )
            .finish(),
    )
    .with_background_color(bg)
    .with_border(Border::all(1.).with_border_color(theme.outline().into_solid()))
    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(ROW_BORDER_RADIUS)))
    .with_vertical_padding(24.)
    .finish()
}
