use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use ai_types::{AIAgentActionId, AIConversationId};
use futures::executor::block_on;
use instant::Instant;
use warpui_core::r#async::Timer;

use super::*;
use crate::agent::action::AIAgentActionType;
use crate::agent::action_result::{AIAgentActionResultType, RequestCommandOutputResult};
use crate::hooks::adapters::HookFormatAdapter;
use crate::hooks::adapters::cursor::CursorAdapter;
use crate::hooks::model::{
    CanonicalToolName, HookBoundary, HookPayload, HookSourceKind, HookSurface, HookVerdict,
    RunMetadata,
};
use crate::hooks::notices::HookNoticeKind;
use crate::hooks::registry::BindingScope;
use crate::hooks::test_util::{
    LatencyRunner, RecordingAdapter, RecordingMessenger, ScriptedRunner, active_binding,
    test_binding, test_envelope, test_source_id, test_source_id_of_kind,
};

fn shell_action(command: &str) -> AIAgentActionType {
    AIAgentActionType::RequestCommandOutput {
        command: command.to_owned(),
        is_read_only: None,
        is_risky: None,
        wait_until_completion: true,
        uses_pager: None,
        rationale: None,
        citations: Vec::new(),
    }
}

fn shell_envelope() -> HookEnvelope {
    let action = shell_action("ls");
    let tool_name = CanonicalToolName::for_action(&action);
    let mut envelope = test_envelope(
        AIConversationId::new(),
        HookPayload::ToolPre { action, tool_name },
    );
    envelope.action_id = Some(AIAgentActionId::from("call_1".to_owned()));
    envelope
}

fn shell_result_envelope() -> HookEnvelope {
    let action = shell_action("ls");
    let tool_name = CanonicalToolName::for_action(&action);
    test_envelope(
        AIConversationId::new(),
        HookPayload::ToolResult {
            action,
            tool_name,
            result: AIAgentActionResultType::RequestCommandOutput(
                RequestCommandOutputResult::CancelledBeforeExecution,
            ),
            duration: None,
        },
    )
}

struct Harness {
    adapter: Arc<RecordingAdapter>,
    runner: Arc<ScriptedRunner>,
    messenger: Arc<RecordingMessenger>,
    dispatcher: HookDispatcher,
}

fn harness() -> Harness {
    let adapter = Arc::new(RecordingAdapter::new());
    let runner = Arc::new(ScriptedRunner::new());
    let messenger = Arc::new(RecordingMessenger::new());
    let dispatcher = HookDispatcher::new(runner.clone(), messenger.clone());
    Harness {
        adapter,
        runner,
        messenger,
        dispatcher,
    }
}

fn snapshot(bindings: Vec<ActiveBinding>) -> Arc<RegistrySnapshot> {
    Arc::new(RegistrySnapshot {
        revision: 7,
        bindings,
        ..Default::default()
    })
}

fn not_cancelled() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

fn dispatch(h: &Harness, envelope: HookEnvelope, bindings: Vec<ActiveBinding>) -> DispatchOutcome {
    block_on(h.dispatcher.dispatch(
        envelope,
        snapshot(bindings),
        DispatchPolicy::default(),
        not_cancelled(),
    ))
}

#[test]
fn no_bindings_is_no_opinion_without_touching_the_runner() {
    let h = harness();
    let outcome = dispatch(&h, shell_envelope(), Vec::new());
    assert_eq!(outcome.effects, MergedEffects::no_opinion());
    assert!(!outcome.cancelled, "nothing to run is a real no opinion");
    assert!(outcome.invocations.is_empty());
    assert_eq!(outcome.registry_revision, 7);
    assert_eq!(h.runner.invocation_count(), 0);
    assert_eq!(h.adapter.encoded_count(), 0);
}

#[test]
fn bindings_that_do_not_match_are_skipped() {
    let h = harness();
    let binding = test_binding(test_source_id("s"), HookBoundary::ToolPre, 0, Some("Read"));
    let outcome = dispatch(
        &h,
        shell_envelope(),
        vec![active_binding(
            binding,
            h.adapter.clone(),
            BindingScope::Everywhere,
        )],
    );
    assert_eq!(outcome.effects.verdict, HookVerdict::NoOpinion);
    assert_eq!(h.runner.invocation_count(), 0);
}

