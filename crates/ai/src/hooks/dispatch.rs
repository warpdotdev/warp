use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures::FutureExt;
use futures::future::BoxFuture;
use uuid::Uuid;

use crate::hooks::model::{
    DiagnosticSeverity, HookBindingId, HookDiagnostic, HookEffects, HookEnvelope, MergedEffects,
    reduce_effects,
};
use crate::hooks::notices::{HookMessenger, HookNotice, HookNoticeKind};
use crate::hooks::registry::{ActiveBinding, RegistrySnapshot};
use crate::hooks::runner::{HookInvocation, HookRunner, ProcessExit};

/// Longest stderr excerpt written to the log for one invocation.
const STDERR_TAIL_LOG_LIMIT: usize = 500;

/// What one hook process did, kept for logs and later diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvocationRecord {
    pub invocation_id: Uuid,
    pub binding: HookBindingId,
    pub exit: ProcessExit,
    pub duration: Duration,
    pub stderr_tail: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DispatchOutcome {
    pub effects: MergedEffects,
    pub invocations: Vec<InvocationRecord>,
    pub registry_revision: u64,
}

impl DispatchOutcome {
    fn no_opinion(registry_revision: u64) -> Self {
        Self {
            effects: MergedEffects::no_opinion(),
            invocations: Vec::new(),
            registry_revision,
        }
    }
}

/// The one call the agent loop makes to run hooks: fire an envelope, get merged effects.
///
/// Bindings run one at a time in priority order. Concurrency, deadlines, and per-binding failure
/// policy are not applied here yet; the runner enforces the invocation's timeout.
pub struct HookDispatcher {
    runner: Arc<dyn HookRunner>,
    messenger: Arc<dyn HookMessenger>,
}

impl HookDispatcher {
    pub fn new(runner: Arc<dyn HookRunner>, messenger: Arc<dyn HookMessenger>) -> Self {
        Self { runner, messenger }
    }

    pub fn messenger(&self) -> &Arc<dyn HookMessenger> {
        &self.messenger
    }

    /// Runs every binding in `pinned` that matches `envelope`. `cancel` belongs to this firing:
    /// once set, no further binding starts and the runner is asked to stop the current one.
    pub fn dispatch(
        &self,
        envelope: HookEnvelope,
        pinned: Arc<RegistrySnapshot>,
        cancel: Arc<AtomicBool>,
    ) -> BoxFuture<'static, DispatchOutcome> {
        let registry_revision = pinned.revision;
        let matching = pinned
            .bindings_for(
                envelope.boundary(),
                envelope.cwd.as_deref(),
                envelope.execution_host,
            )
            .into_iter()
            .filter(|active| active.adapter.matches(&active.binding, &envelope))
            .collect::<Vec<_>>();
        if matching.is_empty() || cancel.load(Ordering::Acquire) {
            return futures::future::ready(DispatchOutcome::no_opinion(registry_revision)).boxed();
        }

        let runner = self.runner.clone();
        let messenger = self.messenger.clone();
        async move {
            let mut ordered = Vec::with_capacity(matching.len());
            let mut invocations = Vec::with_capacity(matching.len());
            for active in matching {
                if cancel.load(Ordering::Acquire) {
                    break;
                }
                let (record, effects) =
                    run_binding(&*runner, &active, &envelope, cancel.clone()).await;
                invocations.push(record);
                ordered.push((active.binding.id.clone(), effects));
            }

            let effects = reduce_effects(ordered);
            route_notices(&*messenger, &envelope, &effects);
            DispatchOutcome {
                effects,
                invocations,
                registry_revision,
            }
        }
        .boxed()
    }
}

async fn run_binding(
    runner: &dyn HookRunner,
    active: &ActiveBinding,
    envelope: &HookEnvelope,
    cancel: Arc<AtomicBool>,
) -> (InvocationRecord, HookEffects) {
    let mut per_invocation = envelope.clone();
    per_invocation.invocation_id = Uuid::new_v4();
    let input = active
        .adapter
        .encode_input(&active.binding, &per_invocation);
    let stdin = serde_json::to_vec(&input).unwrap_or_default();

    let outcome = runner
        .run(HookInvocation {
            binding: active.binding.id.clone(),
            command: active.binding.command.clone(),
            cwd: active.working_directory.clone(),
            env: Vec::new(),
            stdin,
            timeout: active.binding.timeout,
            cancel,
        })
        .await;

    let stderr_tail = truncate_to_boundary(&outcome.stderr_tail, STDERR_TAIL_LOG_LIMIT);
    log::info!(
        "hook invocation: event={} source={} binding={} exit={:?} duration_ms={} stderr_tail={:?}",
        envelope.boundary(),
        active.binding.id.source,
        active.binding.id,
        outcome.exit,
        outcome.duration.as_millis(),
        stderr_tail
    );

    let effects = active.adapter.decode_output(&active.binding, &outcome);
    let record = InvocationRecord {
        invocation_id: per_invocation.invocation_id,
        binding: active.binding.id.clone(),
        exit: outcome.exit,
        duration: outcome.duration,
        stderr_tail: stderr_tail.to_owned(),
    };
    (record, effects)
}

fn route_notices(messenger: &dyn HookMessenger, envelope: &HookEnvelope, effects: &MergedEffects) {
    for text in &effects.user_notices {
        messenger.notify(HookNotice {
            conversation_id: envelope.conversation_id,
            action_id: envelope.action_id.clone(),
            kind: HookNoticeKind::Info,
            text: text.clone(),
        });
    }
    for unsupported in &effects.unsupported {
        messenger.diagnostic(HookDiagnostic {
            source: unsupported.binding.source.clone(),
            severity: DiagnosticSeverity::Warning,
            message: format!(
                "{} produced an unsupported {:?} effect: {}",
                unsupported.binding, unsupported.effect, unsupported.reason
            ),
        });
    }
    for conflict in &effects.conflicts {
        log::warn!(
            "hook merge conflict: event={} {conflict}",
            envelope.boundary()
        );
    }
}

/// Cuts `text` to at most `limit` bytes without splitting a character.
fn truncate_to_boundary(text: &str, limit: usize) -> &str {
    if text.len() <= limit {
        return text;
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

#[cfg(test)]
#[path = "dispatch_tests.rs"]
mod tests;
