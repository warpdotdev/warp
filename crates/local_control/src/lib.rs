pub mod auth;
pub mod catalog;
pub mod client;
pub mod discovery;
pub mod protocol;
pub mod selection;
pub mod selectors;

pub use auth::{
    AuthToken, AuthenticatedUserGrant, CredentialGrant, CredentialRequest, ScopedCredential,
};
pub use catalog::{
    ActionImplementationStatus, ActionKind, ActionMetadata, AuthenticatedUserRequirement,
    InvocationContext, PermissionCategory, RiskTier, StateDataCategory, TargetScope,
};
pub use discovery::{
    ControlEndpoint, CredentialBrokerReference, InstanceId, InstanceRecord, RegisteredInstance,
    discovery_dir,
};
pub use protocol::{
    Action, ActionGetParams, ActionGetResult, ActionListParams, ActionListResult,
    ActiveTargetChain, AppActiveParams, AppInspectParams, AppInspectResult, AppVersionResult,
    ControlError, ControlResponse, EmptyParams, ErrorCode, ErrorResponseEnvelope,
    ExecutionContextProof, PROTOCOL_VERSION, PaneListResult, PaneSummary, RequestEnvelope,
    ResponseEnvelope, SessionListResult, SessionSummary, TabListResult, TabSummary,
    WindowListResult, WindowSummary,
};
pub use selectors::{PaneSelector, SessionSelector, TabSelector, TargetSelector, WindowSelector};
