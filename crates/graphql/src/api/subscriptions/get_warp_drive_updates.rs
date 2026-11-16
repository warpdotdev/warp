use crate::object::{CloudObject, ObjectMetadata};
use crate::object_actions::ObjectActionHistory;
use crate::object_permissions::ObjectPermissions;
use crate::scalars::Time;
use crate::schema;
use crate::user::PublicUserProfile;

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RootSubscription")]
pub struct GetWarpDriveUpdates {
    pub warp_drive_updates: WarpDriveUpdate,
}

#[derive(cynic::InlineFragments, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum WarpDriveUpdate {
    ObjectActionOccurred(ObjectActionOccurred),
    ObjectContentUpdated(ObjectContentUpdated),
    ObjectDeleted(ObjectDeleted),
    ObjectMetadataUpdated(ObjectMetadataUpdated),
    ObjectPermissionsUpdated(ObjectPermissionsUpdated),
    TeamMembershipsChanged(TeamMembershipsChanged),
    AmbientTaskUpdated(AmbientTaskUpdated),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct ObjectPermissionsUpdated {
    pub object_uid: cynic::Id,
    pub permissions: ObjectPermissions,
    pub user_profiles: Option<Vec<PublicUserProfile>>,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct ObjectMetadataUpdated {
    pub metadata: ObjectMetadata,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct ObjectDeleted {
    pub object_uid: cynic::Id,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct ObjectContentUpdated {
    pub last_editor: Option<PublicUserProfile>,
    pub object: CloudObject,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct ObjectActionOccurred {
    pub history: ObjectActionHistory,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct TeamMembershipsChanged {
    pub team_memberships_last_updated_ts: Time,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct AmbientTaskUpdated {
    pub task_id: cynic::Id,
    pub task_updated_ts: Time,
}
