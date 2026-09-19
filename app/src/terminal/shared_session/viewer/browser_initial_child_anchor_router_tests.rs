use std::sync::Arc;

use parking_lot::Mutex;
use warpui::{App, ModelHandle, ViewHandle};

use super::*;
use crate::test_util::add_window_with_terminal;
use crate::test_util::terminal::initialize_app_for_terminal_view;

const PARENT_TASK_ID: &str = "11111111-1111-1111-1111-111111111111";
const CHILD_TASK_ID: &str = "22222222-2222-2222-2222-222222222222";
const OTHER_PARENT_TASK_ID: &str = "33333333-3333-3333-3333-333333333333";

#[test]
fn waits_for_seeded_child_registration_without_fetching() {
    App::test((), |mut app| async move {
        let (terminal_view, router) =
            setup(&mut app, ChildAnchor::Selected(task_id(CHILD_TASK_ID)));
        let restored = observe_restorations(&mut app, &terminal_view);
        let child_conversation_id = AIConversationId::new();

        router.update(&mut app, |router, ctx| {
            router.viewer_mode_seeded(task_id(PARENT_TASK_ID), &[task_id(CHILD_TASK_ID)], ctx);
        });
        router.read(&app, |router, _| {
            assert!(!router.initial_anchor_fetch_in_flight);
        });
        assert!(restored.lock().is_empty());
        router.update(&mut app, |router, ctx| {
            router.child_registered(task_id(CHILD_TASK_ID), child_conversation_id, ctx);
        });

        assert_eq!(*restored.lock(), vec![Some(child_conversation_id)]);
    });
}

#[test]
fn ignores_a_completion_after_registration_resolves_the_anchor() {
    App::test((), |mut app| async move {
        let (terminal_view, router) =
            setup(&mut app, ChildAnchor::Selected(task_id(CHILD_TASK_ID)));
        let restored = observe_restorations(&mut app, &terminal_view);
        let child_conversation_id = AIConversationId::new();

        router.update(&mut app, |router, ctx| {
            router.viewer_mode_seeded(task_id(PARENT_TASK_ID), &[task_id(CHILD_TASK_ID)], ctx);
            router.child_registered(task_id(CHILD_TASK_ID), child_conversation_id, ctx);
        });
        router.update(&mut app, |router, ctx| {
            router.finish_initial_anchor_resolution(None, ctx);
        });
        assert_eq!(*restored.lock(), vec![Some(child_conversation_id)]);
    });
}

#[test]
fn clears_an_invalid_anchor_after_the_seed_settles() {
    App::test((), |mut app| async move {
        let (terminal_view, router) = setup(&mut app, ChildAnchor::Invalid);
        let restored = observe_restorations(&mut app, &terminal_view);

        router.update(&mut app, |router, ctx| {
            router.viewer_mode_seeded(task_id(PARENT_TASK_ID), &[], ctx);
        });

        assert_eq!(*restored.lock(), vec![None]);
    });
}

#[test]
fn ignores_hydration_for_another_parent() {
    App::test((), |mut app| async move {
        let (terminal_view, router) = setup(&mut app, ChildAnchor::Invalid);
        let restored = observe_restorations(&mut app, &terminal_view);

        router.update(&mut app, |router, ctx| {
            router.viewer_mode_seeded(task_id(OTHER_PARENT_TASK_ID), &[], ctx);
        });

        router.read(&app, |router, _| {
            assert!(router.seeded_child_ids.is_none());
            assert!(!router.initial_anchor_resolution_emitted);
        });
        assert!(restored.lock().is_empty());
    });
}

fn setup(
    app: &mut App,
    initial_child_anchor: ChildAnchor,
) -> (
    ViewHandle<TerminalView>,
    ModelHandle<BrowserInitialChildAnchorRouter>,
) {
    initialize_app_for_terminal_view(app);
    let terminal_view = add_window_with_terminal(app, None);
    let router = app.add_model(|_| {
        BrowserInitialChildAnchorRouter::new_with_anchor(
            task_id(PARENT_TASK_ID),
            terminal_view.downgrade(),
            initial_child_anchor,
        )
    });
    (terminal_view, router)
}

fn observe_restorations(
    app: &mut App,
    terminal_view: &ViewHandle<TerminalView>,
) -> Arc<Mutex<Vec<Option<AIConversationId>>>> {
    let restored = Arc::new(Mutex::new(vec![]));
    let events = restored.clone();
    app.update(|ctx| {
        ctx.subscribe_to_view(terminal_view, move |_, event, _| {
            if let TerminalViewEvent::RestoreInitialChildAnchor { conversation_id } = event {
                events.lock().push(*conversation_id);
            }
        });
    });
    restored
}

fn task_id(id: &str) -> AmbientAgentTaskId {
    id.parse().expect("hardcoded task id parses")
}
