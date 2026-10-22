//! Agent hooks: user-configured scripts that run at fixed points in the agent loop. This module
//! holds the canonical model, the format adapters, the registry of active bindings, the trust and
//! permission policies, and the dispatcher. The interception points that fire boundaries live in
//! the application crate.

pub mod adapters;
pub mod dispatch;
#[cfg(all(test, feature = "local_fs"))]
mod e2e_slice_tests;
pub mod model;
pub mod notices;
pub mod permission;
pub mod registry;
pub mod runner;
pub mod sources;
#[cfg(any(test, feature = "test-util"))]
pub mod test_util;
pub mod trust;
pub mod watcher;

pub use adapters::{
    DeclaredBinding, DecodePolicy, HookConfigError, HookFormatAdapter, ParsedHookConfig,
};
pub use dispatch::{DispatchOutcome, DispatchPolicy, HookDispatcher, InvocationRecord};
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
    APPROVAL_UNAVAILABLE_PREFIX, DEFAULT_ASK_REASON, DEFAULT_DENY_REASON,
    DEFERRAL_UNSUPPORTED_DENY_REASON, DenyCause, HookPermissionPolicy, PermissionDecision,
    PermissionFloors, REWRITE_UNSUPPORTED_DENY_REASON, RunFacts, WarpPermissionResult,
};
pub use registry::{
    ActiveBinding, BindingScope, HookRegistry, HookRegistryEvent, PinnedRun, RegistrySnapshot,
    SourceStatus, SourceUpdate, resolve_timeout,
};
pub use runner::{
    HookInvocation, HookProcessOutcome, HookRunner, PathCapture, ProcessExit, UnavailableRunner,
};
#[cfg(feature = "local_fs")]
pub use sources::load_explicit_sources;
pub use sources::{
    ExplicitHookSource, ExplicitSourceScope, PROJECT_CURSOR_HOOKS_PATH, WARP_HOOK_DIRS_ENV,
    parse_hook_dirs_value, resolve_hook_dirs, sources_for_hook_dirs,
};
pub use trust::{
    CommandExecutionPosture, DenyAllTrustPolicy, StaticTrustPolicy, TrustDecision, TrustPolicy,
};
#[cfg(feature = "local_fs")]
pub use watcher::HookConfigWatcher;
pub use watcher::watch_matrix;
