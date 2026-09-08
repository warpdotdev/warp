pub use warp_request_context::RequestTeamScope;

use crate::workspaces::user_workspaces::TeamScope;
pub fn request_team_scope(scope: &(impl TeamScope + ?Sized)) -> RequestTeamScope {
    RequestTeamScope::new(scope.team_uid().map(|team_uid| team_uid.uid()))
}
