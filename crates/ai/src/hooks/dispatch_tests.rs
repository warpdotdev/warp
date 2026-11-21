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
    HookAsker, HookBoundary, HookOnAsk, HookPayload, HookSourceKind, HookSurface, HookVerdict,
    RunMetadata,
};
use crate::hooks::notices::HookNoticeKind;
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
    let mut envelope = test_envelope(AIConversationId::new(), HookPayload::ToolPre { action });
    envelope.action_id = Some(AIAgentActionId::from("call_1".to_owned()));
    envelope
}

fn shell_result_envelope() -> HookEnvelope {
    let action = shell_action("ls");
    test_envelope(
        AIConversationId::new(),
        HookPayload::ToolResult {
            action,
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
        diagnostics: Vec::new(),
    })
}

/// `active` with its source marked strict.
fn strict(mut active: ActiveBinding) -> ActiveBinding {
    active.policy.strict = true;
    active
}

fn not_cancelled() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

fn dispatch(h: &Harness, envelope: HookEnvelope, bindings: Vec<ActiveBinding>) -> DispatchOutcome {
    dispatch_with_headlessness(h, envelope, bindings, false)
}

fn dispatch_with_headlessness(
    h: &Harness,
    envelope: HookEnvelope,
    bindings: Vec<ActiveBinding>,
    is_headless: bool,
) -> DispatchOutcome {
    block_on(
        h.dispatcher
            .dispatch(envelope, snapshot(bindings), is_headless, not_cancelled()),
    )
}

