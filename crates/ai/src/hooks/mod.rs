//! Agent hooks: user-configured scripts that run at fixed points in the agent loop. This module
//! holds the canonical model, the format adapters, the registry of active bindings, the permission
//! policy, and the dispatcher. The interception points that fire boundaries live in the
//! application crate.

pub mod adapters;
pub mod dispatch;
#[cfg(all(test, feature = "local_fs"))]
mod e2e_tests;
pub mod model;
pub mod notices;
pub mod permission;
pub mod registry;
pub mod runner;
pub mod sources;
#[cfg(any(test, feature = "test-util"))]
pub mod test_util;

pub use adapters::{
    DeclaredBinding, DecodePolicy, HookConfigError, HookDirectory, HookFormatAdapter,
    HookSourceLayout, ParsedHookConfig, source_layout,
};
pub use dispatch::{DispatchOutcome, HookDispatcher, InvocationRecord};
pub use model::{
    BindingCapabilities, CapabilityWarning, ContinuationRequest, DiagnosticSeverity,
    ENVELOPE_SCHEMA_VERSION, EffectKind, EmptyOutputPolicy, FailurePolicy, FollowUpChainId,
    HOOK_FOLLOW_UP_CAP, HOOK_PROTOCOL_VERSION, HandlerKind, HookAsker, HookBindingId, HookBoundary,
    HookDiagnostic, HookEffects, HookEnvelope, HookExecutionHost, HookFormat, HookOnAsk,
    HookPayload, HookReason, HookSourceId, HookSourceKind, HookSourceLocation, HookSourcePolicy,
    HookSurface, HookTurnId, HookVerdict, MalformedOutputPolicy, MatcherTarget, MergedEffects,
    PermissionFacts, PromptAttachment, PromptAttachmentKind, PromptProvenance, RunMetadata,
    SubagentExecutionMode, SubagentLaunch, TurnStopStatus, UnsupportedEffect, reduce_effects,
    validate_capabilities,
};
pub use notices::{HookMessenger, HookNotice, HookNoticeKind, LogOnlyMessenger};
pub use permission::{
    APPROVAL_UNAVAILABLE_PREFIX, DEFAULT_ASK_REASON, DEFAULT_DENY_REASON,
    DEFERRAL_UNSUPPORTED_DENY_REASON, DenyCause, HookPermissionPolicy, PermissionDecision,
    PermissionFloors, REWRITE_UNSUPPORTED_DENY_REASON, RunFacts, WarpPermissionResult,
};
pub use registry::{ActiveBinding, HookRegistry, HookRegistryEvent, RegistrySnapshot};
pub use runner::{
    HookInvocation, HookProcessOutcome, HookRunner, PathCapture, ProcessExit, UnavailableRunner,
};
#[cfg(feature = "local_fs")]
pub use sources::load_explicit_sources;
pub use sources::{ExplicitHookSource, SourceLoadFailure};
