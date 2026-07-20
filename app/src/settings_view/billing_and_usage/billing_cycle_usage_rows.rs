use std::collections::HashMap;

use itertools::Itertools as _;
use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use warp_core::channel::ChannelState;
use warp_core::ui::appearance::Appearance;
use warpui::elements::{
    Border, ChildAnchor, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, DropShadow,
    Empty, Expanded, Flex, Hoverable, MainAxisAlignment, MainAxisSize, MouseStateHandle,
    OffsetPositioning, ParentAnchor, ParentElement, ParentOffsetBounds, Radius, Shrinkable, Stack,
    Text,
};
use warpui::platform::Cursor;
use warpui::ui_components::components::UiComponent;
use warpui::{AppContext, Element, EventContext, SingletonEntity};

use crate::ai::AIRequestUsageModel;
use crate::auth::AuthStateProvider;
use crate::settings_view::billing_and_usage::billing_cycle_usage_common::{
    BarSegment, BillingUsageMouseStates, ROW_BORDER_RADIUS, ROW_BORDER_WIDTH, TOOLTIP_GAP,
    aggregate_segments, cost_type_color, format_cost_cents, format_credits,
    render_breakdown_tooltip, render_section_subheader,
};
use crate::ui_components::blended_colors;
use crate::ui_components::icons::Icon;
use crate::workspaces::workspace::{
    AiCreditsUsageAndCostSubjectType, AiCreditsUsageAndCostType, AiCreditsUsageBucket,
    AiCreditsUsageSource, BillingCycleUsageEntry, UsageVisibility, UsageVisibilityGranularity,
    Workspace, WorkspaceMember,
};

const BAR_HEIGHT: f32 = 8.;
const MIN_FILL_RATIO: f32 = 0.05;
/// Size of the leading icons in the row credit cluster (coin + credit-card).
const ROW_ICON_SIZE: f32 = 12.;
/// Inner radius so the bar's curve sits flush against the card's inner border.
const BAR_CORNER_RADIUS: f32 = ROW_BORDER_RADIUS - ROW_BORDER_WIDTH;
const ROW_PADDING: f32 = 12.;

const SELF_OWN_KEY: &str = "__self_own__";
const OTHER_MEMBERS_KEY: &str = "__other_members__";

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

/// Aggregated usage for one subject (or the synthetic team aggregate).
#[derive(Debug)]
pub struct MemberUsageRow {
    pub subject_type: AiCreditsUsageAndCostSubjectType,
    pub subject_key: String,
    /// Used to deep-link `ServiceAccount` rows to their Oz agent page.
    pub subject_uid: Option<String>,
    pub display_name: String,
    pub total_credits: i64,
    pub total_cost_cents: i64,
    /// Sorted by cost-type then bucket order; zero-credit entries dropped.
    pub segments: Vec<BarSegment>,
    /// Denominator the row's stacked bar fills against.
    pub bar_max_credits: i64,
}

fn viewer_identity(app: &AppContext) -> (Option<String>, String) {
    let auth_state = AuthStateProvider::as_ref(app).get();
    let viewer_uid = auth_state.user_id().map(|uid| uid.as_string());
    let display_name = auth_state
        .display_name()
        .or_else(|| auth_state.username_for_display())
        .or_else(|| auth_state.user_email())
        .unwrap_or_else(|| "Your usage".to_string());
    (viewer_uid, display_name)
}

struct GroupedSubjectUsage {
    subject_type: AiCreditsUsageAndCostSubjectType,
    display_name: String,
    entries: Vec<BillingCycleUsageEntry>,
}

impl MemberUsageRow {
    fn for_viewer(
        entries: &[BillingCycleUsageEntry],
        viewer_uid: Option<&str>,
        viewer_display_name: String,
        source_filter: SourceFilter,
    ) -> Self {
        let viewer_entries = entries
            .iter()
            .filter(|e| source_filter.matches(&e.usage_source))
            // Defensive: positive-attribute to the viewer only.
            .filter(|e| match (viewer_uid, e.subject_uid.as_deref()) {
                (Some(uid), Some(entry_uid)) => uid == entry_uid,
                _ => false,
            })
            .collect_vec();
        let (segments, total_credits, total_cost_cents) =
            aggregate_segments(viewer_entries.iter().copied());

        Self {
            subject_type: AiCreditsUsageAndCostSubjectType::User,
            subject_key: SELF_OWN_KEY.to_string(),
            subject_uid: viewer_uid.map(str::to_string),
            display_name: viewer_display_name,
            total_credits,
            total_cost_cents,
            segments,
            bar_max_credits: total_credits.max(1),
        }
    }

