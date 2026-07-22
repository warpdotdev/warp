use chrono::{DateTime, Datelike, Local, Utc};
use markdown_parser::{FormattedText, FormattedTextFragment, FormattedTextLine};
use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use warp_core::ui::appearance::Appearance;
use warpui::elements::{
    Border, ChildAnchor, ChildView, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment,
    DropShadow, Empty, Flex, FormattedTextElement, HighlightedHyperlink, Hoverable, HyperlinkLens,
    MainAxisAlignment, MainAxisSize, MouseStateHandle, OffsetPositioning, ParentAnchor,
    ParentElement, ParentOffsetBounds, Radius, Stack, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::platform::Cursor;
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::ai::AIRequestUsageModel;
use crate::auth::{AuthManager, AuthStateProvider};
use crate::menu::{self, Menu, MenuItem, MenuItemFields};
use crate::settings_view::admin_actions::AdminActions;
use crate::settings_view::billing_and_usage::billing_cycle_usage_common::{
    BillingUsageMouseStates, filter_legacy_buckets, has_non_viewer_data, legend_cost_types,
};
use crate::settings_view::billing_and_usage::billing_cycle_usage_rows::{
    SourceFilter, has_cloud_usage, render_own_usage_solo_row, render_own_usage_with_workspace_row,
    render_rows,
};
use crate::settings_view::billing_and_usage::billing_cycle_usage_team_totals::render_team_totals_block;
use crate::settings_view::billing_and_usage_page_v2::{
    AGGREGATE_CREDITS_DOT_COLOR, AMBIENT_CREDITS_DOT_COLOR, BASE_CREDITS_DOT_COLOR,
    BONUS_CREDITS_DOT_COLOR, PAYG_CREDITS_DOT_COLOR,
};
use crate::ui_components::icons::Icon;
use crate::workspaces::update_manager::TeamUpdateManager;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::workspaces::workspace::{
    AiCreditsUsageAndCostType, BillingCycleUsageSummary, MaxPriorCycles, UsageVisibility,
    UsageVisibilityGranularity, Workspace,
};

const HEADER_FONT_SIZE: f32 = 16.;
const LEGEND_DOT_SIZE: f32 = 8.;

pub struct BillingCycleUsageSectionView {
    selected_period_end: Option<DateTime<Utc>>,
    period_selector_mouse_state: MouseStateHandle,
    aggregate_legend_mouse_state: MouseStateHandle,
    period_menu: ViewHandle<Menu<BillingCycleUsageAction>>,
    period_menu_open: bool,
    source_filter: SourceFilter,
    row_mouse_states: BillingUsageMouseStates,
}

#[derive(Clone, Debug)]
pub enum BillingCycleUsageAction {
    SelectPeriod(Option<DateTime<Utc>>),
    TogglePeriodMenu,
    ChangeSourceFilter(SourceFilter),
    OpenUpgrade,
    OpenAdminPanel,
}

impl Entity for BillingCycleUsageSectionView {
    type Event = ();
}

impl BillingCycleUsageSectionView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        ctx.subscribe_to_model(&UserWorkspaces::handle(ctx), |me, _, _, ctx| {
            me.reconcile_selected_period(ctx);
            // If the period menu is open while the workspace or usage data
            // changes, the menu's items become stale and clicking one could
            // select a period_end that no longer exists in the new data
            // (which `current_summary` would then fail to resolve). Rebuild
            // the items in-place so the menu always reflects the live data.
            if me.period_menu_open {
                me.refresh_period_menu_items(ctx);
            }
            ctx.notify();
        });
        ctx.subscribe_to_model(&AIRequestUsageModel::handle(ctx), |_, _, _, ctx| {
            ctx.notify()
        });
        ctx.subscribe_to_model(&AuthManager::handle(ctx), |_, _, _, ctx| ctx.notify());
        ctx.subscribe_to_model(&TeamUpdateManager::handle(ctx), |_, _, _, ctx| ctx.notify());

        // `prevent_interaction_with_other_elements` so a click on the
        // trigger button while the menu is open is consumed by the menu's
        // outside-click dismiss handler — without it, the trigger also
        // received the click and immediately re-toggled the menu open.
        let period_menu = ctx.add_typed_action_view(|_| {
            Menu::new()
                .with_drop_shadow()
                .prevent_interaction_with_other_elements()
        });
        ctx.subscribe_to_view(&period_menu, |me, _, event, ctx| {
            if let menu::Event::Close { .. } = event {
                me.period_menu_open = false;
                ctx.notify();
            }
        });

        Self {
            selected_period_end: None,
            period_selector_mouse_state: MouseStateHandle::default(),
            aggregate_legend_mouse_state: MouseStateHandle::default(),
            period_menu,
            period_menu_open: false,
            source_filter: SourceFilter::default(),
            row_mouse_states: BillingUsageMouseStates::default(),
        }
    }

    fn resolved_viewer_email(app: &AppContext) -> Option<String> {
        AuthStateProvider::as_ref(app).get().user_email()
    }

    fn viewer_is_admin(app: &AppContext) -> bool {
        let Some(team) = UserWorkspaces::as_ref(app).current_team() else {
            return false;
        };
        Self::resolved_viewer_email(app)
            .as_deref()
            .is_some_and(|email| team.has_admin_permissions(email))
    }

    fn current_summary<'a>(
        &self,
        workspace: &'a Workspace,
    ) -> Option<&'a BillingCycleUsageSummary> {
        let summaries = &workspace.billing_cycle_usage.as_ref()?.summaries;
        match self.selected_period_end {
            Some(end) => summaries.iter().find(|s| s.period_end == end),
            None => summaries.first(),
        }
    }

    fn reconcile_selected_period(&mut self, ctx: &AppContext) {
        let Some(selected) = self.selected_period_end else {
            return;
        };
        let still_present = UserWorkspaces::as_ref(ctx)
            .current_workspace()
            .and_then(|ws| ws.billing_cycle_usage.as_ref())
            .map(|data| data.summaries.iter().any(|s| s.period_end == selected))
            .unwrap_or(false);
        if !still_present {
            self.selected_period_end = None;
        }
    }

    /// Whether the "Team" block + "Members" subheader should render. We
    /// hide them when the viewer has no team data to show: `members.len()
    /// > 1` covers the common multi-member case; `has_non_viewer_data`
    /// catches the edge case where the roster shrank to one after a
    /// teammate left mid-cycle but their usage is still attributed against
    /// this cycle. Together they keep solo teams from showing orphan
    /// scaffolding without dropping legitimate team data on departure.
    ///
    /// Note: per the backend invariant `VIS != OwnOnly => viewer is admin`,
    /// so we don't need a separate admin gate here.
    fn shows_team_section(&self, workspace: &Workspace, app: &AppContext) -> bool {
        let visibility = workspace.resolve_usage_visibility(Self::viewer_is_admin(app));
        if visibility.granularity == UsageVisibilityGranularity::OwnOnly {
            return false;
        }
        let entries = filter_legacy_buckets(
            self.current_summary(workspace)
                .map(|s| s.entries.as_slice())
                .unwrap_or_default(),
        );
        let viewer_uid = AuthStateProvider::as_ref(app)
            .get()
            .user_id()
            .map(|uid| uid.as_string());
        workspace.members.len() > 1 || has_non_viewer_data(&entries, viewer_uid.as_deref())
    }
}

