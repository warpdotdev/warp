//! Workspace-level billing/plan accessors: reads of `workspace.billing_metadata` (tier and
//! policy entitlements) that hold regardless of which team a window has selected. See
//! [`crate::workspaces::user_workspaces::team_workspace_settings`] for the workspace-vs-team
//! two-layer model and the team-scoped policies that layer on top.

use warp_core::features::FeatureFlag;
use warpui::{AppContext, SingletonEntity};

use super::UserWorkspaces;
use crate::auth::AuthStateProvider;
use crate::channel::ChannelState;
use crate::workspaces::team::Team;
use crate::workspaces::workspace::{
    BillingMetadata, CustomerType, PurchaseAddOnCreditsPolicy, Workspace,
};

impl UserWorkspaces {
    pub fn current_workspace_billing_metadata(&self) -> Option<&BillingMetadata> {
        self.current_workspace()
            .map(|workspace| &workspace.billing_metadata)
    }

    /// The given team's billing metadata when the team is known, otherwise
    /// the current workspace's. For purchase surfaces that need
    /// team/workspace-scoped state (e.g. delinquency); for the purchase
    /// policy itself use [`Self::purchase_policy`].
    pub fn team_billing_metadata<'a>(
        &'a self,
        team: Option<&'a Team>,
    ) -> Option<&'a BillingMetadata> {
        team.map(|team| &team.billing_metadata)
            .or_else(|| self.current_workspace_billing_metadata())
    }

    pub fn is_custom_llm_enabled_for_team(&self, team: Option<&Team>) -> bool {
        team.map(Team::is_custom_llm_enabled)
            .or_else(|| {
                self.current_workspace()
                    .map(Workspace::is_custom_llm_enabled)
            })
            .unwrap_or(false)
    }

    /// The add-on credits purchase policy for the current viewer context: the
    /// current workspace's policy when one exists, else the user-level policy
    /// from the workspaces-metadata response (how teamless users get one).
    ///
    /// This is workspace-level: `purchase_add_on_credits_policy` is a plan
    /// entitlement, so it does not vary by the window's selected team.
    pub fn purchase_policy(&self) -> Option<PurchaseAddOnCreditsPolicy> {
        self.current_workspace_billing_metadata()
            .and_then(|billing| billing.tier.purchase_add_on_credits_policy)
            .or(self.user_purchase_policy)
    }

    /// Returns `true` if active AI is allowed for the current workspace, based on billing config.
    ///
    /// In the future, we should store active AI enablement on the policy directly. For now, we
    /// proxy whether active AI by checking whether any active AI feature is enabled.
    pub fn is_active_ai_allowed(&self) -> bool {
        self.current_workspace().is_none_or(|workspace| {
            workspace
                .billing_metadata
                .tier
                .warp_ai_policy
                .is_none_or(|policy| {
                    policy.is_prompt_suggestions_toggleable
                        || policy.is_next_command_enabled
                        || policy.is_code_suggestions_toggleable
                        || policy.is_git_operations_ai_enabled
                })
        })
    }

    pub fn ai_allowed_for_team(team: Option<&Team>) -> bool {
        !team.is_some_and(|team| team.billing_metadata.customer_type == CustomerType::Enterprise)
            || team.is_some_and(|team| team.billing_metadata.is_warp_plan())
            || ChannelState::channel().is_dogfood()
    }

    /// Whether Prompt Suggestions should be toggleable for the current user, based on the active policies.
    /// Note that the value may be incorrect if called before the team's billing metadata has been fetched.
    pub fn is_prompt_suggestions_toggleable(&self) -> bool {
        self.current_workspace()
            // If the user has no team, they can toggle prompt suggestions (no restrictions).
            .is_none_or(|workspace| {
                workspace
                    .billing_metadata
                    .tier
                    .warp_ai_policy
                    .is_some_and(|policy| policy.is_prompt_suggestions_toggleable)
            })
    }

    /// Whether Code Suggestions should be toggleable for the current user, based on the active policies.
    /// Note that the value may be incorrect if called before the team's billing metadata has been fetched.
    pub fn is_code_suggestions_toggleable(&self) -> bool {
        self.current_workspace()
            // If the user has no team, they can toggle code suggestions (no restrictions).
            .is_none_or(|workspace| {
                workspace
                    .billing_metadata
                    .tier
                    .warp_ai_policy
                    .is_some_and(|policy| policy.is_code_suggestions_toggleable)
            })
    }

    /// Whether Next Command should be toggleable for the current user, based on the active policies.
    /// Note that the value may be incorrect if called before the team's billing metadata has been fetched.
    pub fn is_next_command_enabled(&self) -> bool {
        self.current_workspace()
            // If the user has no team, they can toggle Next Command (no restrictions).
            .is_none_or(|workspace| {
                workspace
                    .billing_metadata
                    .tier
                    .warp_ai_policy
                    .is_some_and(|policy| policy.is_next_command_enabled)
            })
    }

    /// Whether Git Operations AI is enabled for the current user, based on the active policies.
    /// Note that the value may be incorrect if called before the team's billing metadata has been fetched.
    pub fn is_git_operations_ai_enabled(&self) -> bool {
        self.current_workspace()
            // If the user has no team, they can toggle Git Operations AI (no restrictions).
            .is_none_or(|workspace| {
                workspace
                    .billing_metadata
                    .tier
                    .warp_ai_policy
                    .is_some_and(|policy| policy.is_git_operations_ai_enabled)
            })
    }

    /// Whether voice input should be toggleable for the current user, based on the active policies.
    /// Note that the value may be incorrect if called before the team's billing metadata has been fetched.
    /// If voice input support is not compiled into this build, always returns `false`.
    pub fn is_voice_enabled(&self) -> bool {
        cfg!(feature = "voice_input")
            && self
                .current_workspace()
                // If the user has no team, they can toggle Voice (no restrictions).
                .is_none_or(|workspace| {
                    workspace
                        .billing_metadata
                        .tier
                        .warp_ai_policy
                        .is_some_and(|policy| policy.is_voice_enabled)
                })
    }

    /// Whether BYO API key is enabled for the current user, based on the active policies.
    /// Note that the value may be incorrect if called before the team's billing metadata has been fetched.
    /// For solo users (no workspace), this is controlled by the `SoloUserByok` feature flag.
    /// Anonymous or logged-out users are not allowed to use BYO API keys.
    pub fn is_byo_api_key_enabled(&self, app: &AppContext) -> bool {
        if AuthStateProvider::as_ref(app)
            .get()
            .is_anonymous_or_logged_out()
        {
            return false;
        }
        self.current_workspace()
            .map(|workspace| workspace.billing_metadata.is_byo_api_key_enabled())
            .unwrap_or(FeatureFlag::SoloUserByok.is_enabled())
    }

    /// Whether custom inference endpoints are enabled for the current user.
    /// Anonymous or logged-out users are not allowed to use custom inference.
    /// Controlled by the BYO_ENDPOINT billing policy.
    pub fn is_byo_endpoint_enabled(&self, app: &AppContext) -> bool {
        if AuthStateProvider::as_ref(app)
            .get()
            .is_anonymous_or_logged_out()
        {
            return false;
        }

        self.current_workspace()
            .map(|workspace| workspace.billing_metadata.is_byo_endpoint_enabled())
            .unwrap_or(true)
    }

    /// Whether the current workspace's plan manages BYOK/BYOE centrally.
    ///
    /// A workspace-level plan entitlement that turns on the team-scoped `team_byo` policy; see the
    /// [`crate::workspaces::user_workspaces::team_workspace_settings`] module docs for the
    /// two-layer model.
    pub(crate) fn is_managed_byok_byoe_enabled(&self) -> bool {
        self.current_workspace_billing_metadata()
            .is_some_and(|billing| billing.is_managed_byok_byoe_enabled())
    }
}
