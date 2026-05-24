use std::cell::RefCell;
use std::rc::Rc;

use super::*;

#[test]
fn test_transfer_view_to_window_updates_window_mapping() {
    #[derive(Default)]
    struct TestView {
        value: usize,
    }

    impl Entity for TestView {
        type Event = ();
    }

    impl View for TestView {
        fn render(&self, _: &AppContext) -> Box<dyn Element> {
            Empty::new().finish()
        }

        fn ui_name() -> &'static str {
            "TestView"
        }
    }

    impl TypedActionView for TestView {
        type Action = ();
    }

    App::test((), |mut app| async move {
        let (window_1_id, _) = app.add_window(WindowStyle::NotStealFocus, |_| TestView::default());
        let (window_2_id, _) = app.add_window(WindowStyle::NotStealFocus, |_| TestView::default());

        let view_to_transfer = app.add_view(window_1_id, |_| TestView { value: 42 });
        let view_id = view_to_transfer.id();

        app.read(|ctx| {
            assert_eq!(
                view_to_transfer.window_id(ctx),
                window_1_id,
                "view should initially be in window 1"
            );
        });

        app.read(|ctx| {
            assert!(
                ctx.windows[&window_1_id].views.contains_key(&view_id),
                "view should be in window 1's views map"
            );
            assert!(
                !ctx.windows[&window_2_id].views.contains_key(&view_id),
                "view should not be in window 2's views map yet"
            );
        });

        let success =
            app.update(|ctx| ctx.transfer_view_to_window(view_id, window_1_id, window_2_id));
        assert!(success, "transfer should succeed");

        app.read(|ctx| {
            assert_eq!(
                view_to_transfer.window_id(ctx),
                window_2_id,
                "view should now be in window 2"
            );
        });

        app.read(|ctx| {
            assert!(
                !ctx.windows[&window_1_id].views.contains_key(&view_id),
                "view should no longer be in window 1's views map"
            );
            assert!(
                ctx.windows[&window_2_id].views.contains_key(&view_id),
                "view should now be in window 2's views map"
            );
        });

        view_to_transfer.read(&app, |view, _| {
            assert_eq!(
                view.value, 42,
                "view data should be preserved after transfer"
            );
        });
    });
}

#[test]
fn test_transfer_view_subscriptions_continue_working() {
    #[derive(Default)]
    struct EmitterView;

    impl Entity for EmitterView {
        type Event = usize;
    }

    impl View for EmitterView {
        fn render(&self, _: &AppContext) -> Box<dyn Element> {
            Empty::new().finish()
        }

        fn ui_name() -> &'static str {
            "EmitterView"
        }
    }

    impl TypedActionView for EmitterView {
        type Action = ();
    }

    #[derive(Default)]
    struct SubscriberView {
        received_events: Vec<usize>,
    }

    impl Entity for SubscriberView {
        type Event = ();
    }

    impl View for SubscriberView {
        fn render(&self, _: &AppContext) -> Box<dyn Element> {
            Empty::new().finish()
        }

        fn ui_name() -> &'static str {
            "SubscriberView"
        }
    }

    impl TypedActionView for SubscriberView {
        type Action = ();
    }

    App::test((), |mut app| async move {
        let (window_1_id, _) = app.add_window(WindowStyle::NotStealFocus, |_| EmitterView);
        let (window_2_id, _) =
            app.add_window(WindowStyle::NotStealFocus, |_| SubscriberView::default());

        let emitter = app.add_view(window_1_id, |_| EmitterView);
        let subscriber = app.add_view(window_2_id, |_| SubscriberView::default());

        subscriber.update(&mut app, |_, ctx| {
            ctx.subscribe_to_view(&emitter, |view, _, event, _| {
                view.received_events.push(*event);
            });
        });

        emitter.update(&mut app, |_, ctx| ctx.emit(1));
        subscriber.read(&app, |view, _| {
            assert_eq!(
                view.received_events,
                vec![1],
                "should receive event before transfer"
            );
        });

        let emitter_id = emitter.id();
        let success =
            app.update(|ctx| ctx.transfer_view_to_window(emitter_id, window_1_id, window_2_id));
        assert!(success, "transfer should succeed");

        emitter.update(&mut app, |_, ctx| ctx.emit(2));
        subscriber.read(&app, |view, _| {
            assert_eq!(
                view.received_events,
                vec![1, 2],
                "should receive event after transfer"
            );
        });

        emitter.update(&mut app, |_, ctx| ctx.emit(3));
        subscriber.read(&app, |view, _| {
            assert_eq!(
                view.received_events,
                vec![1, 2, 3],
                "should continue receiving events"
            );
        });
    });
}