impl TypedActionView for BillingCycleUsageSectionView {
    type Action = BillingCycleUsageAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            BillingCycleUsageAction::SelectPeriod(period_end) => {
                self.selected_period_end = *period_end;
                self.period_menu_open = false;
                ctx.notify();
            }
            BillingCycleUsageAction::TogglePeriodMenu => {
                self.period_menu_open = !self.period_menu_open;
                if self.period_menu_open {
                    self.refresh_period_menu_items(ctx);
                }
                ctx.notify();
            }
            BillingCycleUsageAction::ChangeSourceFilter(filter) => {
                self.source_filter = *filter;
                ctx.notify();
            }
            BillingCycleUsageAction::OpenUpgrade => {
                if let Some(team_uid) = UserWorkspaces::as_ref(ctx).current_team_uid() {
                    ctx.open_url(&UserWorkspaces::upgrade_link_for_team(team_uid));
                }
            }
            BillingCycleUsageAction::OpenAdminPanel => {
                if let Some(team_uid) = UserWorkspaces::as_ref(ctx).current_team_uid() {
                    AdminActions::open_admin_panel(team_uid, ctx);
                }
            }
        }
    }
}

impl BillingCycleUsageSectionView {
    fn refresh_period_menu_items(&self, ctx: &mut ViewContext<Self>) {
        let Some(workspace) = UserWorkspaces::as_ref(ctx).current_workspace().cloned() else {
            return;
        };
        let Some(data) = workspace.billing_cycle_usage.as_ref() else {
            return;
        };
        let items = build_period_menu_items(&data.summaries);
        let selected_index = selected_period_index(&data.summaries, self.selected_period_end);

        self.period_menu
            .update(ctx, |menu: &mut Menu<BillingCycleUsageAction>, ctx| {
                menu.set_items(items, ctx);
                if let Some(index) = selected_index {
                    menu.set_selected_by_index(index, ctx);
                }
            });
    }
}