#[test]
fn no_bindings_is_no_opinion_without_touching_the_runner() {
    let h = harness();
    let outcome = dispatch(&h, shell_envelope(), Vec::new());
    assert_eq!(outcome.effects, MergedEffects::no_opinion());
    assert!(!outcome.cancelled, "nothing to run is a real no opinion");
    assert!(outcome.invocations.is_empty());
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
        vec![active_binding(binding, h.adapter.clone())],
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
        vec![active_binding(binding.clone(), h.adapter.clone())],
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
fn a_split_firing_runs_the_binding_once_per_part_and_merges_what_the_parts_said() {
    let h = harness();
    h.adapter
        .fan_out(vec!["src/a.rs", "src/secret.env", "src/b.rs"]);
    let binding = test_binding(test_source_id("s"), HookBoundary::ToolPre, 0, None);
    h.runner.script_part(
        binding.id.clone(),
        "src/secret.env",
        ScriptedRunner::deny_with("secrets stay out of the model"),
    );
    let outcome = dispatch(
        &h,
        shell_envelope(),
        vec![active_binding(binding.clone(), h.adapter.clone())],
    );

    assert_eq!(outcome.effects.verdict, HookVerdict::Deny);
    assert_eq!(outcome.effects.verdict_binding, Some(binding.id.clone()));
    assert_eq!(
        outcome.effects.reasons.len(),
        1,
        "the parts that had no opinion added no reason"
    );
    assert_eq!(
        outcome.effects.reasons[0].text, "src/secret.env: secrets stay out of the model",
        "the reason names the part it was about"
    );

    assert_eq!(outcome.invocations.len(), 3, "one process per part");
    assert_eq!(
        outcome
            .invocations
            .iter()
            .map(|record| record.label.as_deref())
            .collect::<Vec<_>>(),
        vec![Some("src/a.rs"), Some("src/secret.env"), Some("src/b.rs")],
        "records keep the adapter's order"
    );
    assert_eq!(
        outcome.invocations[0].invocation_id,
        h.adapter.encoded_envelopes()[0].invocation_id,
        "the adapter saw the first run's id"
    );
    assert_ne!(
        outcome.invocations[1].invocation_id,
        outcome.invocations[0].invocation_id
    );

    let invocations = h.runner.invocations();
    assert_eq!(invocations.len(), 3);
    let stdin: serde_json::Value =
        serde_json::from_slice(&invocations[1].stdin).expect("stdin is JSON");
    assert_eq!(stdin["part"], "src/secret.env");
    assert_eq!(invocations[1].label.as_deref(), Some("src/secret.env"));
    assert_eq!(
        h.adapter.encoded_count(),
        1,
        "the envelope is encoded once; the adapter splits it"
    );
}

#[test]
fn a_split_firing_whose_parts_all_allow_is_one_allow() {
    let h = harness();
    h.adapter.fan_out(vec!["a", "b"]);
    let binding = test_binding(test_source_id("s"), HookBoundary::ToolPre, 0, None);
    h.runner.script(
        binding.id.clone(),
        ScriptedRunner::exit_with(0, r#"{"verdict":"allow","notices":["checked"]}"#),
    );
    let outcome = dispatch(
        &h,
        shell_envelope(),
        vec![active_binding(binding, h.adapter.clone())],
    );
    assert_eq!(outcome.effects.verdict, HookVerdict::Allow);
    assert!(outcome.effects.reasons.is_empty());
    assert_eq!(
        outcome.effects.user_notices,
        vec!["checked", "checked"],
        "notices concatenate across the parts"
    );
    assert_eq!(outcome.invocations.len(), 2);
}

#[test]
fn a_strict_sources_setting_reaches_the_adapter() {
    let h = harness();
    let binding = test_binding(test_source_id("s"), HookBoundary::ToolPre, 0, None);
    block_on(h.dispatcher.dispatch(
        shell_envelope(),
        snapshot(vec![strict(active_binding(binding, h.adapter.clone()))]),
        false,
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
    let factory = test_binding(
        test_source_id_of_kind("factory", HookSourceKind::FactoryWide),
        HookBoundary::ToolResult,
        0,
        None,
    );
    let agent = test_binding(
        test_source_id_of_kind("agent", HookSourceKind::FactoryAgent),
        HookBoundary::ToolResult,
        0,
        None,
    );
    h.runner.script(
        factory.id.clone(),
        ScriptedRunner::exit_with(
            0,
            r#"{"verdict":"allow","notices":["from factory"],"continuation":"factory"}"#,
        ),
    );
    h.runner.script(
        agent.id.clone(),
        ScriptedRunner::exit_with(
            0,
            r#"{"verdict":"ask","notices":["from agent"],"continuation":"agent"}"#,
        ),
    );
    let outcome = dispatch(
        &h,
        shell_result_envelope(),
        vec![
            active_binding(factory.clone(), h.adapter.clone()),
            active_binding(agent.clone(), h.adapter.clone()),
        ],
    );

    assert_eq!(outcome.effects.verdict, HookVerdict::Ask);
    assert_eq!(outcome.effects.verdict_binding, Some(agent.id.clone()));
    assert_eq!(
        outcome.effects.user_notices,
        vec!["from agent", "from factory"]
    );
    assert_eq!(
        outcome
            .effects
            .continuation
            .as_ref()
            .map(|c| c.text.as_str()),
        Some("agent")
    );
    assert_eq!(outcome.effects.conflicts.len(), 1);
    let order = outcome
        .invocations
        .iter()
        .map(|record| record.binding.clone())
        .collect::<Vec<_>>();
    assert_eq!(order, vec![agent.id, factory.id]);

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
                    .map(|binding| active_binding(binding, adapter.clone()))
                    .collect(),
            ),
            false,
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
                .map(|binding| active_binding(binding, adapter.clone()))
                .collect(),
        ),
        false,
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

/// Every hook that asked is recorded with its own source's on-ask setting, whichever verdict
/// won the merge.
#[test]
fn every_hook_that_asked_is_recorded_with_its_sources_on_ask_setting() {
    let h = harness();
    let source = test_source_id("s");
    let bindings = (0..3)
        .map(|index| test_binding(source.clone(), HookBoundary::ToolPre, index, None))
        .collect::<Vec<_>>();
    h.runner.script(
        bindings[0].id.clone(),
        ScriptedRunner::exit_with(0, r#"{"verdict":"ask"}"#),
    );
    h.runner.script(
        bindings[1].id.clone(),
        ScriptedRunner::exit_with(0, r#"{"verdict":"allow"}"#),
    );
    h.runner.script(
        bindings[2].id.clone(),
        ScriptedRunner::exit_with(0, r#"{"verdict":"ask"}"#),
    );
    let mut fail_run = active_binding(bindings[2].clone(), h.adapter.clone());
    fail_run.policy.on_ask = HookOnAsk::FailRun;
    let outcome = dispatch(
        &h,
        shell_envelope(),
        vec![
            active_binding(bindings[0].clone(), h.adapter.clone()),
            active_binding(bindings[1].clone(), h.adapter.clone()),
            fail_run,
        ],
    );

    assert_eq!(outcome.effects.verdict, HookVerdict::Ask);
    assert_eq!(
        outcome.effects.askers,
        vec![
            HookAsker {
                binding: bindings[0].id.clone(),
                on_ask: HookOnAsk::Deny,
            },
            HookAsker {
                binding: bindings[2].id.clone(),
                on_ask: HookOnAsk::FailRun,
            },
        ]
    );
    assert!(outcome.effects.an_asker_fails_the_run());
}

#[test]
fn askers_that_only_deny_do_not_fail_the_run() {
    let h = harness();
    let binding = test_binding(test_source_id("s"), HookBoundary::ToolPre, 0, None);
    h.runner.script(
        binding.id.clone(),
        ScriptedRunner::exit_with(0, r#"{"verdict":"ask"}"#),
    );
    let outcome = dispatch(
        &h,
        shell_envelope(),
        vec![active_binding(binding, h.adapter.clone())],
    );
    assert_eq!(outcome.effects.askers.len(), 1);
    assert!(!outcome.effects.an_asker_fails_the_run());
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
        snapshot(vec![active_binding(binding, h.adapter.clone())]),
        false,
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
        vec![active_binding(binding.clone(), h.adapter.clone())],
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
        vec![active_binding(binding.clone(), h.adapter.clone())],
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
        vec![active_binding(binding, h.adapter.clone())],
    );
    assert_eq!(outcome.effects.verdict, HookVerdict::Deny);
    assert!(
        outcome.effects.reasons[0]
            .text
            .contains("continue the turn")
    );
}

#[test]
fn defer_denies_in_a_headless_process_before_a_tool_runs() {
    let h = harness();
    let binding = test_binding(test_source_id("s"), HookBoundary::ToolPre, 0, None);
    h.runner.script(
        binding.id.clone(),
        ScriptedRunner::exit_with(0, r#"{"verdict":"defer"}"#),
    );
    let mut envelope = shell_envelope();
    envelope.surface = HookSurface::OzRun;
    let outcome = dispatch_with_headlessness(
        &h,
        envelope,
        vec![active_binding(binding.clone(), h.adapter.clone())],
        true,
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
fn defer_is_ignored_with_a_diagnostic_for_an_interactive_local_child() {
    let h = harness();
    let binding = test_binding(test_source_id("s"), HookBoundary::ToolPre, 0, None);
    h.runner.script(
        binding.id.clone(),
        ScriptedRunner::exit_with(0, r#"{"verdict":"defer"}"#),
    );
    let mut envelope = shell_envelope();
    envelope.surface = HookSurface::LocalChild;
    let outcome = dispatch(
        &h,
        envelope,
        vec![active_binding(binding, h.adapter.clone())],
    );
    assert_eq!(outcome.effects.verdict, HookVerdict::NoOpinion);
    assert!(outcome.effects.reasons.is_empty());
    assert_eq!(h.messenger.diagnostics().len(), 1);
}

#[test]
fn defer_from_a_strict_source_denies_on_an_interactive_surface() {
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
        snapshot(vec![strict(active_binding(
            binding.clone(),
            h.adapter.clone(),
        ))]),
        false,
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
        vec![active_binding(binding, h.adapter.clone())],
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
            queued: Duration::ZERO,
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
                            let mut active = active_binding(binding, adapter.clone());
                            active.policy.strict = strict;
                            active
                        })
                        .collect(),
                ),
                false,
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
        assert!(
            outcome.effects.askers.is_empty(),
            "a strict hook that timed out denies; it did not ask"
        );
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

    /// Two sources time out on the same command. Only the strict one's hook denies; the other
    /// fails open as Cursor documents, whatever the strict source does.
    #[test]
    fn each_hook_is_judged_by_its_own_sources_strict_setting() {
        let lax = cursor_harness(FAIL_OPEN);
        let adapter: Arc<dyn HookFormatAdapter> = lax.adapter.clone();
        let strict_config = lax
            .adapter
            .parse(FAIL_OPEN.as_bytes(), &test_source_id("strict/hooks.json"))
            .expect("parses");
        let strict_binding = lax.adapter.bindings(&strict_config).remove(0);
        let lax_binding = lax.bindings[0].clone();
        lax.runner.script(lax_binding.id.clone(), timed_out());
        lax.runner.script(strict_binding.id.clone(), timed_out());

        let mut strict_active = active_binding(strict_binding.clone(), adapter.clone());
        strict_active.policy.strict = true;
        let outcome = block_on(lax.dispatcher.dispatch(
            shell_envelope(),
            snapshot(vec![
                active_binding(lax_binding.clone(), adapter),
                strict_active,
            ]),
            false,
            not_cancelled(),
        ));

        assert_eq!(outcome.effects.verdict, HookVerdict::Deny);
        assert_eq!(outcome.effects.verdict_binding, Some(strict_binding.id));
        assert_eq!(
            outcome.effects.reasons.len(),
            1,
            "{:?}",
            outcome.effects.reasons
        );
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
        vec![active_binding(binding, h.adapter.clone())],
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

/// A source inside a repository tells its hooks that repository is the project, wherever the
/// operation runs, and a run started on this machine is not marked remote.
#[test]
fn a_repository_source_names_its_root_as_the_project_on_a_local_run() {
    let h = harness();
    let binding = test_binding(test_source_id("s"), HookBoundary::ToolPre, 0, None);
    let mut envelope = shell_envelope();
    envelope.surface = HookSurface::CliRun;
    envelope.roots = vec![PathBuf::from("/work/other")];
    envelope.cwd = Some(PathBuf::from("/work/other/src"));
    let mut active = active_binding(binding, h.adapter.clone());
    active.project_dir = Some(PathBuf::from("/work/repo"));
    dispatch(&h, envelope, vec![active]);
    let env = h.runner.invocations()[0].env.clone();
    for name in [
        "WARP_PROJECT_DIR",
        "CURSOR_PROJECT_DIR",
        "CLAUDE_PROJECT_DIR",
    ] {
        assert!(
            env.contains(&(OsString::from(name), OsString::from("/work/repo"))),
            "{name}: {env:?}"
        );
    }
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

#[test]
fn bindings_are_distinct_by_source_file_and_position() {
    let source = test_source_id("s");
    let adapter: Arc<dyn HookFormatAdapter> = Arc::new(RecordingAdapter::new());
    let two_positions = vec![
        active_binding(
            test_binding(source.clone(), HookBoundary::ToolPre, 0, None),
            adapter.clone(),
        ),
        active_binding(
            test_binding(source.clone(), HookBoundary::ToolPre, 1, None),
            adapter.clone(),
        ),
    ];
    assert!(has_distinct_sources(&two_positions));

    let same_twice = vec![
        active_binding(
            test_binding(source.clone(), HookBoundary::ToolPre, 0, None),
            adapter.clone(),
        ),
        active_binding(
            test_binding(source, HookBoundary::ToolPre, 0, None),
            adapter,
        ),
    ];
    assert!(!has_distinct_sources(&same_twice));
}
