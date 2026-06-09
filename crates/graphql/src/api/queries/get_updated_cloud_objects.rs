use crate::error::UserFacingError;
use crate::folder::Folder;
use crate::generic_string_object::GenericStringObject;
use crate::mcp_gallery_template::MCPGalleryTemplate;
use crate::notebook::Notebook;
use crate::object_actions::ObjectActionHistory;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::scalars::Time;
use crate::schema;
use crate::user::PublicUserProfile;
use crate::workflow::Workflow;

#[derive(cynic::QueryVariables, Debug)]
pub struct GetUpdatedCloudObjectsVariables {
    pub input: UpdatedCloudObjectsInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct UpdatedCloudObjectsOutput {
    pub action_histories: Option<Vec<ObjectActionHistory>>,
    pub deleted_object_uids: DeletedObjectUids,
    pub folders: Option<Vec<Folder>>,
    pub generic_string_objects: Option<Vec<GenericStringObject>>,
    pub mcp_gallery: Option<Vec<MCPGalleryTemplate>>,
    pub notebooks: Option<Vec<Notebook>>,
    pub response_context: ResponseContext,
    pub user_profiles: Option<Vec<PublicUserProfile>>,
    pub workflows: Option<Vec<Workflow>>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootQuery",
    variables = "GetUpdatedCloudObjectsVariables"
)]
pub struct GetUpdatedCloudObjects {
    #[arguments(input: $input, requestContext: $request_context)]
    pub updated_cloud_objects: UpdatedCloudObjectsResult,
}
crate::client::define_operation! {
    get_updated_cloud_objects(GetUpdatedCloudObjectsVariables) -> GetUpdatedCloudObjects;
}

#[derive(cynic::QueryFragment, Debug)]
pub struct DeletedObjectUids {
    pub folder_uids: Option<Vec<cynic::Id>>,
    pub generic_string_object_uids: Option<Vec<cynic::Id>>,
    pub notebook_uids: Option<Vec<cynic::Id>>,
    pub workflow_uids: Option<Vec<cynic::Id>>,
}

#[derive(cynic::InlineFragments, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum UpdatedCloudObjectsResult {
    UpdatedCloudObjectsOutput(UpdatedCloudObjectsOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::InputObject, Debug)]
pub struct UpdatedCloudObjectsInput {
    pub folders: Option<Vec<UpdatedObjectInput>>,
    pub force_refresh: bool,
    pub generic_string_objects: Option<Vec<UpdatedObjectInput>>,
    pub notebooks: Option<Vec<UpdatedObjectInput>>,
    pub workflows: Option<Vec<UpdatedObjectInput>>,
}

#[derive(cynic::InputObject, Debug, Clone)]
pub struct UpdatedObjectInput {
    pub actions_ts: Option<Time>,
    pub metadata_ts: Option<Time>,
    pub permissions_ts: Option<Time>,
    pub revision_ts: Time,
    pub uid: cynic::Id,
}