impl View for BillingCycleUsageSectionView {
    fn ui_name() -> &'static str {
        "BillingCycleUsageSection"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let workspace = UserWorkspaces::as_ref(app).current_workspace().cloned();
        match workspace.as_ref() {
            Some(w) if self.shows_team_section(w, app) => {
                self.render_team_usage(w, appearance, app)
            }
            Some(w) => self.render_own_usage_with_workspace(w, appearance, app),
            None => self.render_own_usage_solo(appearance, app),
        }
    }
}

impl BillingCycleUsageSectionView {
    fn render_team_usage(
        &self,
        workspace: &Workspace,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let is_admin = Self::viewer_is_admin(app);
        let visibility = workspace.resolve_usage_visibility(is_admin);

        let mut column = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
        column.add_child(self.render_header(Some(workspace), &visibility, appearance, app));

        let entries = filter_legacy_buckets(
            self.current_summary(workspace)
                .map(|summary| summary.entries.as_slice())
                .unwrap_or_default(),
        );

        let is_source_filter_shown = visibility.granularity
            == UsageVisibilityGranularity::FullBreakdown
            && has_cloud_usage(&entries);
        let source_filter = if is_source_filter_shown {
            self.source_filter
        } else {
            SourceFilter::All
        };

        column.add_child(
            Container::new(render_team_totals_block(
                &entries,
                &visibility,
                &self.row_mouse_states,
                appearance,
            ))
            .with_margin_top(16.)
            .finish(),
        );

        if is_admin && let Some(banner) = self.render_visibility_cta_banner(workspace, appearance) {
            column.add_child(Container::new(banner).with_margin_top(16.).finish());
        }

        column.add_child(
            Container::new(render_rows(
                workspace,
                &entries,
                &visibility,
                source_filter,
                &self.row_mouse_states,
                appearance,
                app,
                std::sync::Arc::new(|filter, ctx| {
                    ctx.dispatch_typed_action(BillingCycleUsageAction::ChangeSourceFilter(filter));
                }),
            ))
            .with_margin_top(16.)
            .finish(),
        );

        column.finish()
    }

    fn render_own_usage_with_workspace(
        &self,
        workspace: &Workspace,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let visibility = workspace.resolve_usage_visibility(Self::viewer_is_admin(app));
        let entries = filter_legacy_buckets(
            self.current_summary(workspace)
                .map(|s| s.entries.as_slice())
                .unwrap_or_default(),
        );

        let mut column = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
        column.add_child(self.render_header(Some(workspace), &visibility, appearance, app));
        column.add_child(
            Container::new(render_own_usage_with_workspace_row(
                &entries,
                &self.row_mouse_states,
                appearance,
                app,
            ))
            .with_margin_top(16.)
            .finish(),
        );
        column.finish()
    }

    // Here when you're not on a team, there's no workspace to pull billing_cycle_usage data from.
    // So we "fake" a row and source data from the AIRequestUsageModel instead
    fn render_own_usage_solo(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let mut column = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
        column.add_child(self.render_header(None, &UsageVisibility::default(), appearance, app));
        column.add_child(
            Container::new(render_own_usage_solo_row(
                &self.row_mouse_states,
                appearance,
                app,
            ))
            .with_margin_top(16.)
            .finish(),
        );
        column.finish()
    }
}

impl BillingCycleUsageSectionView {
    fn render_header(
        &self,
        workspace: Option<&Workspace>,
        visibility: &UsageVisibility,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let mut row = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max);

        row.add_child(
            Text::new_inline("Usage", appearance.ui_font_family(), HEADER_FONT_SIZE)
                .with_style(Properties::default().weight(Weight::Bold))
                .with_color(theme.active_ui_text_color().into())
                .finish(),
        );

