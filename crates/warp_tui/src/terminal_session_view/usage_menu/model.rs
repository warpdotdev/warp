use chrono::Utc;
use warp::tui_export::{
    AIRequestUsageModel, AiCreditsUsageAndCostType, AuthStateProvider, BonusGrantType,
    UsageVisibilityGranularity, UserWorkspaces,
};
use warpui::SingletonEntity;
use warpui_core::AppContext;

/// One fixed credit balance rendered by the usage panel.
pub(super) struct TuiUsageCreditBar {
    pub(super) used: i64,
    pub(super) limit: i64,
    pub(super) note: String,
}

/// Pay-as-you-go spend for the current billing cycle.
pub(super) struct TuiUsagePayAsYouGo {
    pub(super) credits_used: i64,
    pub(super) cost_cents: i64,
    pub(super) has_kicked_in: bool,
}

/// Account usage captured when the usage panel opens.
pub(in crate::terminal_session_view) struct TuiUsageSnapshot {
    pub(super) plan_name: String,
    pub(super) team_name: Option<String>,
    pub(super) base_credits: Option<TuiUsageCreditBar>,
    pub(super) addon_credits: Option<TuiUsageCreditBar>,
    pub(super) pay_as_you_go: Option<TuiUsagePayAsYouGo>,
    pub(super) manage_billing_url: Option<String>,
}

impl TuiUsageSnapshot {
    pub(in crate::terminal_session_view) fn capture(ctx: &AppContext) -> Self {
        let ai_model = AIRequestUsageModel::as_ref(ctx);
        let workspaces = UserWorkspaces::as_ref(ctx);
        let workspace = workspaces.current_workspace();
        let team = workspace.and_then(|workspace| workspace.teams.first());
        let user_email = AuthStateProvider::as_ref(ctx).get().user_email();
        let is_admin = team
            .zip(user_email.as_deref())
            .is_some_and(|(team, email)| team.has_admin_permissions(email));
        let refresh_time = ai_model
            .next_refresh_time_local()
            .format("%B %-d at %-I:%M%P");

        let base_credits = (ai_model.request_limit() > 0).then(|| TuiUsageCreditBar {
            used: ai_model.requests_used() as i64,
            limit: ai_model.request_limit() as i64,
            note: if ai_model.is_unlimited() {
                "No limit".to_owned()
            } else {
                format!("Resets {refresh_time}")
            },
        });

        let now = Utc::now();
        let current_workspace_uid = workspace.map(|workspace| workspace.uid);
        let (granted, remaining) = ai_model
            .bonus_grants()
            .iter()
            .filter(|grant| grant.grant_type != BonusGrantType::AmbientOnly)
            .filter(|grant| grant.expiration.is_none_or(|expiration| now < expiration))
            .filter(|grant| {
                grant.scope.workspace_uid().is_none()
                    || grant.scope.workspace_uid() == current_workspace_uid
            })
            .fold((0i64, 0i64), |(granted, remaining), grant| {
                (
                    granted + i64::from(grant.request_credits_granted),
                    remaining + i64::from(grant.request_credits_remaining.max(0)),
                )
            });
        let addon_credits = (granted > 0).then(|| {
            let auto_reload_denomination = workspace
                .filter(|workspace| {
                    workspace
                        .settings
                        .addon_credits_settings
                        .auto_reload_enabled
                })
                .and_then(|workspace| {
                    workspace
                        .settings
                        .addon_credits_settings
                        .selected_auto_reload_credit_denomination
                });
            TuiUsageCreditBar {
                used: (granted - remaining).max(0),
                limit: granted,
                note: match auto_reload_denomination {
                    Some(credits) => {
                        format!("Auto-reload {credits} credits {refresh_time}")
                    }
                    None => String::new(),
                },
            }
        });

        let pay_as_you_go = workspace.and_then(|workspace| {
            let payg_available = workspace.are_overages_enabled()
                || workspace
                    .billing_metadata
                    .is_enterprise_pay_as_you_go_enabled();
            if !payg_available
                || !matches!(
                    workspace.resolve_usage_visibility(is_admin).granularity,
                    UsageVisibilityGranularity::OwnOnly | UsageVisibilityGranularity::FullBreakdown
                )
            {
                return None;
            }
            let team_uid = team.map(|team| team.uid.to_string());
            let (credits_used, cost_cents) = workspace
                .billing_cycle_usage
                .as_ref()
                .and_then(|usage| {
                    usage.summaries.iter().find(|summary| {
                        summary.period_start == usage.current_period_start
                            && summary.period_end == usage.current_period_end
                    })
                })
                .into_iter()
                .flat_map(|summary| &summary.entries)
                .filter(|entry| entry.cost_type == AiCreditsUsageAndCostType::Payg)
                .filter(|entry| {
                    team_uid.as_deref().is_none_or(|team_uid| {
                        entry.attributed_team_uid.as_deref() == Some(team_uid)
                    })
                })
                .fold((0i64, 0i64), |(credits, cost), entry| {
                    (
                        credits + i64::from(entry.credits_used),
                        cost + i64::from(entry.cost_cents),
                    )
                });
            Some(TuiUsagePayAsYouGo {
                credits_used,
                cost_cents,
                has_kicked_in: credits_used > 0,
            })
        });

        let manage_billing_url = team
            .filter(|_| is_admin)
            .map(|team| UserWorkspaces::admin_billing_link_for_team(team.uid));

        Self {
            plan_name: workspace
                .map(|workspace| workspace.billing_metadata.tier.name.clone())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| "Free".to_owned()),
            team_name: team.map(|team| team.name.clone()),
            base_credits,
            addon_credits,
            pay_as_you_go,
            manage_billing_url,
        }
    }
}
