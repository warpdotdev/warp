use std::time::Duration;

use ai::hooks::test_util::{ScriptedRunner, test_binding, test_source_id};
use ai::hooks::{DeclaredBinding, HookBoundary, HookPayload, HookTurnId, PromptProvenance};
use instant::Instant;
use warpui::{App, SingletonEntity};

use super::*;
use crate::ai::agent::conversation::ConversationStatus;
use crate::ai::agent::{AIAgentInput, CancellationReason, UserQueryMode};
use crate::ai::blocklist::hooks::prompt::is_pending;
use crate::ai::blocklist::hooks::testing::{
    HookTestApp, add_messages_event, controller_harness, conversation_status, done_event,
    exchange_count, finish_turn_with_text, has_unfinished_actions, hook_test_app,
    let_background_work_settle, record_statuses, root_task_id, settle, shell_tool_call_message,
    stream_init_event, text_message,
};
use crate::ai::blocklist::{
    BlocklistAIHistoryModel, QueuedQuery, QueuedQueryModel, QueuedQueryOrigin,
};
use crate::features::FeatureFlag;

fn stop_binding(loop_limit: Option<u32>) -> DeclaredBinding {
    let mut binding = test_binding(test_source_id("stop"), HookBoundary::TurnStop, 0, None);
    binding.loop_limit = loop_limit;
    binding
}

/// The scripted output that asks for a follow-up.
fn continuation(text: &str) -> ai::hooks::HookProcessOutcome {
    ScriptedRunner::exit_with(0, &format!(r#"{{"continuation": "{text}"}}"#))
}

/// Runs one held stop round by hand: hold, start, wait, resolve.
async fn stop_round(app: &mut App, harness: &HookTestApp) -> TurnStopResolution {
    let held = app.update(|ctx| hold_turn(harness.conversation_id, &harness.active_session, ctx));
    assert!(held, "a stop binding applies, so the turn is held");
    let stop = app
        .update(|ctx| start_turn_stop(harness.conversation_id, ctx))
        .expect("the held turn's hooks start");
    let outcome = stop.await;
    app.update(|ctx| resolve_turn_stop(harness.conversation_id, outcome, ctx))
}

fn queued_texts(app: &App, conversation_id: AIConversationId) -> Vec<(String, PromptProvenance)> {
    app.read(|ctx| {
        let queue = QueuedQueryModel::as_ref(ctx);
        queue
            .queue(conversation_id)
            .iter()
            .map(|row| {
                (
                    row.text().to_owned(),
                    queue.provenance_for(conversation_id, row.id()),
                )
            })
            .collect()
    })
}

/// The text and mode of the user query in the conversation's latest exchange.
fn latest_user_query(
    app: &App,
    conversation_id: AIConversationId,
) -> Option<(String, UserQueryMode)> {
    BlocklistAIHistoryModel::handle(app).read(app, |history, _| {
        history
            .conversation(&conversation_id)?
            .latest_exchange()?
            .input
            .iter()
            .find_map(|input| {
                if let AIAgentInput::UserQuery {
                    query,
                    user_query_mode,
                    ..
                } = input
                {
                    Some((query.clone(), *user_query_mode))
                } else {
                    None
                }
            })
    })
}

/// Starts a turn by hand for a conversation whose first request was mocked.
fn start_turn(app: &mut App, conversation_id: AIConversationId) -> HookTurnId {
    app.update(|ctx| {
        HookRuntime::handle(ctx).update(ctx, |runtime, _| runtime.start_turn(conversation_id))
    })
}

#[test]
fn a_follow_up_is_queued_behind_a_persons_prompt_with_the_hooks_provenance() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = hook_test_app(&mut app);
        let binding = stop_binding(None);
        harness
            .hooks
            .runner
            .script(binding.id.clone(), continuation("keep going"));
        app.update(|ctx| harness.hooks.install_bindings(vec![binding.clone()], ctx));
        app.update(|ctx| {
            QueuedQueryModel::handle(ctx).update(ctx, |queue, ctx| {
                queue.append(
                    harness.conversation_id,
                    QueuedQuery::new(
                        "the person's".to_owned(),
                        QueuedQueryOrigin::AutoQueueToggle,
                    ),
                    ctx,
                );
            });
        });

        let resolution = stop_round(&mut app, &harness).await;
        assert_eq!(resolution, TurnStopResolution::FollowUpQueued);

        let queued = queued_texts(&app, harness.conversation_id);
        assert_eq!(queued.len(), 2);
        assert_eq!(queued[0].0, "the person's");
        assert_eq!(queued[0].1, PromptProvenance::User);
        assert_eq!(queued[1].0, "keep going");
        let PromptProvenance::HookFollowUp {
            binding: from,
            chain_id,
        } = &queued[1].1
        else {
            panic!("the follow-up names the hook that asked for it");
        };
        assert_eq!(*from, binding.id);
        let envelope = &harness.hooks.adapter.encoded_envelopes()[0];
        assert!(matches!(
            envelope.payload,
            HookPayload::TurnStop {
                status: TurnStopStatus::Completed,
                continuation_count: 0
            }
        ));

        // The next stop in the same chain counts the first follow-up.
        let resolution = stop_round(&mut app, &harness).await;
        assert_eq!(resolution, TurnStopResolution::FollowUpQueued);
        let envelope = &harness.hooks.adapter.encoded_envelopes()[1];
        assert!(matches!(
            envelope.payload,
            HookPayload::TurnStop {
                continuation_count: 1,
                ..
            }
        ));
        let queued = queued_texts(&app, harness.conversation_id);
        assert!(matches!(
            &queued[2].1,
            PromptProvenance::HookFollowUp { chain_id: later, .. } if later == chain_id
        ));
    });
}

