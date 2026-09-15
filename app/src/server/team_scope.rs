use crate::server::ids::ServerId;
use crate::workspaces::user_workspaces::TeamScope;

/// The team an outbound request is scoped to, as sent in `X-Warp-Team-Uid`.
///
/// A [`TeamScope`] is the only way to name one; there is no constructor from a bare `ServerId`,
/// because a loose uid cannot say which team resolved it, and "no team" is a scope's answer to
/// give rather than a value to pass. The field is private to this module and this module holds
/// nothing else, so [`Self::from_scope`] is provably the only way to build one -- anything else
/// added here gains the ability to forge a team.
///
/// `Copy`, unlike the [`TeamScope`] types it comes from -- those are deliberately not, so a live
/// scope cannot be stashed where it outlives its window. A resolved snapshot has no such hazard,
/// so `ResponseStream` can reuse one capture across every retry rather than re-resolving to
/// whatever team its window switched to since.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestTeamScope(Option<ServerId>);

impl RequestTeamScope {
    pub fn from_scope(scope: &(impl TeamScope + ?Sized)) -> Self {
        Self(scope.team_uid())
    }

    /// Whether `scope` is still compatible with the team this request was captured for.
    ///
    /// A capture of `None` means the window's team was not yet known (e.g. its initial
    /// workspace-metadata fetch was still in flight), not that the request was scoped to
    /// "no team" on purpose -- so a later resolution to a real team is that same unknown
    /// team becoming known, not a switch. Only two known-but-different teams, or a known
    /// team reverting to unknown, are treated as an actual change.
    pub fn matches_scope(self, scope: &(impl TeamScope + ?Sized)) -> bool {
        match (self.0, scope.team_uid()) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some(captured), Some(current)) => captured == current,
        }
    }

    /// The wire uid. `None` sends no team header, leaving the server to its own default.
    pub(crate) fn team_uid(self) -> Option<ServerId> {
        self.0
    }
}

#[cfg(test)]
#[path = "team_scope_tests.rs"]
mod tests;
