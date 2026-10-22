use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ai_types::AIConversationId;
use futures::executor::block_on;

use super::*;
use crate::agent::action::AIAgentActionType;
use crate::hooks::model::{
    CanonicalToolName, HookBoundary, HookPayload, HookSourceKind, HookVerdict,
};
use crate::hooks::notices::HookNoticeKind;
use crate::hooks::registry::BindingScope;
use crate::hooks::test_util::{
    RecordingAdapter, RecordingMessenger, ScriptedRunner, active_binding, test_binding,
    test_envelope, test_source_id, test_source_id_of_kind,
};

fn shell_envelope() -> HookEnvelope {
    let action = AIAgentActionType::RequestCommandOutput {
        command: "ls".to_owned(),
        is_read_only: None,
        is_risky: None,
        wait_until_completion: true,
        uses_pager: None,
        rationale: None,
        citations: Vec::new(),
    };
    let tool_name = CanonicalToolName::for_action(&action);
    test_envelope(
        AIConversationId::new(),
        HookPayload::ToolPre { action, tool_name },
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

#[test]
fn no_bindings_is_no_opinion_without_touching_the_runner() {
    let h = harness();
    let outcome = block_on(h.dispatcher.dispatch(
        shell_envelope(),
        snapshot(Vec::new()),
        Arc::new(AtomicBool::new(false)),
    ));
    assert_eq!(outcome.effects, MergedEffects::no_opinion());
    assert!(outcome.invocations.is_empty());
    assert_eq!(outcome.registry_revision, 7);
    assert_eq!(h.runner.invocation_count(), 0);
    assert_eq!(h.adapter.encoded_count(), 0);
}

#[test]
fn bindings_that_do_not_match_are_skipped() {
    let h = harness();
    let binding = test_binding(test_source_id("s"), HookBoundary::ToolPre, 0, Some("Read"));
    let outcome = block_on(h.dispatcher.dispatch(
        shell_envelope(),
        snapshot(vec![active_binding(
            binding,
            h.adapter.clone(),
            BindingScope::Everywhere,
        )]),
        Arc::new(AtomicBool::new(false)),
    ));
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
    let outcome = block_on(h.dispatcher.dispatch(
        shell_envelope(),
        snapshot(vec![active_binding(
            binding.clone(),
            h.adapter.clone(),
            BindingScope::Everywhere,
        )]),
        Arc::new(AtomicBool::new(false)),
    ));
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
        vec![DecodePolicy { strict: false }],
        "output is decoded under the vendor defaults until strict mode is wired"
    );
}

#[test]
fn two_bindings_run_in_priority_order_and_reduce() {
    let h = harness();
    let project = test_binding(
        test_source_id_of_kind("project", HookSourceKind::ProjectCursorFile),
        HookBoundary::ToolPre,
        0,
        None,
    );
    let factory = test_binding(
        test_source_id_of_kind("factory", HookSourceKind::FactoryAgent),
        HookBoundary::ToolPre,
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
    let outcome = block_on(h.dispatcher.dispatch(
        shell_envelope(),
        snapshot(vec![
            active_binding(project.clone(), h.adapter.clone(), BindingScope::Everywhere),
            active_binding(factory.clone(), h.adapter.clone(), BindingScope::Everywhere),
        ]),
        Arc::new(AtomicBool::new(false)),
    ));

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
        cancel.clone(),
    ));
    assert_eq!(outcome.effects, MergedEffects::no_opinion());
    assert!(outcome.invocations.is_empty());
    assert_eq!(h.runner.invocation_count(), 0);
    assert!(cancel.load(Ordering::Acquire));
}

#[test]
fn defer_is_reported_to_the_messenger_as_unsupported() {
    let h = harness();
    let binding = test_binding(test_source_id("s"), HookBoundary::ToolPre, 0, None);
    h.runner.script(
        binding.id.clone(),
        ScriptedRunner::exit_with(0, r#"{"verdict":"defer"}"#),
    );
    let outcome = block_on(h.dispatcher.dispatch(
        shell_envelope(),
        snapshot(vec![active_binding(
            binding,
            h.adapter.clone(),
            BindingScope::Everywhere,
        )]),
        Arc::new(AtomicBool::new(false)),
    ));
    assert_eq!(outcome.effects.verdict, HookVerdict::NoOpinion);
    assert_eq!(h.messenger.diagnostics().len(), 1);
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
