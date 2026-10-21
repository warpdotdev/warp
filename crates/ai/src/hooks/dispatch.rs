use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use futures::FutureExt;
use futures::future::{BoxFuture, join_all};
use uuid::Uuid;

use crate::hooks::adapters::{DeclaredBinding, DecodePolicy};
use crate::hooks::model::{
    DiagnosticSeverity, EffectKind, FailurePolicy, HookBindingId, HookBoundary, HookDiagnostic,
    HookEffects, HookEnvelope, HookReason, HookSurface, HookVerdict, MergedEffects,
    UnsupportedEffect, reduce_effects,
};
use crate::hooks::notices::{HookMessenger, HookNotice, HookNoticeKind};
use crate::hooks::registry::{ActiveBinding, BindingScope, RegistrySnapshot};
use crate::hooks::runner::env::{HookIdentity, invocation_environment};
use crate::hooks::runner::{HookInvocation, HookRunner, ProcessExit};

/// Longest stderr excerpt kept on an invocation record.
const STDERR_TAIL_LOG_LIMIT: usize = 500;

/// Reason reported when a fail-closed hook could not be given its input.
pub const INPUT_NOT_ENCODABLE_DENY_REASON: &str =
    "The hook's input could not be prepared, so the hook did not run.";

/// Run-wide choices the caller makes for every dispatch.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DispatchPolicy {
    /// The strict hook policy: every evaluation failure of a hook that can block becomes a
    /// block, and schema-accepted `ask` values the vendor would ignore are honored.
    pub strict: bool,
}

/// What one hook process did, kept for logs and later diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvocationRecord {
    pub invocation_id: Uuid,
    pub binding: HookBindingId,
    pub exit: ProcessExit,
    pub duration: Duration,
    pub stderr_tail: String,
}

/// What one boundary firing produced.
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
/// Every binding that matches runs at the same time (the runner bounds how many processes are
/// alive at once) and each is held to its own timeout, so the dispatch ends when the slowest
/// invocation does. Results are merged in the pinned snapshot's order, never in the order the
/// processes finished. The adapter applies each binding's failure policy and the strict hook
/// policy to what the process did; the dispatcher then refuses any effect the boundary cannot
/// honor in this phase, by denying on a boundary that runs before an operation and by reporting
/// it everywhere else.
pub struct HookDispatcher {
    runner: Arc<dyn HookRunner>,
    messenger: Arc<dyn HookMessenger>,
}

impl HookDispatcher {
    pub fn new(runner: Arc<dyn HookRunner>, messenger: Arc<dyn HookMessenger>) -> Self {
        Self { runner, messenger }
    }

    pub fn runner(&self) -> &Arc<dyn HookRunner> {
        &self.runner
    }

    /// Runs every binding in `pinned` that matches `envelope`. `cancel` belongs to this firing:
    /// once set, no further binding starts, running ones are asked to stop, and whatever they
    /// produced is discarded rather than merged.
    pub fn dispatch(
        &self,
        envelope: HookEnvelope,
        pinned: Arc<RegistrySnapshot>,
        policy: DispatchPolicy,
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
        debug_assert!(
            has_distinct_sources(&matching),
            "the registry handed the dispatcher the same binding twice"
        );
        if matching.is_empty() || cancel.load(Ordering::Acquire) {
            return futures::future::ready(DispatchOutcome::no_opinion(registry_revision)).boxed();
        }

        let runner = self.runner.clone();
        let messenger = self.messenger.clone();
        async move {
            let started = Instant::now();
            let runs = matching.iter().map(|active| {
                run_binding(
                    &*runner,
                    &*messenger,
                    active,
                    &envelope,
                    policy,
                    cancel.clone(),
                )
            });
            // `join_all` yields results in the order the futures were given, so the reduce input
            // is in snapshot order whatever order the processes finished in.
            let finished = join_all(runs).await;

            let mut invocations = Vec::with_capacity(matching.len());
            let mut ordered = Vec::with_capacity(matching.len());
            for (active, (record, effects)) in matching.iter().zip(finished) {
                invocations.extend(record);
                ordered.push((
                    active.binding.id.clone(),
                    refuse_unhonorable_effects(
                        &active.binding,
                        envelope.boundary(),
                        envelope.surface,
                        effects,
                    ),
                ));
            }

            if cancel.load(Ordering::Acquire) {
                log::info!(
                    "hook dispatch cancelled: event={} action={} matched={} duration_ms={}",
                    envelope.boundary(),
                    action_label(&envelope),
                    matching.len(),
                    started.elapsed().as_millis()
                );
                return DispatchOutcome {
                    effects: MergedEffects::no_opinion(),
                    invocations,
                    registry_revision,
                };
            }

            let effects = reduce_effects(ordered);
            route_notices(&*messenger, &envelope, &effects);
            log::info!(
                "hook dispatch: event={} action={} matched={} verdict={:?} halt={} duration_ms={}",
                envelope.boundary(),
                action_label(&envelope),
                matching.len(),
                effects.verdict,
                effects.halt_turn,
                started.elapsed().as_millis()
            );
            DispatchOutcome {
                effects,
                invocations,
                registry_revision,
            }
        }
        .boxed()
    }
}

