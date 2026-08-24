use std::rc::Rc;

use warpui::{AppContext, Entity, SingletonEntity, ViewContext, WeakViewHandle};

use super::UserWorkspaces;
use crate::server::ids::ServerId;
use crate::workspaces::workspace::AiAutonomySettings;

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
/// some fallback. `team_uid() == None` means the scope's own window/operation has no team, and
/// the getter must act as if the operation is not on a team -- not substitute another team's
/// settings. `current_workspace().settings` is a safe fallback only when the user belongs to
/// no team at all: whenever the user has one or more teams, it is one arbitrarily-chosen
/// team's effective settings (`GetEffectiveWorkspaceSettingsForWorkspace` server-side), not
/// workspace-level data. Code with no window at all must not construct a scope to route around
/// this; it should read across every team explicitly, the way
/// `UserWorkspaces::teams_allow_codebase_context` does.
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
    // Nothing constructs a test context yet; remove this `#[allow(dead_code)]` once a Group 1
    // migration PR has a real call site.
    #[allow(dead_code)]
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

    /// Resolves `view`'s window team for one read. See [`TeamContext`].
    ///
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
}
