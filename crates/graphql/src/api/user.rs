use crate::schema;

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct PublicUserProfile {
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub photo_url: Option<String>,
    pub uid: String,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct DiscoverableTeamData {
    pub team_uid: cynic::Id,
    pub num_members: i32,
    pub name: String,
    pub color: Option<String>,
    pub team_accepting_invites: bool,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct JoinableWorkspaceData {
    pub workspace_uid: cynic::Id,
    pub name: String,
    pub joinable_teams: Vec<DiscoverableTeamData>,
}
