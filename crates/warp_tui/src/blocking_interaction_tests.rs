use std::cell::Cell;
use std::rc::Rc;

use warp::tui_export::{
    AIAgentAction, AIAgentActionId, AIAgentActionType, AIConversationId, BlocklistAIActionModel,
    BlocklistAIHistoryModel, CancellationReason, TaskId, queue_tui_permission_action,
};
use warp_core::execution_mode::{AppExecutionMode, ExecutionMode};
use warpui::platform::WindowStyle;
use warpui::{AddWindowOptions, App, EntityId, ModelHandle, SingletonEntity, ViewHandle, WindowId};

use super::*;
use crate::test_fixtures::{
    TestHostView, add_test_action_model_and_events_for_surface, add_test_blocking_interaction_model,
};
use crate::tui_permission_prompt::TuiPermissionPrompt;

struct Fixture {
    action_model: ModelHandle<BlocklistAIActionModel>,
    blocking_model: ModelHandle<TuiBlockingInteractionModel>,
    conversation_id: AIConversationId,
    window_id: WindowId,
}

fn fixture(app: &mut App) -> Fixture {
    app.add_singleton_model(|ctx| AppExecutionMode::new(ExecutionMode::App, false, ctx));
    let terminal_surface_id = EntityId::new();
    let (action_model, _) = add_test_action_model_and_events_for_surface(app, terminal_surface_id);
    let blocking_model = add_test_blocking_interaction_model(app, action_model.clone());
    let conversation_id = app.update(|ctx| {
        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
            let conversation_id =
                history.start_new_conversation(terminal_surface_id, false, false, false, ctx);
            history.set_active_conversation_id(conversation_id, terminal_surface_id, ctx);
            conversation_id
        })
    });
    let (window_id, _) = app.update(|ctx| {
        ctx.add_tui_window(
            AddWindowOptions {
                window_style: WindowStyle::NotStealFocus,
                ..Default::default()
            },
            |_| TestHostView,
        )
    });
    Fixture {
        action_model,
        blocking_model,
        conversation_id,
        window_id,
    }
}

fn action(id: &str) -> AIAgentAction {
    AIAgentAction {
        id: AIAgentActionId::from(id.to_owned()),
        action: AIAgentActionType::InitProject,
        task_id: TaskId::new("task".to_owned()),
        requires_result: true,
    }
}

fn permission_prompt(
    app: &mut App,
    fixture: &Fixture,
    action_id: &AIAgentActionId,
) -> ViewHandle<TuiPermissionPrompt> {
    let action_model = fixture.action_model.clone();
    let action_id = action_id.clone();
    app.update(|ctx| {
        ctx.add_typed_action_tui_view(fixture.window_id, move |ctx| {
            TuiPermissionPrompt::new(action_model, action_id, None, ctx)
        })
    })
}

fn queue_action(app: &mut App, fixture: &Fixture, action: AIAgentAction) {
    fixture.action_model.update(app, |model, ctx| {
        queue_tui_permission_action(model, action, fixture.conversation_id, ctx);
    });
}

fn register_action(
    app: &mut App,
    fixture: &Fixture,
    action_id: AIAgentActionId,
    prompt: ViewHandle<TuiPermissionPrompt>,
) {
    fixture.blocking_model.update(app, |model, ctx| {
        model.register_action(
            action_id,
            TuiBlockingInteraction::Permission(prompt),
            TuiBlockingInteractionPlacement::Transcript,
            ctx,
        );
    });
}

#[test]
fn front_blocked_action_selects_only_its_registered_interaction() {
    App::test((), |mut app| async move {
        let fixture = fixture(&mut app);
        let first = action("first");
        let second = action("second");
        let first_prompt = permission_prompt(&mut app, &fixture, &first.id);
        let second_prompt = permission_prompt(&mut app, &fixture, &second.id);
        register_action(&mut app, &fixture, first.id.clone(), first_prompt.clone());
        register_action(&mut app, &fixture, second.id.clone(), second_prompt);

        queue_action(&mut app, &fixture, first.clone());
        queue_action(&mut app, &fixture, second);

        fixture.blocking_model.read(&app, |model, _| {
            let active = model.active().expect("front blocker should resolve");
            assert_eq!(
                active.identity.owner,
                TuiBlockingInteractionOwner::Action(first.id)
            );
            assert_eq!(active.identity.view_id, first_prompt.id());
            assert_eq!(
                active.placement(),
                TuiBlockingInteractionPlacement::Transcript
            );
        });
    });
}

