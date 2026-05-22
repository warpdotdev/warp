pub mod auth;
pub mod client;
pub mod discovery;
pub mod protocol;
pub mod selection;

pub use auth::{AuthToken, CredentialGrant, CredentialRequest, ScopedCredential};
pub use discovery::{
    ControlEndpoint, CredentialBrokerReference, InstanceId, InstanceRecord, RegisteredInstance,
    discovery_dir,
};
pub use protocol::{
    Action, ActionImplementationStatus, ActionKind, ActionMetadata, ControlError, ControlResponse,
    ErrorCode, ErrorResponseEnvelope, ExecutionContextProof, InvocationContext,
    LocalControlPermission, PROTOCOL_VERSION, PaneSelector, RequestEnvelope, ResponseEnvelope,
    RiskTier, TabSelector, TargetScope, TargetSelector, WindowSelector,
};
