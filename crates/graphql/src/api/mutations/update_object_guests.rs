use crate::error::UserFacingError;
use crate::object_permissions::{AccessLevel, ObjectPermissions};
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct UpdateObjectGuestsVariables {
    pub input: UpdateObjectGuestsInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct UpdateObjectGuestsOutput {
    pub object_permissions: ObjectPermissions,
    pub response_context: ResponseContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "UpdateObjectGuestsVariables"
)]
pub struct UpdateObjectGuests {
    #[arguments(input: $input, requestContext: $request_context)]
    pub update_object_guests: UpdateObjectGuestsResult,
}
crate::client::define_operation! {
    update_object_guests(UpdateObjectGuestsVariables) -> UpdateObjectGuests;
}

#[derive(cynic::InlineFragments, Debug)]
pub enum UpdateObjectGuestsResult {
    UpdateObjectGuestsOutput(UpdateObjectGuestsOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::InputObject, Debug)]
pub struct UpdateObjectGuestsInput {
    pub access_level: AccessLevel,
    pub emails: Option<Vec<String>>,
    pub object_uid: cynic::Id,
}
