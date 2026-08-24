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

use warpui::{AppContext, Entity, SingletonEntity, ViewContext, WeakViewHandle, WindowId};

use super::{SoleTeamError, UserWorkspaces};
use crate::ai::llms::{LLMId, LLMProvider};
use crate::server::ids::ServerId;
use crate::workspaces::team::Team;
use crate::workspaces::workspace::{AiAutonomySettings, TeamByoSettings};

/// The team an operation is scoped to, captured once from the window that started it.
///
/// A logical operation carries its `TeamContextForOperation` from start to finish instead of
/// asking a window which team is selected now, so concurrent windows on different teams stay
/// independent and a later team switch cannot retarget work already in flight.
///
/// Deliberately neither `Clone` nor `Copy`. Moves make the handoff between the parts of an
/// operation explicit and reviewable, whereas copies let a scope leak sideways into work
/// that never established it. Wanting to duplicate one is a sign the second consumer is
/// really a separate operation that should capture its own scope; if the parts genuinely
/// share a lifetime, restructure so they share the single owner instead.
///
/// This is scope, not authority: the server still authorizes every request made under it.
///
/// Prefer [`TeamContext`] when reasonable: convert a `ViewContext` to a `WeakViewHandle`,
/// carry the handle through moved futures and callbacks, and mint the render context only at
/// the point of use, so a policy read reflects the window's team at that moment. Reach for
/// this type instead when the work's *destination* must not move once chosen -- e.g. creating
/// a Drive object in the team the user was in when they clicked New -- and be ready to justify
/// that choice; pinning is deliberate, not the default.
///
/// Only [`UserWorkspaces::team_context_for_operation`] mints one, always from a real window;
/// there is no way to fabricate one without a window (there is deliberately no `teamless()`
/// constructor). Its `team_uid` can still be `None` -- that means the minting window itself
/// has no team selected, and a getter that accepts this scope should act as if the operation
/// is not on a team. It must not read some other team's settings as a substitute: see
/// [`TeamScope`]'s contract. Code with no window at all (e.g. background GEAP token refresh)
/// is not this type's job -- it needs its own accessor that reads across every one of the
/// user's teams explicitly, in the shape of `UserWorkspaces::teams_allow_codebase_context`.
pub(crate) struct TeamContextForOperation {
    team_uid: Option<ServerId>,
}

/// Reads a [`TeamContextForOperation`] or [`TeamContext`]'s team, regardless of which one a
/// caller was handed. Implemented only by those two types — see their docs for what each one
/// promises about when it was resolved and what it can be used for.
///
/// The contract every settings getter built on this trait must follow: take a scope directly
/// (`&impl TeamScope` or `&dyn TeamScope`), never an optional one (`Option<&dyn TeamScope>`).
/// A caller with no scope to give has to confront that rather than pass `None` and inherit
/// some fallback. `team_uid() == None` means the scope's own window/operation has no team, so
/// the getter must not substitute some *other* team's settings.
///
/// `current_workspace().settings` is not such a substitute. It is a backwards-compatibility
/// shape, from when a workspace held exactly one team, a user was in at most one workspace, and
/// being in the workspace meant being in that team -- "the workspace's settings" was then
/// unambiguously "your team's settings", and old clients read nothing else.
///
/// The server still has to populate it, so `GetEffectiveWorkspaceSettingsForWorkspace` elects a
/// team to stand in: the first of the viewer's own teams in that workspace, or the workspace
/// layer over default team settings when they are on none. So a user on one team reads that
/// team, and a user on several reads an arbitrary one of theirs.
///
/// Code with no window at all must not construct a scope to route around this; it should read
/// across every team explicitly, the way `UserWorkspaces::teams_allow_codebase_context` does.
#[allow(dead_code)]
pub trait TeamScope {
    fn team_uid(&self) -> Option<ServerId>;
}

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
    #[allow(dead_code)]
    team_uid: Option<&'a ServerId>,
}

impl TeamScope for TeamContext<'_> {
    fn team_uid(&self) -> Option<ServerId> {
        self.team_uid.copied()
    }
}

/// Resolves a [`TeamContext`] on demand from a view captured up front. See
/// [`UserWorkspaces::team_context_resolver`] for when this is the right tool.
pub type TeamContextResolver = Rc<dyn for<'a> Fn(&'a AppContext) -> TeamContext<'a>>;

/// The team a headless CLI invocation acts as, named on the command line instead of resolved
/// from a window.
///
/// Deliberately cannot be teamless. The other two scopes can be, because a window genuinely may
/// have no team selected; a CLI caller that cannot name one has instead failed to say what it
/// meant, and is told to pass `--team=<UID>`. Minted only by
/// [`UserWorkspaces::team_scope_for_cli`], which checks membership first.
pub struct TeamScopeForCli(ServerId);

