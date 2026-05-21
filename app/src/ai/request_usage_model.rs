use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::AIAgentExchangeId;
use crate::auth::AuthStateProvider;
use crate::pricing::PricingInfoModel;
use crate::server::server_api::ai::AIClient;
use crate::settings::AISettings;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::workspaces::workspace::WorkspaceUid;
use crate::BlocklistAIHistoryModel;
use ai::api_keys::ApiKeyManager;
use chrono::{DateTime, Local, Utc};
use instant::Instant;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use warp_core::user_preferences::GetUserPreferences as _;
use warp_graphql::scalars::time::ServerTimestamp;
use warpui::{AppContext, Entity, ModelContext, SingletonEntity};

pub use warp_graphql::billing::BonusGrantType;

/// Threshold of ambient-only credits at which we surface upgrade/CTA UI.
pub const AMBIENT_AGENT_TRIAL_CREDIT_THRESHOLD: i32 = 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BonusGrantScope {
    User,
    Workspace(WorkspaceUid),
}

#[derive(Clone, Debug, PartialEq, Default)]
pub enum BuyCreditsBannerDisplayState {
    #[default]
    Hidden,
    OutOfCredits,
    MonthlyLimitReached,
}

#[derive(Clone, Debug)]
pub struct BonusGrant {
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub cost_cents: i32,
    pub expiration: Option<chrono::DateTime<chrono::Utc>>,
    pub grant_type: BonusGrantType,
    pub reason: String,
    pub user_facing_message: Option<String>,
    pub request_credits_granted: i32,
    pub request_credits_remaining: i32,
    pub scope: BonusGrantScope,
}

/// The key for the corresponding entry in UserDefaults.
const REQUEST_LIMIT_INFO_CACHE_KEY: &str = "AIRequestLimitInfo";

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub enum RequestLimitRefreshDuration {
    Weekly,
    Monthly,
    EveryTwoWeeks,
}

/// The current rate limit info for the user.
#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub struct RequestLimitInfo {
    pub limit: usize,
    pub num_requests_used_since_refresh: usize,
    pub next_refresh_time: ServerTimestamp,
    pub is_unlimited: bool,
    pub request_limit_refresh_duration: RequestLimitRefreshDuration,
    pub is_unlimited_voice: bool,
    #[serde(default)]
    pub voice_request_limit: usize,
    #[serde(default)]
    pub voice_requests_used_since_last_refresh: usize,
    #[serde(default)]
    pub is_unlimited_codebase_indices: bool,
    #[serde(default)]
    pub max_codebase_indices: usize,
    #[serde(default)]
    pub max_files_per_repo: usize,
    #[serde(default)]
    pub embedding_generation_batch_size: usize,
}

fn default_voice_requests_limit() -> usize {
    10000
}

impl Default for RequestLimitInfo {
    /// This is the default rate limit for the free tier imposed by the server as of 02/10/25.
    fn default() -> Self {
        Self {
            limit: 150,
            num_requests_used_since_refresh: 0,
            next_refresh_time: ServerTimestamp::new(Utc::now() + chrono::Duration::days(30)),
            is_unlimited: false,
            request_limit_refresh_duration: RequestLimitRefreshDuration::Monthly,
            is_unlimited_voice: false,
            voice_request_limit: default_voice_requests_limit(),
            voice_requests_used_since_last_refresh: 0,
            is_unlimited_codebase_indices: false,
            max_codebase_indices: 3,
            max_files_per_repo: 5000,
            embedding_generation_batch_size: 100,
        }
    }
}

#[cfg(test)]
impl RequestLimitInfo {
    pub fn new_for_test(limit: usize, num_requests_used_since_refresh: usize) -> Self {
        Self {
            limit,
            num_requests_used_since_refresh,
            ..Self::default()
        }
    }
}

pub struct CodebaseContextUsageLimit {
    pub max_files_per_repo: usize,
    pub max_indices_allowed: Option<usize>,
    pub embedding_generation_batch_size: usize,
}

/// Contains all usage-related information fetched from the server.
pub struct RequestUsageInfo {
    pub request_limit_info: RequestLimitInfo,
    pub bonus_grants: Vec<BonusGrant>,
}