#[test]
fn nonblocked_finished_stale_and_unregistered_actions_do_not_resolve() {
    App::test((), |mut app| async move {
        let fixture = fixture(&mut app);
        let stale = action("stale");
        let stale_prompt = permission_prompt(&mut app, &fixture, &stale.id);
        register_action(&mut app, &fixture, stale.id, stale_prompt);
        assert!(
            fixture
                .blocking_model
                .read(&app, |model, _| model.active().is_none())
        );

        let unregistered = action("unregistered");
        queue_action(&mut app, &fixture, unregistered.clone());
        assert!(
            fixture
                .blocking_model
                .read(&app, |model, _| model.active().is_none())
        );

        let queued = action("queued");
        let queued_prompt = permission_prompt(&mut app, &fixture, &queued.id);
        register_action(&mut app, &fixture, queued.id.clone(), queued_prompt);
        queue_action(&mut app, &fixture, queued);
        assert!(
            fixture
                .blocking_model
                .read(&app, |model, _| model.active().is_none())
        );

        fixture.action_model.update(&mut app, |model, ctx| {
            model.cancel_action_with_id(
                fixture.conversation_id,
                &unregistered.id,
                CancellationReason::ManuallyCancelled,
                ctx,
            );
        });
        assert!(
            fixture
                .blocking_model
                .read(&app, |model, _| model.active().is_none())
        );
    });
}

#[test]
fn session_interaction_precedence_is_deterministic() {
    App::test((), |mut app| async move {
        let fixture = fixture(&mut app);
        let action = action("action");
        let action_prompt = permission_prompt(&mut app, &fixture, &action.id);
        let session_prompt = permission_prompt(&mut app, &fixture, &action.id);
        register_action(&mut app, &fixture, action.id.clone(), action_prompt.clone());
        queue_action(&mut app, &fixture, action);

        fixture.blocking_model.update(&mut app, |model, ctx| {
            model.set_session_interaction(
                Some((
                    TuiBlockingInteraction::Permission(session_prompt.clone()),
                    TuiBlockingInteractionPlacement::InputArea,
                )),
                ctx,
            );
        });
        fixture.blocking_model.read(&app, |model, _| {
            let active = model.active().expect("session blocker should resolve");
            assert_eq!(active.identity.owner, TuiBlockingInteractionOwner::Session);
            assert_eq!(active.identity.view_id, session_prompt.id());
        });

        fixture.blocking_model.update(&mut app, |model, ctx| {
            model.set_session_interaction(None, ctx);
        });
        fixture.blocking_model.read(&app, |model, _| {
            let active = model.active().expect("action blocker should resume");
            assert!(matches!(
                active.identity.owner,
                TuiBlockingInteractionOwner::Action(_)
            ));
            assert_eq!(active.identity.view_id, action_prompt.id());
        });
    });
}

#[test]
fn removed_registration_cannot_remain_active() {
    App::test((), |mut app| async move {
        let fixture = fixture(&mut app);
        let action = action("action");
        let prompt = permission_prompt(&mut app, &fixture, &action.id);
        register_action(&mut app, &fixture, action.id.clone(), prompt);
        queue_action(&mut app, &fixture, action.clone());
        assert!(
            fixture
                .blocking_model
                .read(&app, |model, _| model.active().is_some())
        );

        fixture.blocking_model.update(&mut app, |model, ctx| {
            model.unregister_action(&action.id, ctx);
        });

        assert!(
            fixture
                .blocking_model
                .read(&app, |model, _| model.active().is_none())
        );
    });
}

#[test]
fn unrelated_action_updates_do_not_notify_or_churn_active_identity() {
    App::test((), |mut app| async move {
        let fixture = fixture(&mut app);
        let first = action("first");
        let second = action("second");
        let first_prompt = permission_prompt(&mut app, &fixture, &first.id);
        register_action(&mut app, &fixture, first.id.clone(), first_prompt.clone());
        queue_action(&mut app, &fixture, first);

        let notifications = Rc::new(Cell::new(0));
        let notifications_for_subscription = notifications.clone();
        app.update(|ctx| {
            ctx.subscribe_to_model(&fixture.blocking_model, move |_, _, _| {
                notifications_for_subscription.set(notifications_for_subscription.get() + 1);
            });
        });
        let identity_before = fixture
            .blocking_model
            .read(&app, |model, _| model.active().unwrap().identity);

        queue_action(&mut app, &fixture, second);

        let identity_after = fixture
            .blocking_model
            .read(&app, |model, _| model.active().unwrap().identity);
        assert_eq!(identity_after, identity_before);
        assert_eq!(notifications.get(), 0);
        assert_eq!(identity_after.view_id, first_prompt.id());
    });
}
