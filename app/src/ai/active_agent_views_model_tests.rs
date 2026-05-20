use std::str::FromStr;

use warpui::{App, EntityId, WindowId};

use super::*;

fn setup_model(app: &mut App) -> ModelHandle<ActiveAgentViewsModel> {
    app.add_singleton_model(|_| ActiveAgentViewsModel::new())
}

fn new_task_id() -> AmbientAgentTaskId {
    AmbientAgentTaskId::from_str(&uuid::Uuid::new_v4().to_string()).unwrap()
}

#[test]
fn conversation_switch_updates_last_focused_terminal_state() {
    App::test((), |mut app| async move {
        let model = setup_model(&mut app);
        let window = WindowId::new();
        let terminal = EntityId::new();
        let conversation_1 = AIConversationId::new();
        let conversation_2 = AIConversationId::new();

        model.update(&mut app, |model, ctx| {
            model.handle_pane_focus_change(window, Some(terminal), None, ctx);
            model.update_focused_conversation_for_terminal(
                terminal,
                Some(ConversationOrTaskId::ConversationId(conversation_1)),
            );
        });
        model.read(&app, |model, _| {
            assert_eq!(model.get_last_focused_terminal_id(), Some(terminal));
            assert_eq!(
                model.get_last_focused_conversation(),
                Some(ConversationOrTaskId::ConversationId(conversation_1))
            );
        });

        model.update(&mut app, |model, _| {
            model.update_focused_conversation_for_terminal(
                terminal,
                Some(ConversationOrTaskId::ConversationId(conversation_2)),
            );
        });
        model.read(&app, |model, _| {
            assert_eq!(model.get_last_focused_terminal_id(), Some(terminal));
            assert_eq!(
                model.get_last_focused_conversation(),
                Some(ConversationOrTaskId::ConversationId(conversation_2))
            );
        });
    });
}

#[test]
fn per_window_focused_state_is_independent() {
    App::test((), |mut app| async move {
        let model = setup_model(&mut app);
        let window_a = WindowId::new();
        let window_b = WindowId::new();
        let terminal_a = EntityId::new();
        let terminal_b = EntityId::new();
        let task_a = new_task_id();
        let task_b = new_task_id();

        model.update(&mut app, |model, ctx| {
            model.handle_pane_focus_change(window_a, Some(terminal_a), Some(task_a), ctx);
            model.handle_pane_focus_change(window_b, Some(terminal_b), Some(task_b), ctx);
        });

        model.read(&app, |model, _| {
            assert_eq!(
                model.get_focused_conversation(window_a),
                Some(ConversationOrTaskId::TaskId(task_a))
            );
            assert_eq!(
                model.get_focused_conversation(window_b),
                Some(ConversationOrTaskId::TaskId(task_b))
            );
        });
    });
}

#[test]
fn clearing_one_window_does_not_affect_other() {
    App::test((), |mut app| async move {
        let model = setup_model(&mut app);
        let window_a = WindowId::new();
        let window_b = WindowId::new();
        let terminal_a = EntityId::new();
        let terminal_b = EntityId::new();
        let task_a = new_task_id();
        let task_b = new_task_id();

        model.update(&mut app, |model, ctx| {
            model.handle_pane_focus_change(window_a, Some(terminal_a), Some(task_a), ctx);
            model.handle_pane_focus_change(window_b, Some(terminal_b), Some(task_b), ctx);
        });

        // Clear window A's focus by passing None for terminal_view_id.
        model.update(&mut app, |model, ctx| {
            model.handle_pane_focus_change(window_a, None, None, ctx);
        });

        model.read(&app, |model, _| {
            assert_eq!(model.get_focused_conversation(window_a), None);
            assert_eq!(
                model.get_focused_conversation(window_b),
                Some(ConversationOrTaskId::TaskId(task_b))
            );
        });
    });
}

#[test]
fn last_focused_terminal_tracks_most_recent_globally() {
    App::test((), |mut app| async move {
        let model = setup_model(&mut app);
        let window_a = WindowId::new();
        let window_b = WindowId::new();
        let terminal_a = EntityId::new();
        let terminal_b = EntityId::new();
        let task_a = new_task_id();
        let task_b = new_task_id();

        model.update(&mut app, |model, ctx| {
            model.handle_pane_focus_change(window_a, Some(terminal_a), Some(task_a), ctx);
        });
        model.read(&app, |model, _| {
            assert_eq!(model.get_last_focused_terminal_id(), Some(terminal_a));
            assert_eq!(
                model.get_last_focused_conversation(),
                Some(ConversationOrTaskId::TaskId(task_a))
            );
        });

        model.update(&mut app, |model, ctx| {
            model.handle_pane_focus_change(window_b, Some(terminal_b), Some(task_b), ctx);
        });
        model.read(&app, |model, _| {
            assert_eq!(model.get_last_focused_terminal_id(), Some(terminal_b));
            assert_eq!(
                model.get_last_focused_conversation(),
                Some(ConversationOrTaskId::TaskId(task_b))
            );
        });

        // Clearing window B's focus should NOT clear last_focused (it persists).
        model.update(&mut app, |model, ctx| {
            model.handle_pane_focus_change(window_b, None, None, ctx);
        });
        model.read(&app, |model, _| {
            assert_eq!(model.get_last_focused_terminal_id(), Some(terminal_b));
            assert_eq!(
                model.get_last_focused_conversation(),
                Some(ConversationOrTaskId::TaskId(task_b))
            );
        });
    });
}

