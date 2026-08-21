use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::Result;
use regex::Regex;
use warp_core::features::FeatureFlag;
use warp_core::settings::{ChangeEventReason, Setting};
use warp_errors::report_error;
use warp_graphql::workspace::FeatureModelChoice;
use warpui::{
    AppContext, Entity, ModelContext, SingletonEntity, Tracked, ViewContext, WeakViewHandle,
    WindowId,
};

#[cfg(test)]
use super::team::TeamVisibility;
use super::team::{DiscoverableTeam, MembershipRole, Team};
#[cfg(test)]
use super::workspace::WorkspaceMemberUsageInfo;
use super::workspace::{
    AdminEnablementSetting, BillingMetadata, CustomerType, EnterpriseSecretRegex,
    HostEnablementSetting, UgcCollectionEnablementSetting, Workspace, WorkspaceUid,
};
use crate::ai::credit_availability::AICreditAvailability;
use crate::ai::llms::{LLMModelHost, LLMProvider};
use crate::ai::request_usage_model::AIRequestUsageModel;
use crate::auth::{AuthStateProvider, UserUid};
use crate::channel::ChannelState;
use crate::cloud_object::model::persistence::CloudModel;
use crate::cloud_object::{CloudObjectEventEntrypoint, ObjectType, Owner, Space};
use crate::pricing::PricingInfoModel;
use crate::server::experiments::{ServerExperiment, ServerExperiments, ServerExperimentsEvent};
use crate::server::ids::ServerId;
use crate::server::server_api::team::TeamClient;
use crate::server::server_api::workspace::{PurchaseAddonCreditsOutcome, WorkspaceClient};
#[cfg(test)]
use crate::server::server_api::{team::MockTeamClient, workspace::MockWorkspaceClient};
use crate::settings::{
    AISettings, AISettingsChangedEvent, CodeSettings, CodeSettingsChangedEvent, PrivacySettings,
};
#[cfg(test)]
use crate::workspaces::workspace::{AIAutonomyPolicy, WorkspaceMember, WorkspaceSettings};
use crate::workspaces::workspace::{
    AiAutonomySettings, AiOverages, LlmHostSettings, LlmSettings, PurchaseAddOnCreditsPolicy,
    SandboxedAgentSettings, UsageBasedPricingSettings,
};

const STRIPE_SUBSCRIPTION_INTERVAL_PAGE_PREFIX: &str = "/upgrade";

#[derive(Debug)]
pub enum UserWorkspacesEvent {
    AddDomainRestrictionsSuccess,
    AddDomainRestrictionsRejected(anyhow::Error),
    DeleteDomainRestrictionSuccess,
    DeleteDomainRestrictionRejected(anyhow::Error),
    EmailInviteSent,
    EmailInviteRejected(anyhow::Error),
    ToggleInviteLinksSuccess,
    ToggleInviteLinksRejected(anyhow::Error),
    ResetInviteLinks,
    ResetInviteLinksRejected(anyhow::Error),
    DeleteTeamInvite,
    DeleteTeamInviteRejected(anyhow::Error),
    GenerateUpgradeLink(String),
    GenerateUpgradeLinkRejected(anyhow::Error),
    GenerateStripeBillingPortalLink(String),
    GenerateStripeBillingPortalLinkRejected(anyhow::Error),
    ToggleTeamDiscoverabilitySuccess,
    ToggleTeamDiscoverabilityRejected(anyhow::Error),
    JoinTeamWithTeamDiscoverySuccess,
    JoinTeamWithTeamDiscoveryRejected(anyhow::Error),
    FetchDiscoverableTeamsSuccess(Vec<DiscoverableTeam>),
    FetchDiscoverableTeamsRejected(anyhow::Error),
    TransferTeamOwnershipSuccess,
    TransferTeamOwnershipRejected(anyhow::Error),
    SetTeamMemberRoleSuccess,
    SetTeamMemberRoleRejected(anyhow::Error),
    RemoveUserFromTeamSuccess,
    RemoveUserFromTeamRejected(anyhow::Error),
    UpdateWorkspaceSettingsSuccess,
    UpdateWorkspaceSettingsRejected(anyhow::Error),
    AiOveragesUpdated,
    PurchaseAddonCreditsSuccess,
    /// The purchase requires the user to complete checkout in the browser
    /// (no saved payment method). Credits arrive via webhook + polling after
    /// checkout completes.
    PurchaseAddonCreditsCheckoutRequired {
        checkout_url: String,
    },
    PurchaseAddonCreditsRejected(anyhow::Error),
    /// Fired whenever the set of teams the user is on changes.
    TeamsChanged,
    /// Fired when the selected workspace actually changes to a different one.
    CurrentWorkspaceChanged,
    /// Fired when a single window's team assignment changes. Windows are independent, so
    /// subscribers that hold per-window state must only react to their own window.
    WindowTeamChanged {
        window_id: WindowId,
    },
    CodebaseContextEnablementChanged,
    /// Fired when a service agreement's sunsetted_to_build_ts field is updated.
    SunsettedToBuildDataUpdated,
}

/// UserWorkspaces is a singleton model that holds workspace metadata (name, members, etc).
/// It should be used for getting information about the workspaces, teams, current teams,
/// and all other things related to operating on workspace and team data.
/// TODO: move other server_api calls to update_manager to correctly update sqlite.
pub struct UserWorkspaces {
    current_workspace_uid: Tracked<Option<WorkspaceUid>>,
    workspaces: Tracked<Vec<Workspace>>,
    window_team_uids: HashMap<WindowId, Option<ServerId>>,
    joinable_teams: Vec<DiscoverableTeam>,
    /// The user-level add-on credits purchase policy from the latest
    /// workspaces-metadata response. Teamless (fresh free) users have no
    /// team and their only workspace is the server's placeholder, which is
    /// filtered out of `workspaces` — this is the only place their purchase
    /// policy survives.
    user_purchase_policy: Option<PurchaseAddOnCreditsPolicy>,
    team_client: Arc<dyn TeamClient>,
    workspace_client: Arc<dyn WorkspaceClient>,
}

/// Represents the workspaces a user potentially has access to.
#[derive(Clone)]
pub struct WorkspacesMetadataResponse {
    /// The list of workspaces the user is currently on.
    pub workspaces: Vec<Workspace>,
    /// The list of discoverable teams that the user can join.
    pub joinable_teams: Vec<DiscoverableTeam>,
    /// The list of experiments applicable to the user.
    pub experiments: Option<Vec<ServerExperiment>>,
    /// TODO(Tyler): Post-workspaces, move this into the workspace object.
    /// Feature model choices may change from user to user and while the app is open, so we need to periodically update this list.
    /// It makes most sense to fetch this in workspaces which is queried every 10 minutes.
    /// This is list of available LLM models for the user.
    pub feature_model_choices: Option<FeatureModelChoice>,
    /// The server-authoritative AI credit availability decision, piggybacked
    /// on the metadata query so every refresh keeps the shared state fresh.
    pub ai_credit_availability: Option<AICreditAvailability>,
    /// The user-level add-on credits purchase policy; the teamless-purchase
    /// fallback (see [`UserWorkspaces::purchase_policy`]).
    pub user_purchase_policy: Option<PurchaseAddOnCreditsPolicy>,
}

// A representation of all data we fetch at a single time via our 10 minute poll.
// Prefer adding to this struct if you need relatively fresh data vs making
// independent queries.
pub struct WorkspacesMetadataWithPricing {
    pub metadata: WorkspacesMetadataResponse,
    pub pricing_info: Option<warp_graphql::billing::PricingInfo>,
}

pub struct CreateTeamResponse {
    pub workspace: Workspace,
    pub team: Team,
}

/// The team an operation is scoped to, captured once from the window that started it.
///
/// A logical operation carries its `TeamContext` from start to finish instead of asking a
/// window which team is selected now, so concurrent windows on different teams stay
/// independent and a later team switch cannot retarget work already in flight.
///
/// Deliberately neither `Clone` nor `Copy`. Moves make the handoff between the parts of an
/// operation explicit and reviewable, whereas copies let a scope leak sideways into work
/// that never established it. Wanting to duplicate one is a sign the second consumer is
/// really a separate operation that should capture its own scope; if the parts genuinely
/// share a lifetime, restructure so they share the single owner instead.
///
/// This is scope, not authority: the server still authorizes every request made under it.
pub(crate) struct TeamContext {
    team_uid: ServerId,
}

/// An opaque key identifying the team scope a [`TeamContext`] was captured for, or a
/// resolved no-team operation. Lets an external per-team cache (e.g. a per-team model or
/// usage-state map) key its entries by team without itself extracting or comparing raw
/// UIDs; the only operations available on it are equality and hashing. Minted only by
/// [`UserWorkspaces::cache_key_for_context`], and checked for continued team membership
/// only by [`UserWorkspaces::is_team_scope_key_current`].
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct TeamScopeKey(Option<ServerId>);

/// The team a view renders as, borrowed for the duration of a single render.
///
/// Current-team UI must reflect the window's team as of this frame, so this is resolved
/// per render rather than cached. The borrow is what enforces that: it cannot be stored in
/// view state or moved into a `'static` future, and it deliberately offers no conversion to
/// a team UID or to a [`TeamContext`]. A [`WeakViewHandle`] locates a window to read from;
/// it is not evidence that the holder is running in that window, which is what minting
/// operation scope requires.
pub(crate) struct TeamRenderContext<'a> {
    team: &'a Team,
}

impl UserWorkspaces {
    #[cfg(any(test, all(feature = "tui", feature = "test-util")))]
    pub fn mock(
        team_client: Arc<dyn TeamClient>,
        workspace_client: Arc<dyn WorkspaceClient>,
        cached_workspaces: Vec<Workspace>,
        _ctx: &mut ModelContext<Self>,
    ) -> Self {
        // In tests, avoid subscribing to [`ServerExperiments`] because it
        // requires us to register that singleton along with _its_ dependencies
        // for all tests that use [`UserWorkspaces`] (a lot of them do).
        Self {
            current_workspace_uid: cached_workspaces.first().map(|w| w.uid).into(),
            workspaces: cached_workspaces.into(),
            window_team_uids: Default::default(),
            joinable_teams: Default::default(),
            user_purchase_policy: None,
            team_client,
            workspace_client,
        }
    }

