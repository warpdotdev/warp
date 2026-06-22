use crate::api::queries::get_feature_model_choices::FeatureModelChoice;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct FreeAvailableModelsVariables {
    pub input: FreeAvailableModelsInput,
    pub request_context: RequestContext,
}

#[derive(cynic::InputObject, Debug)]
pub struct FreeAvailableModelsInput {
    pub referrer: Option<String>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RootQuery", variables = "FreeAvailableModelsVariables")]
pub struct FreeAvailableModels {
    #[arguments(input: $input, requestContext: $request_context)]
    pub free_available_models: FreeAvailableModelsResult,
}

crate::client::define_operation! {
    free_available_models(FreeAvailableModelsVariables) -> FreeAvailableModels;
}

#[derive(cynic::InlineFragments, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum FreeAvailableModelsResult {
    FreeAvailableModelsOutput(FreeAvailableModelsOutput),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct FreeAvailableModelsOutput {
    pub feature_model_choice: FeatureModelChoice,
    #[allow(dead_code)]
    pub response_context: ResponseContext,
}