#[cfg(feature = "agent_mode_evals")]
impl RequestLimitInfo {
    pub fn new_for_evals() -> Self {
        Self {
            limit: 999999,
            num_requests_used_since_refresh: 0,
            next_refresh_time: ServerTimestamp::new(Utc::now() + chrono::Duration::days(30)),
            is_unlimited: true,
            request_limit_refresh_duration: RequestLimitRefreshDuration::Monthly,
            is_unlimited_voice: true,
            voice_request_limit: 999999,
            voice_requests_used_since_last_refresh: 0,
            is_unlimited_codebase_indices: false,
            max_codebase_indices: 40,
            max_files_per_repo: 10000,
            embedding_generation_batch_size: 100,
        }
    }
}

fn cache_request_limit_info(request_limit_info: RequestLimitInfo, app_mut: &mut AppContext) {
    if let Ok(serialized) = serde_json::to_string(&request_limit_info) {
        let _ = app_mut
            .private_user_preferences()
            .write_value(REQUEST_LIMIT_INFO_CACHE_KEY, serialized);
    }
}

fn get_cached_request_limit_info(app_mut: &mut AppContext) -> Option<RequestLimitInfo> {
    app_mut
        .private_user_preferences()
        .read_value(REQUEST_LIMIT_INFO_CACHE_KEY)
        .unwrap_or_default()
        .and_then(|serialized| serde_json::from_str(serialized.as_str()).ok())
}

pub struct AIRequestUsageModel {
    ai_client: Arc<dyn AIClient>,

    /// The last time at which `request_limit_info` was updated.
    last_update_time: Option<Instant>,

    request_limit_info: RequestLimitInfo,

    bonus_grants: Vec<BonusGrant>,

    /// Whether the buy credits banner has been dismissed by the user.
    buy_addon_credits_banner_dismissed: bool,
}

impl Entity for AIRequestUsageModel {
    type Event = AIRequestUsageModelEvent;
}

pub enum AIRequestUsageModelEvent {
    RequestUsageUpdated,
    RequestBonusRefunded {
        requests_refunded: i32,
        server_conversation_id: String,
        request_id: String,
    },
}

impl AIRequestUsageModel {
    pub fn new(ai_client: Arc<dyn AIClient>, ctx: &mut ModelContext<Self>) -> Self {
        // Check if the user has cached request limit info from before.
        // This is only used to show the latest known value before we finish refreshing from the server below.
        let cached_request_limit_info = get_cached_request_limit_info(ctx);
        let request_limit_info = cached_request_limit_info.unwrap_or_default();

        Self {
            ai_client,
            request_limit_info,
            last_update_time: None,
            bonus_grants: vec![],
            buy_addon_credits_banner_dismissed: false,
        }
    }

    #[cfg(test)]
    pub fn new_for_test(ai_client: Arc<dyn AIClient>, _ctx: &mut ModelContext<Self>) -> Self {
        Self {
            ai_client,
            last_update_time: None,
            request_limit_info: RequestLimitInfo::default(),
            bonus_grants: vec![],
            buy_addon_credits_banner_dismissed: false,
        }
    }

    pub fn last_update_time(&self) -> Option<Instant> {
        self.last_update_time
    }

    /// Spawns a task to refresh the latest AI request usage and bonus grants, fetching from the server.
    pub fn refresh_request_usage_async(&mut self, ctx: &mut ModelContext<Self>) {
        if !AuthStateProvider::as_ref(ctx).get().is_logged_in() {
            return;
        }

        let ai_client = self.ai_client.clone();
        ctx.spawn(
            async move { ai_client.get_request_limit_info().await },
            |model, result, ctx| match result {
                Ok(usage_info) => {
                    model.bonus_grants = usage_info.bonus_grants;
                    model.update_request_limit_info(usage_info.request_limit_info, ctx);
                }
                Err(e) => {
                    log::warn!("Failed to retrieve initial request limit info: {e:#}");
                }
            },
        );
    }

    pub fn update_request_limit_info(
        &mut self,
        request_limit_info: RequestLimitInfo,
        ctx: &mut ModelContext<Self>,
    ) {
        self.last_update_time = Some(Instant::now());
        self.request_limit_info = request_limit_info;
        cache_request_limit_info(request_limit_info, ctx);

        AISettings::handle(ctx).update(ctx, |ai_settings, ctx| {
            ai_settings.update_quota_info(&request_limit_info, ctx);
        });

        ctx.emit(AIRequestUsageModelEvent::RequestUsageUpdated);
    }

    pub fn provide_negative_feedback_response_for_ai_conversation(
        &mut self,
        client_conversation_id: AIConversationId,
        request_id: String,
        client_exchange_id: AIAgentExchangeId,
        ctx: &mut ModelContext<Self>,
    ) {
        let server_conversation_id = BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&client_conversation_id)
            .and_then(|conversation| conversation.server_conversation_token());

