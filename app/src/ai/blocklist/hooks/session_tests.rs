use std::ffi::OsString;
use std::time::Duration;

use ai::hooks::test_util::{ScriptedRunner, test_binding, test_source_id};
use ai::hooks::{DeclaredBinding, HookBoundary, HookPayload};
use chrono::Local;
use instant::Instant;
use warpui::{App, SingletonEntity};

use super::*;
use crate::ai::agent::CancellationReason;
use crate::ai::blocklist::hooks::prompt;
use crate::ai::blocklist::hooks::testing::{
    ControllerHarness, controller_harness, exchange_count, hook_test_app,
    let_background_work_settle, settle,
};
use crate::ai::blocklist::{BlocklistAIHistoryModel, RequestInput, ResponseStreamId};
use crate::ai::llms::LLMId;
use crate::features::FeatureFlag;

fn session_binding() -> DeclaredBinding {
    test_binding(
        test_source_id("session"),
        HookBoundary::SessionStart,
        0,
        None,
    )
}

fn prompt_binding() -> DeclaredBinding {
    test_binding(
        test_source_id("prompt"),
        HookBoundary::PromptSubmit,
        0,
        None,
    )
}

fn session_start_count(harness: &ControllerHarness) -> usize {
    harness
        .hooks
        .adapter
        .encoded_envelopes()
        .iter()
        .filter(|envelope| envelope.boundary() == HookBoundary::SessionStart)
        .count()
}

#[test]
fn session_start_fires_once_before_a_new_conversations_first_request() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = controller_harness(&mut app);
        app.update(|ctx| harness.hooks.install_bindings(vec![session_binding()], ctx));

        let conversation_id = harness.send_prompt(&mut app, "hello");
        settle(&mut app, |app| exchange_count(app, conversation_id) == 1).await;
        assert_eq!(session_start_count(&harness), 1);
        let envelope = &harness.hooks.adapter.encoded_envelopes()[0];
        assert_eq!(envelope.conversation_id, conversation_id);
        assert!(matches!(
            envelope.payload,
            HookPayload::SessionStart {
                is_background_agent: false
            }
        ));

        // The conversation's later prompts do not start another session.
        harness.update_controller(&mut app, |controller, ctx| {
            controller.cancel_conversation_progress(
                conversation_id,
                CancellationReason::ManuallyCancelled,
                ctx,
            );
        });
        harness.send_prompt_in(&mut app, conversation_id, "again");
        settle(&mut app, |app| exchange_count(app, conversation_id) == 2).await;
        let_background_work_settle(&mut app).await;
        assert_eq!(session_start_count(&harness), 1);
    });
}

#[test]
fn the_first_request_waits_for_the_session_hooks_and_then_goes_out() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = controller_harness(&mut app);
        app.update(|ctx| harness.hooks.install_bindings(vec![session_binding()], ctx));
        let hold = harness.hooks.runner.hold();

        let conversation_id = harness.send_prompt(&mut app, "hello");
        settle(&mut app, |_| harness.hooks.runner.invocation_count() == 1).await;
        let_background_work_settle(&mut app).await;
        assert_eq!(exchange_count(&app, conversation_id), 0);

        hold.release();
        settle(&mut app, |app| exchange_count(app, conversation_id) == 1).await;
    });
}

#[test]
fn a_session_hook_that_fails_or_times_out_adds_nothing_and_does_not_block_further() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = controller_harness(&mut app);
        let failing = session_binding();
        harness
            .hooks
            .runner
            .script(failing.id.clone(), ScriptedRunner::exit_with(1, ""));
        app.update(|ctx| {
            harness
                .hooks
                .install_bindings(vec![failing, prompt_binding()], ctx)
        });

        let conversation_id = harness.send_prompt(&mut app, "hello");
        settle(&mut app, |app| exchange_count(app, conversation_id) == 1).await;
        let invocations = harness.hooks.runner.invocations();
        assert_eq!(invocations.len(), 2);
        assert!(
            !invocations[1]
                .env
                .iter()
                .any(|(name, _)| name == &OsString::from("HOOK_SESSION")),
            "a failed session.start sets nothing"
        );
        assert!(
            app.read(|ctx| HookRuntime::as_ref(ctx).session_env_for(conversation_id))
                .is_empty()
        );

        // A hook that never answers holds the first request only until its timeout.
        let mut slow = session_binding();
        slow.timeout = Duration::from_millis(100);
        harness.hooks.adapter.clear_encoded();
        app.update(|ctx| harness.hooks.install_bindings(vec![slow], ctx));
        let _hold = harness.hooks.runner.hold();
        let started = Instant::now();
        let slow_conversation = harness.send_prompt(&mut app, "hello again");
        settle(&mut app, |app| exchange_count(app, slow_conversation) == 1).await;
        assert!(started.elapsed() >= Duration::from_millis(100));
        assert_eq!(session_start_count(&harness), 1);
    });
}