#[test]
fn a_bindings_loop_limit_ends_its_follow_ups_with_a_diagnostic() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = hook_test_app(&mut app);
        let binding = stop_binding(Some(2));
        harness
            .hooks
            .runner
            .script(binding.id.clone(), continuation("again"));
        app.update(|ctx| harness.hooks.install_bindings(vec![binding], ctx));

        assert_eq!(
            stop_round(&mut app, &harness).await,
            TurnStopResolution::FollowUpQueued
        );
        assert_eq!(
            stop_round(&mut app, &harness).await,
            TurnStopResolution::FollowUpQueued
        );
        assert_eq!(
            stop_round(&mut app, &harness).await,
            TurnStopResolution::Finalize,
            "the third follow-up is over the definition's loop_limit"
        );
        assert_eq!(queued_texts(&app, harness.conversation_id).len(), 2);
        let diagnostics = harness.hooks.messenger.diagnostics();
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("loop_limit of 2")),
            "{diagnostics:?}"
        );

        // A person's prompt starts a new chain, so the same binding may continue again.
        app.update(|ctx| {
            HookRuntime::handle(ctx).update(ctx, |runtime, _| {
                runtime.turn_stops.start_chain(harness.conversation_id);
            });
        });
        assert_eq!(
            stop_round(&mut app, &harness).await,
            TurnStopResolution::FollowUpQueued
        );
    });
}

#[test]
fn the_host_cap_applies_to_the_whole_chain_across_bindings() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = hook_test_app(&mut app);
        // Two definitions with no limit of their own. The dispatcher keeps the first binding's
        // continuation, so the two are alternated by turning one off at a time.
        let first = stop_binding(None);
        let mut second = test_binding(test_source_id("stop"), HookBoundary::TurnStop, 1, None);
        second.loop_limit = None;
        harness
            .hooks
            .runner
            .script(first.id.clone(), continuation("from the first"));
        harness
            .hooks
            .runner
            .script(second.id.clone(), continuation("from the second"));

        for round in 0..HOOK_FOLLOW_UP_CAP {
            let active = if round % 2 == 0 {
                first.clone()
            } else {
                second.clone()
            };
            app.update(|ctx| harness.hooks.install_bindings(vec![active], ctx));
            assert_eq!(
                stop_round(&mut app, &harness).await,
                TurnStopResolution::FollowUpQueued,
                "follow-up {round} is within the cap"
            );
        }
        app.update(|ctx| harness.hooks.install_bindings(vec![first], ctx));
        assert_eq!(
            stop_round(&mut app, &harness).await,
            TurnStopResolution::Finalize
        );
        assert_eq!(
            queued_texts(&app, harness.conversation_id).len(),
            HOOK_FOLLOW_UP_CAP as usize
        );
        let diagnostics = harness.hooks.messenger.diagnostics();
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("the most Warp allows (8)")),
            "{diagnostics:?}"
        );
        assert_eq!(
            app.read(|ctx| HookRuntime::as_ref(ctx)
                .turn_stops
                .chain_total(harness.conversation_id)),
            HOOK_FOLLOW_UP_CAP
        );
    });
}

