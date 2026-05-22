use crate::error::UserFacingError;
use crate::managed_secrets::ManagedSecret;
use crate::object_permissions::Owner;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct UpdateManagedSecretVariables {
    pub input: UpdateManagedSecretInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "UpdateManagedSecretVariables"
)]
pub struct UpdateManagedSecret {
    #[arguments(input: $input, requestContext: $request_context)]
    pub update_managed_secret: UpdateManagedSecretResult,
}

crate::client::define_operation! {
    update_managed_secret(UpdateManagedSecretVariables) -> UpdateManagedSecret;
}

#[derive(cynic::QueryFragment, Debug)]
pub struct UpdateManagedSecretOutput {
    pub managed_secret: ManagedSecret,
    pub response_context: ResponseContext,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum UpdateManagedSecretResult {
    UpdateManagedSecretOutput(UpdateManagedSecretOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::InputObject, Debug)]
pub struct UpdateManagedSecretInput {
    pub owner: Owner,
    pub name: String,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub encrypted_value: Option<String>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}