#[test]
fn the_session_environment_reaches_every_later_hook_in_the_conversation() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = controller_harness(&mut app);
        let session = session_binding();
        harness.hooks.runner.script(
            session.id.clone(),
            ScriptedRunner::exit_with(0, r#"{"env": [["HOOK_SESSION", "yes"]]}"#),
        );
        app.update(|ctx| {
            harness
                .hooks
                .install_bindings(vec![session, prompt_binding()], ctx)
        });

        let conversation_id = harness.send_prompt(&mut app, "hello");
        settle(&mut app, |app| exchange_count(app, conversation_id) == 1).await;
        let invocations = harness.hooks.runner.invocations();
        assert_eq!(invocations.len(), 2);
        assert_eq!(
            invocations[0].binding.event, "session.start",
            "the session hook runs first"
        );
        assert!(
            invocations[1]
                .env
                .contains(&(OsString::from("HOOK_SESSION"), OsString::from("yes"))),
            "{:?}",
            invocations[1].env
        );
        assert_eq!(
            app.read(|ctx| HookRuntime::as_ref(ctx).session_env_for(conversation_id)),
            vec![("HOOK_SESSION".to_owned(), "yes".to_owned())]
        );
        let other = AIConversationId::new();
        assert!(app.read(|ctx| HookRuntime::as_ref(ctx).session_env_for(other).is_empty()));
    });
}

#[test]
fn a_conversation_with_exchanges_is_not_a_new_session() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = hook_test_app(&mut app);
        app.update(|ctx| harness.hooks.install_bindings(vec![session_binding()], ctx));
        // A restored conversation arrives with its exchanges; the same is true of a resumed one.
        app.update(|ctx| {
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                let task_id = history
                    .conversation(&harness.conversation_id)
                    .unwrap()
                    .get_root_task_id()
                    .clone();
                history
                    .update_conversation_for_new_request_input_for_test(
                        RequestInput {
                            conversation_id: harness.conversation_id,
                            input_messages: Vec::from([(task_id, vec![])]),
                            working_directory: None,
                            model_id: LLMId::from("test-model"),
                            coding_model_id: LLMId::from("test-coding-model"),
                            cli_agent_model_id: LLMId::from("test-cli-agent-model"),
                            computer_use_model_id: LLMId::from("test-computer-use-model"),
                            shared_session_response_initiator: None,
                            request_start_ts: Local::now(),
                            supported_tools_override: None,
                        },
                        ResponseStreamId::new_for_test(),
                        harness.terminal_view_id,
                        ctx,
                    )
                    .unwrap();
            });
        });

        let wait = app.update(|ctx| {
            barrier(
                harness.conversation_id,
                harness.terminal_view_id,
                &harness.active_session,
                ctx,
            )
        });
        assert!(wait.is_none());
        let_background_work_settle(&mut app).await;
        assert_eq!(harness.hooks.adapter.encoded_count(), 0);
        assert_eq!(harness.hooks.runner.invocation_count(), 0);
    });
}