#[test]
fn one_deny_binding_denies_with_its_reason() {
    let h = harness();
    let binding = test_binding(test_source_id("s"), HookBoundary::ToolPre, 0, Some("Shell"));
    h.runner.script(
        binding.id.clone(),
        ScriptedRunner::deny_with("no listing allowed"),
    );
    let outcome = dispatch(
        &h,
        shell_envelope(),
        vec![active_binding(
            binding.clone(),
            h.adapter.clone(),
            BindingScope::Everywhere,
        )],
    );
    assert_eq!(outcome.effects.verdict, HookVerdict::Deny);
    assert_eq!(outcome.effects.reasons.len(), 1);
    assert_eq!(outcome.effects.reasons[0].text, "no listing allowed");
    assert_eq!(outcome.effects.reasons[0].binding, Some(binding.id.clone()));
    assert_eq!(outcome.invocations.len(), 1);
    assert_eq!(outcome.invocations[0].binding, binding.id);
    assert_eq!(outcome.invocations[0].exit, ProcessExit::Code(2));

    let invocations = h.runner.invocations();
    assert_eq!(invocations.len(), 1);
    assert_eq!(invocations[0].command, "test-hook-0");
    assert_eq!(invocations[0].cwd, PathBuf::from("/"));
    let stdin: serde_json::Value =
        serde_json::from_slice(&invocations[0].stdin).expect("stdin is JSON");
    assert_eq!(stdin["tool_name"], "Shell");
    assert_eq!(stdin["boundary"], "tool.pre");

    let encoded = h.adapter.encoded_envelopes();
    assert_eq!(encoded.len(), 1);
    assert_ne!(encoded[0].invocation_id, uuid::Uuid::nil());
    assert_eq!(
        encoded[0].invocation_id,
        outcome.invocations[0].invocation_id
    );
    assert_eq!(
        h.adapter.decode_policies(),
        vec![DecodePolicy { strict: false }]
    );
}

#[test]
fn the_strict_policy_reaches_the_adapter() {
    let h = harness();
    let binding = test_binding(test_source_id("s"), HookBoundary::ToolPre, 0, None);
    block_on(h.dispatcher.dispatch(
        shell_envelope(),
        snapshot(vec![active_binding(
            binding,
            h.adapter.clone(),
            BindingScope::Everywhere,
        )]),
        DispatchPolicy { strict: true },
        not_cancelled(),
    ));
    assert_eq!(
        h.adapter.decode_policies(),
        vec![DecodePolicy { strict: true }]
    );
}

#[test]
fn two_bindings_reduce_in_priority_order() {
    let h = harness();
    let project = test_binding(
        test_source_id_of_kind("project", HookSourceKind::ProjectCursorFile),
        HookBoundary::ToolResult,
        0,
        None,
    );
    let factory = test_binding(
        test_source_id_of_kind("factory", HookSourceKind::FactoryAgent),
        HookBoundary::ToolResult,
        0,
        None,
    );
    h.runner.script(
        project.id.clone(),
        ScriptedRunner::exit_with(
            0,
            r#"{"verdict":"allow","notices":["from project"],"continuation":"project"}"#,
        ),
    );
    h.runner.script(
        factory.id.clone(),
        ScriptedRunner::exit_with(
            0,
            r#"{"verdict":"ask","notices":["from factory"],"continuation":"factory"}"#,
        ),
    );
    let outcome = dispatch(
        &h,
        shell_result_envelope(),
        vec![
            active_binding(project.clone(), h.adapter.clone(), BindingScope::Everywhere),
            active_binding(factory.clone(), h.adapter.clone(), BindingScope::Everywhere),
        ],
    );

    assert_eq!(outcome.effects.verdict, HookVerdict::Ask);
    assert_eq!(outcome.effects.verdict_binding, Some(factory.id.clone()));
    assert_eq!(
        outcome.effects.user_notices,
        vec!["from factory", "from project"]
    );
    assert_eq!(
        outcome
            .effects
            .continuation
            .as_ref()
            .map(|c| c.text.as_str()),
        Some("factory")
    );
    assert_eq!(outcome.effects.conflicts.len(), 1);
    let order = outcome
        .invocations
        .iter()
        .map(|record| record.binding.clone())
        .collect::<Vec<_>>();
    assert_eq!(order, vec![factory.id, project.id]);

    let notices = h.messenger.notices();
    assert_eq!(notices.len(), 2);
    assert!(notices.iter().all(|n| n.kind == HookNoticeKind::Info));
}