#[test]
fn test_transfer_view_app_subscriptions_continue_working() {
    #[derive(Default)]
    struct TestView;

    impl Entity for TestView {
        type Event = usize;
    }

    impl View for TestView {
        fn render(&self, _: &AppContext) -> Box<dyn Element> {
            Empty::new().finish()
        }

        fn ui_name() -> &'static str {
            "TestView"
        }
    }

    impl TypedActionView for TestView {
        type Action = ();
    }

    App::test((), |mut app| async move {
        let (window_1_id, _) = app.add_window(WindowStyle::NotStealFocus, |_| TestView);
        let (window_2_id, _) = app.add_window(WindowStyle::NotStealFocus, |_| TestView);

        let emitter = app.add_view(window_1_id, |_| TestView);
        let received_events: Rc<RefCell<Vec<usize>>> = Rc::new(RefCell::new(Vec::new()));
        let received_events_clone = received_events.clone();

        app.update(|ctx| {
            ctx.subscribe_to_view(&emitter, move |_view, event, _ctx| {
                received_events_clone.borrow_mut().push(*event);
            });
        });

        emitter.update(&mut app, |_, ctx| ctx.emit(1));
        assert_eq!(
            *received_events.borrow(),
            vec![1],
            "should receive event before transfer"
        );

        let emitter_id = emitter.id();
        let success =
            app.update(|ctx| ctx.transfer_view_to_window(emitter_id, window_1_id, window_2_id));
        assert!(success, "transfer should succeed");

        emitter.update(&mut app, |_, ctx| ctx.emit(2));
        assert_eq!(
            *received_events.borrow(),
            vec![1, 2],
            "should receive event after transfer"
        );
    });
}

#[test]
fn test_transfer_view_observations_continue_working() {
    #[derive(Default)]
    struct ObserverView {
        observed_counts: Vec<usize>,
    }

    impl Entity for ObserverView {
        type Event = ();
    }

    impl View for ObserverView {
        fn render(&self, _: &AppContext) -> Box<dyn Element> {
            Empty::new().finish()
        }

        fn ui_name() -> &'static str {
            "ObserverView"
        }
    }

    impl TypedActionView for ObserverView {
        type Action = ();
    }

    #[derive(Default)]
    struct ObservedModel {
        count: usize,
    }

    impl Entity for ObservedModel {
        type Event = ();
    }

    App::test((), |mut app| async move {
        let (window_1_id, _) =
            app.add_window(WindowStyle::NotStealFocus, |_| ObserverView::default());
        let (window_2_id, _) =
            app.add_window(WindowStyle::NotStealFocus, |_| ObserverView::default());

        let model = app.add_model(|_| ObservedModel { count: 0 });
        let observer = app.add_view(window_1_id, |_| ObserverView::default());

        observer.update(&mut app, |_, ctx| {
            ctx.observe(&model, |view, observed, ctx| {
                view.observed_counts.push(observed.as_ref(ctx).count);
            });
        });

        model.update(&mut app, |m, ctx| {
            m.count = 1;
            ctx.notify();
        });
        observer.read(&app, |view, _| {
            assert_eq!(
                view.observed_counts,
                vec![1],
                "should observe before transfer"
            );
        });

        let observer_id = observer.id();
        let success =
            app.update(|ctx| ctx.transfer_view_to_window(observer_id, window_1_id, window_2_id));
        assert!(success, "transfer should succeed");

        model.update(&mut app, |m, ctx| {
            m.count = 2;
            ctx.notify();
        });
        observer.read(&app, |view, _| {
            assert_eq!(
                view.observed_counts,
                vec![1, 2],
                "should observe after transfer"
            );
        });
    });
}