impl TeamScope for TeamScopeForCli {
    fn team_uid(&self) -> Option<ServerId> {
        Some(self.0)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CliTeamError {
    #[error(transparent)]
    NoSoleTeam(#[from] SoleTeamError),
    #[error("you are not on team {team_uid}")]
    NotAMember { team_uid: ServerId },
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

    /// Captures `view` as a reusable source of [`TeamContext`], for consumers that cannot name
    /// a view at the boundaries where they need one.
    ///
    /// Reach for this only when that is genuinely the case -- a model or executor several
    /// layers below the view that owns it, such as `BlocklistAIActionModel` and the action
    /// executors it builds, whose methods receive an [`AppContext`] and no handle. Threading a
    /// `WeakViewHandle` of the owning view's type through those layers would make each of them
    /// generic over a view they otherwise know nothing about.
    ///
    /// A view resolving *itself* is not that case: it should hold a `WeakViewHandle<Self>` and
    /// call [`Self::team_context`] at the point of use, which costs the same one field and
    /// keeps the resolution target visible in the struct rather than captured in a closure.
    ///
    /// The captured handle is resolved on each call, so the scope still follows the view's
    /// window; it is the handle that is fixed here, not the team.
    pub fn team_context_resolver<T: Entity>(view: WeakViewHandle<T>) -> TeamContextResolver {
        Rc::new(move |app| Self::as_ref(app).team_context(&view, app))
    }

    /// A resolver for tests that build a model without a window to resolve against.
    #[cfg(any(test, feature = "test-util"))]
    pub fn teamless_context_resolver_for_test() -> TeamContextResolver {
        Rc::new(|_| TeamContext { team_uid: None })
    }

    /// The team a CLI invocation acts as: the one it named, or its sole team when it named none.
    ///
    /// Membership is checked here so a mistyped uid fails loudly, rather than resolving to a team
    /// whose policy [`Self::team_byo_for_scope`] cannot find and being denied everything for a
    /// reason the user cannot see.
    pub fn cli_team_uid(&self, requested: Option<ServerId>) -> Result<ServerId, CliTeamError> {
        match requested {
            Some(team_uid) => self
                .team_from_uid(team_uid)
                .map(|team| team.uid)
                .ok_or(CliTeamError::NotAMember { team_uid }),
            None => Ok(self.sole_team_uid()?),
        }
    }

    /// [`Self::cli_team_uid`] as a scope, for the policy reads a CLI command makes. Both are fed
    /// the same requested uid so an object's owner and the credentials it may use cannot disagree.
    pub fn team_scope_for_cli(
        &self,
        requested: Option<ServerId>,
    ) -> Result<TeamScopeForCli, CliTeamError> {
        self.cli_team_uid(requested).map(TeamScopeForCli)
    }

    /// A view that has left its window resolves the same way as a window with no team: to no
    /// team. Both mean there is no team to govern the read, and a caller on a render path has
    /// no better answer to give than that.
    pub(crate) fn team_context<'a, T: Entity>(
        &'a self,
        view: &WeakViewHandle<T>,
        app: &AppContext,
    ) -> TeamContext<'a> {
        let team_uid = self.team_for_view_handle(view, app).map(|team| &team.uid);
        TeamContext { team_uid }
    }

    /// [`Self::team_context`] for code holding a [`ViewContext`] rather than a
    /// [`WeakViewHandle`]: a view inside its own constructor, where a handle would not yet
    /// resolve, or a nested `update` closure over a child view whose own type is not worth
    /// naming. [`ViewContext::window_id`] is a plain field valid in both cases, which is why
    /// this takes the context.
    ///
    /// The exchange for a scope is deliberately a view or a `ViewContext`, never a raw
    /// [`WindowId`]: an id-taking form would incentivise passing ids around, and an id is
    /// weaker evidence than a live context because a pane dragged between windows carries its
    /// models with it.
    pub(crate) fn team_context_for_view<T: Entity>(&self, ctx: &ViewContext<T>) -> TeamContext<'_> {
        self.team_context_for_window_id(ctx.window_id())
    }

    fn team_context_for_window_id(&self, window_id: WindowId) -> TeamContext<'_> {
        TeamContext {
            team_uid: self
                .team_uid_for_window(window_id)
                .and_then(|team_uid| self.team_from_uid(team_uid))
                .map(|team| &team.uid),
        }
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

    /// The `team_byo` policy that governs `scope`.
    ///
    /// A scope that names a team reads that team's policy and only that team's: an unresolvable
    /// team yields `None`, never another team's policy.
    ///
    /// A scope with no team falls back on the current workspace, but only where that has an
    /// unambiguous answer: `workspace.settings` when the user is on no team there, and their
    /// own team's policy when they are on exactly one. On several teams there is nothing to
    /// fall back to -- `workspace.settings` would be an arbitrarily elected one of them, see
    /// [`TeamScope`] -- so the policy is absent and the callers deny.
    fn team_byo_for_scope<S: TeamScope + ?Sized>(&self, scope: &S) -> Option<&TeamByoSettings> {
        match scope.team_uid() {
            Some(_) => self
                .team_from_scope(scope)
                .and_then(|team| team.settings.team_byo.as_ref()),
            None => {
                let workspace = self.current_workspace()?;
                match workspace.teams.as_slice() {
                    [] => workspace.settings.team_byo.as_ref(),
                    [team] => team.settings.team_byo.as_ref(),
                    _ => None,
                }
            }
        }
    }

    /// The AI autonomy policy that applies to `scope`'s team.
    pub(crate) fn ai_autonomy_settings<S: TeamScope + ?Sized>(
        &self,
        scope: &S,
    ) -> AiAutonomySettings {
        match scope.team_uid().and_then(|id| self.team_from_uid(id)) {
            Some(team) => AiAutonomySettings::from(&team.settings.ai_autonomy),
            None => self
                .current_workspace()
                .map(|workspace| workspace.settings.ai_autonomy_settings.clone())
                .unwrap_or_default(),
        }
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
}
