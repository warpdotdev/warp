use super::*;

/// Test that a weak handle correctly fails to upgrade after the last strong
/// handle is dropped during event processing.
///
/// When the sole `ModelHandle` to a model is dropped inside an event callback,
/// the model should be cleaned up by `remove_dropped_items` and any
/// `WeakModelHandle::upgrade` attempted afterward must return `None`.
#[test]
fn test_weak_handle_fails_after_last_strong_handle_dropped_in_event_callback() {
    struct Emitter;

    impl Entity for Emitter {
        type Event = ();
    }

    struct Subscriber {
        emitter: Option<ModelHandle<Emitter>>,
        emitter_weak: Option<WeakModelHandle<Emitter>>,
    }

    impl Entity for Subscriber {
        type Event = ();
    }

    App::test((), |mut app| async move {
        let subscriber = app.add_model(|_| Subscriber {
            emitter: None,
            emitter_weak: None,
        });

        // Create the emitter in a block so the test-scope handle is dropped,
        // leaving the subscriber as the sole strong-handle holder.
        {
            let emitter = app.add_model(|_| Emitter);
            let emitter_weak = emitter.downgrade();
            let emitter_for_subscribe = emitter.clone();

            subscriber.update(&mut app, |sub, ctx| {
                sub.emitter = Some(emitter.clone());
                sub.emitter_weak = Some(emitter_weak);

                // In the event callback, drop the only remaining strong
                // handle and immediately try to upgrade the weak handle.
                ctx.subscribe_to_model(
                    &emitter_for_subscribe,
                    |sub: &mut Subscriber, _event, ctx| {
                        sub.emitter.take();

                        // The weak upgrade should fail because the last
                        // strong handle was just dropped.
                        let upgrade_result = sub.emitter_weak.as_ref().unwrap().upgrade(ctx);
                        assert!(
                            upgrade_result.is_none(),
                            "weak upgrade should return None immediately after \
                             the last strong handle is dropped"
                        );
                    },
                );
            });
        }

        // Trigger the emit from within the subscriber, since it holds the
        // only handle to the emitter.
        subscriber.update(&mut app, |sub, ctx| {
            if let Some(handle) = sub.emitter.clone() {
                handle.update(ctx, |_emitter, ctx| {
                    ctx.emit(());
                });
            }
        });

        // After effect processing, the emitter should have been removed from
        // the store. The weak handle must fail to upgrade.
        subscriber.read(&app, |sub, ctx| {
            let upgrade_result = sub.emitter_weak.as_ref().unwrap().upgrade(ctx);
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
    use crate::platform::WindowStyle;

    struct TargetView;

    impl Entity for TargetView {
        type Event = ();
    }

    impl super::super::View for TargetView {
        fn render(&self, _: &AppContext) -> RenderOutput {
            crate::test::empty_render_output()
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
                ctx.subscribe_to_model(&trigger_clone, |orch: &mut Orchestrator, _event, ctx| {
                    orch.target_view.take();

                    let upgrade_result = orch.target_weak.as_ref().unwrap().upgrade(ctx);
                    assert!(
                        upgrade_result.is_none(),
                        "weak view upgrade should return None immediately \
                             after the last strong handle is dropped"
                    );
                });
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