/// Whether no two bindings come from the same resolved source file at the same position. The
/// registry runs a file reached under two kinds or two roots once; this checks that promise
/// before any process starts.
fn has_distinct_sources(matching: &[ActiveBinding]) -> bool {
    let mut seen = HashSet::new();
    matching.iter().all(|active| {
        let id = &active.binding.id;
        seen.insert((&id.source.location, &id.event, id.index))
    })
}

fn action_label(envelope: &HookEnvelope) -> String {
    envelope
        .action_id
        .as_ref()
        .map_or_else(|| "none".to_owned(), ToString::to_string)
}

/// What a binding that never received its input contributes. A fail-closed binding denies,
/// since it could not evaluate the operation; a fail-open one has no opinion.
fn effects_for_unencodable_input(binding: &DeclaredBinding) -> HookEffects {
    match binding.capabilities.failure {
        FailurePolicy::FailClosed => HookEffects {
            verdict: HookVerdict::Deny,
            reasons: vec![HookReason {
                text: INPUT_NOT_ENCODABLE_DENY_REASON.to_owned(),
                binding: Some(binding.id.clone()),
            }],
            ..Default::default()
        },
        FailurePolicy::FailOpen => HookEffects::default(),
    }
}

/// Runs one binding and decodes what it did. A binding that finds the firing already cancelled
/// does not start; its effects are never merged, so they are left empty.
async fn run_binding(
    runner: &dyn HookRunner,
    messenger: &dyn HookMessenger,
    active: &ActiveBinding,
    envelope: &HookEnvelope,
    policy: DispatchPolicy,
    cancel: Arc<AtomicBool>,
) -> (Option<InvocationRecord>, HookEffects) {
    if cancel.load(Ordering::Acquire) {
        return (None, HookEffects::default());
    }
    let mut per_invocation = envelope.clone();
    per_invocation.invocation_id = Uuid::new_v4();
    let input = active
        .adapter
        .encode_input(&active.binding, &per_invocation);
    let stdin = match serde_json::to_vec(&input) {
        Ok(stdin) => stdin,
        Err(error) => {
            log::error!(
                "hook input could not be encoded: event={} binding={} error={error}",
                envelope.boundary(),
                active.binding.id
            );
            messenger.diagnostic(HookDiagnostic {
                source: active.binding.id.source.clone(),
                severity: DiagnosticSeverity::Error,
                message: format!(
                    "{} did not run: its input could not be encoded ({error})",
                    active.binding.id
                ),
            });
            return (None, effects_for_unencodable_input(&active.binding));
        }
    };

    let outcome = runner
        .run(HookInvocation {
            binding: active.binding.id.clone(),
            command: active.binding.command.clone(),
            cwd: active.working_directory.clone(),
            env: invocation_environment(&hook_identity(active, envelope), &[]),
            stdin,
            timeout: active.binding.timeout,
            cancel,
        })
        .await;

    let effects = active.adapter.decode_output(
        &active.binding,
        &outcome,
        &DecodePolicy {
            strict: policy.strict,
        },
    );
    let record = InvocationRecord {
        invocation_id: per_invocation.invocation_id,
        binding: active.binding.id.clone(),
        exit: outcome.exit,
        duration: outcome.duration,
        stderr_tail: truncate_to_char_boundary(&outcome.stderr_tail, STDERR_TAIL_LOG_LIMIT)
            .to_owned(),
    };
    (Some(record), effects)
}

/// Who the hook is, for the variables the runner sets on its process.
fn hook_identity(active: &ActiveBinding, envelope: &HookEnvelope) -> HookIdentity {
    let remote = match envelope.surface {
        HookSurface::OzRun => true,
        HookSurface::CliRun | HookSurface::LocalChild | HookSurface::Gui | HookSurface::Tui => {
            false
        }
    };
    HookIdentity {
        format: active.adapter.format(),
        project_dir: project_dir(active, envelope),
        warp_version: envelope.warp_version.clone(),
        run_id: envelope.run.as_ref().map(|run| run.run_id.clone()),
        user_email: envelope.user_email.clone(),
        remote,
    }
}

/// The project a hook is told it runs for: a project file's own root; for a source that applies
/// everywhere, the project the operation is in, else the working directory, else the directory
/// the hook runs in.
fn project_dir(active: &ActiveBinding, envelope: &HookEnvelope) -> PathBuf {
    match &active.scope {
        BindingScope::Root(root) => root.clone(),
        BindingScope::Everywhere => envelope
            .roots
            .first()
            .cloned()
            .or_else(|| envelope.cwd.clone())
            .unwrap_or_else(|| active.working_directory.clone()),
    }
}

/// How a boundary answers a hook that asked for an effect it cannot honor.
enum Refusal {
    /// The operation has not run yet, so it is denied.
    DenyAction,
    /// The prompt has not been sent yet, so it is held back.
    HaltPrompt,
    /// Nothing can be stopped any more; the effect is reported.
    ReportOnly,
}

