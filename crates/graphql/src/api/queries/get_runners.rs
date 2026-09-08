use crate::error::UserFacingError;
use crate::object::Space;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::scalars::Time;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct GetRunnersVariables {
    pub request_context: RequestContext,
    pub sort_by: Option<RunnerSortBy>,
}

#[derive(cynic::Enum, Clone, Copy, Debug)]
pub enum RunnerSortBy {
    Name,
    LastUpdated,
}

#[derive(cynic::Enum, Clone, Copy, Debug, PartialEq, Eq)]
#[cynic(graphql_type = "RunnerOS")]
pub enum RunnerOs {
    #[cynic(rename = "LINUX")]
    Linux,
    #[cynic(rename = "MACOS")]
    Macos,
}

#[derive(cynic::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunnerArch {
    #[cynic(rename = "X86_64")]
    X8664,
    #[cynic(rename = "AARCH64")]
    Aarch64,
}

#[derive(cynic::Enum, Clone, Copy, Debug, PartialEq, Eq)]
#[cynic(graphql_type = "RunnerMacOSVersion")]
pub enum RunnerMacOsVersion {
    #[cynic(rename = "MACOS_14")]
    Macos14,
    #[cynic(rename = "MACOS_15")]
    Macos15,
    #[cynic(rename = "MACOS_26")]
    Macos26,
    #[cynic(rename = "MACOS_27")]
    Macos27,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct RunnerInstanceShape {
    pub vcpus: i32,
    pub memory_gb: i32,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "MacOSConfig")]
pub struct MacOsConfig {
    pub version: Option<RunnerMacOsVersion>,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct LinuxConfig {
    pub docker_image: String,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct RunnerConfig {
    pub name: String,
    pub description: Option<String>,
    pub setup_commands: Option<Vec<String>>,
    pub instance_shape: Option<RunnerInstanceShape>,
    pub os: RunnerOs,
    pub arch: RunnerArch,
    pub mac: Option<MacOsConfig>,
    pub linux: Option<LinuxConfig>,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct PublicUserProfile {
    pub uid: String,
    pub email: Option<String>,
    pub display_name: Option<String>,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct Runner {
    pub uid: cynic::Id,
    pub config: RunnerConfig,
    pub last_updated: Time,
    pub scope: Space,
    pub creator: Option<PublicUserProfile>,
    pub last_editor: Option<PublicUserProfile>,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct GetRunnersOutput {
    pub runners: Vec<Runner>,
    pub response_context: ResponseContext,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum GetRunnersResult {
    GetRunnersOutput(GetRunnersOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RootQuery", variables = "GetRunnersVariables")]
pub struct GetRunners {
    #[arguments(requestContext: $request_context, sortBy: $sort_by)]
    pub get_runners: GetRunnersResult,
}

crate::client::define_operation! {
    get_runners(GetRunnersVariables) -> GetRunners;
}