        let mut right_side = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::End);

        // Collapse to a static label when there's effectively one period to
        // pick from: either the tier policy doesn't expose history at all, or
        // the server returned a single canonical cycle.
        if let Some(workspace) = workspace {
            let summary_count = workspace
                .billing_cycle_usage
                .as_ref()
                .map(|d| d.summaries.len())
                .unwrap_or(0);
            let use_selector =
                visibility.max_prior_cycles != MaxPriorCycles::None && summary_count > 1;
            let period_element = if use_selector {
                self.render_period_selector(workspace, appearance)
            } else {
                self.render_period_range_static(workspace, appearance)
            };
            right_side.add_child(period_element);
        }

        row.add_child(right_side.finish());

        let mut column = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
        column.add_child(row.finish());

        let resets_text = self.render_resets_label(appearance, app);
        let legend = workspace.and_then(|workspace| self.render_legend(workspace, appearance));
        if resets_text.is_some() || legend.is_some() {
            let mut secondary_row = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_main_axis_size(MainAxisSize::Max);
            secondary_row.add_child(resets_text.unwrap_or_else(|| Empty::new().finish()));
            secondary_row.add_child(legend.unwrap_or_else(|| Empty::new().finish()));
            column.add_child(
                Container::new(secondary_row.finish())
                    .with_margin_top(4.)
                    .finish(),
            );
        }

        Container::new(column.finish()).finish()
    }

    /// "Resets May 27, 11:24 PM EDT"
    fn render_resets_label(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Option<Box<dyn Element>> {
        if self.selected_period_end.is_some() {
            return None;
        }
        let theme = appearance.theme();
        let reset_str = AIRequestUsageModel::as_ref(app)
            .next_refresh_time_local()
            .format("Resets %b %d, %-I:%M %p")
            .to_string();
        Some(
            Text::new_inline(
                reset_str,
                appearance.ui_font_family(),
                appearance.ui_font_size(),
            )
            .with_color(theme.sub_text_color(theme.background()).into())
            .finish(),
        )
    }

    // "May 13 - Jun 13, 2026"
    fn render_period_range_static(
        &self,
        workspace: &Workspace,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let label = self
            .current_summary(workspace)
            .map(|s| format_period_range(s.period_start, s.period_end))
            .or_else(|| {
                workspace.billing_cycle_usage.as_ref().map(|data| {
                    format_period_range(data.current_period_start, data.current_period_end)
                })
            })
            .unwrap_or_default();
        Text::new_inline(
            label,
            appearance.ui_font_family(),
            appearance.ui_font_size(),
        )
        .with_color(theme.sub_text_color(theme.background()).into())
        .finish()
    }

    fn render_period_selector(
        &self,
        workspace: &Workspace,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let bg = theme.background();
        let label = self
            .current_summary(workspace)
            .map(|s| format_period_range(s.period_start, s.period_end))
            .unwrap_or_default();

        let mouse_state = self.period_selector_mouse_state.clone();
        let font_family = appearance.ui_font_family();
        let font_size = appearance.ui_font_size();
        let main_text = theme.sub_text_color(bg);

        let button = Hoverable::new(mouse_state, move |_| {
            let mut inner = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_main_axis_size(MainAxisSize::Min);
            inner.add_child(
                Text::new_inline(label.clone(), font_family, font_size)
                    .with_color(main_text.into())
                    .finish(),
            );
            inner.add_child(
                Container::new(
                    ConstrainedBox::new(Icon::ChevronDown.to_warpui_icon(main_text).finish())
                        .with_width(12.)
                        .with_height(12.)
                        .finish(),
                )
                .with_margin_left(4.)
                .finish(),
            );
            inner.finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(BillingCycleUsageAction::TogglePeriodMenu);
        })
        .finish();

        let mut stack = Stack::new();
        stack.add_child(button);
        if self.period_menu_open {
            stack.add_positioned_overlay_child(
                ChildView::new(&self.period_menu).finish(),
                OffsetPositioning::offset_from_parent(
                    vec2f(0., 4.),
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::BottomRight,
                    ChildAnchor::TopRight,
                ),
            );
        }
        stack.finish()
    }

    fn render_legend(
        &self,
        workspace: &Workspace,
        appearance: &Appearance,
    ) -> Option<Box<dyn Element>> {
        let summary = self.current_summary(workspace)?;
        // Only list buckets that actually contribute to the stacked bars: drop
        // legacy buckets and cost types with no usage, so the legend never
        // shows a bucket (e.g. "Base") that has zero credits in the data.
        let present_buckets = legend_cost_types(&summary.entries);
        if present_buckets.is_empty() {
            return None;
        }

        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Min);
        for (idx, bucket) in present_buckets.iter().enumerate() {
            if idx > 0 {
                row.add_child(
                    Container::new(Empty::new().finish())
                        .with_margin_right(12.)
                        .finish(),
                );
            }
            row.add_child(self.render_legend_entry(bucket.clone(), appearance));
        }
        Some(row.finish())
    }

    fn render_legend_entry(
        &self,
        cost_type: AiCreditsUsageAndCostType,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let (color, label) = legend_style_for(cost_type.clone());
        let theme = appearance.theme();
        let entry = {
            let mut row = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_main_axis_size(MainAxisSize::Min);
            row.add_child(
                ConstrainedBox::new(
                    Container::new(Empty::new().finish())
                        .with_background_color(color)
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                            LEGEND_DOT_SIZE / 2.,
                        )))
                        .finish(),
                )
                .with_height(LEGEND_DOT_SIZE)
                .with_width(LEGEND_DOT_SIZE)
                .finish(),
            );
            row.add_child(
                Container::new(
                    Text::new_inline(
                        label,
                        appearance.ui_font_family(),
                        appearance.ui_font_size(),
                    )
                    .with_color(theme.sub_text_color(theme.background()).into())
                    .finish(),
                )
                .with_margin_left(6.)
                .finish(),
            );
            row.finish()
        };

        // The Aggregate bucket replaces per-cost-type detail with a single
        // "Combined" row, which isn't self-explanatory; surface a small
        // hover tooltip clarifying what it includes.
        if !matches!(cost_type, AiCreditsUsageAndCostType::Aggregate) {
            return entry;
        }

        let mouse_state = self.aggregate_legend_mouse_state.clone();
        Hoverable::new(mouse_state, move |state| {
            let mut stack = Stack::new();
            stack.add_child(entry);
            if state.is_hovered() {
                stack.add_positioned_overlay_child(
                    render_aggregate_legend_tooltip(appearance),
                    OffsetPositioning::offset_from_parent(
                        vec2f(0., 6.),
                        ParentOffsetBounds::WindowByPosition,
                        ParentAnchor::BottomMiddle,
                        ChildAnchor::TopMiddle,
                    ),
                );
            }
            stack.finish()
        })
        .finish()
    }

    /// Renders the CTA banner that sits between the team-totals block and
    /// the per-member rows. The copy and action vary by visibility tier:
    /// non-FullBreakdown admins see an upgrade nudge; FullBreakdown admins
    /// see a pointer to the admin panel where per-user spend limits actually
    /// get configured.
    fn render_visibility_cta_banner(
        &self,
        workspace: &Workspace,
        appearance: &Appearance,
    ) -> Option<Box<dyn Element>> {
        let admin_granularity = workspace
            .billing_metadata
            .tier
            .usage_visibility_policy?
            .admin_granularity;
        if admin_granularity == UsageVisibilityGranularity::FullBreakdown
            && !workspace.billing_metadata.is_enterprise_plan()
        {
            return None;
        }
        let (link_text, trailing_copy, action, leading_icon) =
            visibility_cta_for(admin_granularity)?;

        // Only show when there are teammates -- a single-member workspace
        // doesn't benefit from any of the team-level visibility CTAs.
        if workspace.members.len() <= 1 {
            return None;
        }

        let theme = appearance.theme();
        let sub_text = theme.sub_text_color(theme.background());
        let body = FormattedTextElement::new(
            FormattedText::new([FormattedTextLine::Line(vec![
                FormattedTextFragment::hyperlink_action(link_text, action),
                FormattedTextFragment::plain_text(format!(" {trailing_copy}")),
            ])]),
            appearance.ui_font_size(),
            appearance.ui_font_family(),
            appearance.ui_font_family(),
            sub_text.into(),
            HighlightedHyperlink::default(),
        )
        .with_hyperlink_font_color(theme.accent().into_solid())
        .register_default_click_handlers_with_action_support(|lens, event, ctx| match lens {
            HyperlinkLens::Url(u) => ctx.open_url(u),
            HyperlinkLens::Action(a) => {
                if let Some(act) = a.as_any().downcast_ref::<BillingCycleUsageAction>() {
                    event.dispatch_typed_action(act.clone());
                }
            }
        })
        .finish();

        let icon = ConstrainedBox::new(leading_icon.to_warpui_icon(sub_text).finish())
            .with_width(14.)
            .with_height(14.)
            .finish();

        let row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(Container::new(icon).with_margin_right(8.).finish())
            .with_child(body)
            .finish();

        Some(
            Container::new(row)
                .with_background_color(theme.surface_1().into_solid())
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
                .with_uniform_padding(12.)
                .finish(),
        )
    }
}

