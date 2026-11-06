use std::sync::Arc;

use chrono::{DateTime, Local, Utc};
use warp_core::ui::appearance::Appearance;
use warp_graphql::queries::get_included_usage_pools::IncludedUsagePool;
use warpui::elements::Text;
use warpui::fonts::{Properties, Weight};
use warpui::{AppContext, Element, Entity, SingletonEntity, View, ViewContext};

use crate::ai::blocklist::view_util::usage_display_unit;
use crate::auth::{AuthManager, AuthStateProvider};
use crate::server::server_api::ServerApiProvider;
use crate::server::server_api::workspace::WorkspaceClient;
use crate::settings::ai::{AISettings, AISettingsChangedEvent, UsageDisplayUnit};
use crate::settings_view::billing_and_usage_page_v2::{
    BASE_CREDITS_DOT_COLOR, BalanceAmount, render_balance_card, render_balance_card_contents,
};
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::workspaces::workspace::{CustomerType, Workspace, WorkspaceUid};

pub(crate) fn is_scale_workspace(workspace: Option<&Workspace>) -> bool {
    workspace
        .is_some_and(|workspace| workspace.billing_metadata.customer_type == CustomerType::Scale)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PoolKey {
    workspace_uid: WorkspaceUid,
    user_uid: String,
}

#[derive(Default)]
enum PoolState {
    #[default]
    Idle,
    Loading,
    Ready(IncludedUsagePool),
    Unavailable,
}

#[derive(Default)]
struct PoolRequest {
    key: Option<PoolKey>,
    generation: u64,
    state: PoolState,
}

impl PoolRequest {
    fn begin(&mut self, key: Option<PoolKey>) -> u64 {
        self.generation += 1;
        self.state = if key.is_some() {
            PoolState::Loading
        } else {
            PoolState::Idle
        };
        self.key = key;
        self.generation
    }

    fn complete(&mut self, generation: u64, result: anyhow::Result<Option<IncludedUsagePool>>) {
        if self.generation == generation {
            self.state = result
                .ok()
                .flatten()
                .map_or(PoolState::Unavailable, PoolState::Ready);
        }
    }
}

pub struct IncludedUsagePoolView {
    client: Arc<dyn WorkspaceClient>,
    request: PoolRequest,
    selected: bool,
}

impl IncludedUsagePoolView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        ctx.subscribe_to_model(&AISettings::handle(ctx), |_, _, event, ctx| {
            if matches!(event, AISettingsChangedEvent::UsageDisplayUnit { .. }) {
                ctx.notify();
            }
        });
        ctx.subscribe_to_model(&UserWorkspaces::handle(ctx), |me, _, _, ctx| {
            me.refresh_if_workspace_changed(ctx);
            ctx.notify();
        });
        ctx.subscribe_to_model(&AuthManager::handle(ctx), |me, _, _, ctx| {
            me.refresh_if_workspace_changed(ctx);
            ctx.notify();
        });
        Self {
            client: ServerApiProvider::as_ref(ctx).get_workspace_client(),
            request: PoolRequest::default(),
            selected: false,
        }
    }

    fn current_key(ctx: &AppContext) -> Option<PoolKey> {
        let auth = AuthStateProvider::as_ref(ctx).get();
        if !auth.is_logged_in() {
            return None;
        }
        let workspace = UserWorkspaces::as_ref(ctx).current_workspace()?;
        is_scale_workspace(Some(workspace)).then(|| PoolKey {
            workspace_uid: workspace.uid,
            user_uid: auth.user_id().unwrap_or_default().to_string(),
        })
    }

    fn refresh_if_workspace_changed(&mut self, ctx: &mut ViewContext<Self>) {
        if self.selected && self.request.key != Self::current_key(ctx) {
            self.refresh(ctx);
        }
    }

    pub fn refresh(&mut self, ctx: &mut ViewContext<Self>) {
        self.selected = true;
        let key = Self::current_key(ctx);
        if key == self.request.key && matches!(self.request.state, PoolState::Loading) {
            return;
        }
        let generation = self.request.begin(key.clone());
        ctx.notify();
        let Some(key) = key else { return };
        let client = self.client.clone();
        // This query is deliberately separate: older servers must not break core usage refreshes.
        ctx.spawn(
            async move { client.included_usage_pool(key.workspace_uid).await },
            move |me, result, ctx| {
                if me.request.key == Self::current_key(ctx) {
                    me.request.complete(generation, result);
                    ctx.notify();
                }
            },
        );
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
        let current_key = Self::current_key(ctx);
        let state = if current_key.is_some() && current_key == self.request.key {
            &self.request.state
        } else {
            &PoolState::Idle
        };
        render_card(
            state,
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
        PoolState::Idle | PoolState::Loading => PoolBalance::Loading,
        PoolState::Unavailable => PoolBalance::Unavailable,
        PoolState::Ready(pool) => {
            if pool.monthly_cents <= 0
                || !pool.monthly_credits.is_finite()
                || !pool.used_credits.is_finite()
                || pool.monthly_credits <= 0.0
                || pool.used_credits < 0.0
                || !pool.granted_cents.is_finite()
                || !pool.remaining_cents.is_finite()
                || pool.granted_cents < 0.0
                || pool.remaining_cents < 0.0
                || pool.expires_at.is_some_and(|expiry| expiry.utc() <= now)
            {
                return PoolBalance::Unavailable;
            }
            let monthly_cents = f64::from(pool.monthly_cents);
            PoolBalance::Available {
                remaining_credits: (pool.monthly_credits - pool.used_credits).max(0.0),
                monthly_credits: pool.monthly_credits,
                remaining_cents: pool.remaining_cents,
                monthly_cents,
                reset: pool
                    .expires_at
                    .map(|expiry| {
                        expiry
                            .utc()
                            .with_timezone(&Local)
                            .format("Resets %b %d at %-I:%M %p")
                            .to_string()
                    })
                    .unwrap_or_default(),
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