    /// Viewer row built from a raw used-credits count, with no segment
    /// breakdown. For callers that only have `AIRequestUsageModel`-style
    /// data (no `billing_cycle_usage` entries / no workspace data).
    fn for_viewer_from_total(
        viewer_uid: Option<String>,
        viewer_display_name: String,
        used: i64,
    ) -> Self {
        let segments = if used > 0 {
            vec![BarSegment {
                cost_type: AiCreditsUsageAndCostType::BaseLimit,
                usage_bucket: AiCreditsUsageBucket::Ai,
                credits: used,
                cost_cents: 0,
            }]
        } else {
            Vec::new()
        };
        Self {
            subject_type: AiCreditsUsageAndCostSubjectType::User,
            subject_key: SELF_OWN_KEY.to_string(),
            subject_uid: viewer_uid,
            display_name: viewer_display_name,
            total_credits: used,
            total_cost_cents: 0,
            segments,
            bar_max_credits: used.max(1),
        }
    }

    /// Synthetic "Other members" aggregate row used by TeamAggregate
    /// visibility — represents everyone except the viewer.
    fn for_other_members(entries: &[BillingCycleUsageEntry]) -> Self {
        let team_entries = entries
            .iter()
            .filter(|e| e.subject_type == AiCreditsUsageAndCostSubjectType::Team);
        let (segments, total_credits, total_cost_cents) = aggregate_segments(team_entries);

        Self {
            subject_type: AiCreditsUsageAndCostSubjectType::Team,
            subject_key: OTHER_MEMBERS_KEY.to_string(),
            subject_uid: None,
            display_name: "Other members".to_string(),
            total_credits,
            total_cost_cents,
            segments,
            bar_max_credits: total_credits.max(1),
        }
    }

    /// Per-member rows for `PerUserTotals` / `FullBreakdown` visibility.
    /// Iterates the workspace member list so zero-usage members still
    /// get a row. Service accounts and other non-member subjects surface
    /// as extra rows at the bottom, sorted by total credits desc.
    fn for_each_member(
        entries: &[BillingCycleUsageEntry],
        members: &[WorkspaceMember],
        source_filter: SourceFilter,
    ) -> Vec<Self> {
        // Group entries by subject for joining against the member list below.
        let mut grouped: HashMap<String, GroupedSubjectUsage> = HashMap::new();
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
            let group = grouped.entry(key).or_insert_with(|| GroupedSubjectUsage {
                subject_type: entry.subject_type.clone(),
                display_name: entry
                    .subject_display_name
                    .clone()
                    .unwrap_or_else(|| "Unknown".to_string()),
                entries: Vec::new(),
            });
            group.entries.push(entry.clone());
        }

