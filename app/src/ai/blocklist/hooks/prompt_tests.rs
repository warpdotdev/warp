use std::collections::HashMap;
use std::sync::Arc;

use ai::hooks::test_util::{ScriptedRunner, test_binding, test_source_id};
use ai::hooks::{DeclaredBinding, HookBoundary, HookNoticeKind, HookPayload, PromptProvenance};
use warpui::{App, SingletonEntity};

use super::*;
use crate::ai::agent::conversation::{AIConversation, ConversationStatus};
use crate::ai::agent::{
    AIAgentAttachment, AnyFileContent, CancellationReason, FileContext, ImageContext,
};
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::ai::blocklist::hooks::test_utils::{
    controller_harness, conversation_status, exchange_count, hook_test_app,
    let_background_work_settle, settle,
};

fn prompt_binding() -> DeclaredBinding {
    test_binding(
        test_source_id("prompt"),
        HookBoundary::PromptSubmit,
        0,
        None,
    )
}

fn test_model() -> LLMId {
    LLMId::from("test-model")
}

fn user_prompt(text: &str) -> SubmittedPrompt {
    SubmittedPrompt {
        text: text.to_owned(),
        attachments: Vec::new(),
        provenance: PromptProvenance::User,
    }
}

fn file_attachment(path: &str) -> PromptAttachment {
    PromptAttachment {
        kind: PromptAttachmentKind::File,
        path: PathBuf::from(path),
    }
}

fn rule_attachment(path: &str) -> PromptAttachment {
    PromptAttachment {
        kind: PromptAttachmentKind::Rule,
        path: PathBuf::from(path),
    }
}

fn file_reference(name: &str, path: &str) -> AIAgentAttachment {
    AIAgentAttachment::FilePathReference {
        file_id: name.to_owned(),
        file_name: name.to_owned(),
        file_path: path.to_owned(),
    }
}

fn rule_file(path: &str) -> FileContext {
    FileContext {
        file_name: path.to_owned(),
        content: AnyFileContent::StringContent("be nice".to_owned()),
        line_range: None,
        last_modified: None,
        line_count: 1,
    }
}