/// Three bindings with latencies of 300, 100, and 200 ms finish in about 300 ms, not 600, and
/// their results are merged in snapshot order even though the slowest came first in the
/// snapshot and finished last.
#[test]
fn matching_bindings_run_at_the_same_time_and_merge_in_snapshot_order() {
    let adapter = Arc::new(RecordingAdapter::new());
    let runner = Arc::new(LatencyRunner::new());
    let messenger = Arc::new(RecordingMessenger::new());
    let dispatcher = HookDispatcher::new(runner.clone(), messenger.clone());
    let source = test_source_id("s");
    let bindings = (0..3)
        .map(|index| test_binding(source.clone(), HookBoundary::ToolResult, index, None))
        .collect::<Vec<_>>();
    for (binding, (latency, name)) in
        bindings
            .iter()
            .zip([(300, "slow"), (100, "fast"), (200, "middle")])
    {
        runner.script(
            binding.id.clone(),
            Duration::from_millis(latency),
            ScriptedRunner::exit_with(
                0,
                &format!(r#"{{"notices":["{name}"],"continuation":"{name}"}}"#),
            ),
        );
    }

    let started = Instant::now();
    let outcome = block_on(
        dispatcher.dispatch(
            shell_result_envelope(),
            snapshot(
                bindings
                    .iter()
                    .cloned()
                    .map(|binding| {
                        active_binding(binding, adapter.clone(), BindingScope::Everywhere)
                    })
                    .collect(),
            ),
            DispatchPolicy::default(),
            not_cancelled(),
        ),
    );
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_millis(500),
        "the bindings ran one after another: {elapsed:?}"
    );

    let finished = runner
        .finished()
        .into_iter()
        .map(|(binding, _)| binding.index)
        .collect::<Vec<_>>();
    assert_eq!(finished, vec![1, 2, 0], "completion order is by latency");
    assert_eq!(
        outcome.effects.user_notices,
        vec!["slow", "fast", "middle"],
        "merge order is snapshot order"
    );
    assert_eq!(
        outcome
            .effects
            .continuation
            .as_ref()
            .map(|c| c.text.as_str()),
        Some("slow"),
        "the first binding in snapshot order wins the scalar even though it finished last"
    );
    let record_order = outcome
        .invocations
        .iter()
        .map(|record| record.binding.index)
        .collect::<Vec<_>>();
    assert_eq!(record_order, vec![0, 1, 2]);
}