        let Some(server_conversation_id) = server_conversation_id else {
            return;
        };
        let server_conversation_id_string = server_conversation_id.as_str().to_string();
        let server_conversation_id_string_clone = server_conversation_id_string.clone();

        let request_ids = BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&client_conversation_id)
            .map(|conversation| {
                let mut request_ids = vec![];

                let target_exchange = conversation
                    .root_task_exchanges()
                    .find(|exchange| exchange.id == client_exchange_id);

                let mut found_target = false;

                for exchange in conversation.exchanges_reversed() {
                    if let Some(target_exchange) = target_exchange {
                        if exchange.id == target_exchange.id {
                            found_target = true;
                        }
                    } else {
                        break;
                    }

                    if found_target {
                        if let Some(server_output_id) = exchange.output_status.server_output_id() {
                            request_ids.push(server_output_id.to_string());
                        }

                        if exchange
                            .input
                            .iter()
                            .any(|input| input.user_query().is_some())
                        {
                            break;
                        }
                    }
                }

                request_ids
            })
            .unwrap_or_default();

        // No reason to refund if there are no request ids.
        if request_ids.is_empty() {
            return;
        }

        let ai_client = self.ai_client.clone();
        ctx.spawn(
            async move {
                ai_client
                    .provide_negative_feedback_response_for_ai_conversation(
                        server_conversation_id_string_clone,
                        request_ids,
                    )
                    .await
            },
            |_, result, ctx| match result {
                Ok(requests_refunded) => {
                    if requests_refunded > 0 {
                        ctx.emit(AIRequestUsageModelEvent::RequestBonusRefunded {
                            requests_refunded,
                            server_conversation_id: server_conversation_id_string,
                            request_id,
                        });
                    }
                }
                Err(e) => {
                    log::error!(
                        "Failed to provide negative feedback response for ai conversation: {e:?}"
                    );
                }
            },
        );
    }

    /// Returns the number of remaining requests the user has based on their latest rate limit info.
    /// If the current time is past the next refresh time, then the number of remaining reqs is the limit.
    fn requests_remaining(&self) -> usize {
        if self.next_refresh_time() <= Utc::now() || self.is_unlimited() {
            self.request_limit_info.limit
        } else {
            self.request_limit_info
                .limit
                .saturating_sub(self.request_limit_info.num_requests_used_since_refresh)
        }
    }

    /// Returns `true` if the user has at least one request remaining before hitting the AI request
    /// limit.
    ///
    /// WARNING: This method doesn't account for add-on credits. Consider if you want
    /// [`Self::has_any_ai_remaining`] instead.
    pub fn has_requests_remaining(&self) -> bool {
        self.requests_remaining() > 0
    }

    /// Returns `true` if the user meets one of the following conditions:
    /// 1. user has ai credits from the plan base limit
    /// 2. user has overage enabled
    /// 3. user has bonus grants (either team grants or user grants)
    /// 4. user's team plan has pay-as-you-go enabled (enterprise only)
    /// 5. user's team is on enterprise with bonus grants auto-reload enable (enterprise only)
    /// 6. user has BYOK enabled and has provided at least one API key
    /// Use this method as the starting point for AI availability checking.
    pub fn has_any_ai_remaining(&self, ctx: &AppContext) -> bool {
        let current_workspace = UserWorkspaces::as_ref(ctx).current_workspace();

        let has_base_plan_ai_requests = self.has_requests_remaining();

        let user_bonus_credits = self.total_user_interactive_bonus_credits_remaining() > 0;
        let workspace_bonus_credits = current_workspace
            .map(|workspace| self.total_workspace_bonus_credits_remaining(workspace.uid) > 0)
            .unwrap_or_default();

        let workspace_has_overages =
            current_workspace.is_some_and(|workspace| workspace.are_overages_remaining());

        let is_payg_enabled = current_workspace
            .is_some_and(|w| w.billing_metadata.is_enterprise_pay_as_you_go_enabled());

        let is_enterprise_auto_reload_enabled = current_workspace
            .is_some_and(|w| w.billing_metadata.is_enterprise_auto_reload_enabled());

        // If you have provided your own API key,
        // it doesn't matter if you are out of warp-provided requests.
        let has_byo_api_key = UserWorkspaces::as_ref(ctx).is_byo_api_key_enabled(ctx)
            && ApiKeyManager::as_ref(ctx).keys().has_any_key();

        has_base_plan_ai_requests
            || (user_bonus_credits || workspace_bonus_credits)
            || workspace_has_overages
            || is_payg_enabled
            || is_enterprise_auto_reload_enabled
            || has_byo_api_key
    }

    pub fn requests_used(&self) -> usize {
        if self.next_refresh_time() <= Utc::now() {
            return 0;
        }
        self.request_limit_info.num_requests_used_since_refresh
    }

    pub fn request_percentage_used(&self) -> f32 {
        self.requests_used() as f32 / self.request_limit() as f32
    }

    pub fn request_limit(&self) -> usize {
        self.request_limit_info.limit
    }

    /// Returns the number of indices the user's tier allows them to create and the number of files
    /// the user's tier allows them to index. If the user is allowed unlimited indices, then the
    /// max_indices_allowed is None.
    pub fn codebase_context_limits(&self) -> CodebaseContextUsageLimit {
        CodebaseContextUsageLimit {
            max_files_per_repo: self.request_limit_info.max_files_per_repo,
            max_indices_allowed: if self.request_limit_info.is_unlimited_codebase_indices {
                None
            } else {
                Some(self.request_limit_info.max_codebase_indices)
            },
            embedding_generation_batch_size: self
                .request_limit_info
                .embedding_generation_batch_size,
        }
    }

    /// Returns whether the user has hit their maximum codebase allowance.
    /// (If the user is allowed unlimited indices, this is vacuously false.)
    pub fn hit_codebase_index_limit(&self, current_indices: usize) -> bool {
        self.codebase_context_limits()
            .max_indices_allowed
            .map(|lim| current_indices >= lim)
            .unwrap_or(false)
    }

    pub fn next_refresh_time(&self) -> DateTime<Utc> {
        self.request_limit_info.next_refresh_time.utc()
    }

    pub fn next_refresh_time_local(&self) -> DateTime<Local> {
        self.next_refresh_time().with_timezone(&Local)
    }

    pub fn is_unlimited(&self) -> bool {
        self.request_limit_info.is_unlimited
    }

    pub fn refresh_duration_to_string(&self) -> String {
        match self.request_limit_info.request_limit_refresh_duration {
            RequestLimitRefreshDuration::Weekly => "weekly".to_string(),
            RequestLimitRefreshDuration::Monthly => "monthly".to_string(),
            RequestLimitRefreshDuration::EveryTwoWeeks => "biweekly".to_string(),
        }
    }

    pub fn bonus_grants(&self) -> &[BonusGrant] {
        &self.bonus_grants
    }

    /// Returns the total remaining ambient-only credits for the user.
    /// Returns None if the user has never received any ambient-only grants.
    pub fn ambient_only_credits_remaining(&self) -> Option<i32> {
        let ambient_grants: Vec<_> = self
            .bonus_grants
            .iter()
            .filter(|g| g.grant_type == BonusGrantType::AmbientOnly)
            .collect();
        if ambient_grants.is_empty() {
            None
        } else {
            Some(
                ambient_grants
                    .iter()
                    .map(|g| g.request_credits_remaining)
                    .sum(),
            )
        }
    }

    pub fn total_workspace_bonus_credits_remaining(&self, uid: WorkspaceUid) -> i32 {
        let now = Utc::now();
        self.bonus_grants
            .iter()
            .filter(|grant| grant.scope == BonusGrantScope::Workspace(uid))
            .filter(|grant| grant.expiration.is_none_or(|exp| now < exp))
            .map(|grant| grant.request_credits_remaining)
            .sum()
    }

    pub fn total_current_workspace_bonus_credits_remaining(&self, ctx: &AppContext) -> i32 {
        UserWorkspaces::as_ref(ctx)
            .current_workspace()
            .map(|workspace| self.total_workspace_bonus_credits_remaining(workspace.uid))
            .unwrap_or(0)
    }

    pub fn total_user_interactive_bonus_credits_remaining(&self) -> i32 {
        let now = Utc::now();
        self.bonus_grants
            .iter()
            .filter(|grant| grant.scope == BonusGrantScope::User)
            .filter(|grant| grant.grant_type != BonusGrantType::AmbientOnly)
            .filter(|grant| grant.expiration.is_none_or(|exp| now < exp))
            .map(|grant| grant.request_credits_remaining)
            .sum()
    }

    /// Computes the current banner state based on live conditions.
    /// This is called on-demand and always returns fresh state.
    pub fn compute_buy_addon_credits_banner_display_state(
        &self,
        ctx: &AppContext,
    ) -> BuyCreditsBannerDisplayState {
        // Early return if user dismissed
        if self.buy_addon_credits_banner_dismissed {
            return BuyCreditsBannerDisplayState::Hidden;
        }
        let current_workspace = UserWorkspaces::as_ref(ctx).current_workspace();
        let policy_allows_purchasing = current_workspace
            .map(|w| {
                w.billing_metadata
                    .tier
                    .purchase_add_on_credits_policy
                    .is_some_and(|p| p.enabled)
            })
            .unwrap_or(false);

        // TODO: we might want to suggest credits purchase if request_remain/bonus credits is below certain threshold
        // something to consider after launch
        // Ambient-only credits are usable for cloud agents and should not suppress this banner.
        let now = Utc::now();
        let has_non_ambient_bonus_credits = self
            .bonus_grants
            .iter()
            .filter(|grant| grant.grant_type != BonusGrantType::AmbientOnly)
            .filter(|grant| grant.expiration.is_none_or(|exp| now < exp))
            .filter(|grant| grant.request_credits_remaining > 0)
            .any(|grant| match grant.scope {
                BonusGrantScope::User => true,
                BonusGrantScope::Workspace(uid) => {
                    current_workspace.is_some_and(|workspace| workspace.uid == uid)
                }
            });
        if !policy_allows_purchasing
            || self.has_requests_remaining()
            || has_non_ambient_bonus_credits
        {
            return BuyCreditsBannerDisplayState::Hidden;
        }

        let auto_reload_enabled = current_workspace
            .is_some_and(|w| w.settings.addon_credits_settings.auto_reload_enabled);
        if !auto_reload_enabled {
            return BuyCreditsBannerDisplayState::OutOfCredits;
        }

        let at_monthly_limit =
            current_workspace.is_some_and(|w| w.is_at_addon_credits_monthly_limit());

        let auto_reload_would_exceed = current_workspace
            .and_then(|workspace| {
                let options = PricingInfoModel::as_ref(ctx).addon_credits_options()?;
                let price = workspace.get_auto_reload_price_cents(options)?;
                Some(workspace.would_addon_purchase_reach_limit(price))
            })
            .unwrap_or(false);

        if at_monthly_limit || auto_reload_would_exceed {
            BuyCreditsBannerDisplayState::MonthlyLimitReached
        } else {
            BuyCreditsBannerDisplayState::Hidden
        }
    }

    pub fn dismiss_buy_credits_banner(&mut self, ctx: &mut ModelContext<Self>) {
        self.buy_addon_credits_banner_dismissed = true;
        ctx.notify();
    }

    pub fn enable_buy_credits_banner(&mut self, ctx: &mut ModelContext<Self>) {
        self.buy_addon_credits_banner_dismissed = false;
        ctx.notify();
    }
}

