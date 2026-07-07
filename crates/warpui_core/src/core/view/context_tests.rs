use crate::elements::Empty;
use crate::platform::WindowStyle;
use crate::{App, AppContext, Element, Entity, TypedActionView};

#[test]
fn test_spawn_from_view() {
    #[derive(Default)]
    struct View {
        count: usize,
    }

    impl Entity for View {
        type Event = ();
    }

    impl super::View for View {
        fn render<'a>(&self, _: &AppContext) -> Box<dyn Element> {
            Empty::new().finish()
        }

        fn ui_name() -> &'static str {
            "View"
        }
    }

    impl TypedActionView for View {
        type Action = ();
    }

    App::test((), |mut app| async move {
        let (_, handle) = app.add_window(WindowStyle::NotStealFocus, |_| View::default());

        let (tx, rx) = futures::channel::oneshot::channel();
        handle.update(&mut app, move |_, c| {
            c.spawn(async { 7 }, move |me, output, _| {
                tx.send(()).unwrap();
                me.count = output;
            })
        });
        rx.await.unwrap();

        let (tx, rx) = futures::channel::oneshot::channel();
        handle.read(&app, |view, _| assert_eq!(view.count, 7));
        handle.update(&mut app, move |_, c| {
            c.spawn(async { 14 }, move |me, output, _| {
                tx.send(()).unwrap();
                me.count = output;
            })
        });
        rx.await.unwrap();
        handle.read(&app, |view, _| assert_eq!(view.count, 14));
    });
}

#[ignore]
#[test]
fn test_spawn_abortable_from_view() {
    #[derive(Debug, Default, PartialEq)]
    enum SpawnedOutcome {
        #[default]
        NotStarted,
        Aborted,
        Resolved {
            value: usize,
        },
    }

    #[derive(Default)]
    struct View {
        spawned_outcome: SpawnedOutcome,
    }

    impl Entity for View {
        type Event = ();
    }

    impl super::View for View {
        fn render<'a>(&self, _: &AppContext) -> Box<dyn Element> {
            Empty::new().finish()
        }

        fn ui_name() -> &'static str {
            "View"
        }
    }

    impl TypedActionView for View {
        type Action = ();
    }

    App::test((), |mut app| async move {
        let (_, handle) = app.add_window(WindowStyle::NotStealFocus, |_| View::default());

        let (tx, rx) = futures::channel::oneshot::channel();
        handle.update(&mut app, |_, c| {
            c.spawn_abortable(
                async { 7 },
                move |me, output, _| {
                    tx.send(()).unwrap();
                    me.spawned_outcome = SpawnedOutcome::Resolved { value: output }
                },
                |_, _| {},
            )
        });
        rx.await.unwrap();

        handle.read(&app, |view, _| {
            assert_eq!(view.spawned_outcome, SpawnedOutcome::Resolved { value: 7 })
        });

        let (tx, rx) = futures::channel::oneshot::channel();
        handle.update(&mut app, move |_, c| {
            let abort_handle = c.spawn_abortable(
                async { 7 },
                |_, _, _| {},
                move |me, _| {
                    me.spawned_outcome = SpawnedOutcome::Aborted;
                    tx.send(()).unwrap();
                },
            );
            abort_handle.abort();
        });

        rx.await.unwrap();

        // The call future passed to `spawn_abortable` was successfully aborted.
        handle.read(&app, |view, _| {
            assert_eq!(view.spawned_outcome, SpawnedOutcome::Aborted)
        });
    });
}

#[test]
fn test_view_spawner() {
    #[derive(Default)]
    struct View {
        count: usize,
    }

    impl Entity for View {
        type Event = ();
    }

    impl super::View for View {
        fn render<'a>(&self, _: &AppContext) -> Box<dyn Element> {
            Empty::new().finish()
        }

        fn ui_name() -> &'static str {
            "View"
        }
    }

    impl TypedActionView for View {
        type Action = ();
    }

    App::test((), |mut app| async move {
        let (_, handle) = app.add_window(WindowStyle::NotStealFocus, |_| View::default());

        let spawner = handle.update(&mut app, |_, ctx| ctx.spawner());

        // Test a single spawned task.
        let result = spawner
            .spawn(|view, ctx| {
                view.count += 42;
                ctx.notify();
                view.count
            })
            .await
            .unwrap();

        assert_eq!(result, 42);
        handle.read(&app, |view, _| assert_eq!(view.count, 42));

        // Test multiple spawned tasks.
        let task1 = spawner.spawn(|view, _| {
            view.count *= 2;
            view.count
        });

        let task2 = spawner.spawn(|view, _| {
            view.count += 10;
            view.count
        });

        let (result1, result2) = futures::future::join(task1, task2).await;

        // Note: The exact final value depends on task execution order but both tasks should succeed.
        assert!(result1.is_ok());
        assert!(result2.is_ok());

        handle.read(&app, |view, _| {
            assert!(view.count > 42);
        });
    });
}
