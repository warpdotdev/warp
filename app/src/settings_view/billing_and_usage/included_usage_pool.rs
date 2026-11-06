use std::sync::Arc;

use chrono::{DateTime, Local, Utc};
use warp_core::ui::appearance::Appearance;
use warp_graphql::queries::get_included_usage_pools::IncludedUsagePool;
use warpui::elements::{
    Container, CornerRadius, Flex, MouseStateHandle, ParentElement, Radius, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::UiComponent;
use warpui::{AppContext, Element, Entity, SingletonEntity, View, ViewContext};

use crate::auth::{AuthManager, AuthStateProvider};
use crate::server::server_api::ServerApiProvider;
use crate::server::server_api::workspace::WorkspaceClient;
use crate::settings_view::billing_and_usage_page::BillingAndUsagePageAction;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::workspaces::workspace::{CustomerType, Workspace, WorkspaceUid};

pub(super) const INCLUDED_USAGE_DESCRIPTION: &str =
    "Shared across your team and used before team add-on credits. Resets monthly without rollover.";
pub(crate) const SCALE_ADDON_DESCRIPTION: &str = "Purchased team add-on credits are separate from included team usage. They are used after included usage is depleted, roll over between billing cycles, and expire one year after purchase.";

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
    refresh_button: MouseStateHandle,
}

impl IncludedUsagePoolView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
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
            refresh_button: MouseStateHandle::default(),
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
            self.refresh_button.clone(),
            Appearance::as_ref(ctx),
        )
    }
}

#[derive(Debug, PartialEq)]
struct PoolLabels {
    balance: String,
    remaining: Option<String>,
    reset: Option<String>,
}

fn pool_labels(state: &PoolState, now: DateTime<Utc>) -> PoolLabels {
    let unavailable = || PoolLabels {
        balance: "Included usage is unavailable. Refresh to try again.".to_string(),
        remaining: None,
        reset: None,
    };
    match state {
        PoolState::Idle | PoolState::Loading => PoolLabels {
            balance: "Loading included team usage…".to_string(),
            remaining: None,
            reset: None,
        },
        PoolState::Unavailable => unavailable(),
        PoolState::Ready(pool) => {
            if pool.monthly_cents <= 0
                || !pool.granted_cents.is_finite()
                || !pool.remaining_cents.is_finite()
                || pool.granted_cents < 0.0
                || pool.remaining_cents < 0.0
                || pool.expires_at.is_some_and(|expiry| expiry.utc() <= now)
            {
                return unavailable();
            }
            let remaining = pool.remaining_cents.min(pool.granted_cents);
            let used = (pool.granted_cents - remaining).max(0.0);
            PoolLabels {
                balance: format!(
                    "${:.2} used of ${:.2}",
                    used / 100.0,
                    pool.granted_cents / 100.0
                ),
                remaining: Some(format!("${:.2} remaining", remaining / 100.0)),
                reset: pool.expires_at.map(|expiry| {
                    expiry
                        .utc()
                        .with_timezone(&Local)
                        .format("Resets %b %d at %-I:%M %p")
                        .to_string()
                }),
            }
        }
    }
}

fn render_card(
    state: &PoolState,
    now: DateTime<Utc>,
    refresh_button: MouseStateHandle,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let labels = pool_labels(state, now);
    let theme = appearance.theme();
    let mut content = Flex::column().with_spacing(8.);
    content.add_child(
        Text::new("Included team usage", appearance.ui_font_family(), 16.)
            .with_style(Properties::default().weight(Weight::Semibold))
            .with_color(theme.active_ui_text_color().into())
            .finish(),
    );
    content.add_child(
        Text::new(labels.balance, appearance.ui_font_family(), 14.)
            .with_color(theme.active_ui_text_color().into())
            .finish(),
    );
    for label in labels
        .remaining
        .into_iter()
        .chain(labels.reset)
        .chain([INCLUDED_USAGE_DESCRIPTION.to_string()])
    {
        content.add_child(
            Text::new(label, appearance.ui_font_family(), 12.)
                .with_color(theme.sub_text_color(theme.surface_1()).into())
                .finish(),
        );
    }
    content.add_child(
        appearance
            .ui_builder()
            .button(ButtonVariant::Link, refresh_button)
            .with_text_label("Refresh".to_string())
            .build()
            .on_click(|ctx, _, _| {
                ctx.dispatch_typed_action(BillingAndUsagePageAction::RefreshWorkspaceData);
            })
            .finish(),
    );
    Container::new(content.finish())
        .with_uniform_padding(16.)
        .with_background_color(theme.surface_1().into_solid())
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
        .with_margin_bottom(16.)
        .finish()
}

#[cfg(test)]
#[path = "included_usage_pool_tests.rs"]
mod tests;
