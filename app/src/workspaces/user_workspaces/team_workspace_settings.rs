//! Team-scoped reads of workspace settings, plus the [`TeamScope`] types that name which team a
//! read is for.
//!
//! Two layers gate what a member may do, at different granularities:
//! - Plan entitlements are **workspace-level**: billing metadata is workspace-owned, so they hold
//!   regardless of which team a window has selected, and read `current_workspace()` (e.g.
//!   [`UserWorkspaces::is_managed_byok_byoe_enabled`]).
//! - Admin policies are **team-scoped**: they narrow an entitlement for one team, so they take a
//!   [`TeamScope`] and read that team (e.g. [`UserWorkspaces::team_byo_for_scope`]).
//!
//! A team-scoped policy only bites once its workspace-level entitlement turns it on -- a plan that
//! does not manage credentials centrally has no `team_byo` to enforce, so members fall back to the
//! plan's own BYO entitlement.

use std::rc::Rc;
use std::sync::OnceLock;

use regex::Regex;
use settings::Setting;
use warp_core::features::FeatureFlag;
use warpui::{AppContext, Entity, SingletonEntity, ViewContext, WeakViewHandle, WindowId};

use super::UserWorkspaces;
#[cfg(any(test, feature = "test-util"))]
use crate::ai::llms::LLMInfo;
use crate::ai::llms::{LLMId, LLMModelHost, LLMProvider, ModelsByFeature};
use crate::auth::AuthStateProvider;
use crate::server::ids::ServerId;
use crate::settings::{AISettings, AgentModeCommandExecutionPredicate};
use crate::workspaces::gql_convert::ToAgentModeCommandExecutionPredicates;
use crate::workspaces::team::Team;
use crate::workspaces::workspace::{
    AdminEnablementSetting, AiAutonomySettings, HostEnablementSetting, LlmHostSettings,
    LlmSettings, TeamByoSettings, Workspace,
};

mod sealed {
    pub trait Sealed {}
}

/// Reads a [`TeamContextForOperation`] or [`TeamContext`]'s team.
///
/// Either of [`TeamContextForOperation`] or [`TeamContext`] is the "key" external
/// modules use to obtain a team-level setting. The only external modules can obtain
/// this "key" is by exchanging a ViewContext or a ViewHandle for one. Once minted,
/// both [`TeamContextForOperation`] or [`TeamContext`] cannot be copied, cloned, or
/// moved. This ensures that the external operations which need TeamScopes (i.e. to
/// exchange for a team setting) is scoped to the view (and therefore team-scoped
/// window) that started the operation. External callers shouldn't copy a TeamContext
/// to a Singleton model for example, risking leaking that TeamContext / team info to
/// a different window with another team.
///
/// Sealed: only this module implements [`sealed::Sealed`], so a scope can never be minted
/// outside [`UserWorkspaces`].
#[allow(private_bounds)]
pub trait TeamScope: sealed::Sealed {
    fn team_uid(&self) -> Option<ServerId>;
}

pub(crate) struct TeamContextForOperation {
    team_uid: Option<ServerId>,
}

impl sealed::Sealed for TeamContextForOperation {}

impl TeamScope for TeamContextForOperation {
    fn team_uid(&self) -> Option<ServerId> {
        self.team_uid
    }
}

#[cfg(test)]
impl TeamContextForOperation {
    pub(crate) fn new_for_test(team_uid: ServerId) -> Self {
        Self {
            team_uid: Some(team_uid),
        }
    }
}

/// The team a view renders as, borrowed for the duration of a single read.
///
/// It is resolved at the point of use so policy reads follow the view between windows.
pub struct TeamContext<'a> {
    team_uid: Option<&'a ServerId>,
}

impl sealed::Sealed for TeamContext<'_> {}

impl TeamScope for TeamContext<'_> {
    fn team_uid(&self) -> Option<ServerId> {
        self.team_uid.copied()
    }
}

/// The team a headless CLI invocation acts as, named on the command line instead of resolved
/// from a window.
#[cfg(not(target_family = "wasm"))]
pub struct TeamScopeForCli(ServerId);

#[cfg(not(target_family = "wasm"))]
impl sealed::Sealed for TeamScopeForCli {}