#[test]
fn a_stop_hook_without_a_follow_up_finalizes_and_its_verdicts_are_ignored() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = hook_test_app(&mut app);
        let binding = stop_binding(None);
        harness
            .hooks
            .runner
            .script(binding.id.clone(), ScriptedRunner::deny_with("no more"));
        app.update(|ctx| harness.hooks.install_bindings(vec![binding], ctx));

        assert_eq!(
            stop_round(&mut app, &harness).await,
            TurnStopResolution::Finalize
        );
        assert!(queued_texts(&app, harness.conversation_id).is_empty());
        let diagnostics = harness.hooks.messenger.diagnostics();
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("can only ask for a follow-up")),
            "{diagnostics:?}"
        );
    });
}

#[test]
fn nothing_is_held_without_a_stop_binding() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = hook_test_app(&mut app);
        app.update(|ctx| {
            harness.hooks.install_bindings(
                vec![test_binding(
                    test_source_id("s"),
                    HookBoundary::ToolPre,
                    0,
                    None,
                )],
                ctx,
            )
        });
        let held =
            app.update(|ctx| hold_turn(harness.conversation_id, &harness.active_session, ctx));
        assert!(!held);
        assert!(
            app.update(|ctx| start_turn_stop(harness.conversation_id, ctx))
                .is_none()
        );
        assert_eq!(harness.hooks.runner.invocation_count(), 0);
    });
}

#[test]
fn an_answer_for_a_cancelled_hold_is_dropped() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = hook_test_app(&mut app);
        let binding = stop_binding(None);
        harness
            .hooks
            .runner
            .script(binding.id.clone(), continuation("too late"));
        app.update(|ctx| harness.hooks.install_bindings(vec![binding], ctx));
        let hold = harness.hooks.runner.hold();

        assert!(app.update(|ctx| hold_turn(harness.conversation_id, &harness.active_session, ctx)));
        let stop = app
            .update(|ctx| start_turn_stop(harness.conversation_id, ctx))
            .expect("the hooks start");
        app.update(|ctx| cancel_held_turn(harness.conversation_id, ctx));
        hold.release();
        let outcome = stop.await;
        assert_eq!(
            app.update(|ctx| resolve_turn_stop(harness.conversation_id, outcome, ctx)),
            TurnStopResolution::Dropped
        );
        assert!(queued_texts(&app, harness.conversation_id).is_empty());
    });
}

#[test]
fn a_hold_that_times_out_finalizes_without_the_hooks() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = hook_test_app(&mut app);
        let mut binding = stop_binding(None);
        binding.timeout = Duration::from_millis(100);
        harness
            .hooks
            .runner
            .script(binding.id.clone(), continuation("never arrives"));
        app.update(|ctx| harness.hooks.install_bindings(vec![binding], ctx));
        let _hold = harness.hooks.runner.hold();

        let started = Instant::now();
        assert_eq!(
            stop_round(&mut app, &harness).await,
            TurnStopResolution::Finalize
        );
        assert!(started.elapsed() >= Duration::from_millis(100));
        assert!(queued_texts(&app, harness.conversation_id).is_empty());
    });
}

#[test]
fn success_is_not_shown_while_the_stop_hooks_run() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = controller_harness(&mut app);
        let binding = stop_binding(None);
        app.update(|ctx| harness.hooks.install_bindings(vec![binding], ctx));
        let hold = harness.hooks.runner.hold();
        let (conversation_id, stream) = harness.mock_stream(&mut app);
        let (_recorder, statuses) = record_statuses(&mut app, conversation_id);

        finish_turn_with_text(&mut app, conversation_id, &stream, "All done.");
        settle(&mut app, |_| harness.hooks.runner.invocation_count() == 1).await;
        let_background_work_settle(&mut app).await;
        // The run's driver exits on `Success`; nothing may show it while the hooks run.
        assert!(
            !statuses
                .lock()
                .unwrap()
                .contains(&ConversationStatus::Success),
            "{:?}",
            statuses.lock().unwrap()
        );
        assert_eq!(
            conversation_status(&app, conversation_id),
            Some(ConversationStatus::InProgress)
        );
        assert!(app.read(|ctx| HookRuntime::as_ref(ctx).turn_stops.is_held(conversation_id)));

        hold.release();
        settle(&mut app, |app| {
            conversation_status(app, conversation_id) == Some(ConversationStatus::Success)
        })
        .await;
        assert_eq!(
            statuses.lock().unwrap().as_slice(),
            &[ConversationStatus::Success]
        );
        assert!(!app.read(|ctx| HookRuntime::as_ref(ctx).turn_stops.is_held(conversation_id)));
    });
}

