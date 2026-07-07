use super::create_generic_string_object::CreateGenericStringObjectOutput;
use crate::error::UserFacingError;
use crate::generic_string_object::GenericStringObjectInput;
use crate::object_permissions::Owner;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct BulkCreateObjectsVariables {
    pub input: BulkCreateObjectsInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "BulkCreateObjectsVariables"
)]
pub struct BulkCreateObjects {
    #[arguments(requestContext: $request_context, input: $input)]
    pub bulk_create_objects: BulkCreateObjectsResult,
}
crate::client::define_operation! {
    bulk_create_objects(BulkCreateObjectsVariables) -> BulkCreateObjects;
}

#[derive(cynic::QueryFragment, Debug)]
pub struct BulkCreateObjectsOutput {
    pub generic_string_objects: Option<BulkCreateGenericStringObjectsOutput>,
    pub response_context: ResponseContext,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct BulkCreateGenericStringObjectsOutput {
    pub objects: Vec<CreateGenericStringObjectOutput>,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum BulkCreateObjectsResult {
    BulkCreateObjectsOutput(BulkCreateObjectsOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::InputObject, Debug)]
pub struct BulkCreateObjectsInput {
    pub generic_string_objects: Option<BulkCreateGenericStringObjectsInput>,
}

#[derive(cynic::InputObject, Debug)]
pub struct BulkCreateGenericStringObjectsInput {
    pub objects: Vec<GenericStringObjectInput>,
    pub owner: Owner,
}