/// A firing cancelled while two of three invocations are still running: the runner sees the
/// flag, and nothing that ran is merged or routed.
#[test]
fn cancelling_a_running_dispatch_discards_the_partial_result() {
    let adapter = Arc::new(RecordingAdapter::new());
    let runner = Arc::new(LatencyRunner::new());
    let messenger = Arc::new(RecordingMessenger::new());
    let dispatcher = HookDispatcher::new(runner.clone(), messenger.clone());
    let source = test_source_id("s");
    let bindings = (0..3)
        .map(|index| test_binding(source.clone(), HookBoundary::ToolPre, index, None))
        .collect::<Vec<_>>();
    runner.script(
        bindings[0].id.clone(),
        Duration::from_millis(20),
        ScriptedRunner::exit_with(0, r#"{"verdict":"deny","notices":["already done"]}"#),
    );
    for binding in &bindings[1..] {
        runner.script(
            binding.id.clone(),
            Duration::from_secs(5),
            ScriptedRunner::exit_with(0, r#"{"verdict":"allow"}"#),
        );
    }

    let cancel = not_cancelled();
    let dispatch = dispatcher.dispatch(
        shell_envelope(),
        snapshot(
            bindings
                .iter()
                .cloned()
                .map(|binding| active_binding(binding, adapter.clone(), BindingScope::Everywhere))
                .collect(),
        ),
        DispatchPolicy::default(),
        cancel.clone(),
    );
    let trigger = cancel.clone();
    let started = Instant::now();
    let (outcome, ()) = block_on(futures::future::join(dispatch, async move {
        Timer::after(Duration::from_millis(100)).await;
        trigger.store(true, Ordering::Release);
    }));
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "the running invocations stopped soon after the flag was set"
    );

    assert_eq!(outcome.effects, MergedEffects::no_opinion());
    assert!(
        outcome.cancelled,
        "a cancelled firing says so, so no caller reads it as permission"
    );
    assert_eq!(
        outcome.invocations.len(),
        3,
        "every invocation is on record"
    );
    let exits = runner
        .finished()
        .into_iter()
        .map(|(binding, exit)| (binding.index, exit))
        .collect::<Vec<_>>();
    assert_eq!(exits[0], (0, ProcessExit::Code(0)));
    assert!(
        exits[1..]
            .iter()
            .all(|(_, exit)| *exit == ProcessExit::Cancelled)
    );
    assert!(messenger.notices().is_empty(), "nothing partial was routed");
}

#[test]
fn a_cancel_flag_set_before_dispatch_runs_nothing() {
    let h = harness();
    let binding = test_binding(test_source_id("s"), HookBoundary::ToolPre, 0, None);
    h.runner
        .script(binding.id.clone(), ScriptedRunner::deny_with("denied"));
    let cancel = Arc::new(AtomicBool::new(true));
    let outcome = block_on(h.dispatcher.dispatch(
        shell_envelope(),
        snapshot(vec![active_binding(
            binding,
            h.adapter.clone(),
            BindingScope::Everywhere,
        )]),
        DispatchPolicy::default(),
        cancel.clone(),
    ));
    assert_eq!(outcome.effects, MergedEffects::no_opinion());
    assert!(outcome.cancelled);
    assert!(outcome.invocations.is_empty());
    assert_eq!(h.runner.invocation_count(), 0);
    assert!(cancel.load(Ordering::Acquire));
}

#[test]
fn a_binding_that_asks_to_rewrite_a_pending_action_denies_it() {
    let h = harness();
    let binding = test_binding(test_source_id("s"), HookBoundary::ToolPre, 0, None);
    h.runner.script(
        binding.id.clone(),
        ScriptedRunner::exit_with(
            0,
            r#"{"verdict":"allow","replacement_input":{"command":"ls -la"}}"#,
        ),
    );
    let outcome = dispatch(
        &h,
        shell_envelope(),
        vec![active_binding(
            binding.clone(),
            h.adapter.clone(),
            BindingScope::Everywhere,
        )],
    );
    assert_eq!(outcome.effects.verdict, HookVerdict::Deny);
    assert_eq!(outcome.effects.verdict_binding, Some(binding.id.clone()));
    let reason = &outcome.effects.reasons[0];
    assert_eq!(reason.binding, Some(binding.id.clone()));
    assert!(
        reason.text.contains(&binding.id.to_string()) && reason.text.contains("rewrite the input"),
        "{}",
        reason.text
    );
    assert!(reason.text.contains("not supported"), "{}", reason.text);
    assert!(
        outcome.effects.replacement_input.is_some(),
        "the request is kept for the record"
    );
    let diagnostics = h.messenger.diagnostics();
    assert_eq!(diagnostics.len(), 1);
    assert!(diagnostics[0].message.contains("ReplacementInput"));
}

#[test]
fn a_binding_that_asks_to_rewrite_after_the_fact_is_only_reported() {
    let h = harness();
    let binding = test_binding(test_source_id("s"), HookBoundary::ToolResult, 0, None);
    h.runner.script(
        binding.id.clone(),
        ScriptedRunner::exit_with(
            0,
            r#"{"verdict":"allow","replacement_input":{"command":"ls -la"}}"#,
        ),
    );
    let outcome = dispatch(
        &h,
        shell_result_envelope(),
        vec![active_binding(
            binding.clone(),
            h.adapter.clone(),
            BindingScope::Everywhere,
        )],
    );
    assert_eq!(outcome.effects.verdict, HookVerdict::Allow);
    assert!(outcome.effects.reasons.is_empty());
    let diagnostics = h.messenger.diagnostics();
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].source, binding.id.source);
    assert!(
        diagnostics[0].message.contains("rewrite the input"),
        "{}",
        diagnostics[0].message
    );
}