#[test]
fn a_follow_up_keeps_the_conversation_in_progress_until_its_own_request() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = controller_harness(&mut app);
        let stop = stop_binding(None);
        harness
            .hooks
            .runner
            .script(stop.id.clone(), continuation("and then do this"));
        let prompt = test_binding(
            test_source_id("prompt"),
            HookBoundary::PromptSubmit,
            0,
            None,
        );
        app.update(|ctx| {
            harness
                .hooks
                .install_bindings(vec![stop.clone(), prompt], ctx)
        });
        let (conversation_id, stream) = harness.mock_stream(&mut app);
        let first_turn = start_turn(&mut app, conversation_id);
        let (_recorder, statuses) = record_statuses(&mut app, conversation_id);

        finish_turn_with_text(&mut app, conversation_id, &stream, "First answer.");
        settle(&mut app, |app| exchange_count(app, conversation_id) == 2).await;

        let envelopes = harness.hooks.adapter.encoded_envelopes();
        let stop_envelope = envelopes
            .iter()
            .find(|envelope| envelope.boundary() == HookBoundary::TurnStop)
            .expect("the stop hooks ran");
        assert_eq!(stop_envelope.turn_id, Some(first_turn));
        let follow_up = envelopes
            .iter()
            .find(|envelope| envelope.boundary() == HookBoundary::PromptSubmit)
            .expect("the follow-up ran the prompt hooks");
        let HookPayload::PromptSubmit {
            text, provenance, ..
        } = &follow_up.payload
        else {
            panic!("a prompt.submit payload");
        };
        assert_eq!(text, "and then do this");
        assert!(matches!(
            provenance,
            PromptProvenance::HookFollowUp { binding, .. } if *binding == stop.id
        ));
        assert!(
            follow_up.turn_id.is_some(),
            "the follow-up starts a turn of its own"
        );
        assert_ne!(follow_up.turn_id, Some(first_turn));
        assert_eq!(
            follow_up.turn_id,
            app.read(|ctx| HookRuntime::as_ref(ctx).turn_id_for(conversation_id))
        );
        let seen = statuses.lock().unwrap().clone();
        assert!(
            !seen.contains(&ConversationStatus::Success),
            "the conversation never looked finished between the stop hook and its follow-up: \
             {seen:?}"
        );
        assert!(queued_texts(&app, conversation_id).is_empty());
    });
}

#[test]
fn a_follow_ups_text_is_sent_as_written() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = controller_harness(&mut app);
        let stop = stop_binding(None);
        // A slash command for a person; for a hook, text like any other.
        harness
            .hooks
            .runner
            .script(stop.id.clone(), continuation("/compact the rest"));
        let prompt = test_binding(
            test_source_id("prompt"),
            HookBoundary::PromptSubmit,
            0,
            None,
        );
        app.update(|ctx| harness.hooks.install_bindings(vec![stop, prompt], ctx));
        let (conversation_id, stream) = harness.mock_stream(&mut app);

        finish_turn_with_text(&mut app, conversation_id, &stream, "First answer.");
        settle(&mut app, |app| exchange_count(app, conversation_id) == 2).await;

        assert_eq!(
            latest_user_query(&app, conversation_id),
            Some(("/compact the rest".to_owned(), UserQueryMode::Normal))
        );
        let follow_up = harness
            .hooks
            .adapter
            .encoded_envelopes()
            .into_iter()
            .find(|envelope| envelope.boundary() == HookBoundary::PromptSubmit)
            .expect("the follow-up ran the prompt hooks");
        assert!(matches!(
            &follow_up.payload,
            HookPayload::PromptSubmit { text, .. } if text == "/compact the rest"
        ));
    });
}