#[test]
fn a_second_request_while_the_hooks_run_waits_on_the_same_firing() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = hook_test_app(&mut app);
        app.update(|ctx| harness.hooks.install_bindings(vec![session_binding()], ctx));
        let hold = harness.hooks.runner.hold();

        let first = app
            .update(|ctx| {
                barrier(
                    harness.conversation_id,
                    harness.terminal_view_id,
                    &harness.active_session,
                    ctx,
                )
            })
            .expect("the first request waits");
        let second = app
            .update(|ctx| {
                barrier(
                    harness.conversation_id,
                    harness.terminal_view_id,
                    &harness.active_session,
                    ctx,
                )
            })
            .expect("the second request waits on the same hooks");
        settle(&mut app, |_| harness.hooks.runner.invocation_count() == 1).await;
        assert!(app.read(|ctx| {
            HookRuntime::as_ref(ctx)
                .session_starts
                .is_in_flight(harness.conversation_id)
        }));

        hold.release();
        first.await;
        second.await;
        settle(&mut app, |app| {
            !app.read(|ctx| {
                HookRuntime::as_ref(ctx)
                    .session_starts
                    .is_in_flight(harness.conversation_id)
            })
        })
        .await;
        assert_eq!(harness.hooks.runner.invocation_count(), 1);
        assert!(
            app.update(|ctx| barrier(
                harness.conversation_id,
                harness.terminal_view_id,
                &harness.active_session,
                ctx,
            ))
            .is_none(),
            "a conversation whose session started does not start it again"
        );
    });
}

#[test]
fn a_newer_prompt_replaces_one_waiting_only_on_the_session_barrier() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = controller_harness(&mut app);
        // No prompt hook: the first prompt waits on nothing but the session hooks.
        app.update(|ctx| harness.hooks.install_bindings(vec![session_binding()], ctx));
        let hold = harness.hooks.runner.hold();

        let conversation_id = harness.send_prompt(&mut app, "first");
        settle(&mut app, |_| harness.hooks.runner.invocation_count() == 1).await;
        assert!(app.read(|ctx| prompt::is_pending(conversation_id, ctx)));
        harness.send_prompt_in(&mut app, conversation_id, "second");

        hold.release();
        settle(&mut app, |app| exchange_count(app, conversation_id) == 1).await;
        let_background_work_settle(&mut app).await;
        assert_eq!(
            exchange_count(&app, conversation_id),
            1,
            "only the newer prompt went out"
        );
        let sent = BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
            history
                .conversation(&conversation_id)
                .and_then(|conversation| conversation.initial_query())
        });
        assert_eq!(sent.as_deref(), Some("second"));
        assert!(!app.read(|ctx| prompt::is_pending(conversation_id, ctx)));
    });
}

#[test]
fn cancelling_during_the_session_wait_drops_the_prompt() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = controller_harness(&mut app);
        app.update(|ctx| harness.hooks.install_bindings(vec![session_binding()], ctx));
        let hold = harness.hooks.runner.hold();

        let conversation_id = harness.send_prompt(&mut app, "hello");
        settle(&mut app, |_| harness.hooks.runner.invocation_count() == 1).await;
        harness.update_controller(&mut app, |controller, ctx| {
            controller.cancel_conversation_progress(
                conversation_id,
                CancellationReason::ManuallyCancelled,
                ctx,
            );
        });
        assert!(!app.read(|ctx| prompt::is_pending(conversation_id, ctx)));

        hold.release();
        let_background_work_settle(&mut app).await;
        assert_eq!(exchange_count(&app, conversation_id), 0);
    });
}

#[test]
fn viewer_and_transcript_conversations_do_not_fire_session_start() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = hook_test_app(&mut app);
        app.update(|ctx| harness.hooks.install_bindings(vec![session_binding()], ctx));
        let (viewer, transcript) = app.update(|ctx| {
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |model, ctx| {
                (
                    model.start_new_conversation(harness.terminal_view_id, false, true, false, ctx),
                    model.start_new_conversation(harness.terminal_view_id, false, false, true, ctx),
                )
            })
        });
        for conversation_id in [viewer, transcript] {
            let wait = app.update(|ctx| {
                barrier(
                    conversation_id,
                    harness.terminal_view_id,
                    &harness.active_session,
                    ctx,
                )
            });
            assert!(wait.is_none());
        }
        let_background_work_settle(&mut app).await;
        assert_eq!(harness.hooks.adapter.encoded_count(), 0);
    });
}
