//! Agent hooks: user-configured scripts that run at fixed points in the agent loop. This module
//! holds the canonical model, the format adapters, the registry of active bindings, the trust and
//! permission policies, and the dispatcher. The interception points that fire boundaries live in
//! the application crate.

pub mod adapters;
pub mod dispatch;
pub mod model;
pub mod notices;
pub mod permission;
pub mod registry;
pub mod runner;
#[cfg(any(test, feature = "test-util"))]
pub mod test_util;
pub mod trust;

pub use adapters::{
    DeclaredBinding, DecodePolicy, HookConfigError, HookFormatAdapter, ParsedHookConfig,
};
pub use dispatch::{DispatchOutcome, HookDispatcher, InvocationRecord};
pub use model::{
    BindingCapabilities, CanonicalToolName, CapabilityWarning, ContinuationRequest,
    DiagnosticSeverity, ENVELOPE_SCHEMA_VERSION, EffectKind, EmptyOutputPolicy, FailurePolicy,
    FollowUpChainId, HOOK_FOLLOW_UP_CAP, HOOK_PROTOCOL_VERSION, HandlerKind, HookBindingId,
    HookBoundary, HookDiagnostic, HookEffects, HookEnvelope, HookExecutionHost, HookFormat,
    HookPayload, HookReason, HookSourceId, HookSourceKind, HookSourceLocation, HookSurface,
    HookTurnId, HookVerdict, MalformedOutputPolicy, MatcherTarget, MergedEffects, PermissionFacts,
    PromptProvenance, RunMetadata, TurnStopStatus, UnsupportedEffect, reduce_effects,
    validate_capabilities,
};
pub use notices::{HookMessenger, HookNotice, HookNoticeKind, LogOnlyMessenger};
pub use permission::{
    DEFAULT_DENY_REASON, DenyCause, HookPermissionPolicy, PermissionDecision, PermissionFloors,
    REWRITE_UNSUPPORTED_DENY_REASON, RunFacts, WarpPermissionResult,
};
pub use registry::{
    ActiveBinding, BindingScope, HookRegistry, HookRegistryEvent, RegistrySnapshot,
};
pub use runner::{
    HookInvocation, HookProcessOutcome, HookRunner, PathCapture, ProcessExit, UnavailableRunner,
};
pub use trust::{CommandExecutionPosture, DenyAllTrustPolicy, TrustDecision, TrustPolicy};