#[test]
fn a_turn_that_came_and_went_during_the_hold_leaves_nothing_behind() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = controller_harness(&mut app);
        let stop = stop_binding(None);
        harness
            .hooks
            .runner
            .script(stop.id.clone(), continuation("later"));
        app.update(|ctx| harness.hooks.install_bindings(vec![stop], ctx));
        let hold = harness.hooks.runner.hold();
        let (conversation_id, stream) = harness.mock_stream(&mut app);

        finish_turn_with_text(&mut app, conversation_id, &stream, "First answer.");
        settle(&mut app, |_| harness.hooks.runner.invocation_count() == 1).await;
        // No prompt hook applies, so this prompt goes out on the same call.
        harness.update_controller(&mut app, |controller, ctx| {
            controller.send_user_query_in_conversation(
                "something else".to_owned(),
                conversation_id,
                None,
                ctx,
            );
        });
        assert_eq!(exchange_count(&app, conversation_id), 2);

        // The hooks answer while the newer turn owns the conversation.
        hold.release();
        settle(&mut app, |app| {
            !app.read(|ctx| HookRuntime::as_ref(ctx).turn_stops.is_held(conversation_id))
        })
        .await;
        let_background_work_settle(&mut app).await;

        // The newer turn is cancelled, which settles the status.
        harness.update_controller(&mut app, |controller, ctx| {
            controller.cancel_conversation_progress(
                conversation_id,
                CancellationReason::ManuallyCancelled,
                ctx,
            );
        });
        settle(&mut app, |app| {
            conversation_status(app, conversation_id) == Some(ConversationStatus::Cancelled)
        })
        .await;

        // A later cancellation of the idle conversation finds nothing left over from the hold,
        // so it does not write a status of its own.
        harness.update_controller(&mut app, |controller, ctx| {
            controller.cancel_conversation_progress(
                conversation_id,
                CancellationReason::Reverted,
                ctx,
            );
        });
        let_background_work_settle(&mut app).await;
        assert_eq!(
            conversation_status(&app, conversation_id),
            Some(ConversationStatus::Cancelled)
        );
    });
}

#[test]
fn success_is_not_shown_while_a_replaced_prompts_successor_still_waits() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = controller_harness(&mut app);
        let stop = stop_binding(None);
        let prompt = test_binding(
            test_source_id("prompt"),
            HookBoundary::PromptSubmit,
            0,
            None,
        );
        app.update(|ctx| harness.hooks.install_bindings(vec![stop, prompt], ctx));
        let (conversation_id, stream) = harness.mock_stream(&mut app);
        let (_recorder, statuses) = record_statuses(&mut app, conversation_id);

        // Each hold covers the invocations started while it is the current one.
        let stop_hold = harness.hooks.runner.hold();
        finish_turn_with_text(&mut app, conversation_id, &stream, "First answer.");
        settle(&mut app, |_| harness.hooks.runner.invocation_count() == 1).await;
        let first_prompt_hold = harness.hooks.runner.hold();
        harness.update_controller(&mut app, |controller, ctx| {
            controller.send_user_query_in_conversation(
                "one".to_owned(),
                conversation_id,
                None,
                ctx,
            );
        });
        settle(&mut app, |_| harness.hooks.runner.invocation_count() == 2).await;
        let second_prompt_hold = harness.hooks.runner.hold();
        harness.update_controller(&mut app, |controller, ctx| {
            controller.send_user_query_in_conversation(
                "two".to_owned(),
                conversation_id,
                None,
                ctx,
            );
        });
        settle(&mut app, |_| harness.hooks.runner.invocation_count() == 3).await;

        // The replaced prompt settles first, then the stop hooks; the second prompt still waits.
        first_prompt_hold.release();
        let_background_work_settle(&mut app).await;
        stop_hold.release();
        settle(&mut app, |app| {
            !app.read(|ctx| HookRuntime::as_ref(ctx).turn_stops.is_held(conversation_id))
        })
        .await;
        let_background_work_settle(&mut app).await;
        assert!(app.read(|ctx| is_pending(conversation_id, ctx)));
        assert_eq!(exchange_count(&app, conversation_id), 1);
        assert!(
            !statuses
                .lock()
                .unwrap()
                .contains(&ConversationStatus::Success),
            "{:?}",
            statuses.lock().unwrap()
        );

        second_prompt_hold.release();
        settle(&mut app, |app| exchange_count(app, conversation_id) == 2).await;
        assert_eq!(
            latest_user_query(&app, conversation_id).map(|(text, _)| text),
            Some("two".to_owned())
        );
        assert!(
            !statuses
                .lock()
                .unwrap()
                .contains(&ConversationStatus::Success)
        );
    });
}