    #[cfg(test)]
    pub fn default_mock(ctx: &mut ModelContext<Self>) -> Self {
        Self::mock(
            Arc::new(MockTeamClient::new()),
            Arc::new(MockWorkspaceClient::new()),
            vec![],
            ctx,
        )
    }

    pub fn new(
        team_client: Arc<dyn TeamClient>,
        workspace_client: Arc<dyn WorkspaceClient>,
        cached_workspaces: Vec<Workspace>,
        current_workspace_uid: Option<WorkspaceUid>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(&ServerExperiments::handle(ctx), |me, _, event, ctx| {
            let ServerExperimentsEvent::ExperimentsUpdated = event;
            me.update_session_sharing_enablement(ctx);
        });

        ctx.subscribe_to_model(
            &CodeSettings::handle(ctx),
            |_, _, code_settings_event, ctx| match code_settings_event {
                CodeSettingsChangedEvent::CodebaseContextEnabled { .. }
                | CodeSettingsChangedEvent::AutoIndexingEnabled { .. } => {
                    ctx.emit(UserWorkspacesEvent::CodebaseContextEnablementChanged);
                }
                _ => {}
            },
        );

        ctx.subscribe_to_model(&AISettings::handle(ctx), |_, _, ai_settings_event, ctx| {
            if let AISettingsChangedEvent::IsAnyAIEnabled { .. } = ai_settings_event {
                ctx.emit(UserWorkspacesEvent::CodebaseContextEnablementChanged);
            }
        });

        Self {
            current_workspace_uid: current_workspace_uid.into(),
            workspaces: cached_workspaces.into(),
            window_team_uids: Default::default(),
            joinable_teams: Default::default(),
            user_purchase_policy: None,
            team_client,
            workspace_client,
        }
    }

    pub fn upgrade_link(user_id: UserUid) -> String {
        format!(
            "{}{}/{}/{}",
            ChannelState::server_root_url(),
            STRIPE_SUBSCRIPTION_INTERVAL_PAGE_PREFIX,
            "user",
            user_id.as_str()
        )
    }

    pub fn upgrade_link_for_team(team_uid: ServerId) -> String {
        format!(
            "{}{}/{}",
            ChannelState::server_root_url(),
            STRIPE_SUBSCRIPTION_INTERVAL_PAGE_PREFIX,
            team_uid
        )
    }

    pub fn warp_agent_cli_upgrade_link(user_id: Option<UserUid>) -> String {
        let upgrade_link = user_id.map_or_else(
            || {
                format!(
                    "{}{}",
                    ChannelState::server_root_url().trim_end_matches('/'),
                    STRIPE_SUBSCRIPTION_INTERVAL_PAGE_PREFIX
                )
            },
            Self::upgrade_link,
        );
        format!("{upgrade_link}?source=warp-agent-cli")
    }
    pub fn admin_billing_link_for_team(team_uid: ServerId) -> String {
        format!(
            "{}/admin/{team_uid}/billing",
            ChannelState::server_root_url().trim_end_matches('/')
        )
    }

    pub fn admin_billing_link_for_default_team(&self, user_email: &str) -> Option<String> {
        let team_uid = self.inherited_or_default_team_uid(None)?;
        self.team_from_uid(team_uid)
            .filter(|team| team.has_admin_permissions(user_email))
            .map(|_| Self::admin_billing_link_for_team(team_uid))
    }

    pub fn team_from_uid(&self, team_uid: ServerId) -> Option<&Team> {
        self.current_workspace()
            .and_then(|w| w.teams.iter().find(|t| t.uid == team_uid))
    }

    pub fn register_window(
        &mut self,
        window_id: WindowId,
        team_uid: Option<ServerId>,
        ctx: &mut ModelContext<Self>,
    ) {
        let previous_team_uid = self.team_uid_for_window(window_id);
        self.window_team_uids.entry(window_id).or_insert(team_uid);
        if self.team_uid_for_window(window_id) != previous_team_uid {
            ctx.emit(UserWorkspacesEvent::WindowTeamChanged { window_id });
        }
        ctx.notify();
    }
    pub fn inherited_or_default_team_uid(
        &self,
        source_window_id: Option<WindowId>,
    ) -> Option<ServerId> {
        source_window_id
            .and_then(|source_window_id| self.team_uid_for_window(source_window_id))
            .or_else(|| {
                self.current_workspace()
                    .and_then(|workspace| workspace.teams.first())
                    .map(|team| team.uid)
            })
    }

    pub fn set_team_for_window(
        &mut self,
        window_id: WindowId,
        team_uid: ServerId,
        ctx: &mut ModelContext<Self>,
    ) {
        let window_team_uid = self.window_team_uids.entry(window_id).or_default();
        if window_team_uid.is_none() {
            *window_team_uid = Some(team_uid);
            ctx.emit(UserWorkspacesEvent::WindowTeamChanged { window_id });
            ctx.notify();
        }
    }

    pub fn team_uid_for_window(&self, window_id: WindowId) -> Option<ServerId> {
        self.window_team_uids.get(&window_id).copied().flatten()
    }

    /// Returns `true` when the user belongs to more than one team in the current
    /// workspace, meaning the team-switcher pill and dropdown should be shown.
    /// Single-team and no-workspace users return `false` so their UI is unchanged.
    pub fn can_switch_teams(&self) -> bool {
        self.current_workspace()
            .map(|ws| ws.teams.len() > 1)
            .unwrap_or(false)
    }
    pub fn team_for_window(&self, window_id: WindowId) -> Option<&Team> {
        self.team_uid_for_window(window_id)
            .and_then(|team_uid| self.team_from_uid(team_uid))
    }
    pub fn team_for_view<T: Entity>(&self, ctx: &ViewContext<T>) -> Option<&Team> {
        self.team_for_window(ctx.window_id())
    }

    pub fn team_for_view_handle<T: Entity>(
        &self,
        view_handle: &WeakViewHandle<T>,
        ctx: &AppContext,
    ) -> Option<&Team> {
        view_handle
            .window_id(ctx)
            .and_then(|window_id| self.team_for_window(window_id))
    }

    /// Captures the team selected in `ctx`'s window as an operation's [`TeamContext`]. This
    /// is the only way application code mints one.
    pub(crate) fn team_context_for_view<T: Entity>(
        &self,
        ctx: &ViewContext<T>,
    ) -> Option<TeamContext> {
        self.team_context_for_window(ctx.window_id())
    }

    pub(crate) fn team_context_for_window(&self, window_id: WindowId) -> Option<TeamContext> {
        self.team_uid_for_window(window_id)
            .map(|team_uid| TeamContext { team_uid })
    }

