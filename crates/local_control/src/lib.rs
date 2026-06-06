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
    InvocationContext, TargetScope,
};
pub use discovery::{
    ControlEndpoint, CredentialBrokerReference, InstanceId, InstanceRecord, RegisteredInstance,
    discovery_dir,
};
pub use protocol::{
    Action, ActionNameParams, ActionParams, BindingNameParams, BlockIdParams, BlockInspectResult,
    BlockListParams, BlockListResult, BlockSummary, ControlError, ControlResponse,
    DriveInspectParams, DriveInspectResult, DriveListParams, DriveMutationAudit,
    DriveMutationResult, DriveObjectCreateParams, DriveObjectId, DriveObjectInsertParams,
    DriveObjectListParams, DriveObjectSummary, DriveObjectType, DriveObjectUpdateParams,
    EmptyParams, ErrorCode, ErrorResponseEnvelope, ExecutionContextProof, HistoryListParams,
    HistoryListResult, InputStateResult, KeybindingGetParams, LimitParams, LocalControlAuditRecord,
    PROTOCOL_VERSION, RequestEnvelope, ResponseEnvelope, SettingGetParams, ThemeStateResult,
    WorkflowArgument, WorkflowRunParams,
};
pub use scripting::{ScriptingGrant, ScriptingIdentitySource};
pub use selectors::{PaneSelector, SessionSelector, TabSelector, TargetSelector, WindowSelector};