#[test]
fn a_follow_up_for_a_conversation_the_surface_left_waits_for_its_drain() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = controller_harness(&mut app);
        let stop = stop_binding(None);
        harness
            .hooks
            .runner
            .script(stop.id.clone(), continuation("later"));
        app.update(|ctx| harness.hooks.install_bindings(vec![stop], ctx));
        let hold = harness.hooks.runner.hold();
        let (left_behind, stream) = harness.mock_stream(&mut app);
        finish_turn_with_text(&mut app, left_behind, &stream, "First answer.");
        settle(&mut app, |_| harness.hooks.runner.invocation_count() == 1).await;
        // The person moves on to another conversation on the same surface.
        let (current, _current_stream) = harness.mock_stream(&mut app);

        hold.release();
        settle(&mut app, |app| {
            conversation_status(app, left_behind) == Some(ConversationStatus::Success)
        })
        .await;
        let queued = queued_texts(&app, left_behind);
        assert_eq!(
            queued.len(),
            1,
            "the follow-up waits for the conversation's next drain"
        );
        assert_eq!(queued[0].0, "later");
        assert_eq!(exchange_count(&app, left_behind), 1);
        assert_eq!(
            conversation_status(&app, current),
            Some(ConversationStatus::InProgress),
            "the conversation the person is in was not touched"
        );
    });
}

#[test]
fn clearing_the_surface_during_a_hold_settles_the_cleared_turn() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = controller_harness(&mut app);
        let stop = stop_binding(None);
        harness
            .hooks
            .runner
            .script(stop.id.clone(), continuation("ignored"));
        app.update(|ctx| harness.hooks.install_bindings(vec![stop], ctx));
        let hold = harness.hooks.runner.hold();
        let (conversation_id, stream) = harness.mock_stream(&mut app);
        finish_turn_with_text(&mut app, conversation_id, &stream, "First answer.");
        settle(&mut app, |_| harness.hooks.runner.invocation_count() == 1).await;

        let terminal_id = harness.terminal_id();
        app.update(|ctx| {
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                history.clear_conversations_for_terminal_surface(terminal_id, ctx);
            });
        });
        assert_eq!(
            conversation_status(&app, conversation_id),
            Some(ConversationStatus::Cancelled)
        );

        hold.release();
        let_background_work_settle(&mut app).await;
        assert_eq!(
            conversation_status(&app, conversation_id),
            Some(ConversationStatus::Cancelled)
        );
        assert!(queued_texts(&app, conversation_id).is_empty());
    });
}

#[test]
fn a_persons_prompt_during_the_hold_takes_over_and_the_follow_up_waits_behind_it() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = controller_harness(&mut app);
        let stop = stop_binding(None);
        harness
            .hooks
            .runner
            .script(stop.id.clone(), continuation("later"));
        app.update(|ctx| harness.hooks.install_bindings(vec![stop], ctx));
        let hold = harness.hooks.runner.hold();
        let (conversation_id, stream) = harness.mock_stream(&mut app);
        let (_recorder, statuses) = record_statuses(&mut app, conversation_id);

        finish_turn_with_text(&mut app, conversation_id, &stream, "First answer.");
        settle(&mut app, |_| harness.hooks.runner.invocation_count() == 1).await;
        harness.update_controller(&mut app, |controller, ctx| {
            controller.send_user_query_in_conversation(
                "something else".to_owned(),
                conversation_id,
                None,
                ctx,
            );
        });
        assert_eq!(exchange_count(&app, conversation_id), 2);

        hold.release();
        settle(&mut app, |app| {
            !queued_texts(app, conversation_id).is_empty()
        })
        .await;
        let queued = queued_texts(&app, conversation_id);
        assert_eq!(queued[0].0, "later");
        assert!(
            !statuses
                .lock()
                .unwrap()
                .contains(&ConversationStatus::Success),
            "the newer turn owns the status"
        );
        assert_eq!(exchange_count(&app, conversation_id), 2);
    });
}

