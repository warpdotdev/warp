use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

/// The transport the agent process drives a run's third-party harness over.
#[derive(cynic::Enum, Clone, Copy, Debug, PartialEq)]
pub enum HarnessTransport {
    #[cynic(rename = "ACP")]
    Acp,
    #[cynic(rename = "PTY")]
    Pty,
}

/// Why the harness is driven over a PTY although the run was dispatched over ACP.
#[derive(cynic::Enum, Clone, Copy, Debug, PartialEq)]
pub enum HarnessTransportFallbackReason {
    /// No coding CLI sidecar was mounted.
    #[cynic(rename = "NO_SIDECAR")]
    NoSidecar,
    /// A sidecar was mounted but carries no ACP adapter.
    #[cynic(rename = "SIDECAR_WITHOUT_ADAPTER")]
    SidecarWithoutAdapter,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct ReportHarnessTransportVariables {
    pub input: ReportHarnessTransportInput,
    pub request_context: RequestContext,
}

#[derive(cynic::InputObject, Debug)]
pub struct ReportHarnessTransportInput {
    pub transport: HarnessTransport,
    /// Omitted when no fallback happened.
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub fallback_reason: Option<HarnessTransportFallbackReason>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "ReportHarnessTransportVariables"
)]
pub struct ReportHarnessTransport {
    #[arguments(input: $input, requestContext: $request_context)]
    pub report_harness_transport: ReportHarnessTransportResult,
}

crate::client::define_operation! {
    report_harness_transport(ReportHarnessTransportVariables) -> ReportHarnessTransport;
}

#[derive(cynic::InlineFragments, Debug)]
pub enum ReportHarnessTransportResult {
    ReportHarnessTransportOutput(ReportHarnessTransportOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct ReportHarnessTransportOutput {
    pub response_context: ResponseContext,
}

#[cfg(test)]
#[path = "report_harness_transport_tests.rs"]
mod tests;
