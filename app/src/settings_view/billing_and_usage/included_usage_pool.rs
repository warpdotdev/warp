use chrono::Local;
use warp_core::ui::appearance::Appearance;
use warp_graphql::queries::get_request_limit_info::WorkspaceIncludedUsageInfo;
use warpui::Element;

use crate::settings::ai::UsageDisplayUnit;
use crate::settings_view::billing_and_usage_page_v2::{
    BASE_CREDITS_DOT_COLOR, BalanceAmount, render_balance_card,
};

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

pub(crate) fn render_included_usage_card(
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
