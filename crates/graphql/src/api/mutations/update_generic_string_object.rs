use crate::error::UserFacingError;
use crate::generic_string_object::GenericStringObject;
use crate::object::ObjectUpdateSuccess;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::scalars::Time;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct UpdateGenericStringObjectVariables {
    pub input: UpdateGenericStringObjectInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct UpdateGenericStringObjectOutput {
    pub response_context: ResponseContext,
    pub update: GenericStringObjectUpdate,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "UpdateGenericStringObjectVariables"
)]
pub struct UpdateGenericStringObject {
    #[arguments(input: $input, requestContext: $request_context)]
    pub update_generic_string_object: UpdateGenericStringObjectResult,
}
crate::client::define_operation! {
    update_generic_string_object(UpdateGenericStringObjectVariables) -> UpdateGenericStringObject;
}

#[derive(cynic::QueryFragment, Debug)]
pub struct GenericStringObjectUpdateRejected {
    pub conflicting_generic_string_object: GenericStringObject,
    pub revision_ts: Time,
}

#[derive(cynic::InlineFragments, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum GenericStringObjectUpdate {
    GenericStringObjectUpdateRejected(GenericStringObjectUpdateRejected),
    ObjectUpdateSuccess(ObjectUpdateSuccess),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::InlineFragments, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum UpdateGenericStringObjectResult {
    UpdateGenericStringObjectOutput(UpdateGenericStringObjectOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::InputObject, Debug)]
pub struct UpdateGenericStringObjectInput {
    pub revision_ts: Option<Time>,
    pub serialized_model: String,
    pub uid: cynic::Id,
}
