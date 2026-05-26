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
    ActionImplementationStatus, ActionKind, ActionMetadata, ActionParameterSpec, ActionResultSpec,
    AuthenticatedUserRequirement, EXCLUDED_FILE_CONTENT_ACTION_NAMES, InvocationContext,
    PermissionCategory, RiskTier, StateDataCategory, TargetScope,
};
pub use discovery::{
    ControlEndpoint, CredentialBrokerReference, InstanceId, InstanceRecord, RegisteredInstance,
    discovery_dir,
};
pub use protocol::{
    Action, ActionParams, ApiKeySource, AppSurfaceParams, AppearanceFontSizeParams,
    AppearanceMutationResult, AppearanceSetParams, AppearanceStateResult, AppearanceZoomParams,
    BlockListParams, BlockListResult, BlockOutputFormat, BlockOutputParams, BlockOutputResult,
    BlockSummary, ControlError, ControlResponse, ControlResult, Direction, DriveInspectParams,
    DriveInspectResult, DriveListParams, DriveListResult, DriveMutationAudit, DriveMutationResult,
    DriveObjectCreateParams, DriveObjectInsertParams, DriveObjectSummary, DriveObjectUpdateParams,
    ErrorCode, ErrorResponseEnvelope, ExecutionContextProof, FileListResult, FileOpenParams,
    FileSummary, HistoryEntrySummary, HistoryListParams, HistoryListResult, HorizontalDirection,
    InputClearParams, InputInsertParams, InputMode, InputModeSetParams, InputReplaceParams,
    InputRunParams, InputStateResult, PROTOCOL_VERSION, PaneDirection, PaneMaximizeParams,
    PaneMutationResult, PaneNavigateParams, PaneResizeParams, PaneSplitParams, ProjectActiveResult,
    ProjectListResult, ProjectSummary, RequestEnvelope, ResponseEnvelope, SettingGetParams,
    SettingGetResult, SettingListResult, SettingMutationResult, SettingSetParams, SettingSummary,
    SettingToggleParams, SizeAdjustment, TabActivateParams, TabActivationMode, TabActivationTarget,
    TabCloseMode, TabCloseParams, TabCloseScope, TabCreateParams, TabMoveParams, TabMutationResult,
    TabType, ThemeListResult, ThemeSetParams, ThemeSummary, WindowCloseParams, WindowCreateParams,
};
pub use scripting::{
    ApiKeySecret, ApiKeyStatus, ApiKeyStorageRef, AuthStatusSummary, ScriptingGrant,
    ScriptingIdentitySource, ScriptingScope,
};
pub use selectors::{
    BlockSelector, BlockTarget, DriveObjectId, DriveObjectTarget, DriveObjectType, FileTarget,
    InstanceTarget, PaneSelector, PaneTarget, ProjectTarget, SessionSelector, SessionTarget,
    TabSelector, TabTarget, TargetSelector, WindowSelector, WindowTarget,
};
