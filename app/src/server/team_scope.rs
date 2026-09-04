use crate::cloud_object::Owner;
use crate::server::ids::ServerId;
use crate::workspaces::user_workspaces::TeamScope;

/// The validated team scope for an outbound request, sent in `X-Warp-Team-Uid` when selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestTeamScope(Option<ServerId>);

impl RequestTeamScope {
    pub fn from_scope(scope: &impl TeamScope) -> Self {
        Self(scope.team_uid())
    }

    /// The wire uid. `None` sends no team header, leaving the server to its own default.
    pub(crate) fn team_uid(self) -> Option<ServerId> {
        self.0
    }

    pub(crate) fn includes_owner_for_sync(self, owner: Owner) -> bool {
        match owner {
            Owner::User { .. } => true,
            Owner::Team { team_uid } => self.0 == Some(team_uid),
        }
    }

    pub(crate) fn allows_scoped_deletion(self, owner: Owner) -> bool {
        match owner {
            Owner::User { .. } => false,
            Owner::Team { team_uid } => self.0 == Some(team_uid),
        }
    }
}