#[test]
fn unknown_window_returns_none() {
    App::test((), |mut app| async move {
        let model = setup_model(&mut app);
        let window_a = WindowId::new();
        let unknown_window = WindowId::new();
        let terminal = EntityId::new();
        let task = new_task_id();

        model.update(&mut app, |model, ctx| {
            model.handle_pane_focus_change(window_a, Some(terminal), Some(task), ctx);
        });

        model.read(&app, |model, _| {
            assert_eq!(model.get_focused_conversation(unknown_window), None);
        });
    });
}

#[test]
fn focus_change_without_task_id_has_no_conversation() {
    App::test((), |mut app| async move {
        let model = setup_model(&mut app);
        let window = WindowId::new();
        let terminal = EntityId::new();

        // No agent view handles registered, no task_id → active_conversation_id should be None.
        model.update(&mut app, |model, ctx| {
            model.handle_pane_focus_change(window, Some(terminal), None, ctx);
        });

        model.read(&app, |model, _| {
            assert_eq!(model.get_focused_conversation(window), None);
        });
    });
}

#[test]
fn ambient_session_registration_replaces_stale_terminal_for_same_task() {
    App::test((), |mut app| async move {
        let model = setup_model(&mut app);
        let terminal_a = EntityId::new();
        let terminal_b = EntityId::new();
        let task = new_task_id();

        model.update(&mut app, |model, ctx| {
            model.register_ambient_session(terminal_a, task, ctx);
            model.register_ambient_session(terminal_b, task, ctx);
        });

        model.read(&app, |model, _| {
            assert_eq!(
                model.get_terminal_view_id_for_ambient_task(task),
                Some(terminal_b)
            );
            assert_eq!(model.ambient_sessions.len(), 1);
        });
    });
}

#[test]
fn ambient_session_unregister_keeps_task_open_until_last_terminal_is_removed() {
    App::test((), |mut app| async move {
        let model = setup_model(&mut app);
        let terminal_a = EntityId::new();
        let terminal_b = EntityId::new();
        let task = new_task_id();

        model.update(&mut app, |model, _| {
            model.ambient_sessions.insert(terminal_a, task);
            model.ambient_sessions.insert(terminal_b, task);
            model
                .last_opened_times
                .insert(ConversationOrTaskId::TaskId(task), Utc::now());
        });

        model.update(&mut app, |model, ctx| {
            model.unregister_ambient_session(terminal_a, ctx);
        });

        model.read(&app, |model, _| {
            assert_eq!(
                model.get_terminal_view_id_for_ambient_task(task),
                Some(terminal_b)
            );
            assert!(model
                .last_opened_times
                .contains_key(&ConversationOrTaskId::TaskId(task)));
        });

        model.update(&mut app, |model, ctx| {
            model.unregister_ambient_session(terminal_b, ctx);
        });

        model.read(&app, |model, _| {
            assert_eq!(model.get_terminal_view_id_for_ambient_task(task), None);
            assert!(!model
                .last_opened_times
                .contains_key(&ConversationOrTaskId::TaskId(task)));
        });
    });
}

#[test]
fn remove_focused_state_for_window_cleans_up() {
    App::test((), |mut app| async move {
        let model = setup_model(&mut app);
        let window_a = WindowId::new();
        let window_b = WindowId::new();
        let terminal_a = EntityId::new();
        let terminal_b = EntityId::new();
        let task_a = new_task_id();
        let task_b = new_task_id();

        model.update(&mut app, |model, ctx| {
            model.handle_pane_focus_change(window_a, Some(terminal_a), Some(task_a), ctx);
            model.handle_pane_focus_change(window_b, Some(terminal_b), Some(task_b), ctx);
        });

        // Remove window A's state (simulating undo-close expiry).
        model.update(&mut app, |model, ctx| {
            model.remove_focused_state_for_window(window_a, ctx);
        });

        model.read(&app, |model, _| {
            assert_eq!(model.get_focused_conversation(window_a), None);
            assert_eq!(
                model.get_focused_conversation(window_b),
                Some(ConversationOrTaskId::TaskId(task_b))
            );
        });

        // Removing again is a no-op.
        model.update(&mut app, |model, ctx| {
            model.remove_focused_state_for_window(window_a, ctx);
        });
    });
}

#[test]
fn overwriting_same_window_updates_state() {
    App::test((), |mut app| async move {
        let model = setup_model(&mut app);
        let window = WindowId::new();
        let terminal_1 = EntityId::new();
        let terminal_2 = EntityId::new();
        let task_1 = new_task_id();
        let task_2 = new_task_id();

        model.update(&mut app, |model, ctx| {
            model.handle_pane_focus_change(window, Some(terminal_1), Some(task_1), ctx);
        });
        model.read(&app, |model, _| {
            assert_eq!(
                model.get_focused_conversation(window),
                Some(ConversationOrTaskId::TaskId(task_1))
            );
        });

        model.update(&mut app, |model, ctx| {
            model.handle_pane_focus_change(window, Some(terminal_2), Some(task_2), ctx);
        });
        model.read(&app, |model, _| {
            assert_eq!(
                model.get_focused_conversation(window),
                Some(ConversationOrTaskId::TaskId(task_2))
            );
        });
    });
}