fn refusal_for(boundary: HookBoundary) -> Refusal {
    match boundary {
        HookBoundary::ToolPre
        | HookBoundary::ToolPermissionRequest
        | HookBoundary::FileExposure
        | HookBoundary::SubagentStart => Refusal::DenyAction,
        HookBoundary::PromptSubmit => Refusal::HaltPrompt,
        HookBoundary::SessionStart
        | HookBoundary::SessionEnd
        | HookBoundary::ToolResult
        | HookBoundary::ToolFailure
        | HookBoundary::SubagentStop
        | HookBoundary::TurnStop
        | HookBoundary::TurnInterrupt
        | HookBoundary::CompactionPre
        | HookBoundary::CompactionPost
        | HookBoundary::ResponseComplete
        | HookBoundary::ReasoningComplete => Refusal::ReportOnly,
    }
}

/// What a hook asked for through an effect it may not produce here, in the words a refusal
/// reason uses. `None` for effects that only inform someone and never block anything.
fn control_effect_words(kind: EffectKind) -> Option<&'static str> {
    match kind {
        EffectKind::Verdict => Some("decide permission through a field this event does not accept"),
        EffectKind::ReplacementInput => Some("rewrite the input"),
        EffectKind::ReplacementResult => Some("replace the result"),
        EffectKind::HaltTurn => Some("stop the turn"),
        EffectKind::Continuation => Some("continue the turn"),
        EffectKind::ModelContext | EffectKind::UserNotice | EffectKind::SessionEnv => None,
    }
}

/// Applies the phase's limits to what one binding asked for. A control effect that cannot be
/// honored is never dropped silently: before an operation it blocks the operation, with a reason
/// that names the binding and the effect; afterwards it is reported. Model context, which this
/// phase cannot deliver, is observational and only logged.
fn refuse_unhonorable_effects(
    binding: &DeclaredBinding,
    boundary: HookBoundary,
    surface: HookSurface,
    mut effects: HookEffects,
) -> HookEffects {
    if !effects.model_context.is_empty() {
        log::info!(
            "hook {} returned model context, which cannot be delivered yet; it was dropped",
            binding.id
        );
    }
    let refusal = refusal_for(boundary);

    // Fields the adapter refused to apply because the event does not define them. They are
    // already in `unsupported`; here they only add words to the refusal reason.
    let mut asked = effects
        .unsupported
        .iter()
        .filter_map(|unsupported| control_effect_words(unsupported.effect))
        .collect::<Vec<_>>();

    // Channels the adapter did fill that this boundary cannot honor in this phase.
    let mut unhonorable: Vec<(EffectKind, &'static str)> = Vec::new();
    if effects.replacement_input.is_some() {
        unhonorable.push((EffectKind::ReplacementInput, "rewrite the input"));
    }
    if effects.replacement_result.is_some() {
        unhonorable.push((EffectKind::ReplacementResult, "replace the result"));
    }
    match refusal {
        Refusal::DenyAction => {
            if effects.halt_turn {
                unhonorable.push((EffectKind::HaltTurn, "stop the turn"));
            }
            if effects.continuation.is_some() {
                unhonorable.push((EffectKind::Continuation, "continue the turn"));
            }
        }
        Refusal::HaltPrompt => {
            if effects.continuation.is_some() {
                unhonorable.push((EffectKind::Continuation, "continue the turn"));
            }
        }
        Refusal::ReportOnly => {}
    }
    // `reduce_effects` reports a deferral it ignores, which is the vendor's "warn and ignore"
    // behavior for an interactive surface. On a headless surface nobody could resume the action
    // later, so before an operation the deferral is refused instead.
    if effects.verdict == HookVerdict::Defer
        && surface.is_headless()
        && !matches!(refusal, Refusal::ReportOnly)
    {
        unhonorable.push((EffectKind::Verdict, "defer the action for later"));
    }

    let reported = effects
        .unsupported
        .iter()
        .map(|unsupported| unsupported.effect)
        .collect::<HashSet<_>>();
    for (kind, words) in &unhonorable {
        if !asked.contains(words) {
            asked.push(words);
        }
        if !reported.contains(kind) {
            effects.unsupported.push(UnsupportedEffect {
                binding: binding.id.clone(),
                effect: *kind,
                reason: format!("the hook asked to {words}, which is not supported yet"),
            });
        }
    }

    if asked.is_empty() {
        return effects;
    }
    let asked = asked.join(" and ");
    match refusal {
        Refusal::DenyAction => {
            effects.verdict = HookVerdict::Deny;
            effects.reasons.push(HookReason {
                text: format!(
                    "Hook {} asked to {asked}, which is not supported here, so the action was \
                     denied.",
                    binding.id
                ),
                binding: Some(binding.id.clone()),
            });
        }
        Refusal::HaltPrompt => {
            effects.halt_turn = true;
            effects.reasons.push(HookReason {
                text: format!(
                    "Hook {} asked to {asked}, which is not supported here, so the prompt was \
                     not sent.",
                    binding.id
                ),
                binding: Some(binding.id.clone()),
            });
        }
        Refusal::ReportOnly => {}
    }
    effects
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
fn truncate_to_char_boundary(text: &str, limit: usize) -> &str {
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
