use crate::{
    error::UserFacingError, managed_secrets::ManagedSecret, request_context::RequestContext, schema,
};

/// Input-side `AgentHarness` enum without fallback, required by cynic for input objects.
/// The output-side `crate::ai::AgentHarness` has `Other(String)` which makes it
/// unusable in `InputObject` derives.
#[derive(cynic::Enum, Clone, Copy, Debug, PartialEq)]
#[cynic(graphql_type = "AgentHarness")]
pub enum AgentHarnessInput {
    Oz,
    ClaudeCode,
    Gemini,
    Codex,
}

impl From<crate::ai::AgentHarness> for Option<AgentHarnessInput> {
    fn from(h: crate::ai::AgentHarness) -> Self {
        match h {
            crate::ai::AgentHarness::Oz => Some(AgentHarnessInput::Oz),
            crate::ai::AgentHarness::ClaudeCode => Some(AgentHarnessInput::ClaudeCode),
            crate::ai::AgentHarness::Gemini => Some(AgentHarnessInput::Gemini),
            crate::ai::AgentHarness::Codex => Some(AgentHarnessInput::Codex),
            crate::ai::AgentHarness::Other(_) => None,
        }
    }
}

/// A GraphQL query to list managed secrets that authenticate the given harness.
#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootQuery",
    variables = "ListHarnessAuthSecretsVariables"
)]
pub struct ListHarnessAuthSecrets {
    #[arguments(input: $input, requestContext: $request_context)]
    pub harness_auth_secrets: HarnessAuthSecretsResult,
}

crate::client::define_operation! {
    list_harness_auth_secrets(ListHarnessAuthSecretsVariables) -> ListHarnessAuthSecrets;
}

#[derive(cynic::QueryVariables, Debug)]
pub struct ListHarnessAuthSecretsVariables {
    pub input: ListHarnessAuthSecretsInput,
    pub request_context: RequestContext,
}

#[derive(cynic::InputObject, Debug)]
pub struct ListHarnessAuthSecretsInput {
    pub harness: AgentHarnessInput,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum HarnessAuthSecretsResult {
    HarnessAuthSecretsOutput(HarnessAuthSecretsOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct HarnessAuthSecretsOutput {
    pub managed_secrets: Vec<ManagedSecret>,
}