        let mut rows: Vec<Self> = Vec::with_capacity(members.len());

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
                Some(group) => aggregate_segments(group.entries.iter()),
                None => (Vec::new(), 0, 0),
            };

            rows.push(Self {
                subject_type: AiCreditsUsageAndCostSubjectType::User,
                subject_key: key,
                subject_uid: Some(member.uid.as_str().to_string()),
                display_name: member.email.clone(),
                total_credits,
                total_cost_cents,
                segments,
                bar_max_credits: 0,
            });
        }

        // Subjects not in the member list (typically service accounts) render after.
        for (key, group) in grouped {
            if seen_keys.contains(&key) {
                continue;
            }
            // All entries in a group share the same subject_uid by construction
            // (it's part of the grouping key), so first.is representative.
            let subject_uid = group.entries.first().and_then(|e| e.subject_uid.clone());
            let (segments, total_credits, total_cost_cents) =
                aggregate_segments(group.entries.iter());
            rows.push(Self {
                subject_type: group.subject_type,
                subject_key: key,
                subject_uid,
                display_name: group.display_name,
                total_credits,
                total_cost_cents,
                segments,
                bar_max_credits: 0,
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
}

fn build_rows(
    workspace: &Workspace,
    entries: &[BillingCycleUsageEntry],
    visibility: &UsageVisibility,
    source_filter: SourceFilter,
    app: &AppContext,
) -> Vec<MemberUsageRow> {
    let mut rows: Vec<MemberUsageRow> = match visibility.granularity {
        UsageVisibilityGranularity::OwnOnly => {
            let (viewer_uid, display_name) = viewer_identity(app);
            vec![MemberUsageRow::for_viewer(
                entries,
                viewer_uid.as_deref(),
                display_name,
                source_filter,
            )]
        }
        UsageVisibilityGranularity::TeamAggregate => {
            // Force SourceFilter::All — TeamAggregate has no toggle.
            let (viewer_uid, display_name) = viewer_identity(app);
            let mut rows = vec![MemberUsageRow::for_viewer(
                entries,
                viewer_uid.as_deref(),
                display_name,
                SourceFilter::All,
            )];
            rows.push(MemberUsageRow::for_other_members(entries));
            rows
        }
        UsageVisibilityGranularity::PerUserTotals | UsageVisibilityGranularity::FullBreakdown => {
            MemberUsageRow::for_each_member(entries, &workspace.members, source_filter)
        }
    };

    if matches!(
        visibility.granularity,
        UsageVisibilityGranularity::PerUserTotals | UsageVisibilityGranularity::FullBreakdown
    ) {
        let top = rows
            .iter()
            .map(|r| r.total_credits)
            .max()
            .unwrap_or(0)
            .max(1);
        for row in &mut rows {
            row.bar_max_credits = top;
        }
    }

    rows
}

/// True if any entry is cloud-sourced; gates the source filter toggle.
pub fn has_cloud_usage(entries: &[BillingCycleUsageEntry]) -> bool {
    entries
        .iter()
        .any(|e| e.usage_source == AiCreditsUsageSource::Cloud)
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

/// Small text-only tooltip surfaced on hover of the service-account info
/// icon. Mirrors the visual treatment of `render_aggregate_legend_tooltip`.
fn render_service_account_info_tooltip(appearance: &Appearance) -> Box<dyn Element> {
    let theme = appearance.theme();
    let text = Text::new_inline(
        "This is an automated agent on your team.".to_string(),
        appearance.ui_font_family(),
        12.,
    )
    .with_color(theme.sub_text_color(theme.background()).into())
    .finish();
    Container::new(text)
        .with_background_color(theme.background().into_solid())
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
        .with_border(Border::all(1.).with_border_color(theme.outline().into_solid()))
        .with_horizontal_padding(12.)
        .with_vertical_padding(6.)
        .with_drop_shadow(
            DropShadow::new_with_standard_offset_and_spread(ColorU::new(0, 0, 0, 48))
                .with_offset(vec2f(0., 4.)),
        )
        .finish()
}

/// Renders one row card (stacked bar + name/totals).
fn render_row_card(
    row: &MemberUsageRow,
    team_max_credits: i64,
    mouse_states: &BillingUsageMouseStates,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let card_bg = theme.background().into_solid();
    let main = blended_colors::text_main(theme, card_bg);

    let bar = render_stacked_bar(
        &row.segments,
        row.total_credits,
        team_max_credits,
        appearance,
    );

    let is_service_account = matches!(
        row.subject_type,
        AiCreditsUsageAndCostSubjectType::ServiceAccount
    );
    // Service accounts with a known UID deep-link to their Oz agent page,
    // mirroring the web admin panel's `getOzAgentHref` behavior.
    let agent_href = if is_service_account {
        row.subject_uid.as_deref().map(|uid| {
            format!(
                "{}/agents/{}",
                ChannelState::oz_root_url(),
                urlencoding::encode(uid)
            )
        })
    } else {
        None
    };

    let display_name_element: Box<dyn Element> = if let Some(href) = agent_href {
        let link_state =
            mouse_states.tooltip_mouse_state(&format!("{}__agent_link", row.subject_key));
        appearance
            .ui_builder()
            .link(row.display_name.clone(), Some(href), None, link_state)
            .build()
            .finish()
    } else {
        Text::new_inline(
            row.display_name.clone(),
            appearance.ui_font_family(),
            appearance.ui_font_size(),
        )
        .with_color(main)
        .finish()
    };

    let mut name_row = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(display_name_element);

    if is_service_account {
        let info_state =
            mouse_states.tooltip_mouse_state(&format!("{}__agent_info", row.subject_key));
        let info_icon = Hoverable::new(info_state, move |state| {
            let info_color = appearance
                .theme()
                .sub_text_color(appearance.theme().background());
            let icon = ConstrainedBox::new(Icon::Info.to_warpui_icon(info_color).finish())
                .with_width(ROW_ICON_SIZE)
                .with_height(ROW_ICON_SIZE)
                .finish();
            let mut stack = Stack::new();
            stack.add_child(icon);
            if state.is_hovered() {
                stack.add_positioned_overlay_child(
                    render_service_account_info_tooltip(appearance),
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
        .finish();
        name_row.add_child(Container::new(info_icon).with_margin_left(6.).finish());
    }

    let credits_text = Text::new_inline(
        format_credits(row.total_credits),
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
    mouse_states: &BillingUsageMouseStates,
    appearance: &Appearance,
) -> Box<dyn Element> {
    // No segments => no tooltip needed.
    if row.segments.is_empty() {
        return render_row_card(row, team_max_credits, mouse_states, appearance);
    }

    // The info icon sits inside the row card, so hovering it would otherwise
    // trigger both this row's breakdown tooltip and the icon's own tooltip
    // on top of each other. Pull the icon's hover state up so we can
    // suppress the breakdown tooltip while the icon is hovered.
    let info_state = matches!(
        row.subject_type,
        AiCreditsUsageAndCostSubjectType::ServiceAccount
    )
    .then(|| mouse_states.tooltip_mouse_state(&format!("{}__agent_info", row.subject_key)));

    Hoverable::new(tooltip_mouse_state, move |state| {
        let mut stack = Stack::new();
        stack.add_child(render_row_card(
            row,
            team_max_credits,
            mouse_states,
            appearance,
        ));

        let info_hovered = info_state
            .as_ref()
            .is_some_and(|s| s.lock().is_ok_and(|guard| guard.is_hovered()));

        if state.is_hovered() && !info_hovered {
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

pub type FilterChangeFn = std::sync::Arc<dyn Fn(SourceFilter, &mut EventContext) + 'static>;

/// All / Local / Cloud pill toggle.
fn render_source_filter_toggle(
    current: SourceFilter,
    mouse_states: &BillingUsageMouseStates,
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

pub fn render_own_usage_with_workspace_row(
    entries: &[BillingCycleUsageEntry],
    mouse_states: &BillingUsageMouseStates,
    appearance: &Appearance,
    app: &AppContext,
) -> Box<dyn Element> {
    let (viewer_uid, display_name) = viewer_identity(app);
    let row = MemberUsageRow::for_viewer(
        entries,
        viewer_uid.as_deref(),
        display_name,
        SourceFilter::All,
    );
    render_member_row_list(std::slice::from_ref(&row), mouse_states, appearance)
}

pub fn render_own_usage_solo_row(
    mouse_states: &BillingUsageMouseStates,
    appearance: &Appearance,
    app: &AppContext,
) -> Box<dyn Element> {
    let (viewer_uid, display_name) = viewer_identity(app);
    let model = AIRequestUsageModel::as_ref(app);
    let row = MemberUsageRow::for_viewer_from_total(
        viewer_uid,
        display_name,
        model.requests_used() as i64,
    );
    render_member_row_list(std::slice::from_ref(&row), mouse_states, appearance)
}

#[allow(clippy::too_many_arguments)]
pub fn render_rows(
    workspace: &Workspace,
    entries: &[BillingCycleUsageEntry],
    visibility: &UsageVisibility,
    source_filter: SourceFilter,
    mouse_states: &BillingUsageMouseStates,
    appearance: &Appearance,
    app: &AppContext,
    on_filter_change: FilterChangeFn,
) -> Box<dyn Element> {
    let rows = build_rows(workspace, entries, visibility, source_filter, app);

    let mut column = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_spacing(8.);
    if let Some(header) = render_member_header(
        visibility,
        entries,
        source_filter,
        mouse_states,
        appearance,
        on_filter_change,
    ) {
        column.add_child(header);
    }
    column.add_child(render_member_row_list(&rows, mouse_states, appearance));
    column.finish()
}

fn render_member_header(
    visibility: &UsageVisibility,
    entries: &[BillingCycleUsageEntry],
    source_filter: SourceFilter,
    mouse_states: &BillingUsageMouseStates,
    appearance: &Appearance,
    on_filter_change: FilterChangeFn,
) -> Option<Box<dyn Element>> {
    let show_toggle = visibility.granularity == UsageVisibilityGranularity::FullBreakdown
        && has_cloud_usage(entries);

    let subheader = render_section_subheader("Members", appearance);
    let header = if show_toggle {
        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(subheader)
            .with_child(render_source_filter_toggle(
                source_filter,
                mouse_states,
                appearance,
                on_filter_change,
            ))
            .finish()
    } else {
        subheader
    };

    Some(Container::new(header).with_margin_bottom(8.).finish())
}

fn render_member_row_list(
    rows: &[MemberUsageRow],
    mouse_states: &BillingUsageMouseStates,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let mut column = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_spacing(8.);
    for row in rows {
        let tooltip_state = mouse_states.tooltip_mouse_state(&row.subject_key);
        column.add_child(render_member_row(
            row,
            row.bar_max_credits,
            tooltip_state,
            mouse_states,
            appearance,
        ));
    }
    column.finish()
}

#[cfg(test)]
#[path = "billing_cycle_usage_rows_tests.rs"]
mod tests;