#[cfg(not(target_family = "wasm"))]
impl TeamScope for TeamScopeForCli {
    fn team_uid(&self) -> Option<ServerId> {
        Some(self.0)
    }
}

pub struct ResolvedTeamScope(Option<ServerId>);

impl ResolvedTeamScope {
    pub fn from_scope(scope: &(impl TeamScope + ?Sized)) -> Self {
        Self(scope.team_uid())
    }

    #[cfg(feature = "agent_mode_evals")]
    pub(crate) fn teamless() -> Self {
        Self(None)
    }
}

impl sealed::Sealed for ResolvedTeamScope {}

impl TeamScope for ResolvedTeamScope {
    fn team_uid(&self) -> Option<ServerId> {
        self.0
    }
}

/// A teamless [`TeamScope`] for tests that pass a scope without standing up a window.
#[cfg(test)]
pub(crate) struct TeamlessScopeForTest;

#[cfg(test)]
impl sealed::Sealed for TeamlessScopeForTest {}

#[cfg(test)]
impl TeamScope for TeamlessScopeForTest {
    fn team_uid(&self) -> Option<ServerId> {
        None
    }
}

/// Resolves a [`TeamContext`] on demand from a view captured up front. See
/// [`UserWorkspaces::team_context_resolver`].
pub type TeamContextResolver = Rc<dyn for<'a> Fn(&'a AppContext) -> TeamContext<'a>>;

#[cfg(not(target_family = "wasm"))]
#[derive(Debug, thiserror::Error)]
#[error("you are not on team {team_uid}")]
pub struct NotATeamMemberError {
    pub team_uid: ServerId,
}

