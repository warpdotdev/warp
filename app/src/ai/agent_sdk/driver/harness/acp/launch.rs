use serde::Deserialize;

/// The process to run for a harness driven over the Agent Client Protocol.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct AcpLaunchSpec {
    pub(super) program: String,
    pub(super) args: Vec<String>,
}