#[test]
fn test_on_window_transferred_callback_fires() {
    struct TransferTrackingView {
        transfer_events: Vec<(WindowId, WindowId)>,
    }

    impl Entity for TransferTrackingView {
        type Event = ();
    }

    impl View for TransferTrackingView {
        fn render(&self, _: &AppContext) -> Box<dyn Element> {
            Empty::new().finish()
        }

        fn ui_name() -> &'static str {
            "TransferTrackingView"
        }

        fn on_window_transferred(
            &mut self,
            source_window_id: WindowId,
            target_window_id: WindowId,
            _ctx: &mut ViewContext<Self>,
        ) {
            self.transfer_events
                .push((source_window_id, target_window_id));
        }
    }

    impl TypedActionView for TransferTrackingView {
        type Action = ();
    }

    App::test((), |mut app| async move {
        let (window_1_id, _) =
            app.add_window(WindowStyle::NotStealFocus, |_| TransferTrackingView {
                transfer_events: Vec::new(),
            });
        let (window_2_id, _) =
            app.add_window(WindowStyle::NotStealFocus, |_| TransferTrackingView {
                transfer_events: Vec::new(),
            });

        let view = app.add_view(window_1_id, |_| TransferTrackingView {
            transfer_events: Vec::new(),
        });

        view.read(&app, |v, _| {
            assert!(v.transfer_events.is_empty(), "no transfers yet");
        });

        let view_id = view.id();
        app.update(|ctx| ctx.transfer_view_to_window(view_id, window_1_id, window_2_id));

        view.read(&app, |v, _| {
            assert_eq!(
                v.transfer_events,
                vec![(window_1_id, window_2_id)],
                "callback should fire with correct window IDs"
            );
        });
    });
}

#[test]
fn test_weak_view_handle_upgrade_after_transfer() {
    #[derive(Default)]
    struct TestView {
        value: usize,
    }

    impl Entity for TestView {
        type Event = ();
    }

    impl View for TestView {
        fn render(&self, _: &AppContext) -> Box<dyn Element> {
            Empty::new().finish()
        }

        fn ui_name() -> &'static str {
            "TestView"
        }
    }

    impl TypedActionView for TestView {
        type Action = ();
    }

    App::test((), |mut app| async move {
        let (window_1_id, _) = app.add_window(WindowStyle::NotStealFocus, |_| TestView::default());
        let (window_2_id, _) = app.add_window(WindowStyle::NotStealFocus, |_| TestView::default());

        let view = app.add_view(window_1_id, |_| TestView { value: 42 });
        let weak = view.downgrade();

        app.read(|ctx| {
            let upgraded = weak.upgrade(ctx);
            assert!(
                upgraded.is_some(),
                "weak handle should upgrade before transfer"
            );
            assert_eq!(
                upgraded.as_ref().map(|v| v.window_id(ctx)),
                Some(window_1_id)
            );
        });

        let view_id = view.id();
        app.update(|ctx| ctx.transfer_view_to_window(view_id, window_1_id, window_2_id));

        app.read(|ctx| {
            let upgraded = weak.upgrade(ctx);
            assert!(
                upgraded.is_some(),
                "weak handle should upgrade after transfer"
            );
            assert_eq!(
                upgraded.as_ref().map(|v| v.window_id(ctx)),
                Some(window_2_id),
                "upgraded handle should point to new window"
            );
        });

        view.read(&app, |v, _| {
            assert_eq!(v.value, 42, "view data preserved");
        });
    });
}

#[test]
fn test_transfer_nonexistent_view_returns_false() {
    #[derive(Default)]
    struct TestView;

    impl Entity for TestView {
        type Event = ();
    }

    impl View for TestView {
        fn render(&self, _: &AppContext) -> Box<dyn Element> {
            Empty::new().finish()
        }

        fn ui_name() -> &'static str {
            "TestView"
        }
    }

    impl TypedActionView for TestView {
        type Action = ();
    }

    App::test((), |mut app| async move {
        let (window_1_id, _) = app.add_window(WindowStyle::NotStealFocus, |_| TestView);
        let (window_2_id, _) = app.add_window(WindowStyle::NotStealFocus, |_| TestView);

        let fake_view_id = EntityId::new();
        let success =
            app.update(|ctx| ctx.transfer_view_to_window(fake_view_id, window_1_id, window_2_id));
        assert!(!success, "transfer of nonexistent view should return false");
    });
}

