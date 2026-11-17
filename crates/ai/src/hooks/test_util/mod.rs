mod latency_runner;
mod recording_adapter;
mod recording_messenger;
mod scripted_runner;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ai_types::AIConversationId;
pub use latency_runner::LatencyRunner;
pub use recording_adapter::RecordingAdapter;
pub use recording_messenger::RecordingMessenger;
pub use scripted_runner::{RunnerHold, ScriptedRunner};
use uuid::Uuid;

use crate::hooks::adapters::{DeclaredBinding, HookDirectory, HookFormatAdapter};
use crate::hooks::model::{
    BindingCapabilities, EffectKind, EmptyOutputPolicy, FailurePolicy, HOOK_PROTOCOL_VERSION,
    HandlerKind, HookBindingId, HookBoundary, HookEnvelope, HookExecutionHost, HookFormat,
    HookPayload, HookSourceId, HookSourceKind, HookSourceLocation, HookSourcePolicy, HookSurface,
    MalformedOutputPolicy, MatcherTarget, PermissionFacts,
};
use crate::hooks::registry::ActiveBinding;
use crate::hooks::sources::ExplicitHookSource;

pub fn test_source_id(name: &str) -> HookSourceId {
    HookSourceId {
        kind: HookSourceKind::FactoryAgent,
        location: HookSourceLocation::Named(name.to_owned()),
    }
}

/// A Cursor source in `directory` on the worker, outside any repository.
pub fn worker_source(
    directory: &Path,
    kind: HookSourceKind,
    policy: HookSourcePolicy,
) -> ExplicitHookSource {
    ExplicitHookSource::from_directory(
        HookFormat::Cursor,
        &HookDirectory::Worker {
            directory: directory.to_path_buf(),
        },
        kind,
        policy,
    )
}

/// A Cursor source in `directory` inside the repository checked out at `root`.
pub fn repository_source(
    root: &Path,
    directory: &Path,
    kind: HookSourceKind,
    policy: HookSourcePolicy,
) -> ExplicitHookSource {
    ExplicitHookSource::from_directory(
        HookFormat::Cursor,
        &HookDirectory::Repository {
            root: root.to_path_buf(),
            directory: directory.to_path_buf(),
        },
        kind,
        policy,
    )
}

pub fn test_source_id_of_kind(name: &str, kind: HookSourceKind) -> HookSourceId {
    HookSourceId {
        kind,
        location: HookSourceLocation::Named(name.to_owned()),
    }
}

pub fn test_binding_id(source: HookSourceId, event: &str, index: usize) -> HookBindingId {
    HookBindingId {
        source,
        event: event.to_owned(),
        index,
    }
}

/// Capabilities that can only deny or notify, which every Phase 1 boundary can honor.
pub fn verdict_and_notice_capabilities() -> BindingCapabilities {
    BindingCapabilities {
        may_produce: HashSet::from([EffectKind::Verdict, EffectKind::UserNotice]),
        failure: FailurePolicy::FailOpen,
        malformed_output: MalformedOutputPolicy::Deny,
        empty_output: EmptyOutputPolicy::NoOpinion,
        matcher_target: MatcherTarget::ToolName,
        handler: HandlerKind::Command,
    }
}

pub fn test_binding(
    source: HookSourceId,
    boundary: HookBoundary,
    index: usize,
    matcher: Option<&str>,
) -> DeclaredBinding {
    DeclaredBinding {
        id: test_binding_id(source, boundary.canonical_name(), index),
        boundary,
        vendor_event: boundary.canonical_name().to_owned(),
        capabilities: verdict_and_notice_capabilities(),
        matcher: matcher.map(str::to_owned),
        command: format!("test-hook-{index}"),
        timeout: boundary.default_timeout(),
        loop_limit: None,
    }
}

pub fn active_binding(
    binding: DeclaredBinding,
    adapter: Arc<dyn HookFormatAdapter>,
) -> ActiveBinding {
    ActiveBinding {
        binding,
        adapter,
        host: HookExecutionHost::Local,
        working_directory: PathBuf::from("/"),
        project_dir: None,
        policy: HookSourcePolicy::default(),
    }
}

/// An envelope with neutral common fields, for tests that only care about the payload.
pub fn test_envelope(conversation_id: AIConversationId, payload: HookPayload) -> HookEnvelope {
    HookEnvelope {
        schema_version: 1,
        occurrence_id: Uuid::new_v4(),
        invocation_id: Uuid::nil(),
        conversation_id,
        turn_id: None,
        exchange_id: None,
        action_id: None,
        run: None,
        surface: HookSurface::CliRun,
        execution_host: HookExecutionHost::Local,
        cwd: None,
        roots: Vec::new(),
        source_revision: 0,
        model_id: None,
        user_email: None,
        warp_version: "test".to_owned(),
        protocol_version: HOOK_PROTOCOL_VERSION,
        permission_facts: PermissionFacts::default(),
        session_env: Vec::new(),
        payload,
    }
}