    /// Resolves `view`'s window team for one render. See [`TeamRenderContext`].
    pub(crate) fn team_render_context_for_view_handle<'a, T: Entity>(
        &'a self,
        view: &WeakViewHandle<T>,
        app: &AppContext,
    ) -> Option<TeamRenderContext<'a>> {
        let window_id = view.window_id(app)?;
        let team_uid = self.team_uid_for_window(window_id)?;
        let team = self.team_from_uid(team_uid)?;

        Some(TeamRenderContext { team })
    }

    /// Reads a captured team's metadata. Returns `None` once that team is gone from the
    /// current workspace, e.g. after the user leaves it.
    pub(crate) fn team_for_context(&self, context: &TeamContext) -> Option<&Team> {
        self.team_from_uid(context.team_uid)
    }

    /// Mints the opaque cache key for `context`'s scope, or for a resolved no-team
    /// operation when `context` is `None`. See [`TeamScopeKey`].
    pub(crate) fn cache_key_for_context(context: Option<&TeamContext>) -> TeamScopeKey {
        TeamScopeKey(context.map(|context| context.team_uid))
    }

    /// Whether `key`'s scope is still current: a resolved no-team scope always is; a real
    /// team scope is current only while that team is still a member of some workspace.
    /// Used to drop a cache entry (or an in-flight fetch's result) for a team the user has
    /// left, without the caller ever needing the raw UID `key` was minted from.
    pub(crate) fn is_team_scope_key_current(&self, key: TeamScopeKey) -> bool {
        key.0
            .is_none_or(|uid| self.team_uids_across_all_workspaces().contains(&uid))
    }

    /// Returns the windows whose team assignment changed.
    #[must_use]
    fn reconcile_window_team_assignments(&mut self) -> Vec<WindowId> {
        let team_uids = self
            .current_workspace()
            .map(|workspace| {
                workspace
                    .teams
                    .iter()
                    .map(|team| team.uid)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let fallback_team_uid = team_uids.first().copied();

        let mut reassigned_windows = Vec::new();
        for (window_id, window_team_uid) in self.window_team_uids.iter_mut() {
            if window_team_uid.is_none_or(|team_uid| !team_uids.contains(&team_uid))
                && *window_team_uid != fallback_team_uid
            {
                *window_team_uid = fallback_team_uid;
                reassigned_windows.push(*window_id);
            }
        }
        reassigned_windows
    }

    fn emit_window_team_changed(windows: Vec<WindowId>, ctx: &mut ModelContext<Self>) {
        for window_id in windows {
            ctx.emit(UserWorkspacesEvent::WindowTeamChanged { window_id });
        }
    }

    pub fn team_from_uid_across_all_workspaces(&self, team_uid: ServerId) -> Option<&Team> {
        self.workspaces
            .iter()
            .flat_map(|w| w.teams.iter())
            .find(|t| t.uid == team_uid)
    }

    /// The teams [`Self::owner_to_space`] recognizes. An owner naming a team outside this set
    /// resolves to the shared space instead of that team's space, so a change here remaps
    /// objects between spaces without any of them changing.
    pub fn team_uids_across_all_workspaces(&self) -> HashSet<ServerId> {
        self.workspaces
            .iter()
            .flat_map(|workspace| workspace.teams.iter())
            .map(|team| team.uid)
            .collect()
    }

    pub fn workspace_from_uid(&self, workspace_uid: WorkspaceUid) -> Option<&Workspace> {
        self.workspaces.iter().find(|w| w.uid == workspace_uid)
    }

    pub fn workspace_from_uid_mut(
        &mut self,
        workspace_uid: WorkspaceUid,
    ) -> Option<&mut Workspace> {
        self.workspaces.iter_mut().find(|w| w.uid == workspace_uid)
    }

    pub fn is_at_tier_limit_for_object_type(
        team_uid: ServerId,
        object_type: ObjectType,
        ctx: &AppContext,
    ) -> bool {
        match object_type {
            ObjectType::Notebook => {
                !UserWorkspaces::has_capacity_for_shared_notebooks(team_uid, ctx, 1)
            }
            ObjectType::Workflow => {
                !UserWorkspaces::has_capacity_for_shared_workflows(team_uid, ctx, 1)
            }
            ObjectType::Folder => false,
            ObjectType::GenericStringObject(_) => false,
        }
    }

    pub fn is_at_tier_limit_for_some_warp_drive_objects(
        team_uid: ServerId,
        ctx: &AppContext,
    ) -> bool {
        UserWorkspaces::is_at_tier_limit_for_object_type(team_uid, ObjectType::Notebook, ctx)
            || UserWorkspaces::is_at_tier_limit_for_object_type(team_uid, ObjectType::Workflow, ctx)
    }

    // Checks if the team has capacity for another shared notebook for their current
    // billing tier, given their current notebook count and delinquency status.
    pub fn has_capacity_for_shared_notebooks(
        team_uid: ServerId,
        ctx: &AppContext,
        new_shared_notebooks: usize,
    ) -> bool {
        let current_shared_notebooks = CloudModel::as_ref(ctx)
            .active_notebooks_in_space(Space::Team { team_uid }, ctx)
            .count();

        let team = UserWorkspaces::as_ref(ctx).team_from_uid(team_uid);
        if let Some(team) = team {
            // If the team is past due or unpaid, then don't allow new notebooks.
            if team.billing_metadata.is_delinquent_due_to_payment_issue() {
                return false;
            }

            if let Some(policy) = team.billing_metadata.tier.shared_notebooks_policy {
                // Allow new notebooks if policy is unlimited or if the number of notebooks
                // is less than the limit.
                policy.is_unlimited
                    || current_shared_notebooks + new_shared_notebooks
                        <= policy
                            .limit
                            .try_into()
                            .expect("shared notebooks limit should be within max i64 range")
            } else {
                // If no policy is set, then allow it to go through by default (should still be enforced server-side)
                true
            }
        } else {
            // If the team is not found, then allow it to go through by default (should still be enforced server-side)
            true
        }
    }

    // Checks if the team has capacity for another shared workflow for their current
    // billing tier, given their current workflow count and delinquency status.
    pub fn has_capacity_for_shared_workflows(
        team_uid: ServerId,
        ctx: &AppContext,
        new_shared_workflows: usize,
    ) -> bool {
        let current_shared_workflows = CloudModel::as_ref(ctx)
            .active_workflows_in_space(Space::Team { team_uid }, ctx)
            .count();

        let team = UserWorkspaces::as_ref(ctx).team_from_uid(team_uid);
        if let Some(team) = team {
            // If the team is past due or unpaid, then don't allow new workflows.
            if team.billing_metadata.is_delinquent_due_to_payment_issue() {
                return false;
            }

            if let Some(policy) = team.billing_metadata.tier.shared_workflows_policy {
                // Allow new workflows if policy is unlimited or if the number of workflows
                // is less than the limit.
                policy.is_unlimited
                    || current_shared_workflows + new_shared_workflows
                        <= policy
                            .limit
                            .try_into()
                            .expect("shared workflows limit should be within max i64 range")
            } else {
                // If no policy is set, then allow it to go through by default (should still be enforced server-side)
                true
            }
        } else {
            // If the team is not found, then allow it to go through by default (should still be enforced server-side)
            true
        }
    }

    pub fn sole_team(&self) -> Option<&Team> {
        let [team] = self.current_workspace()?.teams.as_slice() else {
            return None;
        };
        Some(team)
    }

    pub fn sole_team_uid(&self) -> Option<ServerId> {
        self.sole_team().map(|team| team.uid)
    }

    /// Note that the workspace is populated with dummy data until the initial fetch
    /// completes (only workspace name/ID and workspace team's name/ID are cached in
    /// sqlite locally).
    /// Consider whether you need to wait for the results of the fetch before checking the
    /// values of other fields.
    pub fn current_workspace(&self) -> Option<&Workspace> {
        self.current_workspace_uid
            .and_then(|workspace_uid| self.workspace_from_uid(workspace_uid))
    }
    pub fn current_workspace_billing_metadata(&self) -> Option<&BillingMetadata> {
        self.current_workspace()
            .map(|workspace| &workspace.billing_metadata)
    }

    /// The given team's billing metadata when the team is known, otherwise
    /// the current workspace's. For purchase surfaces that need
    /// team/workspace-scoped state (e.g. delinquency); for the purchase
    /// policy itself use [`Self::purchase_policy_for_team`], which adds the
    /// user-level fallback for teamless users.
    pub fn team_billing_metadata<'a>(
        &'a self,
        team: Option<&'a Team>,
    ) -> Option<&'a BillingMetadata> {
        team.map(|team| &team.billing_metadata)
            .or_else(|| self.current_workspace_billing_metadata())
    }

    fn llm_settings_for_team<'a>(&'a self, team: Option<&'a Team>) -> Option<&'a LlmSettings> {
        match team {
            Some(team) => Some(&team.settings.llm_settings),
            None => self
                .current_workspace()
                .map(|workspace| &workspace.settings.llm_settings),
        }
    }

    pub(crate) fn llm_settings_for_context(
        &self,
        context: Option<&TeamContext>,
    ) -> Option<&LlmSettings> {
        match context {
            Some(context) => self
                .team_for_context(context)
                .map(|team| &team.settings.llm_settings),
            None => self.llm_settings_for_team(None),
        }
    }

    pub(crate) fn llm_settings_for_render_context<'a>(
        &'a self,
        context: Option<&TeamRenderContext<'a>>,
    ) -> Option<&'a LlmSettings> {
        self.llm_settings_for_team(context.map(|context| context.team))
    }

    fn llm_settings_for_team_uid(&self, team_uid: Option<ServerId>) -> Option<&LlmSettings> {
        match team_uid {
            Some(team_uid) => self
                .team_from_uid(team_uid)
                .map(|team| &team.settings.llm_settings),
            None => self.llm_settings_for_team(None),
        }
    }

    pub fn is_custom_llm_enabled_for_team(&self, team: Option<&Team>) -> bool {
        self.llm_settings_for_team(team)
            .is_some_and(|settings| settings.enabled)
    }

    /// The add-on credits purchase policy for the current viewer context: the
    /// current workspace's policy when one exists, else the user-level policy
    /// from the workspaces-metadata response (how teamless users get one).
    ///
    /// Callers bound to a view/window should use
    /// [`Self::purchase_policy_for_team`] instead, since their team can
    /// differ from the current workspace's in multi-team situations.
    pub fn purchase_policy(&self) -> Option<PurchaseAddOnCreditsPolicy> {
        self.current_workspace_billing_metadata()
            .and_then(|billing| billing.tier.purchase_add_on_credits_policy)
            .or(self.user_purchase_policy)
    }

    /// [`Self::purchase_policy`], preferring the given team's policy when the
    /// team is known (e.g. resolved from a view or window).
    pub fn purchase_policy_for_team(
        &self,
        team: Option<&Team>,
    ) -> Option<PurchaseAddOnCreditsPolicy> {
        team.and_then(|team| team.billing_metadata.tier.purchase_add_on_credits_policy)
            .or_else(|| self.purchase_policy())
    }

    /// Updates the user-level add-on credits purchase policy captured from a
    /// workspaces-metadata response. Must be called on every path that
    /// applies such a response so the teamless fallback can't go stale.
    pub fn set_user_purchase_policy(&mut self, policy: Option<PurchaseAddOnCreditsPolicy>) {
        self.user_purchase_policy = policy;
    }

    pub fn current_workspace_mut(&mut self) -> Option<&mut Workspace> {
        self.current_workspace_uid
            .and_then(|workspace_uid| self.workspace_from_uid_mut(workspace_uid))
    }

    pub fn workspaces(&self) -> &Vec<Workspace> {
        &self.workspaces
    }

    pub fn set_current_workspace_uid(
        &mut self,
        workspace_uid: WorkspaceUid,
        ctx: &mut ModelContext<Self>,
    ) {
        let changed = *self.current_workspace_uid != Some(workspace_uid);
        *self.current_workspace_uid = Some(workspace_uid);
        let reassigned_windows = self.reconcile_window_team_assignments();
        self.notify_and_emit_teams_changed(ctx);
        Self::emit_window_team_changed(reassigned_windows, ctx);
        if changed {
            ctx.emit(UserWorkspacesEvent::CurrentWorkspaceChanged);
        }
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
    pub(crate) fn are_member_byo_keys_allowed_for_context(
        &self,
        context: Option<&TeamContext>,
    ) -> bool {
        self.are_member_byo_keys_allowed_for_team(context.map(|context| context.team_uid))
    }

    pub(crate) fn are_member_byo_endpoints_allowed_for_context(
        &self,
        context: Option<&TeamContext>,
    ) -> bool {
        self.are_member_byo_endpoints_allowed_for_team(context.map(|context| context.team_uid))
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
            .map(|workspace| workspace.is_byo_api_key_enabled())
            .unwrap_or(FeatureFlag::SoloUserByok.is_enabled())
    }

    /// Whether the current workspace's managed BYOK/BYOE policy allows members
    /// to use their own provider API keys. Users with no workspace, or
    /// workspaces without the managed BYOK/BYOE policy, have no team-level
    /// restriction, so this returns true and the normal BYO entitlement applies.
    pub fn are_member_byo_keys_allowed(&self) -> bool {
        self.current_workspace().is_none_or(|workspace| {
            !workspace.billing_metadata.is_managed_byok_byoe_enabled()
                || workspace
                    .settings
                    .team_byo
                    .as_ref()
                    .is_some_and(|team_byo| {
                        team_byo.first_party_enabled && team_byo.allow_user_keys
                    })
        })
    }
    /// Whether custom inference endpoints are enabled for the current user.
    /// Anonymous or logged-out users are not allowed to use custom inference.
    /// Controlled by the BYO_ENDPOINT billing policy.
    pub fn is_custom_inference_enabled(&self, app: &AppContext) -> bool {
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

    /// Whether the current workspace's managed BYOK/BYOE policy allows members
    /// to use their own custom endpoints. Users with no workspace, or
    /// workspaces without the managed BYOK/BYOE policy, have no team-level
    /// restriction, so this returns true and the normal BYO entitlement applies.
    pub fn are_member_byo_endpoints_allowed(&self) -> bool {
        self.current_workspace().is_none_or(|workspace| {
            !workspace.billing_metadata.is_managed_byok_byoe_enabled()
                || workspace
                    .settings
                    .team_byo
                    .as_ref()
                    .is_some_and(|team_byo| {
                        team_byo.endpoints_enabled && team_byo.allow_user_endpoints
                    })
        })
    }

    /// Whether the current workspace's plan manages BYOK/BYOE centrally. Billing metadata is
    /// workspace-owned, so this is a plan entitlement independent of which team a window has
    /// selected; it gates the team-scoped `team_byo` policy that
    /// [`Self::agent_settings_are_member_byo_keys_allowed`],
    /// [`Self::agent_settings_are_member_byo_endpoints_allowed`], and
    /// [`Self::agent_settings_has_team_first_party_key`] read.
    pub fn is_managed_byok_byoe_enabled(&self) -> bool {
        self.current_workspace_billing_metadata()
            .is_some_and(|billing| billing.is_managed_byok_byoe_enabled())
    }

    /// [`Self::are_member_byo_keys_allowed`], but reading `team_uid`'s team's effective
    /// `TeamSettings.team_byo` policy instead of the workspace-level settings.
    pub(crate) fn are_member_byo_keys_allowed_for_team(&self, team_uid: Option<ServerId>) -> bool {
        !self.is_managed_byok_byoe_enabled()
            || match team_uid {
                Some(team_uid) => self.team_from_uid(team_uid).is_some_and(|team| {
                    team.settings.team_byo.as_ref().is_some_and(|team_byo| {
                        team_byo.first_party_enabled && team_byo.allow_user_keys
                    })
                }),
                None => true,
            }
    }

    /// [`Self::are_member_byo_endpoints_allowed`], but reading `team_uid`'s team's effective
    /// `TeamSettings.team_byo` policy instead of the workspace-level settings.
    pub(crate) fn are_member_byo_endpoints_allowed_for_team(
        &self,
        team_uid: Option<ServerId>,
    ) -> bool {
        !self.is_managed_byok_byoe_enabled()
            || match team_uid {
                Some(team_uid) => self.team_from_uid(team_uid).is_some_and(|team| {
                    team.settings.team_byo.as_ref().is_some_and(|team_byo| {
                        team_byo.endpoints_enabled && team_byo.allow_user_endpoints
                    })
                }),
                None => true,
            }
    }

    /// [`Self::are_member_byo_keys_allowed_for_team`], scoped to `context`'s team.
    pub(crate) fn agent_settings_are_member_byo_keys_allowed(
        &self,
        context: Option<&TeamRenderContext<'_>>,
    ) -> bool {
        self.are_member_byo_keys_allowed_for_team(context.map(|context| context.team.uid))
    }

    /// [`Self::are_member_byo_endpoints_allowed_for_team`], scoped to `context`'s team.
    pub(crate) fn agent_settings_are_member_byo_endpoints_allowed(
        &self,
        context: Option<&TeamRenderContext<'_>>,
    ) -> bool {
        self.are_member_byo_endpoints_allowed_for_team(context.map(|context| context.team.uid))
    }

    /// Whether `team_uid`'s team has provided its own first-party key for `provider`.
    /// `team_uid` is `None`, or names a team no longer in the current workspace, for a
    /// no-team window, which has no team-provided key and returns `false`.
    pub fn has_team_first_party_key_for_team(
        &self,
        team_uid: Option<ServerId>,
        provider: LLMProvider,
    ) -> bool {
        self.is_managed_byok_byoe_enabled()
            && team_uid
                .and_then(|team_uid| self.team_from_uid(team_uid))
                .is_some_and(|team| {
                    team.settings.team_byo.as_ref().is_some_and(|team_byo| {
                        team_byo.first_party_enabled
                            && team_byo
                                .first_party_keys
                                .iter()
                                .any(|key| key.provider == provider)
                    })
                })
    }

    /// Whether `team_uid`'s team has configured and enabled a BYO endpoint model whose
    /// `config_key` is `model_config_key`. `team_uid` is `None`, or names a team no longer in
    /// the current workspace, for a no-team window, which has no team-provided endpoint and
    /// returns `false`.
    pub fn has_team_byo_endpoint_for_model_for_team(
        &self,
        team_uid: Option<ServerId>,
        model_config_key: &str,
    ) -> bool {
        self.is_managed_byok_byoe_enabled()
            && team_uid
                .and_then(|team_uid| self.team_from_uid(team_uid))
                .is_some_and(|team| {
                    team.settings.team_byo.as_ref().is_some_and(|team_byo| {
                        team_byo.endpoints_enabled
                            && team_byo.endpoints.iter().any(|endpoint| {
                                endpoint.enabled
                                    && endpoint.models.iter().any(|model| {
                                        model.enabled && model.config_key == model_config_key
                                    })
                            })
                    })
                })
    }

    /// [`Self::has_team_first_party_key_for_team`], scoped to `context`'s team.
    pub(crate) fn agent_settings_has_team_first_party_key(
        &self,
        context: Option<&TeamRenderContext<'_>>,
        provider: LLMProvider,
    ) -> bool {
        self.has_team_first_party_key_for_team(context.map(|context| context.team.uid), provider)
    }

    /// Whether `user_email` has admin permissions on `context`'s team.
    pub(crate) fn agent_settings_team_has_admin_permissions(
        &self,
        context: &TeamRenderContext<'_>,
        user_email: &str,
    ) -> bool {
        context.team.has_admin_permissions(user_email)
    }

    /// The Build-plan upgrade link for `context`'s team.
    pub(crate) fn agent_settings_upgrade_link_for_team(
        &self,
        context: &TeamRenderContext<'_>,
    ) -> String {
        Self::upgrade_link_for_team(context.team.uid)
    }

    fn host_settings(
        llm_settings: Option<&LlmSettings>,
        host: LLMModelHost,
    ) -> Option<&LlmHostSettings> {
        llm_settings?.host_configs.get(&host)
    }

    fn host_is_available(llm_settings: Option<&LlmSettings>, host: LLMModelHost) -> bool {
        llm_settings.is_some_and(|llm_settings| {
            llm_settings.enabled
                && Self::host_settings(Some(llm_settings), host)
                    .is_some_and(|settings| settings.enabled)
        })
    }

    fn host_enablement_setting(
        llm_settings: Option<&LlmSettings>,
        host: LLMModelHost,
    ) -> HostEnablementSetting {
        Self::host_settings(llm_settings, host)
            .map(|settings| settings.enablement_setting.clone())
            .unwrap_or_default()
    }

    fn host_credentials_enabled(
        llm_settings: Option<&LlmSettings>,
        host: LLMModelHost,
        user_setting_enabled: bool,
    ) -> bool {
        if !Self::host_is_available(llm_settings, host.clone()) {
            return false;
        }

        match Self::host_enablement_setting(llm_settings, host) {
            HostEnablementSetting::Enforce => true,
            HostEnablementSetting::RespectUserSetting => user_setting_enabled,
        }
    }

    pub(crate) fn is_aws_bedrock_credentials_enabled_for_context(
        &self,
        context: Option<&TeamContext>,
        app: &AppContext,
    ) -> bool {
        Self::host_credentials_enabled(
            self.llm_settings_for_context(context),
            LLMModelHost::AwsBedrock,
            *AISettings::as_ref(app)
                .aws_bedrock_credentials_enabled
                .value(),
        )
    }

    pub(crate) fn is_aws_bedrock_available_for_render_context(
        &self,
        context: Option<&TeamRenderContext<'_>>,
    ) -> bool {
        Self::host_is_available(
            self.llm_settings_for_render_context(context),
            LLMModelHost::AwsBedrock,
        )
    }

    pub(crate) fn aws_bedrock_host_enablement_setting_for_render_context(
        &self,
        context: Option<&TeamRenderContext<'_>>,
    ) -> HostEnablementSetting {
        Self::host_enablement_setting(
            self.llm_settings_for_render_context(context),
            LLMModelHost::AwsBedrock,
        )
    }

    pub(crate) fn is_aws_bedrock_credentials_toggleable_for_render_context(
        &self,
        context: Option<&TeamRenderContext<'_>>,
    ) -> bool {
        matches!(
            self.aws_bedrock_host_enablement_setting_for_render_context(context),
            HostEnablementSetting::RespectUserSetting
        )
    }

    pub(crate) fn is_aws_bedrock_credentials_enabled_for_render_context(
        &self,
        context: Option<&TeamRenderContext<'_>>,
        app: &AppContext,
    ) -> bool {
        Self::host_credentials_enabled(
            self.llm_settings_for_render_context(context),
            LLMModelHost::AwsBedrock,
            *AISettings::as_ref(app)
                .aws_bedrock_credentials_enabled
                .value(),
        )
    }

    pub(crate) fn gemini_enterprise_host_settings_for_context(
        &self,
        context: Option<&TeamContext>,
    ) -> Option<&LlmHostSettings> {
        Self::host_settings(
            self.llm_settings_for_context(context),
            LLMModelHost::GeminiEnterprise,
        )
    }

    pub(crate) fn gemini_enterprise_host_settings_for_render_context<'a>(
        &'a self,
        context: Option<&TeamRenderContext<'a>>,
    ) -> Option<&'a LlmHostSettings> {
        Self::host_settings(
            self.llm_settings_for_render_context(context),
            LLMModelHost::GeminiEnterprise,
        )
    }

    pub(crate) fn is_gemini_enterprise_credentials_enabled_for_context(
        &self,
        context: Option<&TeamContext>,
        app: &AppContext,
    ) -> bool {
        if !FeatureFlag::GeminiEnterprise.is_enabled()
            || AuthStateProvider::as_ref(app)
                .get()
                .is_anonymous_or_logged_out()
        {
            return false;
        }

        Self::host_credentials_enabled(
            self.llm_settings_for_context(context),
            LLMModelHost::GeminiEnterprise,
            *AISettings::as_ref(app)
                .gemini_enterprise_credentials_enabled
                .value(),
        )
    }

    pub(crate) fn is_gemini_enterprise_available_for_render_context(
        &self,
        context: Option<&TeamRenderContext<'_>>,
    ) -> bool {
        Self::host_is_available(
            self.llm_settings_for_render_context(context),
            LLMModelHost::GeminiEnterprise,
        )
    }

    pub(crate) fn gemini_enterprise_host_enablement_setting_for_render_context(
        &self,
        context: Option<&TeamRenderContext<'_>>,
    ) -> HostEnablementSetting {
        Self::host_enablement_setting(
            self.llm_settings_for_render_context(context),
            LLMModelHost::GeminiEnterprise,
        )
    }

    pub(crate) fn is_gemini_enterprise_credentials_toggleable_for_render_context(
        &self,
        context: Option<&TeamRenderContext<'_>>,
    ) -> bool {
        matches!(
            self.gemini_enterprise_host_enablement_setting_for_render_context(context),
            HostEnablementSetting::RespectUserSetting
        )
    }

    pub(crate) fn is_aws_bedrock_credentials_enabled_for_team_uid(
        &self,
        team_uid: Option<ServerId>,
        app: &AppContext,
    ) -> bool {
        Self::host_credentials_enabled(
            self.llm_settings_for_team_uid(team_uid),
            LLMModelHost::AwsBedrock,
            *AISettings::as_ref(app)
                .aws_bedrock_credentials_enabled
                .value(),
        )
    }

    pub(crate) fn is_gemini_enterprise_credentials_enabled_for_team_uid(
        &self,
        team_uid: Option<ServerId>,
        app: &AppContext,
    ) -> bool {
        if !FeatureFlag::GeminiEnterprise.is_enabled()
            || AuthStateProvider::as_ref(app)
                .get()
                .is_anonymous_or_logged_out()
        {
            return false;
        }

        Self::host_credentials_enabled(
            self.llm_settings_for_team_uid(team_uid),
            LLMModelHost::GeminiEnterprise,
            *AISettings::as_ref(app)
                .gemini_enterprise_credentials_enabled
                .value(),
        )
    }

    pub(crate) fn is_aws_bedrock_credentials_enabled_for_any_scope(
        &self,
        app: &AppContext,
    ) -> bool {
        let Some(workspace) = self.current_workspace() else {
            return false;
        };
        let user_setting_enabled = *AISettings::as_ref(app)
            .aws_bedrock_credentials_enabled
            .value();
        if workspace.teams.is_empty() {
            return Self::host_credentials_enabled(
                Some(&workspace.settings.llm_settings),
                LLMModelHost::AwsBedrock,
                user_setting_enabled,
            );
        }
        workspace.teams.iter().any(|team| {
            Self::host_credentials_enabled(
                Some(&team.settings.llm_settings),
                LLMModelHost::AwsBedrock,
                user_setting_enabled,
            )
        })
    }

    pub(crate) fn is_gemini_enterprise_credentials_enabled_for_render_context(
        &self,
        context: Option<&TeamRenderContext<'_>>,
        app: &AppContext,
    ) -> bool {
        if !FeatureFlag::GeminiEnterprise.is_enabled()
            || AuthStateProvider::as_ref(app)
                .get()
                .is_anonymous_or_logged_out()
        {
            return false;
        }

        Self::host_credentials_enabled(
            self.llm_settings_for_render_context(context),
            LLMModelHost::GeminiEnterprise,
            *AISettings::as_ref(app)
                .gemini_enterprise_credentials_enabled
                .value(),
        )
    }

    /// Returns the AI autonomy settings that are enforced by the workspace for all its members.
    /// If a setting is `None`, the workspace doesn't enforce a particular setting.
    pub fn ai_autonomy_settings(&self) -> AiAutonomySettings {
        self.current_workspace()
            .map(|workspace| workspace.settings.ai_autonomy_settings.clone())
            .unwrap_or_default()
    }

    /// Returns the sandboxed agent settings enforced by the workspace, if any.
    pub fn sandboxed_agent_settings(&self) -> Option<SandboxedAgentSettings> {
        self.current_workspace()
            .and_then(|workspace| workspace.settings.sandboxed_agent_settings.clone())
    }

    /// Returns true iff AI autonomy features are allowed for this client.
    /// TODO: This should be deleted soon. AI autonomy settings have been moved into organization
    /// settings (see `ai_autonomy_settings` above), but there could be an interim time where we
    /// have not set up the org settings yet for an enterprise that previously had the entire
    /// feature set disabled. To capture that case, we'll see if all the settings are `None`;
    /// if so, we'll fall back to their billing metadata's value. Once we've migrated everyone
    /// into org settings, we should remove `is_enabled` from the policy and delete this function.
    pub fn is_ai_autonomy_allowed(&self) -> bool {
        self.current_workspace().is_none_or(|workspace| {
            let settings = &workspace.settings.ai_autonomy_settings;
            let all_settings_none = settings.apply_code_diffs_setting.is_none()
                && settings.read_files_setting.is_none()
                && settings.read_files_allowlist.is_none()
                && settings.execute_commands_setting.is_none()
                && settings.execute_commands_allowlist.is_none()
                && settings.execute_commands_denylist.is_none();

            if all_settings_none {
                workspace
                    .billing_metadata
                    .tier
                    .ai_autonomy_policy
                    .is_some_and(|policy| policy.is_enabled)
            } else {
                true
            }
        })
    }

    // Returns a Vec of the user's active spaces, based on their
    // team membership.
    pub fn team_spaces(&self) -> Vec<Space> {
        if let Some(workspace) = self.current_workspace() {
            workspace
                .teams
                .iter()
                .map(|team| Space::Team { team_uid: team.uid })
                .collect()
        } else {
            // If the user has no workspace, they have no team spaces.
            vec![]
        }
    }

    pub fn total_teammates_in_joinable_teams(&self) -> i64 {
        self.joinable_teams
            .iter()
            .map(|team| team.num_members)
            .sum()
    }

    pub fn num_joinable_teams(&self) -> usize {
        self.joinable_teams.len()
    }

    pub fn spaces_for_window(&self, window_id: WindowId, ctx: &AppContext) -> Vec<Space> {
        if AuthStateProvider::as_ref(ctx)
            .get()
            .is_user_web_anonymous_user()
            .unwrap_or_default()
        {
            return vec![Space::Shared];
        }
        let mut spaces = vec![];
        if let Some(team) = self.team_for_window(window_id) {
            spaces.push(Space::Team { team_uid: team.uid });
        }

        if FeatureFlag::SharedWithMe.is_enabled()
            && CloudModel::as_ref(ctx).has_directly_shared_objects(self, ctx)
        {
            spaces.push(Space::Shared);
        }
        spaces.push(Space::Personal);

        spaces
    }

    // Returns the [`Owner`] for the user's personal drive. If the user is not authenticated, this
    // returns `None`.
    pub fn personal_drive(&self, ctx: &AppContext) -> Option<Owner> {
        AuthStateProvider::as_ref(ctx)
            .get()
            .user_id()
            .map(|user_uid| Owner::User { user_uid })
    }

    // Maps a [`Space`] into an [`Owner`], based on the user's team memberships. If the space
    // does not directly identify an owner (it's the space for shared objects), returns `None`.
    pub fn space_to_owner(&self, space: Space, ctx: &AppContext) -> Option<Owner> {
        match space {
            Space::Team { team_uid } => Some(Owner::Team { team_uid }),
            Space::Personal => self.personal_drive(ctx),
            Space::Shared => None,
        }
    }

    // Maps an [`Owner`] into a [`Space`], based on the user's team memberships.
    // This is always possible, as unknown owners imply the shared space.
    pub fn owner_to_space(&self, owner: Owner, ctx: &AppContext) -> Space {
        match owner {
            Owner::User { user_uid } => {
                if !FeatureFlag::SharedWithMe.is_enabled() {
                    return Space::Personal;
                }

                let current_user = AuthStateProvider::as_ref(ctx).get().user_id();
                if Some(user_uid) == current_user {
                    Space::Personal
                } else {
                    Space::Shared
                }
            }
            Owner::Team { team_uid } => {
                if !FeatureFlag::SharedWithMe.is_enabled()
                    || self.team_from_uid_across_all_workspaces(team_uid).is_some()
                {
                    Space::Team { team_uid }
                } else {
                    Space::Shared
                }
            }
        }
    }

    pub fn has_teams(&self) -> bool {
        if let Some(workspace) = self.current_workspace() {
            !workspace.teams.is_empty()
        } else {
            false
        }
    }

    pub fn has_workspaces(&self) -> bool {
        !self.workspaces.is_empty()
    }

    pub fn update_workspaces(&mut self, workspaces: Vec<Workspace>, ctx: &mut ModelContext<Self>) {
        // Check if sunsetted_to_build_ts changed for any workspace
        let sunsetted_to_build_changed = self.has_sunsetted_to_build_data_changed(&workspaces);

        *self.workspaces = workspaces;
        let reassigned_windows = self.reconcile_window_team_assignments();
        self.notify_and_emit_teams_changed(ctx);
        Self::emit_window_team_changed(reassigned_windows, ctx);

        if sunsetted_to_build_changed {
            ctx.emit(UserWorkspacesEvent::SunsettedToBuildDataUpdated);
        }
    }

    /// Checks if any workspace's service agreement sunsetted_to_build_ts field has changed.
    fn has_sunsetted_to_build_data_changed(&self, new_workspaces: &[Workspace]) -> bool {
        for new_workspace in new_workspaces {
            // Find the corresponding old workspace
            let old_workspace = self.workspaces.iter().find(|w| w.uid == new_workspace.uid);

            if let Some(old_workspace) = old_workspace {
                // Check if any team's service agreement sunsetted_to_build_ts changed
                for new_team in &new_workspace.teams {
                    let old_team = old_workspace.teams.iter().find(|t| t.uid == new_team.uid);

                    if let Some(old_team) = old_team {
                        let old_sunsetted = old_team
                            .billing_metadata
                            .service_agreements
                            .first()
                            .and_then(|sa| sa.sunsetted_to_build_ts);

                        let new_sunsetted = new_team
                            .billing_metadata
                            .service_agreements
                            .first()
                            .and_then(|sa| sa.sunsetted_to_build_ts);

                        // Detect if it changed from None to Some or changed value
                        if old_sunsetted != new_sunsetted {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    fn notify_and_emit_teams_changed(&self, ctx: &mut ModelContext<Self>) {
        // Update session-sharing enablement since it depends on what teams the user
        // is part of.
        self.update_session_sharing_enablement(ctx);

        // PrivacySettings can't observe UserWorkspaces for updates, as it's initialized too early in
        // the app initialization flow. So, we update it manually whenever teams data changes.
        PrivacySettings::handle(ctx).update(ctx, |settings, ctx| {
            settings.set_is_telemetry_force_enabled(self.is_telemetry_force_enabled());
            settings.set_enterprise_secret_redaction_settings(
                self.is_enterprise_secret_redaction_enabled(),
                self.get_enterprise_secret_redaction_regex_list(),
                ChangeEventReason::CloudSync,
                ctx,
            );
        });

        ctx.emit(UserWorkspacesEvent::TeamsChanged);
        ctx.emit(UserWorkspacesEvent::CodebaseContextEnablementChanged);
        ctx.notify();
    }

    pub fn update_joinable_teams(
        &mut self,
        joinable_teams: Vec<DiscoverableTeam>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.joinable_teams.clone_from(&joinable_teams);
        ctx.emit(UserWorkspacesEvent::FetchDiscoverableTeamsSuccess(
            joinable_teams,
        ));
        ctx.notify();
    }

    // TODO follow up with moving other modifying calls out of UserWorkspaces to TeamUpdateManager
    fn on_workspaces_updated(
        &mut self,
        result: Result<WorkspacesMetadataWithPricing>,
        ctx: &mut ModelContext<Self>,
    ) {
        match result {
            Ok(response) => {
                if let Some(pricing_info) = response.pricing_info {
                    PricingInfoModel::handle(ctx).update(ctx, |model, ctx| {
                        model.update_pricing_info(pricing_info, ctx);
                    });
                }

                if let Some(availability) = response.metadata.ai_credit_availability {
                    AIRequestUsageModel::handle(ctx).update(ctx, |usage_model, ctx| {
                        usage_model.apply_server_availability(Ok(availability), ctx);
                    });
                }

                let workspaces = response.metadata.workspaces;
                let joinable_teams = response.metadata.joinable_teams;

                self.set_user_purchase_policy(response.metadata.user_purchase_policy);
                self.update_workspaces(workspaces.clone(), ctx);
                self.update_joinable_teams(joinable_teams, ctx);

                // Check if the current workspace is still in the list of workspaces.
                // If it's not, then set the current workspace to the first workspace in the list.
                if let Some(current_workspace) = self.current_workspace() {
                    if !self
                        .workspaces
                        .iter()
                        .any(|w| w.uid == current_workspace.uid)
                        && let Some(workspace_uid) = workspaces.first().map(|w| w.uid)
                    {
                        self.set_current_workspace_uid(workspace_uid, ctx);
                    }
                } else if let Some(workspace_uid) = workspaces.first().map(|w| w.uid) {
                    self.set_current_workspace_uid(workspace_uid, ctx);
                }
            }
            Err(e) => {
                report_error!(e.context("Failed to load user workspaces"));
            }
        }
    }

    pub fn team_created(
        &mut self,
        create_team_response: &CreateTeamResponse,
        ctx: &mut ModelContext<Self>,
    ) {
        self.workspaces.push(create_team_response.workspace.clone());
        self.set_current_workspace_uid(create_team_response.workspace.uid, ctx);
        self.notify_and_emit_teams_changed(ctx);
    }

    fn on_remove_user_from_team(
        &mut self,
        result: Result<WorkspacesMetadataWithPricing>,
        ctx: &mut ModelContext<Self>,
    ) {
        match result {
            Err(err) => ctx.emit(UserWorkspacesEvent::RemoveUserFromTeamRejected(err)),
            Ok(result) => {
                self.on_workspaces_updated(Ok(result), ctx);
                ctx.emit(UserWorkspacesEvent::RemoveUserFromTeamSuccess);
            }
        };
        ctx.notify();
    }

    pub fn remove_user_from_team(
        &mut self,
        user_uid: UserUid,
        team_uid: ServerId,
        entrypoint: CloudObjectEventEntrypoint,
        ctx: &mut ModelContext<Self>,
    ) {
        let team_client = self.team_client.clone();
        let _ = ctx.spawn(
            async move {
                team_client
                    .remove_user_from_team(user_uid, team_uid, entrypoint)
                    .await
            },
            Self::on_remove_user_from_team,
        );
    }

    fn on_add_invite_link_domain_restrictions(
        &mut self,
        result: Result<WorkspacesMetadataWithPricing>,
        ctx: &mut ModelContext<Self>,
    ) {
        match result {
            Err(err) => ctx.emit(UserWorkspacesEvent::AddDomainRestrictionsRejected(err)),
            Ok(result) => {
                self.on_workspaces_updated(Ok(result), ctx);
                ctx.emit(UserWorkspacesEvent::AddDomainRestrictionsSuccess);
            }
        };
        ctx.notify();
    }

    pub fn add_invite_link_domain_restrictions(
        &mut self,
        team_uid: ServerId,
        domains: Vec<String>,
        ctx: &mut ModelContext<Self>,
    ) {
        for domain in domains {
            let team_client = self.team_client.clone();
            let _ = ctx.spawn(
                async move {
                    team_client
                        .add_invite_link_domain_restriction(team_uid, domain)
                        .await
                },
                Self::on_add_invite_link_domain_restrictions,
            );
        }
    }

    fn on_delete_invite_link_domain_restriction(
        &mut self,
        result: Result<WorkspacesMetadataWithPricing>,
        ctx: &mut ModelContext<Self>,
    ) {
        match result {
            Err(err) => ctx.emit(UserWorkspacesEvent::DeleteDomainRestrictionRejected(err)),
            Ok(result) => {
                self.on_workspaces_updated(Ok(result), ctx);
                ctx.emit(UserWorkspacesEvent::DeleteDomainRestrictionSuccess);
            }
        };
        ctx.notify();
    }

    pub fn delete_invite_link_domain_restriction(
        &mut self,
        team_uid: ServerId,
        domain_uid: ServerId,
        ctx: &mut ModelContext<Self>,
    ) {
        let team_client = self.team_client.clone();
        let _ = ctx.spawn(
            async move {
                team_client
                    .delete_invite_link_domain_restriction(team_uid, domain_uid)
                    .await
            },
            Self::on_delete_invite_link_domain_restriction,
        );
    }

    fn on_email_invite_sent(
        &mut self,
        result: Result<WorkspacesMetadataWithPricing>,
        ctx: &mut ModelContext<Self>,
    ) {
        match result {
            Err(err) => ctx.emit(UserWorkspacesEvent::EmailInviteRejected(err)),
            Ok(result) => {
                self.on_workspaces_updated(Ok(result), ctx);
                ctx.emit(UserWorkspacesEvent::EmailInviteSent);
            }
        };
        ctx.notify();
    }

    pub fn send_email_invites(
        &mut self,
        team_uid: ServerId,
        emails: Vec<String>,
        ctx: &mut ModelContext<Self>,
    ) {
        for email in emails {
            let team_client = self.team_client.clone();
            let _ = ctx.spawn(
                async move { team_client.send_team_invite_email(team_uid, email).await },
                Self::on_email_invite_sent,
            );
        }
    }

    pub fn on_is_invite_link_enabled_set(
        &mut self,
        result: Result<WorkspacesMetadataWithPricing>,
        ctx: &mut ModelContext<Self>,
    ) {
        match result {
            Err(err) => ctx.emit(UserWorkspacesEvent::ToggleInviteLinksRejected(err)),
            Ok(result) => {
                self.on_workspaces_updated(Ok(result), ctx);
                ctx.emit(UserWorkspacesEvent::ToggleInviteLinksSuccess);
            }
        };
        ctx.notify();
    }

    pub fn set_is_invite_link_enabled(
        &mut self,
        team_uid: ServerId,
        new_value: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let team_client = self.team_client.clone();
        let _ = ctx.spawn(
            async move {
                team_client
                    .set_is_invite_link_enabled(team_uid, new_value)
                    .await
            },
            Self::on_is_invite_link_enabled_set,
        );
    }

    pub fn on_invite_links_reset(
        &mut self,
        result: Result<WorkspacesMetadataWithPricing>,
        ctx: &mut ModelContext<Self>,
    ) {
        match result {
            Err(err) => ctx.emit(UserWorkspacesEvent::ResetInviteLinksRejected(err)),
            Ok(result) => {
                self.on_workspaces_updated(Ok(result), ctx);
                ctx.emit(UserWorkspacesEvent::ResetInviteLinks);
            }
        };
        ctx.notify();
    }

    pub fn reset_invite_links(&mut self, team_uid: ServerId, ctx: &mut ModelContext<Self>) {
        let team_client = self.team_client.clone();
        let _ = ctx.spawn(
            async move { team_client.reset_invite_links(team_uid).await },
            Self::on_invite_links_reset,
        );
    }

    pub fn on_team_discoverability_set(
        &mut self,
        result: Result<WorkspacesMetadataWithPricing>,
        ctx: &mut ModelContext<Self>,
    ) {
        match result {
            Err(err) => ctx.emit(UserWorkspacesEvent::ToggleTeamDiscoverabilityRejected(err)),
            Ok(result) => {
                self.on_workspaces_updated(Ok(result), ctx);
                ctx.emit(UserWorkspacesEvent::ToggleTeamDiscoverabilitySuccess);
            }
        };
        ctx.notify();
    }

    pub fn set_team_discoverability(
        &mut self,
        team_uid: ServerId,
        discoverable: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let team_client = self.team_client.clone();
        let _ = ctx.spawn(
            async move {
                team_client
                    .set_team_discoverability(team_uid, discoverable)
                    .await
            },
            Self::on_team_discoverability_set,
        );
    }

    pub fn on_join_team_with_team_discovery(
        &mut self,
        result: Result<WorkspacesMetadataWithPricing>,
        ctx: &mut ModelContext<Self>,
    ) {
        match result {
            Err(err) => ctx.emit(UserWorkspacesEvent::JoinTeamWithTeamDiscoveryRejected(err)),
            Ok(result) => {
                self.on_workspaces_updated(Ok(result), ctx);
                ctx.emit(UserWorkspacesEvent::JoinTeamWithTeamDiscoverySuccess);
            }
        };
        ctx.notify();
    }

    pub fn join_team_with_team_discovery(
        &mut self,
        team_uid: ServerId,
        ctx: &mut ModelContext<Self>,
    ) {
        let team_client = self.team_client.clone();
        let _ = ctx.spawn(
            async move { team_client.join_team_with_team_discovery(team_uid).await },
            Self::on_join_team_with_team_discovery,
        );
    }

    fn on_fetch_discoverable_teams(
        &mut self,
        teams: Result<Vec<DiscoverableTeam>, anyhow::Error>,
        ctx: &mut ModelContext<Self>,
    ) {
        match teams {
            Err(e) => ctx.emit(UserWorkspacesEvent::FetchDiscoverableTeamsRejected(e)),
            Ok(teams) => {
                self.update_joinable_teams(teams, ctx);
            }
        }
    }

    /// Make request to get list of discoverable teams for a user
    pub fn fetch_discoverable_teams(&mut self, ctx: &mut ModelContext<Self>) {
        let team_client = self.team_client.clone();
        let _ = ctx.spawn(
            async move { team_client.get_discoverable_teams().await },
            Self::on_fetch_discoverable_teams,
        );
    }

    fn on_team_ownership_transferred(
        &mut self,
        result: Result<WorkspacesMetadataWithPricing>,
        ctx: &mut ModelContext<Self>,
    ) {
        match result {
            Err(err) => ctx.emit(UserWorkspacesEvent::TransferTeamOwnershipRejected(err)),
            Ok(result) => {
                self.on_workspaces_updated(Ok(result), ctx);
                ctx.emit(UserWorkspacesEvent::TransferTeamOwnershipSuccess);
            }
        };
        ctx.notify();
    }

    pub fn transfer_team_ownership(
        &mut self,
        new_owner_email: String,
        ctx: &mut ModelContext<Self>,
    ) {
        let team_client = self.team_client.clone();
        let _ = ctx.spawn(
            async move { team_client.transfer_team_ownership(new_owner_email).await },
            Self::on_team_ownership_transferred,
        );
    }

    fn on_team_member_role_set(
        &mut self,
        result: Result<WorkspacesMetadataWithPricing>,
        ctx: &mut ModelContext<Self>,
    ) {
        match result {
            Err(err) => ctx.emit(UserWorkspacesEvent::SetTeamMemberRoleRejected(err)),
            Ok(result) => {
                self.on_workspaces_updated(Ok(result), ctx);
                ctx.emit(UserWorkspacesEvent::SetTeamMemberRoleSuccess);
            }
        };
        ctx.notify();
    }

    pub fn set_team_member_role(
        &mut self,
        user_uid: UserUid,
        team_uid: ServerId,
        role: MembershipRole,
        ctx: &mut ModelContext<Self>,
    ) {
        let team_client = self.team_client.clone();
        let _ = ctx.spawn(
            async move {
                team_client
                    .set_team_member_role(user_uid, team_uid, role)
                    .await
            },
            Self::on_team_member_role_set,
        );
    }

    pub fn on_delete_team_invite(
        &mut self,
        result: Result<WorkspacesMetadataWithPricing>,
        ctx: &mut ModelContext<Self>,
    ) {
        match result {
            Err(err) => ctx.emit(UserWorkspacesEvent::DeleteTeamInviteRejected(err)),
            Ok(result) => {
                self.on_workspaces_updated(Ok(result), ctx);
                ctx.emit(UserWorkspacesEvent::DeleteTeamInvite);
            }
        };
        ctx.notify();
    }

    pub fn delete_team_invite(
        &mut self,
        team_uid: ServerId,
        invitee_email: String,
        ctx: &mut ModelContext<Self>,
    ) {
        let team_client = self.team_client.clone();
        let _ = ctx.spawn(
            async move {
                team_client
                    .delete_team_invite(team_uid, invitee_email)
                    .await
            },
            Self::on_delete_team_invite,
        );
    }

    pub fn on_generate_upgrade_link(
        &mut self,
        result: Result<String>,
        ctx: &mut ModelContext<Self>,
    ) {
        match result {
            Err(err) => ctx.emit(UserWorkspacesEvent::GenerateUpgradeLinkRejected(err)),
            Ok(upgrade_link) => {
                ctx.emit(UserWorkspacesEvent::GenerateUpgradeLink(upgrade_link));
            }
        };
        ctx.notify();
    }

    pub fn generate_upgrade_link(&mut self, team_uid: ServerId, ctx: &mut ModelContext<Self>) {
        Self::on_generate_upgrade_link(
            self,
            Ok(UserWorkspaces::upgrade_link_for_team(team_uid)),
            ctx,
        );
    }

    pub fn on_generate_stripe_billing_portal_link(
        &mut self,
        result: Result<String>,
        ctx: &mut ModelContext<Self>,
    ) {
        match result {
            Err(err) => ctx.emit(UserWorkspacesEvent::GenerateStripeBillingPortalLinkRejected(err)),
            Ok(billing_session_link) => {
                ctx.emit(UserWorkspacesEvent::GenerateStripeBillingPortalLink(
                    billing_session_link,
                ));
            }
        };
        ctx.notify();
    }

    pub fn generate_stripe_billing_portal_link(
        &mut self,
        team_uid: ServerId,
        ctx: &mut ModelContext<Self>,
    ) {
        let workspace_client = self.workspace_client.clone();
        let _ = ctx.spawn(
            async move {
                workspace_client
                    .generate_stripe_billing_portal_link(team_uid)
                    .await
            },
            Self::on_generate_stripe_billing_portal_link,
        );
    }

    pub fn update_usage_based_pricing_settings(
        &mut self,
        team_uid: ServerId,
        usage_based_pricing_enabled: bool,
        max_monthly_spend_cents: Option<u32>,
        ctx: &mut ModelContext<Self>,
    ) {
        let workspace_client = self.workspace_client.clone();
        let _ = ctx.spawn(
            async move {
                workspace_client
                    .update_usage_based_pricing_settings(
                        team_uid,
                        usage_based_pricing_enabled,
                        max_monthly_spend_cents,
                    )
                    .await
            },
            Self::on_update_workspace_metadata,
        );
    }

    fn on_update_workspace_metadata(
        &mut self,
        result: Result<WorkspacesMetadataResponse>,
        ctx: &mut ModelContext<Self>,
    ) {
        match result {
            Ok(result) => {
                let wrapped = WorkspacesMetadataWithPricing {
                    metadata: result,
                    pricing_info: None,
                };
                self.on_workspaces_updated(Ok(wrapped), ctx);
                ctx.emit(UserWorkspacesEvent::UpdateWorkspaceSettingsSuccess);
            }
            Err(err) => {
                let err_for_event = anyhow::anyhow!("{}", err);
                self.on_workspaces_updated(Err(err), ctx);
                ctx.emit(UserWorkspacesEvent::UpdateWorkspaceSettingsRejected(
                    err_for_event,
                ));
            }
        };
        ctx.notify();
    }

    pub fn purchase_addon_credits(
        &mut self,
        team_uid: Option<ServerId>,
        credits: i32,
        ctx: &mut ModelContext<Self>,
    ) {
        let workspace_client = self.workspace_client.clone();
        let _ = ctx.spawn(
            async move {
                workspace_client
                    .purchase_addon_credits(team_uid, credits)
                    .await
            },
            Self::on_purchase_addon_credits,
        );
    }

    fn on_purchase_addon_credits(
        &mut self,
        result: Result<PurchaseAddonCreditsOutcome>,
        ctx: &mut ModelContext<Self>,
    ) {
        match result {
            Ok(PurchaseAddonCreditsOutcome::Completed(result)) => {
                let wrapped = WorkspacesMetadataWithPricing {
                    metadata: *result,
                    pricing_info: None,
                };
                self.on_workspaces_updated(Ok(wrapped), ctx);
                ctx.emit(UserWorkspacesEvent::PurchaseAddonCreditsSuccess);
            }
            Ok(PurchaseAddonCreditsOutcome::CheckoutRequired { checkout_url }) => {
                ctx.emit(UserWorkspacesEvent::PurchaseAddonCreditsCheckoutRequired {
                    checkout_url,
                });
            }
            Err(err) => {
                ctx.emit(UserWorkspacesEvent::PurchaseAddonCreditsRejected(
                    anyhow::anyhow!(err),
                ));
            }
        };
        ctx.notify();
    }

    pub fn refresh_ai_overages(&mut self, ctx: &mut ModelContext<Self>) {
        let workspace_client = self.workspace_client.clone();
        let _ = ctx.spawn(
            async move { workspace_client.refresh_ai_overages().await },
            Self::on_refresh_ai_overages,
        );
    }

    pub fn update_addon_credits_settings(
        &mut self,
        team_uid: ServerId,
        auto_reload_enabled: Option<bool>,
        max_monthly_spend_cents: Option<i32>,
        selected_auto_reload_credit_denomination: Option<i32>,
        ctx: &mut ModelContext<Self>,
    ) {
        let workspace_client = self.workspace_client.clone();
        let _ = ctx.spawn(
            async move {
                workspace_client
                    .update_addon_credits_settings(
                        team_uid,
                        auto_reload_enabled,
                        max_monthly_spend_cents,
                        selected_auto_reload_credit_denomination,
                    )
                    .await
            },
            Self::on_update_workspace_metadata,
        );
    }

    fn on_refresh_ai_overages(&mut self, result: Result<AiOverages>, ctx: &mut ModelContext<Self>) {
        match result {
            Ok(fresh_ai_overages) => {
                // TODO: We really need to stop having duplicate billing metadata...
                if let Some(workspace) = self.current_workspace_mut() {
                    workspace.billing_metadata.ai_overages = Some(fresh_ai_overages.clone());
                    for team in &mut workspace.teams {
                        team.billing_metadata.ai_overages = Some(fresh_ai_overages.clone());
                    }
                }

                ctx.emit(UserWorkspacesEvent::AiOveragesUpdated);
                ctx.notify();
            }
            Err(e) => {
                log::warn!("Failed to refresh AI overages for workspace: {e:?}");
            }
        }
    }

    pub fn usage_based_pricing_settings(&self) -> UsageBasedPricingSettings {
        self.current_workspace()
            .map(|workspace| workspace.settings.usage_based_pricing_settings.clone())
            .unwrap_or_default()
    }

    pub fn is_telemetry_force_enabled(&self) -> bool {
        self.current_workspace()
            .map(|workspace| workspace.settings.telemetry_settings.force_enabled)
            .unwrap_or(false)
    }

    pub fn is_enterprise_secret_redaction_enabled(&self) -> bool {
        self.current_workspace()
            .map(|workspace| workspace.settings.secret_redaction_settings.enabled)
            .unwrap_or(false)
    }

    pub fn get_enterprise_secret_redaction_regex_list(&self) -> Vec<EnterpriseSecretRegex> {
        self.current_workspace()
            .map(|workspace| workspace.settings.secret_redaction_settings.regexes.clone())
            .unwrap_or_default()
    }

    pub fn get_ugc_collection_enablement_setting(&self) -> UgcCollectionEnablementSetting {
        self.current_workspace()
            .map(|workspace| workspace.settings.ugc_collection_settings.setting.clone())
            .unwrap_or_default()
    }

    pub fn get_cloud_conversation_storage_enablement_setting(&self) -> AdminEnablementSetting {
        self.current_workspace()
            .map(|workspace| {
                workspace
                    .settings
                    .cloud_conversation_storage_settings
                    .setting
                    .clone()
            })
            .unwrap_or_default()
    }

    pub fn is_ai_allowed_in_remote_sessions(&self) -> bool {
        self.current_workspace()
            .map(|workspace| {
                workspace
                    .settings
                    .ai_permissions_settings
                    .allow_ai_in_remote_sessions
            })
            .unwrap_or(true)
    }

    pub fn get_remote_session_regex_list(&self) -> Vec<Regex> {
        self.current_workspace()
            .map(|workspace| {
                workspace
                    .settings
                    .ai_permissions_settings
                    .remote_session_regex_list
                    .clone()
            })
            .unwrap_or_default()
    }

    pub fn is_anyone_with_link_sharing_enabled(&self) -> bool {
        self.current_workspace()
            .map(|workspace| {
                workspace
                    .settings
                    .link_sharing_settings
                    .anyone_with_link_sharing_enabled
            })
            .unwrap_or(true)
    }

    pub fn is_direct_link_sharing_enabled(&self) -> bool {
        self.current_workspace()
            .map(|workspace| {
                workspace
                    .settings
                    .link_sharing_settings
                    .direct_link_sharing_enabled
            })
            .unwrap_or(true)
    }

    /// Whether invite links are enabled for the current workspace. This is a
    /// workspace-level setting; the teams-settings page reads it from here rather
    /// than from the `Team` struct.
    pub fn is_invite_link_enabled(&self) -> bool {
        self.current_workspace()
            .map(|workspace| workspace.settings.is_invite_link_enabled)
            .unwrap_or(false)
    }

    /// Whether the current workspace's team is discoverable. This is a
    /// workspace-level setting; the teams-settings page reads it from here rather
    /// than from the `Team` struct.
    pub fn is_discoverable(&self) -> bool {
        self.current_workspace()
            .map(|workspace| workspace.settings.is_discoverable)
            .unwrap_or(false)
    }

    /// Returns the codebase context settings, taking into account the organization,
    /// global AI settings, and codebase-specific settings.
    /// Prefer this function to determine whether to show indexing-related functionality.
    pub fn is_codebase_context_enabled(&self, app: &AppContext) -> bool {
        // If the organization has an explicit setting, respect it and make user toggle irrelevant.
        // - Enable: forced ON by org, regardless of user preference.
        // - Disable: forced OFF by org.
        // - RespectUserSetting: respect the user setting.
        let org_setting = self.team_allows_codebase_context();
        let ai_globally_enabled = AISettings::as_ref(app).is_any_ai_enabled(app);

        match org_setting {
            AdminEnablementSetting::Enable => ai_globally_enabled,
            AdminEnablementSetting::Disable => false,
            AdminEnablementSetting::RespectUserSetting => {
                ai_globally_enabled && *CodeSettings::as_ref(app).codebase_context_enabled.value()
            }
        }
    }

    pub fn default_host_slug(&self) -> Option<&str> {
        self.current_workspace()
            .and_then(|workspace| workspace.settings.default_host_slug.as_deref())
    }

    /// Returns the team-level agent attribution setting.
    ///
    /// Use this to decide whether the user's attribution toggle should be locked
    /// (`Enable`/`Disable`) or editable (`RespectUserSetting`).
    pub fn get_agent_attribution_setting(&self) -> AdminEnablementSetting {
        self.current_workspace()
            .map(|workspace| workspace.settings.enable_warp_attribution.clone())
            .unwrap_or_default()
    }

    /// Returns only the organization-specific codebase context enablement setting.
    /// Do not use this function to determine whether codebase context is generally enabled --
    /// use `is_codebase_context_enabled` instead.
    pub fn team_allows_codebase_context(&self) -> AdminEnablementSetting {
        self.current_workspace()
            .map(|workspace| workspace.settings.codebase_context_settings.setting.clone())
            .unwrap_or_default()
    }

    /// Updates whether or not session sharing is enabled based on the current team's tier policy.
    fn update_session_sharing_enablement(&self, ctx: &AppContext) {
        if cfg!(any(test, feature = "integration_tests")) {
            return;
        }

        // If we have experiment state to unconditionally enable / disable the feature,
        // then we defer to that.
        let server_experiments = ServerExperiments::as_ref(ctx);
        if server_experiments.is_experiment_enabled(&ServerExperiment::SessionSharingControl)
            || server_experiments.is_experiment_enabled(&ServerExperiment::SessionSharingExperiment)
        {
            return;
        }

        let is_session_sharing_enabled_via_tier_policy = self
            .current_workspace()
            .and_then(|workspace| workspace.billing_metadata.tier.session_sharing_policy)
            .map(|policy| policy.is_enabled)
            .unwrap_or(true);
        FeatureFlag::CreatingSharedSessions.set_enabled(is_session_sharing_enabled_via_tier_policy);
    }
}

#[cfg(test)]
impl UserWorkspaces {
    /// Creates a test workspace with a team and sets it as the current workspace.
    /// Returns the workspace UID and admin UID for use in tests.
    pub fn setup_test_workspace(&mut self, ctx: &mut ModelContext<Self>) {
        let workspace_uid = WorkspaceUid::from(ServerId::from(1));
        let owner_uid = UserUid::new("test_owner");

        let workspace_settings = WorkspaceSettings::default();

        let workspace = Workspace {
            uid: workspace_uid,
            name: "Test Workspace".to_string(),
            stripe_customer_id: None,
            teams: vec![Team {
                uid: ServerId::from(2),
                name: "Test Team".to_string(),
                settings: Default::default(),
                color: None,
                billing_metadata: BillingMetadata::default(),
                members: vec![],
                invite_link: None,
                pending_email_invites: vec![],
                invite_link_domain_restrictions: vec![],
                stripe_customer_id: None,
                is_eligible_for_discovery: false,
                has_billing_history: false,
                visibility: TeamVisibility::Open,
            }],
            members: vec![WorkspaceMember {
                uid: owner_uid,
                email: "test@example.com".to_string(),
                role: MembershipRole::Owner,
                usage_info: WorkspaceMemberUsageInfo {
                    requests_used_since_last_refresh: 0,
                    request_limit: 1000,
                    is_unlimited: false,
                    is_request_limit_prorated: false,
                },
            }],
            billing_metadata: BillingMetadata::default(),
            bonus_grants_purchased_this_month: Default::default(),
            billing_cycle_usage: None,
            has_billing_history: false,
            settings: workspace_settings,
            invite_link_domain_restrictions: vec![],
            pending_email_invites: vec![],
            is_eligible_for_discovery: false,
            total_requests_used_since_last_refresh: 0,
        };

        self.update_workspaces(vec![workspace], ctx);
        self.set_current_workspace_uid(workspace_uid, ctx);
    }

    /// Updates the current workspace by applying a mutation function.
    pub fn update_current_workspace<F>(&mut self, f: F, ctx: &mut ModelContext<Self>)
    where
        F: FnOnce(&mut Workspace),
    {
        if let Some(workspace) = self.current_workspace() {
            if workspace.teams.is_empty() {
                panic!("No team found in current workspace. Did you call setup_test_workspace()?");
            }

            let mut new_workspace = workspace.clone();
            f(&mut new_workspace);

            self.update_workspaces(vec![new_workspace], ctx);
        } else {
            panic!("No workspace found. Did you call setup_test_workspace()?");
        }
    }

    pub fn update_sandboxed_agent_settings<F>(&mut self, f: F, ctx: &mut ModelContext<Self>)
    where
        F: FnOnce(&mut Option<SandboxedAgentSettings>),
    {
        self.update_current_workspace(
            |workspace| {
                f(&mut workspace.settings.sandboxed_agent_settings);
            },
            ctx,
        );
    }

    pub fn update_ai_autonomy_settings<F>(&mut self, f: F, ctx: &mut ModelContext<Self>)
    where
        F: FnOnce(&mut AiAutonomySettings),
    {
        self.update_current_workspace(
            |workspace| {
                f(&mut workspace.settings.ai_autonomy_settings);
            },
            ctx,
        );
    }

    pub fn update_ai_autonomy_policy_flag(&mut self, enabled: bool, ctx: &mut ModelContext<Self>) {
        self.update_current_workspace(
            |workspace| {
                if let Some(team) = workspace.teams.first_mut() {
                    team.billing_metadata.tier.ai_autonomy_policy = Some(AIAutonomyPolicy {
                        is_enabled: enabled,
                        toggleable: true,
                    });
                } else {
                    panic!(
                        "No team found in current workspace. Did you call setup_test_workspace()?"
                    );
                }
            },
            ctx,
        );
    }
}

impl Entity for UserWorkspaces {
    type Event = UserWorkspacesEvent;
}

/// Mark UserWorkspaces as global application state.
impl SingletonEntity for UserWorkspaces {}

#[cfg(test)]
#[path = "user_workspaces_tests.rs"]
mod user_workspaces_tests;
