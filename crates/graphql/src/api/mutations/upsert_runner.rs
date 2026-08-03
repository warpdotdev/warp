use crate::error::UserFacingError;
use crate::object_permissions::Owner;
use crate::queries::get_runners::{Runner, RunnerArch, RunnerMacOsVersion, RunnerOs};
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct UpsertRunnerVariables {
    pub input: UpsertRunnerInput,
    pub request_context: RequestContext,
}

#[derive(cynic::InputObject, Debug)]
pub struct RunnerInstanceShapeInput {
    pub vcpus: i32,
    pub memory_gb: i32,
}

#[derive(cynic::InputObject, Debug)]
#[cynic(graphql_type = "MacOSConfigInput")]
pub struct MacOsConfigInput {
    pub version: Option<RunnerMacOsVersion>,
}

#[derive(cynic::InputObject, Debug)]
pub struct LinuxConfigInput {
    pub docker_image: String,
}

#[derive(cynic::InputObject, Debug)]
pub struct RunnerInput {
    pub name: String,
    pub description: Option<String>,
    pub setup_commands: Option<Vec<String>>,
    pub instance_shape: Option<RunnerInstanceShapeInput>,
    pub os: Option<RunnerOs>,
    pub arch: Option<RunnerArch>,
    pub mac: Option<MacOsConfigInput>,
    pub linux: Option<LinuxConfigInput>,
}

#[derive(cynic::InputObject, Debug)]
pub struct UpsertRunnerInput {
    pub uid: Option<cynic::Id>,
    pub owner: Option<Owner>,
    pub runner: RunnerInput,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct UpsertRunnerOutput {
    pub runner: Runner,
    pub is_update: bool,
    pub response_context: ResponseContext,
}

#[derive(cynic::InlineFragments, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum UpsertRunnerResult {
    UpsertRunnerOutput(UpsertRunnerOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RootMutation", variables = "UpsertRunnerVariables")]
pub struct UpsertRunner {
    #[arguments(input: $input, requestContext: $request_context)]
    pub upsert_runner: UpsertRunnerResult,
}

crate::client::define_operation! {
    upsert_runner(UpsertRunnerVariables) -> UpsertRunner;
}