#[test]
fn prompt_submit_reaches_the_adapter_with_the_text_and_conversation() {
    App::test((), |mut app| async move {
        let harness = hook_test_app(&mut app);
        app.update(|ctx| harness.hooks.install_bindings(vec![prompt_binding()], ctx));

        let preparation = app.update(|ctx| {
            prepare_turn(
                Some(user_prompt("fix the build")),
                test_model(),
                harness.conversation_id,
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
        assert_eq!(
            envelope.model_id,
            Some(test_model()),
            "the event names the model the request uses, with no exchange to read it from"
        );
        let HookPayload::PromptSubmit {
            text,
            attachments,
            provenance,
        } = &envelope.payload
        else {
            panic!("expected a prompt.submit payload");
        };
        assert_eq!(text, "fix the build");
        assert!(attachments.is_empty());
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
fn the_prompts_attachments_reach_the_adapter() {
    App::test((), |mut app| async move {
        let harness = hook_test_app(&mut app);
        app.update(|ctx| harness.hooks.install_bindings(vec![prompt_binding()], ctx));
        let prompt = SubmittedPrompt {
            text: "review these".to_owned(),
            attachments: vec![
                file_attachment("/work/repo/src/lib.rs"),
                rule_attachment("/work/repo/AGENTS.md"),
            ],
            provenance: PromptProvenance::User,
        };

        let preparation = app.update(|ctx| {
            prepare_turn(
                Some(prompt),
                test_model(),
                harness.conversation_id,
                &harness.active_session,
                ctx,
            )
        });
        let TurnPreparation::Pending(decision) = preparation else {
            panic!("a prompt hook applies, so the send waits for it");
        };
        decision.await;
        settle(&mut app, |_| harness.hooks.runner.invocation_count() == 1).await;

        let envelope = &harness.hooks.adapter.encoded_envelopes()[0];
        let HookPayload::PromptSubmit { attachments, .. } = &envelope.payload else {
            panic!("expected a prompt.submit payload");
        };
        assert_eq!(
            attachments,
            &[
                file_attachment("/work/repo/src/lib.rs"),
                rule_attachment("/work/repo/AGENTS.md"),
            ]
        );
    });
}

#[test]
fn attachments_are_the_inputs_file_references_then_its_active_rules() {
    let referenced_attachments = HashMap::from([
        (
            "b.rs".to_owned(),
            file_reference("b.rs", "/work/repo/src/b.rs"),
        ),
        (
            "a.rs".to_owned(),
            file_reference("a.rs", "/work/repo/src/a.rs"),
        ),
        (
            "notes".to_owned(),
            AIAgentAttachment::PlainText("not a file".to_owned()),
        ),
    ]);
    let context: Arc<[AIAgentContext]> = Arc::from(vec![
        AIAgentContext::Image(ImageContext {
            data: String::new(),
            mime_type: "image/png".to_owned(),
            file_name: "shot.png".to_owned(),
            is_figma: false,
        }),
        AIAgentContext::ProjectRules {
            root_path: "/work/repo".to_owned(),
            active_rules: vec![
                rule_file("/home/dev/.agents/AGENTS.md"),
                rule_file("/work/repo/WARP.md"),
            ],
            additional_rule_paths: vec!["/work/repo/crates/x/AGENTS.md".to_owned()],
        },
    ]);
    let input = AIAgentInput::UserQuery {
        query: "hi".to_owned(),
        context,
        static_query_type: None,
        referenced_attachments,
        user_query_mode: Default::default(),
        running_command: None,
        intended_agent: None,
        base: None,
    };

    assert_eq!(
        attachments_in(&input),
        vec![
            file_attachment("/work/repo/src/a.rs"),
            file_attachment("/work/repo/src/b.rs"),
            rule_attachment("/home/dev/.agents/AGENTS.md"),
            rule_attachment("/work/repo/WARP.md"),
        ],
        "files in path order, then the rules whose content went out; an image, a text \
         attachment, and a rule only listed by path are not files the model reads"
    );
    assert!(
        attachments_in(&AIAgentInput::AgentWake).is_empty(),
        "an input Warp built has no attachments"
    );
}

#[test]
fn a_prompt_sent_with_file_attachments_reports_them_to_the_hook() {
    App::test((), |mut app| async move {
        let harness = controller_harness(&mut app);
        app.update(|ctx| harness.hooks.install_bindings(vec![prompt_binding()], ctx));
        let conversation_id = app.update(|ctx| {
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                history.start_new_conversation(harness.terminal_id(), false, false, false, ctx)
            })
        });
        harness.update_controller(&mut app, |controller, ctx| {
            controller.send_user_query_in_conversation_with_attachments(
                "look at this".to_owned(),
                conversation_id,
                None,
                HashMap::from([(
                    "spec.md".to_owned(),
                    file_reference("spec.md", "/work/repo/docs/spec.md"),
                )]),
                None,
                ctx,
            );
        });
        settle(&mut app, |app| exchange_count(app, conversation_id) == 1).await;

        let envelope = &harness.hooks.adapter.encoded_envelopes()[0];
        let HookPayload::PromptSubmit { attachments, .. } = &envelope.payload else {
            panic!("expected a prompt.submit payload");
        };
        assert_eq!(attachments, &[file_attachment("/work/repo/docs/spec.md")]);
    });
}

#[test]
fn prompt_submit_does_not_fire_for_a_shared_session_viewer() {
    App::test((), |mut app| async move {
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
                    test_model(),
                    conversation_id,
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
                    test_model(),
                    harness.conversation_id,
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
fn before_the_registry_is_finalized_a_prompt_is_sent_on_the_same_call() {
    App::test((), |mut app| async move {
        let harness = controller_harness(&mut app);
        app.update(|ctx| harness.hooks.stage_bindings(vec![prompt_binding()], ctx));
        let conversation_id = harness.send_prompt(&mut app, "right away");
        assert_eq!(exchange_count(&app, conversation_id), 1);
        assert_eq!(harness.hooks.adapter.encoded_count(), 0);
        assert_eq!(
            app.read(|ctx| HookRuntime::as_ref(ctx).turn_id_for(conversation_id)),
            None
        );
    });
}
