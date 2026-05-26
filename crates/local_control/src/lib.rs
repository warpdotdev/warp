//! Shared protocol, discovery, authentication, and client types for local Warp control.
//!
//! The `local_control` crate is intentionally UI-agnostic so the Warp app and
//! `warpctrl` CLI can share the same wire envelopes, action catalog, discovery
//! records, selectors, and credential validation rules.
pub mod auth;
pub mod catalog;
pub mod client;
pub mod discovery;
pub mod protocol;
pub mod scripting;
pub mod selection;
pub mod selectors;

pub use auth::{
    AuthToken, AuthenticatedUserGrant, CredentialGrant, CredentialRequest, ScopedCredential,
    TerminalSessionProof, TerminalSessionProofRegistry,
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
    Action, ActionNameParams, BindingNameParams, BlockIdParams, BlockInspectResult, BlockListParams,
    BlockListResult, BlockSummary, ControlError, ControlResponse, DriveInspectParams,
    DriveInspectResult, DriveObjectListParams, DriveObjectSummary, DriveObjectType, EmptyParams,
    ErrorCode, ErrorResponseEnvelope, ExecutionContextProof, HistoryListParams, HistoryListResult, InputStateResult,
    KeybindingGetParams, LimitParams, PROTOCOL_VERSION, RequestEnvelope, ResponseEnvelope, SettingGetParams, ThemeStateResult,
};
pub use selectors::{
    PaneSelector, SessionSelector, TabSelector, TargetSelector, WindowSelector,
};
pub use scripting::{ScriptingGrant, ScriptingIdentitySource, ScriptingScope};
