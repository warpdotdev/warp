use ai::hooks::test_util::{ScriptedRunner, test_binding, test_source_id};
use ai::hooks::{DeclaredBinding, HookBoundary, HookNoticeKind, HookPayload, PromptProvenance};
use warpui::{App, SingletonEntity};

use super::*;
use crate::ai::agent::CancellationReason;
use crate::ai::agent::conversation::{AIConversation, ConversationStatus};
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::ai::blocklist::hooks::testing::{
    controller_harness, conversation_status, exchange_count, hook_test_app,
    let_background_work_settle, settle,
};
use crate::features::FeatureFlag;

fn prompt_binding() -> DeclaredBinding {
    test_binding(
        test_source_id("prompt"),
        HookBoundary::PromptSubmit,
        0,
        None,
    )
}

fn user_prompt(text: &str) -> SubmittedPrompt {
    SubmittedPrompt {
        text: text.to_owned(),
        provenance: PromptProvenance::User,
    }
}

#[test]
fn prompt_submit_reaches_the_adapter_with_the_text_and_conversation() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = hook_test_app(&mut app);
        app.update(|ctx| harness.hooks.install_bindings(vec![prompt_binding()], ctx));

        let preparation = app.update(|ctx| {
            prepare_turn(
                Some(user_prompt("fix the build")),
                harness.conversation_id,
                harness.terminal_view_id,
                &harness.active_session,
                ctx,
            )
        });
        let TurnPreparation::Pending(decision) = preparation else {
            panic!("a prompt hook applies, so the send waits for it");
        };
        let decision = decision.await;
        assert!(matches!(decision, PromptDecision::Proceed(_)));
        settle(&mut app, |_| harness.hooks.runner.invocation_count() == 1).await;

        let envelope = &harness.hooks.adapter.encoded_envelopes()[0];
        assert_eq!(envelope.boundary(), HookBoundary::PromptSubmit);
        assert_eq!(envelope.conversation_id, harness.conversation_id);
        let HookPayload::PromptSubmit {
            text,
            attachments_summary,
            provenance,
        } = &envelope.payload
        else {
            panic!("expected a prompt.submit payload");
        };
        assert_eq!(text, "fix the build");
        assert!(attachments_summary.is_empty());
        assert_eq!(*provenance, PromptProvenance::User);
        assert_eq!(
            envelope.turn_id,
            app.read(|ctx| HookRuntime::as_ref(ctx).turn_id_for(harness.conversation_id)),
            "the prompt's envelope carries the turn it starts"
        );
        assert!(envelope.turn_id.is_some());
    });
}

#[test]
fn prompt_submit_does_not_fire_for_a_shared_session_viewer() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = hook_test_app(&mut app);
        app.update(|ctx| harness.hooks.install_bindings(vec![prompt_binding()], ctx));
        let viewer = AIConversation::new(true, false);
        let viewer_id = viewer.id();
        app.update(|ctx| {
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |model, ctx| {
                model.restore_conversations(harness.terminal_view_id, vec![viewer], ctx);
            });
        });
        for conversation_id in [viewer_id, AIConversationId::new()] {
            let preparation = app.update(|ctx| {
                prepare_turn(
                    Some(user_prompt("mirrored")),
                    conversation_id,
                    harness.terminal_view_id,
                    &harness.active_session,
                    ctx,
                )
            });
            assert!(matches!(preparation, TurnPreparation::Ready));
        }
        let_background_work_settle(&mut app).await;
        assert_eq!(harness.hooks.adapter.encoded_count(), 0);
    });
}

#[test]
fn every_prompt_starts_a_new_turn_and_the_turns_envelopes_carry_its_id() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = hook_test_app(&mut app);
        // Only a tool binding is active, so a prompt starts a turn without waiting on anything.
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
        let mut turn_ids = Vec::new();
        for prompt in [
            Some(user_prompt("first")),
            Some(user_prompt("second")),
            None,
        ] {
            let preparation = app.update(|ctx| {
                prepare_turn(
                    prompt,
                    harness.conversation_id,
                    harness.terminal_view_id,
                    &harness.active_session,
                    ctx,
                )
            });
            assert!(matches!(preparation, TurnPreparation::Ready));
            turn_ids.push(
                app.read(|ctx| HookRuntime::as_ref(ctx).turn_id_for(harness.conversation_id))
                    .expect("a turn started"),
            );
        }
        assert_ne!(turn_ids[0], turn_ids[1]);
        assert_ne!(turn_ids[1], turn_ids[2]);
        assert_eq!(
            app.read(|ctx| HookRuntime::as_ref(ctx).turn_id_for(AIConversationId::new())),
            None
        );
    });
}