#[test]
fn cancelling_a_held_turn_settles_it_as_cancelled() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = controller_harness(&mut app);
        let stop = stop_binding(None);
        harness
            .hooks
            .runner
            .script(stop.id.clone(), continuation("ignored"));
        app.update(|ctx| harness.hooks.install_bindings(vec![stop], ctx));
        let hold = harness.hooks.runner.hold();
        let (conversation_id, stream) = harness.mock_stream(&mut app);

        finish_turn_with_text(&mut app, conversation_id, &stream, "First answer.");
        settle(&mut app, |_| harness.hooks.runner.invocation_count() == 1).await;
        harness.update_controller(&mut app, |controller, ctx| {
            controller.cancel_conversation_progress(
                conversation_id,
                CancellationReason::ManuallyCancelled,
                ctx,
            );
        });
        assert_eq!(
            conversation_status(&app, conversation_id),
            Some(ConversationStatus::Cancelled)
        );

        hold.release();
        let_background_work_settle(&mut app).await;
        assert_eq!(
            conversation_status(&app, conversation_id),
            Some(ConversationStatus::Cancelled)
        );
        assert!(queued_texts(&app, conversation_id).is_empty());
    });
}

#[test]
fn a_newer_turn_whose_actions_still_run_is_not_finished_under_them() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = controller_harness(&mut app);
        let stop = stop_binding(None);
        harness
            .hooks
            .runner
            .script(stop.id.clone(), continuation("later"));
        app.update(|ctx| harness.hooks.install_bindings(vec![stop], ctx));
        let hold = harness.hooks.runner.hold();
        let (conversation_id, stream) = harness.mock_stream(&mut app);
        finish_turn_with_text(&mut app, conversation_id, &stream, "First answer.");
        settle(&mut app, |_| harness.hooks.runner.invocation_count() == 1).await;

        // A newer turn: its request went out and its reply queued a command that waits for
        // approval, so its stream is gone while the turn is still under way.
        let second = harness.mock_request(&mut app, conversation_id);
        let task_id = root_task_id(&app, conversation_id);
        second.update(&mut app, |stream, ctx| {
            stream.emit_response_event_for_test(stream_init_event(), ctx);
            stream.emit_response_event_for_test(
                add_messages_event(
                    &task_id,
                    vec![
                        text_message(&task_id, "Let me look."),
                        shell_tool_call_message(&task_id, "ls"),
                    ],
                ),
                ctx,
            );
            stream.emit_response_event_for_test(done_event(), ctx);
            stream.emit_after_stream_finished_for_test(ctx);
        });
        assert!(!harness.update_controller(&mut app, |controller, ctx| {
            controller.has_active_stream_for_conversation(conversation_id, ctx)
        }));
        // The command is pending once its (asynchronous) preprocessing is over.
        settle(&mut app, |app| {
            has_unfinished_actions(app, &harness, conversation_id)
        })
        .await;
        let (_recorder, statuses) = record_statuses(&mut app, conversation_id);

        // The earlier turn's hooks answer: the follow-up is queued, and nothing else happens
        // to the turn under way.
        hold.release();
        settle(&mut app, |app| {
            !app.read(|ctx| HookRuntime::as_ref(ctx).turn_stops.is_held(conversation_id))
        })
        .await;
        let_background_work_settle(&mut app).await;
        assert!(
            !statuses
                .lock()
                .unwrap()
                .contains(&ConversationStatus::Success),
            "{:?}",
            statuses.lock().unwrap()
        );
        assert!(
            matches!(
                conversation_status(&app, conversation_id),
                Some(ConversationStatus::Blocked { .. })
            ),
            "the newer turn's command still waits for approval: {:?}",
            conversation_status(&app, conversation_id)
        );
        assert!(
            has_unfinished_actions(&app, &harness, conversation_id),
            "the newer turn's command was not cancelled"
        );
        assert_eq!(
            exchange_count(&app, conversation_id),
            2,
            "the follow-up was not sent"
        );
        let queued = queued_texts(&app, conversation_id);
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].0, "later");
    });
}