#[test]
fn test_transfer_to_same_window_is_noop() {
    #[derive(Default)]
    struct TestView {
        value: usize,
    }

    impl Entity for TestView {
        type Event = usize;
    }

    impl View for TestView {
        fn render(&self, _: &AppContext) -> Box<dyn Element> {
            Empty::new().finish()
        }

        fn ui_name() -> &'static str {
            "TestView"
        }
    }

    impl TypedActionView for TestView {
        type Action = ();
    }

    App::test((), |mut app| async move {
        let (window_id, _) = app.add_window(WindowStyle::NotStealFocus, |_| TestView::default());

        let view = app.add_view(window_id, |_| TestView { value: 42 });
        let view_id = view.id();

        let success = app.update(|ctx| ctx.transfer_view_to_window(view_id, window_id, window_id));
        assert!(success, "transfer to same window should return true");

        app.read(|ctx| {
            assert_eq!(
                view.window_id(ctx),
                window_id,
                "view should still be in same window"
            );
            assert!(
                ctx.windows[&window_id].views.contains_key(&view_id),
                "view should still be in window's views map"
            );
        });

        view.update(&mut app, |_, ctx| ctx.emit(1));
        view.read(&app, |v, _| {
            assert_eq!(v.value, 42, "view should still work normally");
        });
    });
}

#[test]
fn test_transfer_view_drop_and_reference_counting() {
    #[derive(Default)]
    struct TestView {
        value: usize,
    }

    impl Entity for TestView {
        type Event = ();
    }

    impl View for TestView {
        fn render(&self, _: &AppContext) -> Box<dyn Element> {
            Empty::new().finish()
        }

        fn ui_name() -> &'static str {
            "TestView"
        }
    }

    impl TypedActionView for TestView {
        type Action = ();
    }

    App::test((), |mut app| async move {
        let (window_1_id, _) = app.add_window(WindowStyle::NotStealFocus, |_| TestView::default());
        let (window_2_id, _) = app.add_window(WindowStyle::NotStealFocus, |_| TestView::default());

        let view = app.add_view(window_1_id, |_| TestView { value: 42 });
        let view_id = view.id();
        let view_clone = view.clone();

        app.update(|ctx| ctx.transfer_view_to_window(view_id, window_1_id, window_2_id));

        drop(view_clone);

        app.read(|ctx| {
            assert!(
                ctx.windows[&window_2_id].views.contains_key(&view_id),
                "view should still exist after dropping one handle"
            );
            assert!(
                ctx.view_to_window.contains_key(&view_id),
                "view_to_window mapping should still exist"
            );
        });

        view.read(&app, |v, _| {
            assert_eq!(v.value, 42, "view data should be intact");
        });

        drop(view);

        // Trigger cleanup via app.update which calls flush_effects -> remove_dropped_items
        app.update(|_| {});

        // Verify the view was removed from the correct window (window_2, not window_1)
        // and that the view_to_window mapping was cleaned up
        app.read(|ctx| {
            assert!(
                !ctx.windows[&window_1_id].views.contains_key(&view_id),
                "view should not be in original window"
            );
            assert!(
                !ctx.windows[&window_2_id].views.contains_key(&view_id),
                "view should be removed from target window after drop"
            );
            assert!(
                !ctx.view_to_window.contains_key(&view_id),
                "view_to_window mapping should be cleaned up"
            );
        });
    });
}

#[test]
fn test_transfer_structural_children_follows_parent() {
    #[derive(Default)]
    struct TestView {
        value: usize,
    }

    impl Entity for TestView {
        type Event = ();
    }

    impl View for TestView {
        fn render(&self, _: &AppContext) -> Box<dyn Element> {
            Empty::new().finish()
        }

        fn ui_name() -> &'static str {
            "TestView"
        }
    }

    impl TypedActionView for TestView {
        type Action = ();
    }

    App::test((), |mut app| async move {
        let (window_1_id, root_1) =
            app.add_window(WindowStyle::NotStealFocus, |_| TestView::default());
        let (window_2_id, _) = app.add_window(WindowStyle::NotStealFocus, |_| TestView::default());

        let parent = root_1.update(&mut app, |_, ctx| {
            ctx.add_typed_action_view(|_| TestView { value: 1 })
        });

        let structural_child = parent.update(&mut app, |_, ctx| {
            ctx.add_typed_action_view(|_| TestView { value: 2 })
        });

        let parent_id = parent.id();
        let child_id = structural_child.id();

        app.read(|ctx| {
            assert!(
                ctx.windows[&window_1_id].views.contains_key(&child_id),
                "child should initially be in window 1"
            );
        });

        let transferred =
            app.update(|ctx| ctx.transfer_view_tree_to_window(parent_id, window_1_id, window_2_id));

        assert!(
            transferred.contains(&parent_id),
            "parent should be in transferred list"
        );
        assert!(
            transferred.contains(&child_id),
            "structural child should be in transferred list"
        );

        app.read(|ctx| {
            assert!(
                ctx.windows[&window_2_id].views.contains_key(&parent_id),
                "parent should be in window 2"
            );
            assert!(
                ctx.windows[&window_2_id].views.contains_key(&child_id),
                "structural child should be in window 2"
            );
            assert!(
                !ctx.windows[&window_1_id].views.contains_key(&parent_id),
                "parent should no longer be in window 1"
            );
            assert!(
                !ctx.windows[&window_1_id].views.contains_key(&child_id),
                "structural child should no longer be in window 1"
            );
        });

        structural_child.read(&app, |v, _| {
            assert_eq!(v.value, 2, "structural child data should be preserved");
        });
    });
}