#[test]
fn a_halting_hook_stops_the_prompt_and_tells_the_person_why() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = controller_harness(&mut app);
        let binding = prompt_binding();
        harness.hooks.runner.script(
            binding.id.clone(),
            ScriptedRunner::exit_with(0, r#"{"halt": true, "reason": "not before lunch"}"#),
        );
        app.update(|ctx| harness.hooks.install_bindings(vec![binding], ctx));

        let conversation_id = harness.send_prompt(&mut app, "deploy everything");
        let status_before = conversation_status(&app, conversation_id);
        settle(&mut app, |_| !harness.hooks.messenger.notices().is_empty()).await;
        let_background_work_settle(&mut app).await;

        assert_eq!(
            exchange_count(&app, conversation_id),
            0,
            "no request went out"
        );
        assert_eq!(conversation_status(&app, conversation_id), status_before);
        let notices = harness.hooks.messenger.notices();
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].kind, HookNoticeKind::Denied);
        assert_eq!(notices[0].text, "not before lunch");
        assert_eq!(notices[0].conversation_id, conversation_id);
        assert!(!app.read(|ctx| is_pending(conversation_id, ctx)));
    });
}

#[test]
fn a_denying_or_asking_hook_halts_the_prompt_too() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = controller_harness(&mut app);
        let binding = prompt_binding();
        app.update(|ctx| harness.hooks.install_bindings(vec![binding.clone()], ctx));

        harness
            .hooks
            .runner
            .script(binding.id.clone(), ScriptedRunner::deny_with("denied it"));
        let denied = harness.send_prompt(&mut app, "one");
        settle(&mut app, |_| harness.hooks.messenger.notices().len() == 1).await;

        harness.hooks.runner.script(
            binding.id.clone(),
            ScriptedRunner::exit_with(0, r#"{"verdict": "ask", "reason": "are you sure"}"#),
        );
        let asked = harness.send_prompt(&mut app, "two");
        settle(&mut app, |_| harness.hooks.messenger.notices().len() == 2).await;
        let_background_work_settle(&mut app).await;

        assert_eq!(exchange_count(&app, denied), 0);
        assert_eq!(exchange_count(&app, asked), 0);
        let notices = harness.hooks.messenger.notices();
        assert_eq!(notices[0].text, "denied it");
        assert!(notices[1].text.starts_with(ASK_UNAVAILABLE_HALT_REASON));
        assert!(notices[1].text.contains("are you sure"));
    });
}

#[test]
fn a_hook_with_no_opinion_lets_the_prompt_through_after_it_answers() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = controller_harness(&mut app);
        let binding = prompt_binding();
        app.update(|ctx| harness.hooks.install_bindings(vec![binding], ctx));
        let hold = harness.hooks.runner.hold();

        let conversation_id = harness.send_prompt(&mut app, "go ahead");
        settle(&mut app, |_| harness.hooks.runner.invocation_count() == 1).await;
        assert_eq!(
            exchange_count(&app, conversation_id),
            0,
            "the request waits for the hook"
        );
        assert!(app.read(|ctx| is_pending(conversation_id, ctx)));

        hold.release();
        settle(&mut app, |app| exchange_count(app, conversation_id) == 1).await;
        assert!(!app.read(|ctx| is_pending(conversation_id, ctx)));
        assert_eq!(
            conversation_status(&app, conversation_id),
            Some(ConversationStatus::InProgress)
        );
        assert!(harness.hooks.messenger.notices().is_empty());
    });
}

#[test]
fn a_newer_prompt_for_the_same_conversation_replaces_one_still_waiting() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = controller_harness(&mut app);
        let binding = prompt_binding();
        app.update(|ctx| harness.hooks.install_bindings(vec![binding], ctx));
        let hold = harness.hooks.runner.hold();

        let conversation_id = harness.send_prompt(&mut app, "first");
        settle(&mut app, |_| harness.hooks.runner.invocation_count() == 1).await;
        harness.update_controller(&mut app, |controller, ctx| {
            controller.send_user_query_in_conversation(
                "second".to_owned(),
                conversation_id,
                None,
                ctx,
            );
        });
        settle(&mut app, |_| harness.hooks.runner.invocation_count() == 2).await;

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
    });
}

#[test]
fn cancelling_the_conversation_drops_a_waiting_prompt() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = controller_harness(&mut app);
        let binding = prompt_binding();
        app.update(|ctx| harness.hooks.install_bindings(vec![binding], ctx));
        let hold = harness.hooks.runner.hold();

        let conversation_id = harness.send_prompt(&mut app, "first");
        settle(&mut app, |_| harness.hooks.runner.invocation_count() == 1).await;
        harness.update_controller(&mut app, |controller, ctx| {
            controller.cancel_conversation_progress(
                conversation_id,
                CancellationReason::ManuallyCancelled,
                ctx,
            );
        });
        assert!(!app.read(|ctx| is_pending(conversation_id, ctx)));

        hold.release();
        let_background_work_settle(&mut app).await;
        assert_eq!(exchange_count(&app, conversation_id), 0);
    });
}

#[test]
fn with_hooks_off_a_prompt_is_sent_on_the_same_call() {
    App::test((), |mut app| async move {
        let harness = controller_harness(&mut app);
        app.update(|ctx| harness.hooks.install_bindings(vec![prompt_binding()], ctx));
        let conversation_id = harness.send_prompt(&mut app, "right away");
        assert_eq!(exchange_count(&app, conversation_id), 1);
        assert_eq!(harness.hooks.adapter.encoded_count(), 0);
        assert_eq!(
            app.read(|ctx| HookRuntime::as_ref(ctx).turn_id_for(conversation_id)),
            None
        );
    });
}
