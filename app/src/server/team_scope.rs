use crate::server::ids::ServerId;
use crate::workspaces::user_workspaces::TeamScope;

/// The team an outbound request is scoped to, as sent in `X-Warp-Team-Uid`.
///
/// A [`TeamScope`] is the only way to name a team; there is no constructor from a bare `ServerId`,
/// because a loose uid cannot say which team resolved it. The temporary managed-secrets fallback
/// can only omit the header and cannot forge a team.
///
/// `Copy`, unlike the [`TeamScope`] types it comes from -- those are deliberately not, so a live
/// scope cannot be stashed where it outlives its window. A resolved snapshot has no such hazard,
/// so `ResponseStream` can reuse one capture across every retry rather than re-resolving to
/// whatever team its window switched to since.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestTeamScope(Option<ServerId>);

impl RequestTeamScope {
    pub fn from_scope(scope: &impl TeamScope) -> Self {
        Self(scope.team_uid())
    }
    // TODO: Delete this fallback as follow-up PRs migrate managed-secret callers to explicit scopes.
    pub(crate) fn temporary_managed_secrets_server_fallback() -> Self {
        Self(None)
    }

    /// The wire uid. `None` sends no team header, leaving the server to its own default.
    pub(crate) fn team_uid(self) -> Option<ServerId> {
        self.0
    }
}