#[test]
fn a_continuation_request_before_a_tool_runs_denies_it() {
    let h = harness();
    let binding = test_binding(test_source_id("s"), HookBoundary::ToolPre, 0, None);
    h.runner.script(
        binding.id.clone(),
        ScriptedRunner::exit_with(0, r#"{"verdict":"allow","continuation":"keep going"}"#),
    );
    let outcome = dispatch(
        &h,
        shell_envelope(),
        vec![active_binding(
            binding,
            h.adapter.clone(),
            BindingScope::Everywhere,
        )],
    );
    assert_eq!(outcome.effects.verdict, HookVerdict::Deny);
    assert!(
        outcome.effects.reasons[0]
            .text
            .contains("continue the turn")
    );
}

#[test]
fn defer_denies_on_a_headless_surface_before_a_tool_runs() {
    let h = harness();
    let binding = test_binding(test_source_id("s"), HookBoundary::ToolPre, 0, None);
    h.runner.script(
        binding.id.clone(),
        ScriptedRunner::exit_with(0, r#"{"verdict":"defer"}"#),
    );
    let mut envelope = shell_envelope();
    envelope.surface = HookSurface::OzRun;
    let outcome = dispatch(
        &h,
        envelope,
        vec![active_binding(
            binding.clone(),
            h.adapter.clone(),
            BindingScope::Everywhere,
        )],
    );
    assert_eq!(outcome.effects.verdict, HookVerdict::Deny);
    assert_eq!(outcome.effects.verdict_binding, Some(binding.id));
    assert!(
        outcome.effects.reasons[0]
            .text
            .contains("defer the action for later")
    );
    assert_eq!(h.messenger.diagnostics().len(), 1);
}

#[test]
fn defer_is_ignored_with_a_diagnostic_on_an_interactive_surface() {
    let h = harness();
    let binding = test_binding(test_source_id("s"), HookBoundary::ToolPre, 0, None);
    h.runner.script(
        binding.id.clone(),
        ScriptedRunner::exit_with(0, r#"{"verdict":"defer"}"#),
    );
    let mut envelope = shell_envelope();
    envelope.surface = HookSurface::Gui;
    let outcome = dispatch(
        &h,
        envelope,
        vec![active_binding(
            binding,
            h.adapter.clone(),
            BindingScope::Everywhere,
        )],
    );
    assert_eq!(outcome.effects.verdict, HookVerdict::NoOpinion);
    assert!(outcome.effects.reasons.is_empty());
    assert_eq!(h.messenger.diagnostics().len(), 1);
}

#[test]
fn defer_denies_under_the_strict_policy_on_an_interactive_surface() {
    let h = harness();
    let binding = test_binding(test_source_id("s"), HookBoundary::ToolPre, 0, None);
    h.runner.script(
        binding.id.clone(),
        ScriptedRunner::exit_with(0, r#"{"verdict":"defer"}"#),
    );
    let mut envelope = shell_envelope();
    envelope.surface = HookSurface::Gui;
    let outcome = block_on(h.dispatcher.dispatch(
        envelope,
        snapshot(vec![active_binding(
            binding.clone(),
            h.adapter.clone(),
            BindingScope::Everywhere,
        )]),
        DispatchPolicy { strict: true },
        not_cancelled(),
    ));
    assert_eq!(outcome.effects.verdict, HookVerdict::Deny);
    assert_eq!(outcome.effects.verdict_binding, Some(binding.id));
    assert!(
        outcome.effects.reasons[0]
            .text
            .contains("defer the action for later")
    );
}

#[test]
fn defer_after_the_fact_is_only_reported_even_when_headless() {
    let h = harness();
    let binding = test_binding(test_source_id("s"), HookBoundary::ToolResult, 0, None);
    h.runner.script(
        binding.id.clone(),
        ScriptedRunner::exit_with(0, r#"{"verdict":"defer"}"#),
    );
    let mut envelope = shell_result_envelope();
    envelope.surface = HookSurface::OzRun;
    let outcome = dispatch(
        &h,
        envelope,
        vec![active_binding(
            binding,
            h.adapter.clone(),
            BindingScope::Everywhere,
        )],
    );
    assert_eq!(outcome.effects.verdict, HookVerdict::NoOpinion);
    assert_eq!(h.messenger.diagnostics().len(), 1);
}

/// The per-binding failure policy and the strict hook policy, through the real Cursor adapter
/// with scripted process outcomes.
mod failure_policy {
    use super::*;

    struct CursorHarness {
        adapter: Arc<CursorAdapter>,
        runner: Arc<ScriptedRunner>,
        dispatcher: HookDispatcher,
        bindings: Vec<DeclaredBinding>,
    }

    fn cursor_harness(hooks_json: &str) -> CursorHarness {
        let adapter = Arc::new(CursorAdapter::new());
        let config = adapter
            .parse(hooks_json.as_bytes(), &test_source_id("hooks.json"))
            .expect("parses");
        let bindings = adapter.bindings(&config);
        let runner = Arc::new(ScriptedRunner::new());
        let dispatcher = HookDispatcher::new(runner.clone(), Arc::new(RecordingMessenger::new()));
        CursorHarness {
            adapter,
            runner,
            dispatcher,
            bindings,
        }
    }

    fn timed_out() -> crate::hooks::runner::HookProcessOutcome {
        crate::hooks::runner::HookProcessOutcome {
            exit: ProcessExit::TimedOut,
            stdout: Vec::new(),
            stdout_truncated: false,
            stderr_tail: String::new(),
            duration: Duration::from_secs(30),
        }
    }

    fn run(h: &CursorHarness, strict: bool) -> DispatchOutcome {
        let adapter: Arc<dyn HookFormatAdapter> = h.adapter.clone();
        block_on(
            h.dispatcher.dispatch(
                shell_envelope(),
                snapshot(
                    h.bindings
                        .iter()
                        .cloned()
                        .map(|binding| {
                            active_binding(binding, adapter.clone(), BindingScope::Everywhere)
                        })
                        .collect(),
                ),
                DispatchPolicy { strict },
                not_cancelled(),
            ),
        )
    }

    const FAIL_OPEN: &str =
        r#"{ "version": 1, "hooks": { "beforeShellExecution": [ { "command": "./a.sh" } ] } }"#;
    const FAIL_CLOSED: &str = r#"{ "version": 1, "hooks": { "beforeShellExecution": [ { "command": "./a.sh", "failClosed": true } ] } }"#;
    const PRE_TOOL_USE: &str =
        r#"{ "version": 1, "hooks": { "preToolUse": [ { "command": "./a.sh" } ] } }"#;

    #[test]
    fn a_fail_open_timeout_has_no_opinion_by_default() {
        let h = cursor_harness(FAIL_OPEN);
        h.runner.script(h.bindings[0].id.clone(), timed_out());
        let outcome = run(&h, false);
        assert_eq!(outcome.effects.verdict, HookVerdict::NoOpinion);
        assert_eq!(outcome.invocations[0].exit, ProcessExit::TimedOut);
    }

    #[test]
    fn a_fail_closed_timeout_denies() {
        let h = cursor_harness(FAIL_CLOSED);
        h.runner.script(h.bindings[0].id.clone(), timed_out());
        let outcome = run(&h, false);
        assert_eq!(outcome.effects.verdict, HookVerdict::Deny);
        assert!(outcome.effects.reasons[0].text.contains("timed out"));
    }

    #[test]
    fn malformed_output_on_a_permission_binding_denies_even_when_it_fails_open() {
        let h = cursor_harness(FAIL_OPEN);
        h.runner.script(
            h.bindings[0].id.clone(),
            ScriptedRunner::exit_with(0, "not json at all"),
        );
        let outcome = run(&h, false);
        assert_eq!(outcome.effects.verdict, HookVerdict::Deny);
    }

    #[test]
    fn strict_mode_turns_a_fail_open_timeout_into_a_denial() {
        let h = cursor_harness(FAIL_OPEN);
        h.runner.script(h.bindings[0].id.clone(), timed_out());
        let outcome = run(&h, true);
        assert_eq!(outcome.effects.verdict, HookVerdict::Deny);
        assert!(outcome.effects.reasons[0].text.contains("timed out"));
    }

    #[test]
    fn strict_mode_honors_ask_on_pre_tool_use_which_is_ignored_by_default() {
        let h = cursor_harness(PRE_TOOL_USE);
        h.runner.script(
            h.bindings[0].id.clone(),
            ScriptedRunner::exit_with(0, r#"{"permission":"ask"}"#),
        );
        assert_eq!(run(&h, false).effects.verdict, HookVerdict::NoOpinion);
        assert_eq!(run(&h, true).effects.verdict, HookVerdict::Ask);
    }

    #[test]
    fn strict_mode_never_weakens_a_denial() {
        let h = cursor_harness(FAIL_OPEN);
        h.runner
            .script(h.bindings[0].id.clone(), ScriptedRunner::exit_with(2, ""));
        assert_eq!(run(&h, true).effects.verdict, HookVerdict::Deny);
    }

    /// A control field the event does not define is refused by naming the field's purpose, not
    /// by claiming what the hook asked for; and a value that asks for nothing (`continue: true`,
    /// which Claude-shaped output carries as a matter of course) is not refused at all.
    #[test]
    fn a_field_the_event_does_not_accept_is_refused_in_its_own_words() {
        let h = cursor_harness(FAIL_OPEN);
        h.runner.script(
            h.bindings[0].id.clone(),
            ScriptedRunner::exit_with(0, r#"{"permission":"allow","continue":true}"#),
        );
        let outcome = run(&h, false);
        assert_eq!(outcome.effects.verdict, HookVerdict::Allow);
        assert!(outcome.effects.reasons.is_empty());
        assert!(outcome.effects.unsupported.is_empty());

        h.runner.script(
            h.bindings[0].id.clone(),
            ScriptedRunner::exit_with(0, r#"{"permission":"allow","continue":false}"#),
        );
        let outcome = run(&h, false);
        assert_eq!(outcome.effects.verdict, HookVerdict::Deny);
        let reason = &outcome.effects.reasons[0].text;
        assert!(
            reason.contains(
                "returned a field that controls whether the turn continues, which this event \
                 does not accept"
            ),
            "{reason}"
        );
        assert!(!reason.contains("stop the turn"), "{reason}");
        assert!(reason.ends_with("so the action was denied."), "{reason}");
    }
}

#[test]
fn the_invocation_carries_the_identity_variables_for_its_format() {
    let h = harness();
    let binding = test_binding(test_source_id("s"), HookBoundary::ToolPre, 0, None);
    let mut envelope = shell_envelope();
    envelope.surface = HookSurface::OzRun;
    envelope.roots = vec![PathBuf::from("/work/repo")];
    envelope.cwd = Some(PathBuf::from("/work/repo/src"));
    envelope.warp_version = "v1.2.3".to_owned();
    envelope.user_email = Some("dev@example.com".to_owned());
    envelope.run = Some(RunMetadata {
        run_id: "run-9".to_owned(),
        parent_run_id: None,
        task_id: None,
    });
    dispatch(
        &h,
        envelope,
        vec![active_binding(
            binding,
            h.adapter.clone(),
            BindingScope::Everywhere,
        )],
    );
    let env = h.runner.invocations()[0].env.clone();
    let value = |name: &str| {
        env.iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
    };
    assert_eq!(
        value("WARP_PROJECT_DIR"),
        Some(OsString::from("/work/repo"))
    );
    assert_eq!(value("WARP_VERSION"), Some(OsString::from("v1.2.3")));
    assert_eq!(value("WARP_RUN_ID"), Some(OsString::from("run-9")));
    assert_eq!(
        value("WARP_USER_EMAIL"),
        Some(OsString::from("dev@example.com"))
    );
    assert_eq!(value("WARP_CODE_REMOTE"), Some(OsString::from("true")));
    assert_eq!(
        value("CURSOR_PROJECT_DIR"),
        Some(OsString::from("/work/repo"))
    );
    assert_eq!(
        value("CLAUDE_PROJECT_DIR"),
        Some(OsString::from("/work/repo"))
    );
}

#[test]
fn a_project_binding_is_told_its_own_root_and_a_local_run_is_not_remote() {
    let h = harness();
    let binding = test_binding(
        test_source_id_of_kind("project", HookSourceKind::ProjectCursorFile),
        HookBoundary::ToolPre,
        0,
        None,
    );
    let mut envelope = shell_envelope();
    envelope.surface = HookSurface::CliRun;
    envelope.cwd = Some(PathBuf::from("/work/repo/src"));
    envelope.roots = vec![PathBuf::from("/elsewhere")];
    let pinned = Arc::new(RegistrySnapshot {
        revision: 7,
        bindings: vec![active_binding(
            binding,
            h.adapter.clone(),
            BindingScope::Root(PathBuf::from("/work/repo")),
        )],
        project_roots: vec![PathBuf::from("/work/repo")],
        ..Default::default()
    });
    block_on(
        h.dispatcher
            .dispatch(envelope, pinned, DispatchPolicy::default(), not_cancelled()),
    );
    let env = h.runner.invocations()[0].env.clone();
    assert!(env.contains(&(
        OsString::from("WARP_PROJECT_DIR"),
        OsString::from("/work/repo")
    )));
    assert!(!env.iter().any(|(key, _)| key == "WARP_CODE_REMOTE"));
}

#[test]
fn stderr_tail_is_cut_on_a_character_boundary() {
    let text = "é".repeat(300);
    let cut = truncate_to_char_boundary(&text, STDERR_TAIL_LOG_LIMIT);
    assert!(cut.len() <= STDERR_TAIL_LOG_LIMIT);
    assert!(cut.chars().all(|c| c == 'é'));
    assert_eq!(truncate_to_char_boundary("short", 10), "short");
}

#[test]
fn a_binding_whose_input_cannot_be_encoded_follows_its_failure_policy() {
    let mut binding = test_binding(test_source_id("s"), HookBoundary::ToolPre, 0, None);
    binding.capabilities.failure = FailurePolicy::FailOpen;
    assert_eq!(
        effects_for_unencodable_input(&binding),
        HookEffects::default()
    );

    binding.capabilities.failure = FailurePolicy::FailClosed;
    let effects = effects_for_unencodable_input(&binding);
    assert_eq!(effects.verdict, HookVerdict::Deny);
    assert_eq!(
        effects.reasons,
        vec![HookReason {
            text: INPUT_NOT_ENCODABLE_DENY_REASON.to_owned(),
            binding: Some(binding.id),
        }]
    );
}

/// The registry runs a file reached under two kinds once; the dispatcher checks that promise.
#[cfg(feature = "local_fs")]
mod source_dedupe {
    use std::fs;

    use tempfile::TempDir;

    use super::*;
    use crate::hooks::model::{HookFormat, HookSourceId, HookSourceLocation};
    use crate::hooks::registry::{HookRegistry, SourceUpdate};
    use crate::hooks::trust::TrustDecision;

    #[test]
    fn a_file_delivered_with_the_run_and_found_as_a_project_file_runs_once() {
        let dir = TempDir::new().expect("tempdir");
        let root = dunce::canonicalize(dir.path()).expect("canonical root");
        let hooks_json = root.join(".cursor").join("hooks.json");
        fs::create_dir_all(hooks_json.parent().expect("parent")).expect("mkdir");
        let text =
            r#"{ "version": 1, "hooks": { "beforeShellExecution": [ { "command": "./a.sh" } ] } }"#;
        fs::write(&hooks_json, text).expect("write");

        let mut registry = HookRegistry::new();
        for (kind, project_root) in [
            (HookSourceKind::RunDelivered, None),
            (HookSourceKind::ProjectCursorFile, Some(root.clone())),
        ] {
            registry.upsert_source(SourceUpdate {
                source: HookSourceId {
                    kind,
                    location: HookSourceLocation::CanonicalPath(hooks_json.clone()),
                },
                format: HookFormat::Cursor,
                bytes: text.as_bytes(),
                project_root,
                trust: TrustDecision::Activate,
            });
        }
        let snapshot = registry.snapshot();
        assert_eq!(snapshot.bindings.len(), 1, "one source runs the file");

        let runner = Arc::new(ScriptedRunner::new());
        let dispatcher = HookDispatcher::new(runner.clone(), Arc::new(RecordingMessenger::new()));
        let mut envelope = shell_envelope();
        envelope.cwd = Some(root.join("src"));
        let outcome = block_on(dispatcher.dispatch(
            envelope,
            snapshot,
            DispatchPolicy::default(),
            not_cancelled(),
        ));
        assert_eq!(outcome.invocations.len(), 1);
        assert_eq!(runner.invocation_count(), 1);
        assert_eq!(
            outcome.invocations[0].binding.source.kind,
            HookSourceKind::RunDelivered
        );
    }

    #[test]
    fn distinct_sources_are_told_apart_by_file_and_position() {
        let source = test_source_id("s");
        let adapter: Arc<dyn HookFormatAdapter> = Arc::new(RecordingAdapter::new());
        let two_positions = vec![
            active_binding(
                test_binding(source.clone(), HookBoundary::ToolPre, 0, None),
                adapter.clone(),
                BindingScope::Everywhere,
            ),
            active_binding(
                test_binding(source.clone(), HookBoundary::ToolPre, 1, None),
                adapter.clone(),
                BindingScope::Everywhere,
            ),
        ];
        assert!(has_distinct_sources(&two_positions));

        let same_twice = vec![
            active_binding(
                test_binding(source.clone(), HookBoundary::ToolPre, 0, None),
                adapter.clone(),
                BindingScope::Everywhere,
            ),
            active_binding(
                test_binding(source, HookBoundary::ToolPre, 0, None),
                adapter,
                BindingScope::Everywhere,
            ),
        ];
        assert!(!has_distinct_sources(&same_twice));
    }
}