/// Returns the (link text, trailing copy, action, icon) tuple for the
/// visibility CTA banner, or `None` to suppress the banner entirely.
fn visibility_cta_for(
    granularity: UsageVisibilityGranularity,
) -> Option<(&'static str, &'static str, BillingCycleUsageAction, Icon)> {
    match granularity {
        UsageVisibilityGranularity::OwnOnly => Some((
            "Upgrade to Build",
            "to see team-level credit usage.",
            BillingCycleUsageAction::OpenUpgrade,
            Icon::ArrowCircleBrokenUp,
        )),
        UsageVisibilityGranularity::TeamAggregate => Some((
            "Upgrade to Business",
            "to see per-user credit attribution.",
            BillingCycleUsageAction::OpenUpgrade,
            Icon::ArrowCircleBrokenUp,
        )),
        UsageVisibilityGranularity::PerUserTotals => Some((
            "Upgrade to Enterprise",
            "to see fine-grained credit attribution and set per-user spend limits.",
            BillingCycleUsageAction::OpenUpgrade,
            Icon::ArrowCircleBrokenUp,
        )),
        // FullBreakdown viewers already have full visibility; nudge them to
        // the admin panel where per-user spend limits actually get configured.
        UsageVisibilityGranularity::FullBreakdown => Some((
            "Open the admin panel",
            "to set per-user spend limits.",
            BillingCycleUsageAction::OpenAdminPanel,
            Icon::Users,
        )),
    }
}

