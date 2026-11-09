use chrono::{DateTime, Local, Utc};
use warp_core::ui::appearance::Appearance;
use warpui::elements::Text;
use warpui::fonts::{Properties, Weight};
use warpui::{AppContext, Element, Entity, SingletonEntity, View, ViewContext};

use crate::ai::blocklist::view_util::usage_display_unit;
use crate::ai::{AIRequestUsageModel, WorkspaceIncludedUsageState as PoolState};
use crate::settings::ai::{AISettings, AISettingsChangedEvent, UsageDisplayUnit};
use crate::settings_view::billing_and_usage_page_v2::{
    BASE_CREDITS_DOT_COLOR, BalanceAmount, render_balance_card, render_balance_card_contents,
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
        render_card(
            &AIRequestUsageModel::as_ref(ctx).workspace_included_usage(ctx),
            Utc::now(),
            usage_display_unit(ctx),
            Appearance::as_ref(ctx),
        )
    }
}

#[derive(Debug, PartialEq)]
enum PoolBalance {
    Loading,
    Unavailable,
    Available {
        remaining_credits: f64,
        monthly_credits: f64,
        remaining_cents: f64,
        monthly_cents: f64,
        reset: String,
    },
}

fn pool_balance(state: &PoolState, now: DateTime<Utc>) -> PoolBalance {
    match state {
        PoolState::Idle => PoolBalance::Loading,
        PoolState::Unavailable => PoolBalance::Unavailable,
        PoolState::Ready(pool) => {
            if !pool.included_usage_cents.is_finite()
                || pool.included_usage_cents <= 0.0
                || !pool.included_usage_credits.is_finite()
                || pool.included_usage_credits <= 0.0
                || !pool.usage_credits_used_since_last_refresh.is_finite()
                || pool.usage_credits_used_since_last_refresh < 0.0
                || !pool.usage_cents_used_since_last_refresh.is_finite()
                || pool.usage_cents_used_since_last_refresh < 0.0
                || pool.next_refresh_time.utc() <= now
            {
                return PoolBalance::Unavailable;
            }
            PoolBalance::Available {
                remaining_credits: (pool.included_usage_credits
                    - pool.usage_credits_used_since_last_refresh)
                    .max(0.0),
                monthly_credits: pool.included_usage_credits,
                remaining_cents: (pool.included_usage_cents
                    - pool.usage_cents_used_since_last_refresh)
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
    }
}

fn render_card(
    state: &PoolState,
    now: DateTime<Utc>,
    unit: UsageDisplayUnit,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let border_color = appearance.theme().outline().into_solid();
    let label = match unit {
        UsageDisplayUnit::Credits => "Included team credits",
        UsageDisplayUnit::Dollars => "Included team usage",
    };
    let status = match pool_balance(state, now) {
        PoolBalance::Available {
            remaining_credits,
            monthly_credits,
            remaining_cents,
            monthly_cents,
            reset,
        } => {
            let (remaining, monthly) = match unit {
                UsageDisplayUnit::Credits => (
                    BalanceAmount::FractionalCredits(remaining_credits),
                    BalanceAmount::FractionalCredits(monthly_credits),
                ),
                UsageDisplayUnit::Dollars => (
                    BalanceAmount::Cents(remaining_cents),
                    BalanceAmount::Cents(monthly_cents),
                ),
            };
            return render_balance_card(
                appearance,
                BASE_CREDITS_DOT_COLOR,
                label,
                &reset,
                remaining,
                Some(monthly),
                border_color,
            );
        }
        PoolBalance::Loading => "Loading…",
        PoolBalance::Unavailable => "Unavailable",
    };
    render_balance_card_contents(
        appearance,
        BASE_CREDITS_DOT_COLOR,
        label,
        "",
        Text::new_inline(status, appearance.ui_font_family(), 24.)
            .with_color(appearance.theme().active_ui_text_color().into())
            .with_style(Properties::default().weight(Weight::Semibold))
            .finish(),
        border_color,
    )
}

#[cfg(test)]
#[path = "included_usage_pool_tests.rs"]
mod tests;
