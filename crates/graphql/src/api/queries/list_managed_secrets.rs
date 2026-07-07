use crate::error::UserFacingError;
use crate::managed_secrets::ManagedSecret;
use crate::request_context::RequestContext;
use crate::schema;

/// A GraphQL query to list all managed secrets for the current user.
#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RootQuery", variables = "ListManagedSecretsVariables")]
pub struct ListManagedSecrets {
    #[arguments(input: $input, requestContext: $request_context)]
    pub managed_secrets: ManagedSecretsResult,
}

crate::client::define_operation! {
    list_managed_secrets(ListManagedSecretsVariables) -> ListManagedSecrets;
}

#[derive(cynic::QueryVariables, Debug)]
pub struct ListManagedSecretsVariables {
    pub input: ManagedSecretsInput,
    pub request_context: RequestContext,
}

#[derive(cynic::InputObject, Debug, Default)]
pub struct ManagedSecretsInput {
    pub cursor: Option<String>,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum ManagedSecretsResult {
    ManagedSecretsOutput(ManagedSecretsOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct ManagedSecretsOutput {
    pub managed_secrets: Vec<ManagedSecret>,
}