#[test]
fn test_transfer_structural_grandchildren_follows_transitively() {
    #[derive(Default)]
    struct TestView {
        value: usize,
    }

    impl Entity for TestView {
        type Event = ();
    }

    impl View for TestView {
        fn render(&self, _: &AppContext) -> Box<dyn Element> {
            Empty::new().finish()
        }

        fn ui_name() -> &'static str {
            "TestView"
        }
    }

    impl TypedActionView for TestView {
        type Action = ();
    }

    App::test((), |mut app| async move {
        let (window_1_id, root_1) =
            app.add_window(WindowStyle::NotStealFocus, |_| TestView::default());
        let (window_2_id, _) = app.add_window(WindowStyle::NotStealFocus, |_| TestView::default());

        let parent = root_1.update(&mut app, |_, ctx| {
            ctx.add_typed_action_view(|_| TestView { value: 1 })
        });

        let child = parent.update(&mut app, |_, ctx| {
            ctx.add_typed_action_view(|_| TestView { value: 2 })
        });

        let grandchild = child.update(&mut app, |_, ctx| {
            ctx.add_typed_action_view(|_| TestView { value: 3 })
        });

        let parent_id = parent.id();
        let child_id = child.id();
        let grandchild_id = grandchild.id();

        let transferred =
            app.update(|ctx| ctx.transfer_view_tree_to_window(parent_id, window_1_id, window_2_id));

        assert!(
            transferred.contains(&parent_id),
            "parent should be transferred"
        );
        assert!(
            transferred.contains(&child_id),
            "child should be transferred"
        );
        assert!(
            transferred.contains(&grandchild_id),
            "grandchild should be transferred transitively"
        );

        app.read(|ctx| {
            assert!(ctx.windows[&window_2_id].views.contains_key(&grandchild_id));
            assert!(!ctx.windows[&window_1_id].views.contains_key(&grandchild_id));
        });

        grandchild.read(&app, |v, _| {
            assert_eq!(v.value, 3, "grandchild data should be preserved");
        });
    });
}

#[test]
fn test_transfer_structural_children_does_not_move_unrelated_views() {
    #[derive(Default)]
    struct TestView;

    impl Entity for TestView {
        type Event = ();
    }

    impl View for TestView {
        fn render(&self, _: &AppContext) -> Box<dyn Element> {
            Empty::new().finish()
        }

        fn ui_name() -> &'static str {
            "TestView"
        }
    }

    impl TypedActionView for TestView {
        type Action = ();
    }

    App::test((), |mut app| async move {
        let (window_1_id, root_1) = app.add_window(WindowStyle::NotStealFocus, |_| TestView);
        let (window_2_id, _) = app.add_window(WindowStyle::NotStealFocus, |_| TestView);

        let parent = root_1.update(&mut app, |_, ctx| ctx.add_typed_action_view(|_| TestView));

        let structural_child =
            parent.update(&mut app, |_, ctx| ctx.add_typed_action_view(|_| TestView));

        let unrelated = app.add_view(window_1_id, |_| TestView);

        let parent_id = parent.id();
        let child_id = structural_child.id();
        let unrelated_id = unrelated.id();

        let transferred =
            app.update(|ctx| ctx.transfer_view_tree_to_window(parent_id, window_1_id, window_2_id));

        assert!(
            transferred.contains(&parent_id),
            "parent should be transferred"
        );
        assert!(
            transferred.contains(&child_id),
            "structural child should be transferred"
        );
        assert!(
            !transferred.contains(&unrelated_id),
            "unrelated view should NOT be transferred"
        );

        app.read(|ctx| {
            assert!(
                ctx.windows[&window_1_id].views.contains_key(&unrelated_id),
                "unrelated view should remain in window 1"
            );
            assert!(
                !ctx.windows[&window_2_id].views.contains_key(&unrelated_id),
                "unrelated view should NOT be in window 2"
            );
        });
    });
}
