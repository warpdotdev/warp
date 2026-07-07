use crate::error::UserFacingError;
use crate::managed_secrets::{ManagedSecret, ManagedSecretType};
use crate::object_permissions::Owner;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct CreateManagedSecretVariables {
    pub input: CreateManagedSecretInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "CreateManagedSecretVariables"
)]
pub struct CreateManagedSecret {
    #[arguments(input: $input, requestContext: $request_context)]
    pub create_managed_secret: CreateManagedSecretResult,
}

crate::client::define_operation! {
    create_managed_secret(CreateManagedSecretVariables) -> CreateManagedSecret;
}

#[derive(cynic::QueryFragment, Debug)]
pub struct CreateManagedSecretOutput {
    pub managed_secret: ManagedSecret,
    pub response_context: ResponseContext,
}

#[derive(cynic::InlineFragments, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum CreateManagedSecretResult {
    CreateManagedSecretOutput(CreateManagedSecretOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::InputObject, Debug)]
pub struct CreateManagedSecretInput {
    pub description: Option<String>,
    pub encrypted_value: String,
    pub name: String,
    pub owner: Owner,
    #[cynic(rename = "type")]
    pub type_: ManagedSecretType,
}
