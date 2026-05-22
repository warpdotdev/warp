use crate::error::UserFacingError;
use crate::object_permissions::{AccessLevel, ObjectPermissions};
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;
use crate::user::PublicUserProfile;

#[derive(cynic::QueryVariables, Debug)]
pub struct AddObjectGuestsVariables {
    pub input: AddObjectGuestsInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RootMutation", variables = "AddObjectGuestsVariables")]
pub struct AddObjectGuests {
    #[arguments(input: $input, requestContext: $request_context)]
    pub add_object_guests: AddObjectGuestsResult,
}
crate::client::define_operation! {
    add_object_guests(AddObjectGuestsVariables) -> AddObjectGuests;
}

#[derive(cynic::QueryFragment, Debug)]
pub struct AddObjectGuestsOutput {
    pub object_permissions: ObjectPermissions,
    pub response_context: ResponseContext,
    pub user_profiles: Option<Vec<PublicUserProfile>>,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum AddObjectGuestsResult {
    AddObjectGuestsOutput(AddObjectGuestsOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::InputObject, Debug)]
pub struct AddObjectGuestsInput {
    pub access_level: AccessLevel,
    pub object_uid: cynic::Id,
    pub user_emails: Vec<String>,
}
