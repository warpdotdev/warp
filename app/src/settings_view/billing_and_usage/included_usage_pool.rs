use chrono::Local;
use warp_core::ui::appearance::Appearance;
use warp_graphql::queries::get_request_limit_info::WorkspaceIncludedUsageInfo;
use warpui::elements::Empty;
use warpui::{AppContext, Element, Entity, SingletonEntity, View, ViewContext};

use crate::ai::AIRequestUsageModel;
use crate::ai::blocklist::view_util::usage_display_unit;
use crate::settings::ai::{AISettings, AISettingsChangedEvent, UsageDisplayUnit};
use crate::settings_view::billing_and_usage_page_v2::{
    BASE_CREDITS_DOT_COLOR, BalanceAmount, render_balance_card,
};
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::workspaces::workspace::{CustomerType, Workspace};

pub(crate) fn is_scale_workspace(workspace: Option<&Workspace>) -> bool {
    workspace
        .is_some_and(|workspace| workspace.billing_metadata.customer_type == CustomerType::Scale)
}

pub struct IncludedUsagePoolView;

impl IncludedUsagePoolView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        ctx.subscribe_to_model(&AISettings::handle(ctx), |_, _, event, ctx| {
            if matches!(event, AISettingsChangedEvent::UsageDisplayUnit { .. }) {
                ctx.notify();
            }
        });
        ctx.subscribe_to_model(&AIRequestUsageModel::handle(ctx), |_, _, _, ctx| {
            ctx.notify();
        });
        ctx.subscribe_to_model(&UserWorkspaces::handle(ctx), |_, _, _, ctx| {
            ctx.notify();
        });
        Self
    }
}

impl Entity for IncludedUsagePoolView {
    type Event = ();
}

impl View for IncludedUsagePoolView {
    fn ui_name() -> &'static str {
        "Included team usage"
    }

    fn render(&self, ctx: &AppContext) -> Box<dyn Element> {
        match AIRequestUsageModel::as_ref(ctx).workspace_included_usage(ctx) {
            Some(info) => render_card(info, usage_display_unit(ctx), Appearance::as_ref(ctx)),
            None => Empty::new().finish(),
        }
    }
}

#[derive(Debug, PartialEq)]
struct PoolBalance {
    remaining_credits: f64,
    monthly_credits: f64,
    remaining_cents: f64,
    monthly_cents: f64,
    reset: String,
}

fn pool_balance(pool: &WorkspaceIncludedUsageInfo) -> PoolBalance {
    PoolBalance {
        remaining_credits: (pool.included_usage_credits
            - pool.usage_credits_used_since_last_refresh)
            .max(0.0),
        monthly_credits: pool.included_usage_credits,
        remaining_cents: (pool.included_usage_cents - pool.usage_cents_used_since_last_refresh)
            .max(0.0),
        monthly_cents: pool.included_usage_cents,
        reset: pool
            .next_refresh_time
            .utc()
            .with_timezone(&Local)
            .format("Resets %b %d at %-I:%M %p")
            .to_string(),
    }
}

fn render_card(
    info: &WorkspaceIncludedUsageInfo,
    unit: UsageDisplayUnit,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let pool = pool_balance(info);
    let (label, remaining, monthly) = match unit {
        UsageDisplayUnit::Credits => (
            "Included team credits",
            BalanceAmount::RoundedCredits(pool.remaining_credits),
            BalanceAmount::Credits(pool.monthly_credits.round() as i64),
        ),
        UsageDisplayUnit::Dollars => (
            "Included team usage",
            BalanceAmount::Cents(pool.remaining_cents),
            BalanceAmount::Cents(pool.monthly_cents),
        ),
    };
    render_balance_card(
        appearance,
        BASE_CREDITS_DOT_COLOR,
        label,
        &pool.reset,
        remaining,
        Some(monthly),
        appearance.theme().outline().into_solid(),
    )
}

#[cfg(test)]
#[path = "included_usage_pool_tests.rs"]
mod tests;