/// Voice request usage, only available if built with voice input support.
#[cfg(feature = "voice_input")]
impl AIRequestUsageModel {
    fn voice_requests(&self) -> usize {
        self.request_limit_info
            .voice_requests_used_since_last_refresh
    }

    fn voice_requests_limit(&self) -> usize {
        self.request_limit_info.voice_request_limit
    }

    fn is_unlimited_voice_requests(&self) -> bool {
        self.request_limit_info.is_unlimited_voice
    }

    /// Returns the number of remaining requests the user has based on their latest rate limit info.
    /// If the current time is past the next refresh time, then the number of remaining reqs is the limit.
    fn voice_requests_remaining(&self) -> usize {
        if self.next_refresh_time() <= Utc::now() || self.is_unlimited_voice_requests() {
            self.voice_requests_limit()
        } else {
            self.voice_requests_limit()
                .saturating_sub(self.voice_requests())
        }
    }

    /// Returns `true` if the user has at least one voice request before hitting the
    /// limit. Returns `false` otherwise.
    fn has_voice_requests_remaining(&self) -> bool {
        self.voice_requests_remaining() > 0
    }

    /// Checks request limits to see if the user can make a voice request.
    /// Returns true if the user can make a voice request, false otherwise.
    pub fn can_request_voice(&self) -> bool {
        self.has_voice_requests_remaining()
    }
}

impl SingletonEntity for AIRequestUsageModel {}

#[cfg(test)]
#[path = "request_usage_model_tests.rs"]
mod tests;