fn legend_style_for(cost_type: AiCreditsUsageAndCostType) -> (ColorU, &'static str) {
    match cost_type {
        AiCreditsUsageAndCostType::BaseLimit => (BASE_CREDITS_DOT_COLOR, "Base"),
        AiCreditsUsageAndCostType::BonusGrant => (BONUS_CREDITS_DOT_COLOR, "Add-ons"),
        AiCreditsUsageAndCostType::Payg => (PAYG_CREDITS_DOT_COLOR, "Pay-as-you-go"),
        AiCreditsUsageAndCostType::AmbientBonusGrant => (AMBIENT_CREDITS_DOT_COLOR, "Cloud-only"),
        AiCreditsUsageAndCostType::Aggregate => (AGGREGATE_CREDITS_DOT_COLOR, "Combined"),
        AiCreditsUsageAndCostType::Other(_) => (BASE_CREDITS_DOT_COLOR, ""),
    }
}

fn render_aggregate_legend_tooltip(appearance: &Appearance) -> Box<dyn Element> {
    let theme = appearance.theme();
    let text = Text::new_inline(
        "Other team members' usage across add-on, pay-as-you-go, and cloud-only credits."
            .to_string(),
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

fn format_period_range(start: DateTime<Utc>, end: DateTime<Utc>) -> String {
    let start = start.with_timezone(&Local);
    let end = end.with_timezone(&Local);
    if start.year() == end.year() {
        format!("{} - {}", start.format("%b %d"), end.format("%b %d, %Y"))
    } else {
        format!(
            "{} - {}",
            start.format("%b %d, %Y"),
            end.format("%b %d, %Y")
        )
    }
}

fn build_period_menu_items(
    summaries: &[BillingCycleUsageSummary],
) -> Vec<MenuItem<BillingCycleUsageAction>> {
    summaries
        .iter()
        .map(|summary| {
            let label = format_period_range(summary.period_start, summary.period_end);
            MenuItem::Item(MenuItemFields::new(label).with_on_select_action(
                BillingCycleUsageAction::SelectPeriod(Some(summary.period_end)),
            ))
        })
        .collect()
}

fn selected_period_index(
    summaries: &[BillingCycleUsageSummary],
    selected_period_end: Option<DateTime<Utc>>,
) -> Option<usize> {
    if summaries.is_empty() {
        return None;
    }
    match selected_period_end {
        Some(end) => summaries.iter().position(|s| s.period_end == end),
        None => Some(0),
    }
}

#[cfg(test)]
#[path = "billing_cycle_usage_section_tests.rs"]
mod tests;
