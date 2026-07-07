use std::collections::HashSet;

use chrono::{Duration, Utc};
use warp_core::settings::Setting;
use warpui::{Entity, ModelContext, SingletonEntity};

use crate::ai::request_usage_model::{
    AIRequestUsageModel, AIRequestUsageModelEvent, BonusGrant, BonusGrantScope,
};
use crate::terminal::general_settings::GeneralSettings;

pub struct BonusGrantNotificationModel {
    /// In-memory tracking of grants shown during this session. This prevents duplicate
    /// notifications when multiple `AIRequestUsageModelEvent::RequestUsageUpdated` events
    /// fire in quick succession before the persisted settings can be updated.
    shown_grants_session: HashSet<String>,
}

#[derive(Debug, Clone)]
pub enum BonusGrantNotificationEvent {
    ShowNotification { grant: BonusGrant, message: String },
}

impl Entity for BonusGrantNotificationModel {
    type Event = BonusGrantNotificationEvent;
}

impl SingletonEntity for BonusGrantNotificationModel {}

impl BonusGrantNotificationModel {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        ctx.subscribe_to_model(&AIRequestUsageModel::handle(ctx), |me, _, event, ctx| {
            if let AIRequestUsageModelEvent::RequestUsageUpdated = event {
                me.check_for_new_bonus_grants(ctx);
            }
        });

        Self {
            shown_grants_session: HashSet::new(),
        }
    }

    fn check_for_new_bonus_grants(&mut self, ctx: &mut ModelContext<Self>) {
        let usage_model = AIRequestUsageModel::as_ref(ctx);
        let bonus_grants = usage_model.bonus_grants();

        let shown_grants = GeneralSettings::as_ref(ctx)
            .bonus_grants_shown
            .value()
            .clone();

        // Only show grants created in the past 2 weeks
        let cutoff_date = Utc::now() - Duration::days(14);

        let mut grants_to_notify = Vec::new();
        let mut grants_to_persist_to_settings = Vec::new();

        for grant in bonus_grants {
            // Only notify about Warp-granted credits (cost = 0), not user purchases
            if grant.cost_cents != 0 {
                continue;
            }

            // Only show grants created in the past 2 weeks to avoid overwhelming users
            // with old grant notifications
            if grant.created_at < cutoff_date {
                continue;
            }

            // doesn't make sense to show "you've got a bonus grant" message if no credits remain (i.e. your teammate used them all)
            if grant.request_credits_remaining <= 0 {
                continue;
            }

            // Use server-provided message if available, otherwise fall back to generic message
            let message = if let Some(user_facing_message) = &grant.user_facing_message {
                user_facing_message.clone()
            } else {
                Self::format_generic_grant_message(grant)
            };

            let grant_key = Self::create_grant_key(grant);

            let in_persisted = shown_grants.contains(&grant_key);
            let in_session = self.shown_grants_session.contains(&grant_key);

            if !in_persisted && !in_session {
                grants_to_notify.push((grant.clone(), message, grant_key.clone()));
            }

            if !in_persisted {
                grants_to_persist_to_settings.push(grant_key.clone());
            }
        }

        for (grant, message, grant_key) in grants_to_notify {
            self.shown_grants_session.insert(grant_key);
            ctx.emit(BonusGrantNotificationEvent::ShowNotification { grant, message });
        }

        for grant_key in grants_to_persist_to_settings {
            self.mark_grant_as_shown(&grant_key, ctx);
            self.shown_grants_session.insert(grant_key);
        }
    }

    fn format_generic_grant_message(grant: &BonusGrant) -> String {
        let scope_text = match grant.scope {
            BonusGrantScope::User => "account",
            BonusGrantScope::Workspace(_) => "team",
        };
        format!(
            "{} Reload Credits have been added to your {}.",
            grant.request_credits_granted, scope_text
        )
    }

    fn create_grant_key(grant: &BonusGrant) -> String {
        format!("{}:{}", grant.reason, grant.created_at.timestamp())
    }

    fn mark_grant_as_shown(&self, grant_key: &str, ctx: &mut ModelContext<Self>) {
        GeneralSettings::handle(ctx).update(ctx, |settings, ctx| {
            let mut shown_grants = settings.bonus_grants_shown.value().clone();
            shown_grants.insert(grant_key.to_string());

            if let Err(e) = settings.bonus_grants_shown.set_value(shown_grants, ctx) {
                log::warn!("Failed to mark bonus grant as shown: {e}");
            }
        });
    }
}
