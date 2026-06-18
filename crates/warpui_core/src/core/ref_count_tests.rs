use super::*;

/// Test that a weak handle correctly fails to upgrade after the last strong
/// handle is dropped during event processing.
///
/// When the sole `ModelHandle` to a model is dropped inside an event callback,
/// the model should be cleaned up by `remove_dropped_items` and any
/// `WeakModelHandle::upgrade` attempted afterward must return `None`.
#[test]
fn test_weak_handle_fails_after_last_strong_handle_dropped_in_event_callback() {
    /// The model whose events trigger the callback.
    struct Trigger;

    impl Entity for Trigger {
        type Event = ();
    }

    /// The model that is being dropped and weak-upgraded.
    struct Target;

    impl Entity for Target {
        type Event = ();
    }

    struct Subscriber {
        target: Option<ModelHandle<Target>>,
        target_weak: Option<WeakModelHandle<Target>>,
    }

    impl Entity for Subscriber {
        type Event = ();
    }

    App::test((), |mut app| async move {
        let trigger = app.add_model(|_| Trigger);
        let subscriber = app.add_model(|_| Subscriber {
            target: None,
            target_weak: None,
        });

        // Create the target in a block so the test-scope handle is dropped,
        // leaving the subscriber as the sole strong-handle holder.
        {
            let target = app.add_model(|_| Target);
            let target_weak = target.downgrade();

            subscriber.update(&mut app, |sub, ctx| {
                sub.target = Some(target);
                sub.target_weak = Some(target_weak);

                // When the trigger fires, drop the only remaining strong
                // handle to `target` and immediately try to upgrade the
                // weak handle.
                ctx.subscribe_to_model(&trigger, |sub: &mut Subscriber, _, _event, ctx| {
                    sub.target.take();

                    let upgrade_result = sub.target_weak.as_ref().unwrap().upgrade(ctx);
                    assert!(
                        upgrade_result.is_none(),
                        "weak upgrade should return None immediately after \
                             the last strong handle is dropped"
                    );
                });
            });
        }

        // Fire the trigger.
        trigger.update(&mut app, |_, ctx| ctx.emit(()));

        // After effect processing, the target should have been removed from
        // the store. The weak handle must fail to upgrade.
        subscriber.read(&app, |sub, ctx| {
            let upgrade_result = sub.target_weak.as_ref().unwrap().upgrade(ctx);
            assert!(
                upgrade_result.is_none(),
                "weak upgrade should return None after last strong handle was dropped"
            );
        });
    });
}

/// Same as the model test above, but for views: a weak view handle must fail
/// to upgrade after the last strong `ViewHandle` is dropped during event
/// processing.
#[test]
fn test_weak_view_handle_fails_after_last_strong_handle_dropped_in_event_callback() {
    use crate::elements::Empty;
    use crate::platform::WindowStyle;

    struct TargetView;

    impl Entity for TargetView {
        type Event = ();
    }

    impl super::super::View for TargetView {
        fn render(&self, _: &AppContext) -> Box<dyn crate::Element> {
            Empty::new().finish()
        }

        fn ui_name() -> &'static str {
            "TargetView"
        }
    }

    impl super::super::TypedActionView for TargetView {
        type Action = ();
    }

    /// A model that triggers the drop-then-upgrade sequence for a view handle.
    struct Orchestrator {
        target_view: Option<ViewHandle<TargetView>>,
        target_weak: Option<WeakViewHandle<TargetView>>,
    }

    impl Entity for Orchestrator {
        type Event = ();
    }

    /// A separate model whose event triggers the orchestrator's callback.
    struct Trigger;

    impl Entity for Trigger {
        type Event = ();
    }

    App::test((), |mut app| async move {
        let orchestrator = app.add_model(|_| Orchestrator {
            target_view: None,
            target_weak: None,
        });

        let trigger = app.add_model(|_| Trigger);

        // Create a window, then add the target view to it. Using add_view
        // (not add_window) so the window doesn't hold a root-view handle —
        // the orchestrator will be the sole strong-handle holder.
        let (window_id, _root) = app.add_window(WindowStyle::NotStealFocus, |_| TargetView);
        {
            let target = app.add_view(window_id, |_| TargetView);
            let target_weak = target.downgrade();
            let trigger_clone = trigger.clone();

            orchestrator.update(&mut app, |orch, ctx| {
                orch.target_view = Some(target.clone());
                orch.target_weak = Some(target_weak);

                // When the trigger fires, drop the last strong ViewHandle
                // and immediately try to upgrade the weak handle.
                ctx.subscribe_to_model(
                    &trigger_clone,
                    |orch: &mut Orchestrator, _, _event, ctx| {
                        orch.target_view.take();

                        let upgrade_result = orch.target_weak.as_ref().unwrap().upgrade(ctx);
                        assert!(
                            upgrade_result.is_none(),
                            "weak view upgrade should return None immediately \
                             after the last strong handle is dropped"
                        );
                    },
                );
            });
        }

        // Fire the trigger.
        trigger.update(&mut app, |_, ctx| ctx.emit(()));

        // Post-processing: the weak handle should still fail.
        orchestrator.read(&app, |orch, ctx| {
            let upgrade_result = orch.target_weak.as_ref().unwrap().upgrade(ctx);
            assert!(
                upgrade_result.is_none(),
                "weak view upgrade should return None after effect processing"
            );
        });
    });
}
