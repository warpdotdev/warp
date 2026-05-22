use super::object::ObjectMetadata;
use super::object_permissions::ObjectPermissions;
use crate::schema;

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct Workflow {
    pub data: String,
    pub metadata: ObjectMetadata,
    pub permissions: ObjectPermissions,
}