/// What windowless Gemini Enterprise credential minting should mint from. See
/// [`UserWorkspaces::gemini_enterprise_host_for_any_enabling_team`].
#[cfg(not(target_family = "wasm"))]
pub(crate) enum GeminiEnterpriseBackgroundHost<'a> {
    /// No team of the user's enables Gemini Enterprise, so there is nothing to mint.
    NoneEnabled,
    /// Teams enable it against different Google Cloud projects, named here so the caller can
    /// tell the user which teams disagree. Nothing is minted -- there is one credential store
    /// and no window to choose with -- but unlike [`Self::NoneEnabled`] this is a
    /// misconfiguration an admin can fix, and the user should be told so.
    Conflicting(Vec<&'a str>),
    Enabled(&'a LlmHostSettings),
}

impl UserWorkspaces {
    /// Captures the team selected in `ctx`'s window as an operation's
    /// [`TeamContextForOperation`]. This is the only way application code mints one. Always
    /// succeeds -- a window with no team selected still yields a scope, just one whose
    /// `team_uid()` is `None`; see [`TeamScope`]'s contract for what that means to a getter.
    pub(crate) fn team_context_for_operation<T: Entity>(
        &self,
        ctx: &ViewContext<T>,
    ) -> TeamContextForOperation {
        TeamContextForOperation {
            team_uid: self.team_uid_for_window(ctx.window_id()),
        }
    }

    pub(crate) fn team_context<'a, T: Entity>(
        &'a self,
        view: &WeakViewHandle<T>,
        app: &AppContext,
    ) -> TeamContext<'a> {
        let team_uid = self.team_for_view_handle(view, app).map(|team| &team.uid);
        TeamContext { team_uid }
    }

    /// The scope a headless CLI invocation reads team policy through, for a team the caller has
    /// already resolved.
    ///
    /// The sole exception to scopes being window-derived. *Which* team a CLI invocation acts as is
    /// the caller's to settle; all this enforces is that the answer is a team the user is on, so a
    /// scope can never name one whose policy [`Self::team_byo_for_scope`] would fail to find.
    #[cfg(not(target_family = "wasm"))]
    pub(crate) fn team_scope_for_cli(
        &self,
        team_uid: ServerId,
    ) -> Result<TeamScopeForCli, NotATeamMemberError> {
        self.is_member_of_team(team_uid)
            .then_some(TeamScopeForCli(team_uid))
            .ok_or(NotATeamMemberError { team_uid })
    }

    pub(crate) fn team_context_for_view<T: Entity>(&self, ctx: &ViewContext<T>) -> TeamContext<'_> {
        self.team_context_for_window_id(ctx.window_id())
    }

    /// Captures `view` as a reusable source of [`TeamContext`], for consumers that cannot name
    /// a view at the boundaries where they need one.
    pub fn team_context_resolver<T: Entity>(view: WeakViewHandle<T>) -> TeamContextResolver {
        Rc::new(move |app| Self::as_ref(app).team_context(&view, app))
    }

    /// A resolver for tests that build a model without a window to resolve against.
    #[cfg(any(test, feature = "test-util"))]
    pub fn teamless_context_resolver_for_test() -> TeamContextResolver {
        Rc::new(|_| TeamContext { team_uid: None })
    }

    fn team_context_for_window_id(&self, window_id: WindowId) -> TeamContext<'_> {
        TeamContext {
            team_uid: self
                .team_uid_for_window(window_id)
                .and_then(|team_uid| self.team_from_uid(team_uid))
                .map(|team| &team.uid),
        }
    }

    pub fn team_context_for_window(&self, window_id: WindowId) -> TeamContext<'_> {
        self.team_context_for_window_id(window_id)
    }

    /// [`Self::team_context_for_view`] for tests, which build scopes for bare windows rather
    /// than standing up a view for each one. Production exchanges a view or a [`ViewContext`]
    /// for a scope; this is `#[cfg(test)]` precisely so that contract holds.
    #[cfg(test)]
    pub(crate) fn team_context_for_window_for_test(&self, window_id: WindowId) -> TeamContext<'_> {
        self.team_context_for_window_id(window_id)
    }

    /// The team a scope names, when it names one that is still in the current workspace.
    ///
    /// Deliberately private. Callers get a resolved *setting* from a getter that takes their
    /// scope, never a `&Team` they could carry somewhere the scope never reached. Wanting a
    /// `&Team` at a call site means the read belongs behind a new getter here instead.
    fn team_from_scope<S: TeamScope + ?Sized>(&self, scope: &S) -> Option<&Team> {
        scope
            .team_uid()
            .and_then(|team_uid| self.team_from_uid(team_uid))
    }

    /// Whether `scope`'s team admins allows its members to use their own provider API keys.
    ///
    /// Without the managed BYOK/BYOE policy there is no team-level restriction, so this returns
    /// true and the normal BYO entitlement applies.
    pub fn are_member_byo_keys_allowed<S: TeamScope + ?Sized>(&self, scope: &S) -> bool {
        !self.is_managed_byok_byoe_enabled()
            || self
                .team_byo_for_scope(scope)
                .is_some_and(|team_byo| team_byo.first_party_enabled && team_byo.allow_user_keys)
    }

    /// [`Self::are_member_byo_keys_allowed`] for member-configured custom endpoints. Its
    /// entitlement half is [`Self::is_byo_endpoint_enabled`].
    pub(crate) fn are_member_byo_endpoints_allowed<S: TeamScope + ?Sized>(
        &self,
        scope: &S,
    ) -> bool {
        !self.is_managed_byok_byoe_enabled()
            || self
                .team_byo_for_scope(scope)
                .is_some_and(|team_byo| team_byo.endpoints_enabled && team_byo.allow_user_endpoints)
    }

    /// Whether `scope`'s team provides a managed endpoint serving `llm_id`.
    pub fn has_team_byo_endpoint<S: TeamScope + ?Sized>(&self, scope: &S, llm_id: &LLMId) -> bool {
        self.is_managed_byok_byoe_enabled()
            && self.team_byo_for_scope(scope).is_some_and(|team_byo| {
                team_byo.endpoints_enabled
                    && team_byo.endpoints.iter().any(|endpoint| {
                        endpoint.enabled
                            && endpoint
                                .models
                                .iter()
                                .any(|model| model.enabled && model.config_key == llm_id.as_str())
                    })
            })
    }

    /// Whether a first-party key for `provider` has been configured for `scope`.
    pub fn has_team_first_party_key<S: TeamScope + ?Sized>(
        &self,
        scope: &S,
        provider: LLMProvider,
    ) -> bool {
        self.is_managed_byok_byoe_enabled()
            && self.team_byo_for_scope(scope).is_some_and(|team_byo| {
                team_byo.first_party_enabled
                    && team_byo
                        .first_party_keys
                        .iter()
                        .any(|key| key.provider == provider)
            })
    }

    /// [`Self::are_member_byo_endpoints_allowed`] across every team at once, for callers with no
    /// window: id resolution and preference reconciliation act on state that follows the user
    /// between teams and devices, so neither may turn on one arbitrarily elected team's policy.
    pub fn is_byo_endpoint_enabled_for_any_team(&self, app: &AppContext) -> bool {
        self.is_byo_endpoint_enabled(app) && self.any_team_allows_member_byo_endpoints()
    }

    /// Unlike [`Self::team_byo_for_scope`], several teams is not ambiguous here: any one
    /// allowing is enough. No teams still falls back to the workspace's own policy.
    fn any_team_allows_member_byo_endpoints(&self) -> bool {
        if !self.is_managed_byok_byoe_enabled() {
            return true;
        }
        fn allows(team_byo: &TeamByoSettings) -> bool {
            team_byo.endpoints_enabled && team_byo.allow_user_endpoints
        }
        let mut teams = self
            .workspaces
            .iter()
            .flat_map(|workspace| workspace.teams.iter())
            .peekable();
        if teams.peek().is_none() {
            return self
                .current_workspace()
                .and_then(|workspace| workspace.settings.team_byo.as_ref())
                .is_some_and(allows);
        }
        teams.any(|team| team.settings.team_byo.as_ref().is_some_and(allows))
    }

    /// Resolves a per-team setting for `scope`: the scope's own team when it names one, otherwise
    /// `current_workspace().settings`.
    ///
    /// A scope naming an unresolvable team yields `absent`, never another team's value. The
    /// no-team branch reads `current_workspace().settings` unconditionally; for a member on teams
    /// that is the server's arbitrarily-elected stand-in (see [`TeamScope`]), a deliberate
    /// simplification because a windowed terminal is never expected to present a teamless scope,
    /// so in practice only a genuinely teamless user reaches it, whose workspace settings the
    /// server computes from tier defaults.
    fn scoped_or_workspace_setting<'a, S: TeamScope + ?Sized, T>(
        &'a self,
        scope: &S,
        from_team: impl FnOnce(&'a Team) -> T,
        from_workspace: impl FnOnce(&'a Workspace) -> T,
        absent: T,
    ) -> T {
        match scope.team_uid() {
            Some(_) => self.team_from_scope(scope).map_or(absent, from_team),
            None => self.current_workspace().map_or(absent, from_workspace),
        }
    }

    /// The `team_byo` policy that governs `scope`. See [`Self::scoped_or_workspace_setting`] for
    /// the no-team fallback.
    fn team_byo_for_scope<S: TeamScope + ?Sized>(&self, scope: &S) -> Option<&TeamByoSettings> {
        self.scoped_or_workspace_setting(
            scope,
            |team| team.settings.team_byo.as_ref(),
            |workspace| workspace.settings.team_byo.as_ref(),
            None,
        )
    }

    /// The self-hosted worker host slug configured as the default for `scope`'s team. See
    /// [`Self::scoped_or_workspace_setting`] for the no-team fallback.
    pub(crate) fn default_host_slug<S: TeamScope + ?Sized>(&self, scope: &S) -> Option<&str> {
        self.scoped_or_workspace_setting(
            scope,
            |team| team.settings.default_host_slug.as_deref(),
            |workspace| workspace.settings.default_host_slug.as_deref(),
            None,
        )
    }

    /// The agent attribution policy for `scope`'s team: `Enable` and `Disable` lock the user's
    /// attribution toggle, `RespectUserSetting` leaves it editable. See
    /// [`Self::scoped_or_workspace_setting`] for the no-team fallback.
    pub(crate) fn get_agent_attribution_setting<S: TeamScope + ?Sized>(
        &self,
        scope: &S,
    ) -> AdminEnablementSetting {
        self.scoped_or_workspace_setting(
            scope,
            |team| team.settings.enable_warp_attribution.clone(),
            |workspace| workspace.settings.enable_warp_attribution.clone(),
            AdminEnablementSetting::default(),
        )
    }

    pub(crate) fn is_anyone_with_link_sharing_enabled<S: TeamScope + ?Sized>(
        &self,
        scope: &S,
    ) -> bool {
        self.scoped_or_workspace_setting(
            scope,
            |team| {
                team.settings
                    .link_sharing
                    .anyone_with_link_sharing_enabled
                    .value
            },
            |workspace| {
                workspace
                    .settings
                    .link_sharing_settings
                    .anyone_with_link_sharing_enabled
            },
            true,
        )
    }

    pub(crate) fn is_direct_link_sharing_enabled<S: TeamScope + ?Sized>(&self, scope: &S) -> bool {
        self.scoped_or_workspace_setting(
            scope,
            |team| team.settings.link_sharing.direct_link_sharing_enabled.value,
            |workspace| {
                workspace
                    .settings
                    .link_sharing_settings
                    .direct_link_sharing_enabled
            },
            true,
        )
    }

    /// Every team the user belongs to, across all of their workspaces.
    #[cfg(not(target_family = "wasm"))]
    fn all_teams(&self) -> impl Iterator<Item = &Team> {
        self.workspaces
            .iter()
            .flat_map(|workspace| workspace.teams.iter())
    }

    /// The LLM host settings (which hosts an admin has enabled, and how) that apply to
    /// `scope`'s team. See [`Self::scoped_or_workspace_setting`] for the no-team fallback.
    /// `llm_settings` lives on both [`crate::workspaces::workspace::WorkspaceSettings`] and
    /// [`crate::workspaces::workspace::TeamSettings`] for exactly this reason: unlike a plan
    /// entitlement, which host an admin enabled is per-team, not workspace-wide.
    fn llm_settings_for_scope<S: TeamScope + ?Sized>(&self, scope: &S) -> Option<&LlmSettings> {
        self.scoped_or_workspace_setting(
            scope,
            |team| Some(&team.settings.llm_settings),
            |workspace| Some(&workspace.settings.llm_settings),
            None,
        )
    }

    fn host_settings_for_scope<S: TeamScope + ?Sized>(
        &self,
        scope: &S,
        host: LLMModelHost,
    ) -> Option<&LlmHostSettings> {
        self.llm_settings_for_scope(scope)?.host_configs.get(&host)
    }

    pub(crate) fn aws_bedrock_host_settings<S: TeamScope + ?Sized>(
        &self,
        scope: &S,
    ) -> Option<&LlmHostSettings> {
        self.host_settings_for_scope(scope, LLMModelHost::AwsBedrock)
    }

    pub(crate) fn gemini_enterprise_host_settings<S: TeamScope + ?Sized>(
        &self,
        scope: &S,
    ) -> Option<&LlmHostSettings> {
        self.host_settings_for_scope(scope, LLMModelHost::GeminiEnterprise)
    }

    /// Did the admin enable AWS Bedrock for `scope`'s team?
    pub(crate) fn is_aws_bedrock_available<S: TeamScope + ?Sized>(&self, scope: &S) -> bool {
        self.llm_settings_for_scope(scope)
            .is_some_and(|llm_settings| llm_settings.enabled)
            && self
                .aws_bedrock_host_settings(scope)
                .is_some_and(|settings| settings.enabled)
    }

    pub(crate) fn aws_bedrock_host_enablement_setting<S: TeamScope + ?Sized>(
        &self,
        scope: &S,
    ) -> HostEnablementSetting {
        self.aws_bedrock_host_settings(scope)
            .map(|settings| settings.enablement_setting.clone())
            .unwrap_or_default()
    }

    pub(crate) fn is_aws_bedrock_credentials_enabled<S: TeamScope + ?Sized>(
        &self,
        scope: &S,
        app: &AppContext,
    ) -> bool {
        // i.e. did the admin go and toggle on aws bedrock in the admin panel?
        if !self.is_aws_bedrock_available(scope) {
            return false;
        }

        match self.aws_bedrock_host_enablement_setting(scope) {
            HostEnablementSetting::Enforce => true,
            HostEnablementSetting::RespectUserSetting => *AISettings::as_ref(app)
                .aws_bedrock_credentials_enabled
                .value(),
        }
    }

    /// Whether *any* of the user's teams has AWS Bedrock credentials enabled, for work that
    /// belongs to no window at all: loading the local AWS credential chain, and the "does this
    /// user have any usable BYO path" check. A caller with a window must use
    /// [`Self::is_aws_bedrock_credentials_enabled`] instead -- this deliberately answers for
    /// the union of the user's teams, not for the team a window points at.
    #[cfg(not(target_family = "wasm"))]
    pub(crate) fn is_aws_bedrock_credentials_enabled_for_any_team(&self, app: &AppContext) -> bool {
        self.every_applicable_team_and_llm_settings()
            .map(|(_, settings)| settings)
            .any(|llm_settings| {
                Self::host_credentials_enabled(llm_settings, &LLMModelHost::AwsBedrock, || {
                    *AISettings::as_ref(app)
                        .aws_bedrock_credentials_enabled
                        .value()
                })
            })
    }

    /// Did the admin enable Gemini Enterprise (GEAP) for `scope`'s team?
    pub(crate) fn is_gemini_enterprise_available_from_workspace<S: TeamScope + ?Sized>(
        &self,
        scope: &S,
    ) -> bool {
        self.llm_settings_for_scope(scope)
            .is_some_and(|llm_settings| llm_settings.enabled)
            && self
                .gemini_enterprise_host_settings(scope)
                .is_some_and(|settings| settings.enabled)
    }

    pub(crate) fn gemini_enterprise_host_enablement_setting<S: TeamScope + ?Sized>(
        &self,
        scope: &S,
    ) -> HostEnablementSetting {
        self.gemini_enterprise_host_settings(scope)
            .map(|settings| settings.enablement_setting.clone())
            .unwrap_or_default()
    }

    pub(crate) fn is_gemini_enterprise_credentials_toggleable<S: TeamScope + ?Sized>(
        &self,
        scope: &S,
    ) -> bool {
        matches!(
            self.gemini_enterprise_host_enablement_setting(scope),
            HostEnablementSetting::RespectUserSetting
        )
    }

    /// Whether Gemini Enterprise (GEAP) credentials should be minted and attached to requests
    pub(crate) fn is_gemini_enterprise_credentials_enabled<S: TeamScope + ?Sized>(
        &self,
        scope: &S,
        app: &AppContext,
    ) -> bool {
        if !FeatureFlag::GeminiEnterprise.is_enabled() {
            return false;
        }
        if AuthStateProvider::as_ref(app)
            .get()
            .is_anonymous_or_logged_out()
        {
            return false;
        }
        // i.e. did the admin toggle on Gemini Enterprise in the admin panel?
        if !self.is_gemini_enterprise_available_from_workspace(scope) {
            return false;
        }

        match self.gemini_enterprise_host_enablement_setting(scope) {
            HostEnablementSetting::Enforce => true,
            HostEnablementSetting::RespectUserSetting => *AISettings::as_ref(app)
                .gemini_enterprise_credentials_enabled
                .value(),
        }
    }

    #[cfg(not(target_family = "wasm"))]
    fn every_applicable_team_and_llm_settings(
        &self,
    ) -> Box<dyn Iterator<Item = (Option<&Team>, &LlmSettings)> + '_> {
        let mut teams = self.all_teams().peekable();
        if teams.peek().is_none() {
            return Box::new(
                self.current_workspace()
                    .into_iter()
                    .map(|workspace| (None, &workspace.settings.llm_settings)),
            );
        }
        Box::new(teams.map(|team| (Some(team), &team.settings.llm_settings)))
    }

    /// Did the admin turn `host` on, with its credentials resolved against `user_setting_enabled`?
    #[cfg(not(target_family = "wasm"))]
    fn host_credentials_enabled(
        llm_settings: &LlmSettings,
        host: &LLMModelHost,
        user_setting_enabled: impl FnOnce() -> bool,
    ) -> bool {
        if !llm_settings.enabled {
            return false;
        }
        let Some(host_settings) = llm_settings.host_configs.get(host) else {
            return false;
        };
        if !host_settings.enabled {
            return false;
        }
        match host_settings.enablement_setting {
            HostEnablementSetting::Enforce => true,
            HostEnablementSetting::RespectUserSetting => user_setting_enabled(),
        }
    }

    /// What background, windowless Gemini Enterprise credential minting should mint from.
    #[cfg(not(target_family = "wasm"))]
    pub(crate) fn gemini_enterprise_host_for_any_enabling_team(
        &self,
        app: &AppContext,
    ) -> GeminiEnterpriseBackgroundHost<'_> {
        if !FeatureFlag::GeminiEnterprise.is_enabled()
            || AuthStateProvider::as_ref(app)
                .get()
                .is_anonymous_or_logged_out()
        {
            return GeminiEnterpriseBackgroundHost::NoneEnabled;
        }
        let enabling: Vec<(Option<&Team>, &LlmHostSettings)> = self
            .every_applicable_team_and_llm_settings()
            .filter(|(_, llm_settings)| {
                Self::host_credentials_enabled(
                    llm_settings,
                    &LLMModelHost::GeminiEnterprise,
                    || {
                        *AISettings::as_ref(app)
                            .gemini_enterprise_credentials_enabled
                            .value()
                    },
                )
            })
            .filter_map(|(team, llm_settings)| {
                llm_settings
                    .host_configs
                    .get(&LLMModelHost::GeminiEnterprise)
                    .map(|settings| (team, settings))
            })
            .collect();

        let Some((_, first)) = enabling.first().copied() else {
            return GeminiEnterpriseBackgroundHost::NoneEnabled;
        };
        let agree = enabling.iter().all(|(_, settings)| {
            settings.gcp_audience == first.gcp_audience
                && settings.gcp_sa_email == first.gcp_sa_email
        });
        if agree {
            GeminiEnterpriseBackgroundHost::Enabled(first)
        } else {
            GeminiEnterpriseBackgroundHost::Conflicting(
                enabling
                    .iter()
                    .filter_map(|(team, _)| team.map(|team| team.name.as_str()))
                    .collect(),
            )
        }
    }

    /// The AI autonomy policy that applies to `scope`'s team. See
    /// [`Self::scoped_or_workspace_setting`] for the no-team fallback.
    pub(crate) fn ai_autonomy_settings<S: TeamScope + ?Sized>(
        &self,
        scope: &S,
    ) -> AiAutonomySettings {
        self.scoped_or_workspace_setting(
            scope,
            |team| AiAutonomySettings::from(&team.settings.ai_autonomy),
            |workspace| workspace.settings.ai_autonomy_settings.clone(),
            AiAutonomySettings::default(),
        )
    }

    pub(crate) fn feature_model_choice_for_scope<S: TeamScope + ?Sized>(
        &self,
        scope: &S,
    ) -> &ModelsByFeature {
        static DEFAULT: OnceLock<ModelsByFeature> = OnceLock::new();
        self.scoped_or_workspace_setting(
            scope,
            |team| &team.feature_model_choice,
            |workspace| &workspace.feature_model_choice,
            self.workspaceless_models_by_feature
                .as_ref()
                .unwrap_or_else(|| DEFAULT.get_or_init(ModelsByFeature::default)),
        )
    }

    pub(crate) fn feature_model_choice_for_team_uid(
        &self,
        team_uid: Option<ServerId>,
    ) -> &ModelsByFeature {
        self.feature_model_choice_for_scope(&TeamContextForOperation { team_uid })
    }

    pub(crate) fn set_feature_model_choice_for_team_uid(
        &mut self,
        team_uid: Option<ServerId>,
        models: ModelsByFeature,
    ) {
        match team_uid {
            Some(team_uid) => {
                if let Some(workspace) = self.current_workspace_mut()
                    && let Some(team) = workspace.teams.iter_mut().find(|t| t.uid == team_uid)
                {
                    team.feature_model_choice = models;
                }
            }
            None => match self.current_workspace_mut() {
                Some(workspace) => workspace.feature_model_choice = models,
                None => self.set_workspaceless_models_by_feature(models),
            },
        }
    }

    #[cfg(any(test, feature = "test-util"))]
    pub(crate) fn add_agent_mode_model_for_test_for_team_uid(
        &mut self,
        team_uid: Option<ServerId>,
        llm: LLMInfo,
    ) {
        match team_uid {
            Some(team_uid) => {
                if let Some(workspace) = self.current_workspace_mut()
                    && let Some(team) = workspace.teams.iter_mut().find(|t| t.uid == team_uid)
                {
                    team.feature_model_choice
                        .agent_mode
                        .push_choice_for_test(llm);
                }
            }
            None => match self.current_workspace_mut() {
                Some(workspace) => workspace
                    .feature_model_choice
                    .agent_mode
                    .push_choice_for_test(llm),
                None => self
                    .workspaceless_models_by_feature
                    .get_or_insert_with(ModelsByFeature::default)
                    .agent_mode
                    .push_choice_for_test(llm),
            },
        }
    }

    /// The organization-managed command denylist a sandboxed agent must obey, for `scope`'s
    /// team. An empty or unconfigured list is no constraint (a denylist blocks only what it
    /// lists), so both lower to `None`. See [`Self::scoped_or_workspace_setting`] for the
    /// no-team fallback.
    pub(crate) fn sandboxed_agent_execute_commands_denylist_for_scope<S: TeamScope + ?Sized>(
        &self,
        scope: &S,
    ) -> Option<Vec<AgentModeCommandExecutionPredicate>> {
        self.scoped_or_workspace_setting(
            scope,
            |team| {
                // A denylist blocks only what it lists, so an empty list is no constraint.
                let values = &team
                    .settings
                    .sandboxed_agent
                    .execute_commands_denylist
                    .values;
                (!values.is_empty()).then(|| values.clone().to_predicates())
            },
            |workspace| {
                workspace
                    .settings
                    .sandboxed_agent_settings
                    .clone()
                    .and_then(|settings| settings.execute_commands_denylist)
            },
            None,
        )
    }

    /// Returns true iff AI autonomy features are allowed for `scope`'s team.
    /// TODO: This should be deleted soon. AI autonomy settings have been moved into organization
    /// settings (see `ai_autonomy_settings` above), but there could be an interim time where we
    /// have not set up the org settings yet for an enterprise that previously had the entire
    /// feature set disabled. To capture that case, we'll see if all the settings are `None`;
    /// if so, we'll fall back to their billing metadata's value. Once we've migrated everyone
    /// into org settings, we should remove `is_enabled` from the policy and delete this function.
    pub fn is_ai_autonomy_allowed<S: TeamScope + ?Sized>(&self, scope: &S) -> bool {
        let settings = self.ai_autonomy_settings(scope);
        let all_settings_none = settings.apply_code_diffs_setting.is_none()
            && settings.read_files_setting.is_none()
            && settings.read_files_allowlist.is_none()
            && settings.execute_commands_setting.is_none()
            && settings.execute_commands_allowlist.is_none()
            && settings.execute_commands_denylist.is_none();

        if !all_settings_none {
            return true;
        }

        self.current_workspace().is_none_or(|workspace| {
            workspace
                .billing_metadata
                .tier
                .ai_autonomy_policy
                .is_some_and(|policy| policy.is_enabled)
        })
    }

    /// Whether AI is allowed in remote sessions under `scope`'s team. See
    /// [`Self::scoped_or_workspace_setting`] for the no-team fallback. An unresolvable named
    /// team denies rather than guessing, for a control gating AI in an environment the user
    /// may not control.
    ///
    /// `current_workspace()` is only `None` when logged out or before the first metadata fetch
    /// (an authenticated user, teamless or not, always has at least a personal workspace). There
    /// is no admin policy to read in that state, so it permits, preserving the pre-refactor
    /// default. The helper's single `absent` value cannot express both that and the
    /// unresolvable-team deny, so the no-workspace case is handled explicitly first.
    pub(crate) fn is_ai_allowed_in_remote_sessions<S: TeamScope + ?Sized>(
        &self,
        scope: &S,
    ) -> bool {
        if scope.team_uid().is_none() && self.current_workspace().is_none() {
            return true;
        }
        self.scoped_or_workspace_setting(
            scope,
            |team| {
                team.settings
                    .ai_permissions
                    .allow_ai_in_remote_sessions
                    .value
            },
            |workspace| {
                workspace
                    .settings
                    .ai_permissions_settings
                    .allow_ai_in_remote_sessions
            },
            false,
        )
    }

    /// The remote-session command patterns configured by `scope`'s team. See
    /// [`Self::scoped_or_workspace_setting`] for the no-team fallback.
    pub(crate) fn get_remote_session_regex_list<S: TeamScope + ?Sized>(
        &self,
        scope: &S,
    ) -> &[Regex] {
        self.scoped_or_workspace_setting(
            scope,
            |team| {
                team.settings
                    .ai_permissions
                    .remote_session_regex_list
                    .as_slice()
            },
            |workspace| {
                workspace
                    .settings
                    .ai_permissions_settings
                    .remote_session_regex_list
                    .as_slice()
            },
            &[],
        )
    }
}
