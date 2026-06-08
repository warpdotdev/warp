use crate::error::UserFacingError;
use crate::generic_string_object::{GenericStringObject, GenericStringObjectInput};
use crate::object_permissions::Owner;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::scalars::Time;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct CreateGenericStringObjectVariables {
    pub input: CreateGenericStringObjectInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "CreateGenericStringObjectVariables"
)]
pub struct CreateGenericStringObject {
    #[arguments(input: $input, requestContext: $request_context)]
    pub create_generic_string_object: CreateGenericStringObjectResult,
}
crate::client::define_operation! {
    create_generic_string_object(CreateGenericStringObjectVariables) -> CreateGenericStringObject;
}

#[derive(cynic::QueryFragment, Debug)]
pub struct CreateGenericStringObjectOutput {
    pub client_id: cynic::Id,
    pub generic_string_object: GenericStringObject,
    pub response_context: ResponseContext,
    pub revision_ts: Time,
}

#[derive(cynic::InlineFragments, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum CreateGenericStringObjectResult {
    CreateGenericStringObjectOutput(CreateGenericStringObjectOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::InputObject, Debug)]
pub struct CreateGenericStringObjectInput {
    pub generic_string_object: GenericStringObjectInput,
    pub owner: Owner,
}
