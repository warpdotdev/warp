use std::{
    cell::RefCell,
    pin::Pin,
    rc::Rc,
    sync::atomic::AtomicBool,
    sync::Arc,
    task::{Context, Poll},
};

use crate::r#async::Timer;
use anyhow::Result;
use futures_util::{stream, Stream};
use parking_lot::Mutex;

use super::*;
use crate::{
    elements::*,
    keymap::{macros::*, Keystroke},
};

#[path = "transfer_view_tests.rs"]
mod transfer_view_tests;

#[test]
fn test_subscribe_and_emit_from_model() {
    #[derive(Default)]
    struct Model {
        events: Vec<usize>,
    }

    impl Entity for Model {
        type Event = usize;
    }

    App::test((), |mut app| async move {
        let app = &mut app;

        let handle_1 = app.add_model(|_| Model::default());
        let handle_2 = app.add_model(|_| Model::default());
        let handle_2b = handle_2.clone();

        handle_1.update(app, |_, c| {
            c.subscribe_to_model(&handle_2, move |model: &mut Model, event, c| {
                model.events.push(*event);

                c.subscribe_to_model(&handle_2b, |model, event, _| {
                    model.events.push(*event * 2);
                });
            });
        });

        handle_2.update(app, |_, c| c.emit(7));
        handle_1.read(app, |model, _| assert_eq!(model.events, vec![7]));

        handle_2.update(app, |_, c| c.emit(5));
        handle_1.read(app, |model, _| assert_eq!(model.events, vec![7, 10, 5]));
    });
}

#[test]
fn test_observe_and_notify_from_model() {
    #[derive(Default)]
    struct Model {
        count: usize,
        events: Vec<usize>,
    }

    impl Entity for Model {
        type Event = ();
    }

    App::test((), |mut app| async move {
        let app = &mut app;
        let handle_1 = app.add_model(|_| Model::default());
        let handle_2 = app.add_model(|_| Model::default());
        let handle_2b = handle_2.clone();

        handle_1.update(app, |_, c| {
            c.observe(&handle_2, move |model, observed, c| {
                model.events.push(observed.as_ref(c).count);
                c.observe(&handle_2b, |model, observed, c| {
                    model.events.push(observed.as_ref(c).count * 2);
                });
            });
        });

        handle_2.update(app, |model, c| {
            model.count = 7;
            c.notify()
        });
        handle_1.read(app, |model, _| assert_eq!(model.events, vec![7]));

        handle_2.update(app, |model, c| {
            model.count = 5;
            c.notify()
        });
        handle_1.read(app, |model, _| assert_eq!(model.events, vec![7, 10, 5]))
    })
}

#[test]
fn test_observe_model_from_app() {
    #[derive(Default)]
    struct Model {
        count: usize,
    }

    impl Entity for Model {
        type Event = ();
    }

    App::test((), |mut app| async move {
        let app = &mut app;
        let handle_1 = app.add_model(|_| Model::default());
        let handle_2 = app.add_model(|_| Model::default());

        let handle_1_clone = handle_1.clone();
        app.update(|ctx| {
            ctx.observe_model(&handle_2, move |model_2, ctx| {
                let model_2_count = model_2.as_ref(ctx).count;
                handle_1_clone.update(ctx, |model_1, c| {
                    // Set the count of model 1 to that of model 2.
                    model_1.count = model_2_count;
                    c.notify();
                });
            });
        });

        handle_2.update(app, |model_2, c| {
            model_2.count = 7;
            c.notify()
        });

        // Model 1's count should match that of model 2.
        handle_1.read(app, |model_1, _| assert_eq!(model_1.count, 7));
    })
}

#[test]
fn test_subscribe_to_model_from_app() {
    #[derive(Default)]
    struct Model {
        val: usize,
    }

    impl Entity for Model {
        type Event = usize;
    }

    App::test((), |mut app| async move {
        let app: &mut App = &mut app;
        let model = app.add_model(|_| Model::default());

        app.update(|ctx| {
            ctx.subscribe_to_model(&model, move |model, event, ctx| {
                model.update(ctx, |model, _ctx| {
                    model.val = *event;
                })
            });
        });

        app.update(|ctx| {
            model.update(ctx, |_view, ctx| {
                ctx.emit(42);
            })
        });

        model.read(app, |model, _| assert_eq!(model.val, 42));
    })
}

#[test]
fn test_subscribe_to_view_from_app() {
    #[derive(Default)]
    struct View {
        val: usize,
    }

    impl Entity for View {
        type Event = usize;
    }

    impl super::View for View {
        fn render(&self, _: &AppContext) -> Box<dyn Element> {
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
        let app: &mut App = &mut app;
        let (_, view) = app.add_window(WindowStyle::NotStealFocus, |_ctx| View::default());

        app.update(|ctx| {
            ctx.subscribe_to_view(&view, move |view, event, ctx| {
                view.update(ctx, |view, _ctx| {
                    view.val = *event;
                })
            });
        });

        app.update(|ctx| {
            view.update(ctx, |_view, ctx| {
                ctx.emit(42);
            })
        });

        view.read(app, |view, _| assert_eq!(view.val, 42));
    })
}

#[test]
fn test_subscribe_to_view_from_model() {
    #[derive(Default)]
    struct Model {
        val: usize,
    }

    impl Entity for Model {
        type Event = ();
    }

    #[derive(Default)]
    struct View;

    impl Entity for View {
        type Event = usize;
    }

    impl super::View for View {
        fn render(&self, _: &AppContext) -> Box<dyn Element> {
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
        let app = &mut app;

        let (_, view_handle) = app.add_window(WindowStyle::NotStealFocus, |_ctx| View);

        let model_handle = app.add_model(|ctx| {
            let model = Model::default();
            ctx.subscribe_to_view(&view_handle, |model: &mut Model, event, _ctx| {
                model.val = *event;
            });
            model
        });

        app.update(|ctx| {
            view_handle.update(ctx, |_view, ctx| {
                ctx.emit(42);
            })
        });

        // The model value should match the emitted value if the subscription was successful.
        model_handle.read(app, |model_1, _| assert_eq!(model_1.val, 42));
    })
}

#[test]
fn test_spawn_from_model() {
    #[derive(Default)]
    struct Model {
        count: usize,
    }

    impl Entity for Model {
        type Event = ();
    }

    App::test((), |mut app| async move {
        let handle = app.add_model(|_| Model::default());
        handle
            .update(&mut app, |_, c| {
                c.spawn_local(async { 7 }, |model, output, _| {
                    model.count = output;
                })
            })
            .await;
        handle.read(&app, |model, _| assert_eq!(model.count, 7));

        let (tx, rx) = futures::channel::oneshot::channel();
        handle.update(&mut app, |_, c| {
            c.spawn(async { 14 }, move |model, output, _| {
                model.count = output;
                tx.send(()).unwrap();
            })
        });

        rx.await.unwrap();
        handle.read(&app, |model, _| assert_eq!(model.count, 14));
    });
}

#[ignore]
#[test]
fn test_spawn_abortable_from_model() {
    #[derive(Debug, Default, PartialEq)]
    enum SpawnedOutcome {
        #[default]
        NotCompleted,
        Aborted,
        Resolved {
            value: usize,
        },
    }

    #[derive(Default)]
    struct Model {
        spawned_outcome: SpawnedOutcome,
    }

    impl Entity for Model {
        type Event = ();
    }

    App::test((), |mut app| async move {
        let handle = app.add_model(|_| Model::default());

        let (tx, rx) = futures::channel::oneshot::channel();
        handle.update(&mut app, |_, c| {
            c.spawn_abortable(
                async { 14 },
                move |model, _, _| {
                    model.spawned_outcome = SpawnedOutcome::Resolved { value: 14 };
                    tx.send(()).unwrap();
                },
                |_, _| {},
            )
        });
        rx.await.unwrap();

        // The future should be successfully resolved since we never called abort.
        handle.read(&app, |model, _| {
            assert_eq!(
                model.spawned_outcome,
                SpawnedOutcome::Resolved { value: 14 }
            )
        });

        let (tx, rx) = futures::channel::oneshot::channel();
        handle.update(&mut app, |_, c| {
            let spawned_future = c.spawn_abortable(
                async { 14 },
                move |_, _, _| {},
                move |model, _| {
                    model.spawned_outcome = SpawnedOutcome::Aborted;
                    tx.send(()).unwrap();
                },
            );

            spawned_future.abort();
        });
        rx.await.unwrap();

        // The future should be successfully resolved since we never called abort.
        handle.read(&app, |model, _| {
            assert_eq!(model.spawned_outcome, SpawnedOutcome::Aborted)
        });
    });
}

#[test]
fn test_spawn_stream_local_from_model() {
    #[derive(Default)]
    struct Model {
        events: Vec<Option<usize>>,
    }

    impl Entity for Model {
        type Event = ();
    }

    App::test((), |mut app| async move {
        let handle = app.add_model(|_| Model::default());
        handle
            .update(&mut app, |_, c| {
                c.spawn_stream_local(
                    stream::iter(vec![1, 2, 3]),
                    |model, output, _| {
                        model.events.push(Some(output));
                    },
                    |model, _| {
                        model.events.push(None);
                    },
                )
                .into_future()
            })
            .await;

        handle.read(&app, |model, _| {
            assert_eq!(model.events, [Some(1), Some(2), Some(3), None])
        });
    })
}

#[test]
fn test_global_action_from_model() {
    struct Model;

    impl Entity for Model {
        type Event = ();
    }

    struct Argument(String);

    App::test((), |mut app| async move {
        let handled = Rc::new(AtomicBool::new(false));
        let handled_writer = handled.clone();
        app.add_global_action("global_action", move |arg: &Argument, _| {
            handled_writer.store(true, Ordering::SeqCst);
            assert_eq!(arg.0, "global_argument");
        });

        let model_handle = app.add_model(|_| Model);

        model_handle.update(&mut app, |_, ctx| {
            ctx.dispatch_global_action("global_action", Argument("global_argument".into()));
        });

        assert!(handled.load(Ordering::SeqCst));
    })
}

#[test]
fn test_view_handles() {
    struct View {
        other: Option<ViewHandle<View>>,
        events: Vec<String>,
    }

    impl Entity for View {
        type Event = usize;
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

    impl View {
        fn new(other: Option<ViewHandle<View>>, ctx: &mut ViewContext<Self>) -> Self {
            if let Some(other) = other.as_ref() {
                ctx.subscribe_to_view(other, |me, _, event, _| {
                    me.events.push(format!("observed event {event}"));
                });
            }
            Self {
                other,
                events: Vec::new(),
            }
        }
    }

    App::test((), |mut app| async move {
        let app = &mut app;

        let (window_id, _) = app.add_window(WindowStyle::NotStealFocus, |ctx| View::new(None, ctx));
        let handle_1 = app.add_view(window_id, |ctx| View::new(None, ctx));
        let handle_2 = app.add_view(window_id, |ctx| View::new(Some(handle_1.clone()), ctx));
        app.read(|ctx| {
            assert_eq!(ctx.windows[&window_id].views.len(), 3);
        });

        handle_1.update(app, |view, ctx| {
            view.events.push("updated".into());
            ctx.emit(1);
            ctx.emit(2);
        });
        handle_1.read(app, |view, _| {
            assert_eq!(view.events, vec!["updated".to_string()]);
        });
        handle_2.read(app, |view, _| {
            assert_eq!(
                view.events,
                vec![
                    "observed event 1".to_string(),
                    "observed event 2".to_string(),
                ]
            );
        });

        handle_2.update(app, |view, _| {
            drop(handle_1);
            view.other.take();
        });

        app.update(|ctx| {
            assert_eq!(ctx.windows[&window_id].views.len(), 2);
            assert!(ctx.subscriptions.is_empty());
            assert!(ctx.observations.is_empty());
        });
    })
}

#[test]
fn test_view_handles_from_typed_action_views() {
    struct View {
        other: Option<ViewHandle<View>>,
        events: Vec<String>,
    }

    impl Entity for View {
        type Event = usize;
    }

    impl super::View for View {
        fn render(&self, _: &AppContext) -> Box<dyn Element> {
            Empty::new().finish()
        }

        fn ui_name() -> &'static str {
            "View"
        }
    }

    impl TypedActionView for View {
        type Action = ();
    }

    impl View {
        fn new(other: Option<ViewHandle<View>>, ctx: &mut ViewContext<Self>) -> Self {
            if let Some(other) = other.as_ref() {
                ctx.subscribe_to_view(other, |me, _, event, _| {
                    me.events.push(format!("observed event {event}"));
                });
            }
            Self {
                other,
                events: Vec::new(),
            }
        }
    }

    App::test((), |mut app| async move {
        let (window_id, _) = app.add_window(WindowStyle::NotStealFocus, |ctx| View::new(None, ctx));

        // Using a scoped block here to test the reference counting of ViewHandles
        // We return the parent handle so that it remains in-scope, but allow the child
        // handle to be dropped
        let parent_handle = {
            let child = app.add_typed_action_view(window_id, |ctx| View::new(None, ctx));
            let parent =
                app.add_typed_action_view(window_id, |ctx| View::new(Some(child.clone()), ctx));
            app.read(|ctx| {
                assert_eq!(ctx.windows[&window_id].views.len(), 3);
            });

            child.update(&mut app, |view, ctx| {
                view.events.push("updated".into());
                ctx.emit(1);
                ctx.emit(2);
            });

            child.read(&app, |view, _| {
                assert_eq!(view.events, vec!["updated".to_owned()]);
            });
            parent.read(&app, |view, _| {
                assert_eq!(
                    view.events,
                    vec!["observed event 1".to_owned(), "observed event 2".to_owned()]
                );
            });

            // Return the parent handle, allowing the child handle to go out of scope
            parent
        };

        // Remove the child handle from the parent, which removes it completely
        parent_handle.update(&mut app, |view, _| {
            view.other.take();
        });

        app.update(|ctx| {
            assert_eq!(ctx.windows[&window_id].views.len(), 2);
            assert!(ctx.subscriptions.is_empty());
            assert!(ctx.observations.is_empty());
        });
    });
}

#[test]
fn test_global_action_from_view() {
    struct View;

    impl Entity for View {
        type Event = ();
    }

    impl super::View for View {
        fn render(&self, _: &AppContext) -> Box<dyn Element> {
            Empty::new().finish()
        }

        fn ui_name() -> &'static str {
            "View"
        }
    }

    impl TypedActionView for View {
        type Action = ();
    }

    struct Argument(String);

    App::test((), |mut app| async move {
        let handled = Rc::new(AtomicBool::new(false));
        let handled_writer = handled.clone();
        app.add_global_action("global_action", move |arg: &Argument, _| {
            handled_writer.store(true, Ordering::SeqCst);
            assert_eq!(arg.0, "global_argument");
        });

        let (_, view_handle) = app.add_window(WindowStyle::NotStealFocus, |_| View);
        view_handle.update(&mut app, |_, ctx| {
            ctx.dispatch_global_action("global_action", Argument("global_argument".into()));
        });

        assert!(handled.load(Ordering::SeqCst));
    });
}

#[test]
fn test_subscribe_and_emit_from_view() {
    #[derive(Default)]
    struct View {
        events: Vec<usize>,
    }

    impl Entity for View {
        type Event = usize;
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

    struct Model;

    impl Entity for Model {
        type Event = usize;
    }

    App::test((), |mut app| async move {
        let app = &mut app;

        let (window_id, handle_1) = app.add_window(WindowStyle::NotStealFocus, |_| View::default());
        let handle_2 = app.add_view(window_id, |_| View::default());
        let handle_2b = handle_2.clone();
        let handle_3 = app.add_model(|_| Model);

        handle_1.update(app, |_, c| {
            c.subscribe_to_view(&handle_2, move |me, _, event, c| {
                me.events.push(*event);

                c.subscribe_to_view(&handle_2b, |me, _, event, _| {
                    me.events.push(*event * 2);
                });
            });

            c.subscribe_to_model(&handle_3, |me, _, event, _| {
                me.events.push(*event);
            })
        });

        handle_2.update(app, |_, c| c.emit(7));
        handle_1.read(app, |view, _| assert_eq!(view.events, vec![7]));

        handle_2.update(app, |_, c| c.emit(5));
        handle_1.read(app, |view, _| assert_eq!(view.events, vec![7, 10, 5]));

        handle_3.update(app, |_, c| c.emit(9));
        handle_1.read(app, |view, _| assert_eq!(view.events, vec![7, 10, 5, 9]));
    })
}

#[test]
fn test_dropping_subscribers() {
    struct View;

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

    struct Model;

    impl Entity for Model {
        type Event = ();
    }

    App::test((), |mut app| async move {
        let app = &mut app;

        let (window_id, _) = app.add_window(WindowStyle::NotStealFocus, |_| View);
        let observing_view = app.add_view(window_id, |_| View);
        let emitting_view = app.add_view(window_id, |_| View);
        let observing_model = app.add_model(|_| Model);
        let observed_model = app.add_model(|_| Model);

        observing_view.update(app, |_, ctx| {
            ctx.subscribe_to_view(&emitting_view, |_, _, _, _| {});
            ctx.subscribe_to_model(&observed_model, |_, _, _, _| {});
        });
        observing_model.update(app, |_, ctx| {
            ctx.subscribe_to_model(&observed_model, |_, _, _| {});
        });

        app.update(|_| {
            drop(observing_view);
            drop(observing_model);
        });

        emitting_view.update(app, |_, ctx| ctx.emit(()));
        observed_model.update(app, |_, ctx| ctx.emit(()));
    })
}

#[test]
fn test_observe_and_notify_from_view() {
    #[derive(Default)]
    struct View {
        events: Vec<usize>,
    }

    impl Entity for View {
        type Event = usize;
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

    #[derive(Default)]
    struct Model {
        count: usize,
    }

    impl Entity for Model {
        type Event = ();
    }

    App::test((), |mut app| async move {
        let app = &mut app;
        let (_, view) = app.add_window(WindowStyle::NotStealFocus, |_| View::default());
        let model = app.add_model(|_| Model::default());

        view.update(app, |_, c| {
            c.observe(&model, |me, observed, c| {
                me.events.push(observed.as_ref(c).count)
            });
        });

        model.update(app, |model, c| {
            model.count = 11;
            c.notify();
        });
        view.read(app, |view, _| assert_eq!(view.events, vec![11]));
    })
}

#[test]
fn test_dropping_observers() {
    struct View;

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

    struct Model;

    impl Entity for Model {
        type Event = ();
    }

    App::test((), |mut app| async move {
        let app = &mut app;

        let (window_id, _) = app.add_window(WindowStyle::NotStealFocus, |_| View);
        let observing_view = app.add_view(window_id, |_| View);
        let observing_model = app.add_model(|_| Model);
        let observed_model = app.add_model(|_| Model);

        observing_view.update(app, |_, ctx| {
            ctx.observe(&observed_model, |_, _, _| {});
        });
        observing_model.update(app, |_, ctx| {
            ctx.observe(&observed_model, |_, _, _| {});
        });

        app.update(|_| {
            drop(observing_view);
            drop(observing_model);
        });

        observed_model.update(app, |_, ctx| ctx.notify());
    })
}

#[test]
fn test_focus() {
    #[derive(Default)]
    struct View {
        events: Vec<String>,
    }

    impl Entity for View {
        type Event = String;
    }

    impl super::View for View {
        fn render<'a>(&self, _: &AppContext) -> Box<dyn Element> {
            Empty::new().finish()
        }

        fn ui_name() -> &'static str {
            "View"
        }

        fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
            if focus_ctx.is_self_focused() {
                self.events.push("self focused".into());
                ctx.emit("focused".into());
            }
        }

        fn on_blur(&mut self, blur_ctx: &BlurContext, ctx: &mut ViewContext<Self>) {
            if blur_ctx.is_self_blurred() {
                self.events.push("self blurred".into());
                ctx.emit("blurred".into());
            }
        }
    }

    impl TypedActionView for View {
        type Action = ();
    }

    App::test((), |mut app| async move {
        let app = &mut app;
        let (window_id, view_1) = app.add_window(WindowStyle::NotStealFocus, |_| View::default());
        let view_2 = app.add_view(window_id, |_| View::default());

        view_1.update(app, |_, ctx| {
            ctx.subscribe_to_view(&view_2, |view_1, _, event, _| {
                view_1.events.push(format!("view 2 {event}"));
            });
            ctx.focus(&view_2);
        });

        view_1.update(app, |_, ctx| {
            ctx.focus(&view_1);
        });

        view_1.read(app, |view_1, _| {
            assert_eq!(
                view_1.events,
                [
                    "self focused".to_string(),
                    "self blurred".to_string(),
                    "view 2 focused".to_string(),
                    "self focused".to_string(),
                    "view 2 blurred".to_string(),
                ],
            );
        });

        view_1.update(app, |view_1, ctx| {
            view_1.events.clear();
            ctx.focus(&view_2);
        });

        // Return focus to root view if focused view is removed
        view_1.update(app, |_, _| {
            drop(view_2);
        });

        app.read(|ctx| assert_eq!(ctx.focused_view_id(window_id), Some(view_1.id())));

        view_1.read(app, |view_1, _| {
            assert_eq!(
                view_1.events,
                [
                    "self blurred".to_string(),
                    "view 2 focused".to_string(),
                    "self focused".to_string(),
                ],
            );
        });
    })
}

struct NestedView {
    children: Vec<ViewHandle<NestedView>>,
    name: String,
    events: Rc<RefCell<Vec<String>>>,
    hide_children: bool,
}

impl NestedView {
    fn new(
        name: String,
        children: Vec<ViewHandle<NestedView>>,
        events: Rc<RefCell<Vec<String>>>,
    ) -> Self {
        Self {
            name,
            events,
            children,
            hide_children: false,
        }
    }

    fn set_children(&mut self, children: Vec<ViewHandle<NestedView>>, ctx: &mut ViewContext<Self>) {
        self.children = children;
        ctx.notify();
    }

    fn with_hide_children(mut self, hide_children: bool) -> Self {
        self.hide_children = hide_children;
        self
    }
}

impl Entity for NestedView {
    type Event = String;
}

impl super::View for NestedView {
    fn render<'a>(&self, _: &AppContext) -> Box<dyn Element> {
        if self.hide_children {
            Empty::new().finish()
        } else {
            Flex::column()
                .with_children(
                    self.children
                        .iter()
                        .map(|child| ChildView::new(child).finish()),
                )
                .finish()
        }
    }

    fn ui_name() -> &'static str {
        "View"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, _ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            self.events
                .borrow_mut()
                .push(format!("{} self focused", self.name));
        } else {
            self.events
                .borrow_mut()
                .push(format!("{} child focused", self.name));
        }
    }

    fn on_blur(&mut self, blur_ctx: &BlurContext, ctx: &mut ViewContext<Self>) {
        if blur_ctx.is_self_blurred() {
            self.events
                .borrow_mut()
                .push(format!("{} self blurred", self.name));
            ctx.emit("blurred".into());
        } else {
            self.events
                .borrow_mut()
                .push(format!("{} child blurred", self.name));
        }
    }
}

impl TypedActionView for NestedView {
    type Action = ();
}

#[test]
fn test_nested_focus() {
    App::test((), |mut app| async move {
        // Test that focusing child views call the focus callbacks of the ancestor views.
        // View heirarchy
        // View 1
        // - View 2
        // - View 3
        //  - View 4

        let events: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(vec![]));
        let app = &mut app;
        let (window_id, view_1) = app.add_window(WindowStyle::NotStealFocus, |_| {
            NestedView::new("View 1".to_string(), vec![], events.clone())
        });
        let view_2 = app.add_view(window_id, |_| {
            NestedView::new("View 2".to_string(), vec![], events.clone())
        });
        let view_4 = app.add_view(window_id, |_| {
            NestedView::new("View 4".to_string(), vec![], events.clone())
        });
        let view_3 = app.add_view(window_id, |_| {
            NestedView::new("View 3".to_string(), vec![view_4.clone()], events.clone())
        });

        assert_eq!(events.take(), ["View 1 self focused".to_string(),],);

        view_1.update(app, |view_1, ctx| {
            view_1.set_children(vec![view_2.clone(), view_3.clone()], ctx);
        });

        view_1.update(app, |_view, ctx| {
            ctx.focus(&view_2);
        });

        view_1.update(app, |_, ctx| {
            ctx.focus(&view_1);
        });

        assert_eq!(
            events.take(),
            [
                "View 1 self blurred".to_string(),
                "View 2 self focused".to_string(),
                "View 1 child focused".to_string(),
                "View 2 self blurred".to_string(),
                "View 1 child blurred".to_string(),
                "View 1 self focused".to_string(),
            ],
        );

        view_4.update(app, |_, ctx| {
            ctx.focus(&view_4);
        });

        assert_eq!(
            events.take(),
            [
                "View 1 self blurred".to_string(),
                "View 4 self focused".to_string(),
                "View 3 child focused".to_string(),
                "View 1 child focused".to_string(),
            ],
        );
    });
}

#[test]
fn test_nested_focus_with_unrendered_view() {
    // Test that unrendered views that are created in the context of another view work.
    App::test((), |mut app| async move {
        // Test that focusing child views call the focus callbacks of the ancestor views.
        // In this case, the child view is not rendered, but because it was created by
        // ViewContext.add_typed_action_view, the parent view of it should still receive
        // focus and blur events.
        // View heirarchy
        // View 1
        // - View 2
        let events: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(vec![]));
        let app = &mut app;
        let (_window_id, view_1) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let view_2 = ctx.add_typed_action_view(|_ctx| {
                NestedView::new("View 2".to_string(), vec![], events.clone())
            });
            NestedView::new("View 1".to_string(), vec![view_2], events.clone())
                .with_hide_children(true)
        });

        assert_eq!(events.take(), ["View 1 self focused".to_string(),],);

        view_1.update(app, |view, ctx| {
            ctx.focus(&view.children[0]);
        });

        view_1.update(app, |_, ctx| {
            ctx.focus(&view_1);
        });

        assert_eq!(
            events.take(),
            [
                "View 1 self blurred".to_string(),
                "View 2 self focused".to_string(),
                "View 1 child focused".to_string(),
                "View 2 self blurred".to_string(),
                "View 1 child blurred".to_string(),
                "View 1 self focused".to_string(),
            ],
        );
    })
}

#[test]
fn test_spawn_stream_local_from_view() {
    #[derive(Default)]
    struct View {
        events: Vec<Option<usize>>,
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
        handle
            .update(&mut app, |_, c| {
                c.spawn_stream_local(
                    stream::iter(vec![1, 2, 3]),
                    |me, output, _| {
                        me.events.push(Some(output));
                    },
                    |me, _| {
                        me.events.push(None);
                    },
                )
                .into_future()
            })
            .await;

        handle.read(&app, |view, _| {
            assert_eq!(view.events, [Some(1), Some(2), Some(3), None])
        });
    });
}

#[test]
#[ignore]
fn test_spawn_stream_local_from_view_await_after_closing_window() {
    #[derive(Default)]
    struct View {}

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

    struct TestState {
        closed_window: bool,
        done: bool,
    }

    struct YieldOnceOnClosingWindow {
        state: Arc<Mutex<TestState>>,
    }

    impl YieldOnceOnClosingWindow {
        fn new(state: Arc<Mutex<TestState>>) -> Self {
            YieldOnceOnClosingWindow { state }
        }
    }

    impl Stream for YieldOnceOnClosingWindow {
        type Item = ();

        fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<()>> {
            let state = self.state.lock();
            match *state {
                TestState {
                    closed_window: true,
                    done: false,
                } => Poll::Ready(Some(())),
                TestState { done: true, .. } => Poll::Ready(None),
                _ => Poll::Pending,
            }
        }
    }

    App::test((), |mut app| async move {
        let state = Arc::new(Mutex::new(TestState {
            closed_window: false,
            done: false,
        }));
        let (_, handle) = app.add_window(WindowStyle::NotStealFocus, |_| View::default());
        handle
            .update(&mut app, |_, c| {
                let ret = c
                    .spawn_stream_local(
                        YieldOnceOnClosingWindow::new(state.clone()),
                        |_, _, _| {},
                        |_, _| {},
                    )
                    .into_future();
                c.close_window();
                {
                    let mut state = state.lock();
                    state.closed_window = true;
                }
                ret
            })
            .await;
    });
}

#[test]
fn test_dispatch_action() {
    struct ViewA {
        id: usize,
    }

    impl Entity for ViewA {
        type Event = ();
    }

    impl View for ViewA {
        fn render<'a>(&self, _: &AppContext) -> Box<dyn Element> {
            Empty::new().finish()
        }

        fn ui_name() -> &'static str {
            "View"
        }
    }

    impl TypedActionView for ViewA {
        type Action = ();
    }

    struct ViewB {
        id: usize,
    }

    impl Entity for ViewB {
        type Event = ();
    }

    impl View for ViewB {
        fn render<'a>(&self, _: &AppContext) -> Box<dyn Element> {
            Empty::new().finish()
        }

        fn ui_name() -> &'static str {
            "View"
        }
    }

    struct ActionArg {
        foo: String,
    }

    App::test((), |mut app| async move {
        let actions = Rc::new(RefCell::new(Vec::new()));

        let actions_clone = actions.clone();
        app.add_global_action("action", move |_: &ActionArg, _: &mut AppContext| {
            actions_clone.borrow_mut().push("global a".to_string());
        });

        let actions_clone = actions.clone();
        app.add_global_action("action", move |_: &ActionArg, _: &mut AppContext| {
            actions_clone.borrow_mut().push("global b".to_string());
        });

        let actions_clone = actions.clone();
        app.add_action("action", move |view: &mut ViewA, arg: &ActionArg, _ctx| {
            assert_eq!(arg.foo, "bar");
            actions_clone.borrow_mut().push(format!("{} a", view.id));
            false
        });

        let actions_clone = actions.clone();
        app.add_action("action", move |view: &mut ViewA, _: &ActionArg, _ctx| {
            actions_clone.borrow_mut().push(format!("{} b", view.id));
            view.id == 1
        });

        let actions_clone = actions.clone();
        app.add_action("action", move |view: &mut ViewB, _: &ActionArg, _ctx| {
            actions_clone.borrow_mut().push(format!("{} c", view.id));
            false
        });

        let actions_clone = actions.clone();
        app.add_action("action", move |view: &mut ViewB, _: &ActionArg, _ctx| {
            actions_clone.borrow_mut().push(format!("{} d", view.id));
            false
        });

        let (window_id, view_1) = app.add_window(WindowStyle::NotStealFocus, |_| ViewA { id: 1 });
        let view_2 = app.add_view(window_id, |_| ViewB { id: 2 });
        let view_3 = app.add_view(window_id, |_| ViewA { id: 3 });
        let view_4 = app.add_view(window_id, |_| ViewB { id: 4 });

        app.dispatch_action(
            window_id,
            &[view_1.id(), view_2.id(), view_3.id(), view_4.id()],
            "action",
            ActionArg { foo: "bar".into() },
        );

        assert_eq!(
            *actions.borrow(),
            vec!["4 d", "4 c", "3 b", "3 a", "2 d", "2 c", "1 b", "1 a", "global b", "global a"]
        );

        // Remove view_1, which doesn't propagate the action.
        actions.borrow_mut().clear();
        app.dispatch_action(
            window_id,
            &[view_2.id(), view_3.id(), view_4.id()],
            "action",
            ActionArg { foo: "bar".into() },
        );

        assert_eq!(
            *actions.borrow(),
            vec!["4 d", "4 c", "3 b", "3 a", "2 d", "2 c", "global b", "global a"]
        );

        actions.borrow_mut().clear();
        let actions_clone = actions.clone();
        app.add_action("action", move |view: &mut ViewB, _: &ActionArg, _ctx| {
            actions_clone.borrow_mut().push(format!("{} f", view.id));
            true
        });

        app.dispatch_action(
            window_id,
            &[view_1.id(), view_2.id(), view_3.id(), view_4.id()],
            "action",
            ActionArg { foo: "bar".into() },
        );

        // Ensure the action is only fired on the bottom-most view.
        assert_eq!(
            *actions.borrow(),
            vec!["4 f", "4 d", "4 c", "global b", "global a"]
        );
    })
}

#[test]
fn test_dispatch_typed_action() {
    struct ViewA {
        handled: Vec<String>,
    }

    impl ViewA {
        fn new() -> Self {
            Self {
                handled: Vec::new(),
            }
        }
    }

    impl Entity for ViewA {
        type Event = ();
    }

    impl View for ViewA {
        fn render(&self, _: &AppContext) -> Box<dyn Element> {
            Empty::new().finish()
        }

        fn ui_name() -> &'static str {
            "ViewA"
        }
    }

    #[derive(Debug)]
    struct ViewActionA(String);

    impl TypedActionView for ViewA {
        type Action = ViewActionA;

        fn handle_action(&mut self, action: &ViewActionA, _: &mut ViewContext<Self>) {
            self.handled.push(format!("Handling action: {}", action.0));
        }
    }

    struct ViewB {
        handled: Vec<String>,
    }

    impl ViewB {
        fn new() -> Self {
            Self {
                handled: Vec::new(),
            }
        }
    }

    impl Entity for ViewB {
        type Event = ();
    }

    impl View for ViewB {
        fn render(&self, _: &AppContext) -> Box<dyn Element> {
            Empty::new().finish()
        }

        fn ui_name() -> &'static str {
            "ViewB"
        }
    }

    #[derive(Debug)]
    struct ViewActionB(usize);

    impl TypedActionView for ViewB {
        type Action = ViewActionB;

        fn handle_action(&mut self, action: &ViewActionB, _: &mut ViewContext<Self>) {
            self.handled.push(format!("Handling action: {}", action.0));
        }
    }

    App::test((), |mut app| async move {
        let (window_id, root_view) = app.add_window(WindowStyle::NotStealFocus, |_| ViewA::new());

        // First dispatch a typed action with no handlers and make sure we don't leave pending
        // flushes in the wrong state
        app.dispatch_typed_action(
            window_id,
            &[root_view.id()],
            &ViewActionA("Hello world!".into()),
        );

        app.update(|app| {
            // Pending fluses is 1, not 0 because we are in an update closure
            assert_eq!(1, app.pending_flushes());
        });

        let parent_view = app.add_typed_action_view(window_id, |_| ViewA::new());
        let child_view = app.add_typed_action_view(window_id, |_| ViewB::new());
        let grandchild_view = app.add_typed_action_view(window_id, |_| ViewA::new());

        // Dispatching `ViewActionA` should be handled by the lowest instance of ViewA
        app.dispatch_typed_action(
            window_id,
            &[
                root_view.id(),
                parent_view.id(),
                child_view.id(),
                grandchild_view.id(),
            ],
            &ViewActionA("Hello world!".into()),
        );

        // Only grandchild_view should have a record of handling the action
        grandchild_view.read(&app, |view, _| {
            assert_eq!(view.handled, ["Handling action: Hello world!"]);
        });
        child_view.read(&app, |view, _| {
            assert!(view.handled.is_empty());
        });
        parent_view.read(&app, |view, _| {
            assert!(view.handled.is_empty());
        });

        // Dispatching `ViewActionB` should be handled by the only instance of ViewB
        app.dispatch_typed_action(
            window_id,
            &[
                root_view.id(),
                parent_view.id(),
                child_view.id(),
                grandchild_view.id(),
            ],
            &ViewActionB(10),
        );

        child_view.read(&app, |view, _| {
            assert_eq!(view.handled, ["Handling action: 10"],);
        });

        // Dispatching `ViewActionA` without grandchild_view should be handled by parent_view
        app.dispatch_typed_action(
            window_id,
            &[root_view.id(), parent_view.id(), child_view.id()],
            &ViewActionA("Goodbye!".into()),
        );

        parent_view.read(&app, |view, _| {
            assert_eq!(view.handled, ["Handling action: Goodbye!"],);
        });
    });
}

#[test]
fn test_dispatch_close_window_action() {
    struct ViewA;

    impl Entity for ViewA {
        type Event = ();
    }

    impl View for ViewA {
        fn render<'a>(&self, _: &AppContext) -> Box<dyn Element> {
            Empty::new().finish()
        }

        fn ui_name() -> &'static str {
            "View"
        }
    }

    impl TypedActionView for ViewA {
        type Action = ();
    }

    App::test((), |mut app| async move {
        app.add_action("close_window_action", move |_: &mut ViewA, _: &(), ctx| {
            ctx.close_window();
            true
        });

        let (window_id, view_1) = app.add_window(WindowStyle::NotStealFocus, |_| ViewA);
        app.dispatch_action(window_id, &[view_1.id()], "close_window_action", ());
    })
}

#[test]
fn test_dispatch_keystroke() -> Result<()> {
    struct View {
        id: usize,
        keymap_context: keymap::Context,
        handled_action: bool,
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

        fn keymap_context(&self, _: &AppContext) -> keymap::Context {
            self.keymap_context.clone()
        }
    }

    #[derive(Debug)]
    struct Action(String);

    impl super::TypedActionView for View {
        type Action = Action;

        fn handle_action(&mut self, action: &Action, _: &mut ViewContext<Self>) {
            self.handled_action = true;
            assert_eq!(self.id, 2);
            assert_eq!(action.0, "a");
        }
    }

    impl View {
        fn new(id: usize) -> Self {
            View {
                id,
                keymap_context: keymap::Context::default(),
                handled_action: false,
            }
        }
    }

    App::test((), |mut app| async move {
        let mut view_1 = View::new(1);
        let mut view_2 = View::new(2);
        let mut view_3 = View::new(3);
        view_1.keymap_context.set.insert("a");
        view_2.keymap_context.set.insert("b");
        view_3.keymap_context.set.insert("c");

        let (window_id, view_1) = app.add_window(WindowStyle::NotStealFocus, |_| view_1);
        let view_2 = app.add_typed_action_view(window_id, |_| view_2);
        let view_3 = app.add_typed_action_view(window_id, |_| view_3);

        // This keymap's only binding dispatches the action on View 2, because only that view
        // will have "b" in its context
        let binding = keymap::FixedBinding::new("a", Action("a".into()), id!("b"));
        app.update(|ctx| ctx.register_fixed_bindings(vec![binding]));

        app.dispatch_keystroke(
            window_id,
            &[view_1.id(), view_2.id(), view_3.id()],
            &Keystroke::parse("a")?,
            false,
        )?;

        let handled_action = view_2.read(&app, |view, _| view.handled_action);

        assert!(handled_action);
        Ok(())
    })
}

#[test]
fn test_dispatch_keystroke_typed_action() -> Result<()> {
    struct View {
        id: usize,
        keymap_context: keymap::Context,
    }

    impl Entity for View {
        type Event = ();
    }

    impl super::View for View {
        fn render(&self, _: &AppContext) -> Box<dyn Element> {
            Empty::new().finish()
        }

        fn ui_name() -> &'static str {
            "View"
        }

        fn keymap_context(&self, _: &AppContext) -> keymap::Context {
            self.keymap_context.clone()
        }
    }

    #[derive(Debug)]
    struct Action(String);

    impl TypedActionView for View {
        type Action = Action;

        fn handle_action(&mut self, action: &Action, _: &mut ViewContext<Self>) {
            assert_eq!(self.id, 2);
            assert_eq!(action.0, "a");
        }
    }

    impl View {
        fn new(id: usize, context: &'static str) -> Self {
            let mut instance = View {
                id,
                keymap_context: Default::default(),
            };

            instance.keymap_context.set.insert(context);

            instance
        }
    }

    App::test((), |mut app| async move {
        let (window_id, view_1) = app.add_window(WindowStyle::NotStealFocus, |_| View::new(1, "a"));
        let view_2 = app.add_typed_action_view(window_id, |_| View::new(2, "b"));
        let view_3 = app.add_typed_action_view(window_id, |_| View::new(3, "c"));

        // This keymap's only binding dispatches the action on View 2, because only that view
        // will have "b" in its context
        let binding = keymap::FixedBinding::new("a", Action("a".into()), id!("b"));
        app.update(|ctx| ctx.register_fixed_bindings(vec![binding]));

        let handled = app.dispatch_keystroke(
            window_id,
            &[view_1.id(), view_2.id(), view_3.id()],
            &Keystroke::parse("a")?,
            false,
        )?;

        assert!(handled);
        Ok(())
    })
}

#[test]
fn test_dispatch_custom_action_triggers_typed_action() -> Result<()> {
    struct View {
        id: usize,
        keymap_context: keymap::Context,
        action_count: usize,
        keydown_count: Rc<RefCell<usize>>,
    }

    impl Entity for View {
        type Event = ();
    }

    impl super::View for View {
        fn render(&self, _: &AppContext) -> Box<dyn Element> {
            let keydown_count = self.keydown_count.clone();
            EventHandler::new(Empty::new().finish())
                .on_keydown(move |_, _, _| {
                    *keydown_count.borrow_mut() += 1;
                    DispatchEventResult::StopPropagation
                })
                .finish()
        }

        fn ui_name() -> &'static str {
            "View"
        }

        fn keymap_context(&self, _: &AppContext) -> keymap::Context {
            self.keymap_context.clone()
        }
    }

    #[derive(Debug)]
    struct Action(String);

    impl TypedActionView for View {
        type Action = Action;

        fn handle_action(&mut self, action: &Action, _: &mut ViewContext<Self>) {
            assert_eq!(self.id, 1);
            assert_eq!(action.0, "a");
            self.action_count += 1;
        }
    }

    impl View {
        fn new(id: usize, context: &'static str) -> Self {
            let mut instance = View {
                id,
                keymap_context: Default::default(),
                action_count: 0,
                keydown_count: Rc::new(RefCell::new(0)),
            };

            instance.keymap_context.set.insert(context);
            instance
        }
    }

    App::test((), |mut app| async move {
        let (window_id, view_1) = app.add_window(WindowStyle::NotStealFocus, |_| View::new(1, "a"));
        let custom_tag = 123_isize;
        let binding = keymap::FixedBinding::custom(
            custom_tag,
            Action("a".into()),
            "test custom action",
            id!("a"),
        );
        app.update(|ctx| {
            ctx.register_default_keystroke_triggers_for_custom_actions(|_| {
                Some(Keystroke::parse("ctrl-1").expect("failed to parse keystroke"))
            });
            ctx.register_fixed_bindings(vec![binding]);
        });
        view_1.update(&mut app, |_, ctx| {
            ctx.focus_self();
        });
        app.dispatch_custom_action(custom_tag, window_id);
        assert_eq!(view_1.read(&app, |view, _| view.action_count), 1);
        assert_eq!(view_1.read(&app, |view, _| *view.keydown_count.borrow()), 0);

        app.update(|ctx| {
            ctx.disable_key_bindings(window_id);
        });
        app.dispatch_custom_action(custom_tag, window_id);
        assert_eq!(view_1.read(&app, |view, _| view.action_count), 1);
        assert_eq!(view_1.read(&app, |view, _| *view.keydown_count.borrow()), 1);

        app.update(|ctx| {
            ctx.enable_key_bindings(window_id);
        });
        app.dispatch_custom_action(custom_tag, window_id);
        assert_eq!(view_1.read(&app, |view, _| view.action_count), 2);
        assert_eq!(view_1.read(&app, |view, _| *view.keydown_count.borrow()), 1);

        Ok(())
    })
}

#[test]
fn test_ui_and_window_updates() {
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
        let (window_id, _) = app.add_window(WindowStyle::NotStealFocus, |_| View { count: 3 });
        let view_1 = app.add_view(window_id, |_| View { count: 1 });
        let view_2 = app.add_view(window_id, |_| View { count: 2 });

        // Ensure that registering for UI updates after mutating the app still gives us all the
        // updates.

        let window_invalidations = Rc::new(RefCell::new(Vec::new()));
        let window_invalidations_ = window_invalidations.clone();
        app.on_window_invalidated(window_id, move |window_id, ctx| {
            window_invalidations_
                .borrow_mut()
                .push(ctx.take_all_invalidations_for_window(window_id))
        });

        let view_2_id = view_2.id();
        view_1.update(&mut app, |view, ctx| {
            view.count = 7;
            ctx.notify();
            drop(view_2);
        });

        let invalidation = window_invalidations.borrow_mut().drain(..).next().unwrap();
        assert_eq!(invalidation.updated.len(), 1);
        assert!(invalidation.updated.contains(&view_1.id()));
        assert_eq!(invalidation.removed.len(), 1);
        assert!(invalidation.removed.contains(&view_2_id));

        let view_3 = view_1.update(&mut app, |_, ctx| ctx.add_view(|_| View { count: 8 }));

        let invalidation = window_invalidations.borrow_mut().drain(..).next().unwrap();
        assert_eq!(invalidation.updated.len(), 1);
        assert!(invalidation.updated.contains(&view_3.id()));
        assert!(invalidation.removed.is_empty());

        let (tx, rx) = futures::channel::oneshot::channel();
        view_3.update(&mut app, move |_, ctx| {
            ctx.spawn(async { 9 }, move |me, output, ctx| {
                tx.send(()).unwrap();
                me.count = output;
                ctx.notify();
            })
        });

        rx.await.unwrap();

        let invalidation = window_invalidations.borrow_mut().drain(..).next().unwrap();
        assert_eq!(invalidation.updated.len(), 1);
        assert!(invalidation.updated.contains(&view_3.id()));
        assert!(invalidation.removed.is_empty());
    });
}

#[test]
fn test_finish_pending_tasks() {
    struct View;

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

    struct Model;

    impl Entity for Model {
        type Event = ();
    }

    App::test((), |mut app| async move {
        let model = app.add_model(|_| Model);
        let (_, view) = app.add_window(WindowStyle::NotStealFocus, |_| View);

        model.update(&mut app, |_, ctx| {
            let _ = ctx.spawn(async {}, |_, _, _| {});
            let _ = ctx.spawn(async {}, |_, _, _| {});
            let _ = ctx.spawn_stream_local(
                futures::stream::iter(vec![1, 2, 3]),
                |_, _, _| {},
                |_, _| {},
            );
        });

        view.update(&mut app, |_, ctx| {
            let _ = ctx.spawn(async {}, |_, _, _| {});
            let _ = ctx.spawn(async {}, |_, _, _| {});
            let _ = ctx.spawn_stream_local(
                futures::stream::iter(vec![1, 2, 3]),
                |_, _, _| {},
                |_, _| {},
            );
        });

        app.update(|ctx| {
            assert!(!ctx.task_callbacks.is_empty());
        });
        app.finish_pending_tasks().await;
        app.update(|ctx| {
            assert!(ctx.task_callbacks.is_empty());
        });
        app.finish_pending_tasks().await; // Don't block if there are no tasks
    });
}

#[test]
fn test_key_bindings_for_view() {
    use keymap::FixedBinding;
    struct ViewA;
    struct ViewB;
    impl Entity for ViewA {
        type Event = ();
    }
    impl Entity for ViewB {
        type Event = ();
    }
    impl View for ViewA {
        fn render(&self, _: &AppContext) -> Box<dyn Element> {
            Empty::new().finish()
        }

        fn ui_name() -> &'static str {
            "ViewA"
        }
    }
    impl View for ViewB {
        fn render(&self, _: &AppContext) -> Box<dyn Element> {
            Empty::new().finish()
        }

        fn ui_name() -> &'static str {
            "ViewB"
        }
    }

    struct Container {
        child_a: ViewHandle<ViewA>,
        child_b: ViewHandle<ViewB>,
    }
    impl Entity for Container {
        type Event = ();
    }
    impl View for Container {
        fn render(&self, _: &AppContext) -> Box<dyn Element> {
            let mut stack = Stack::new();
            stack.add_child(ChildView::new(&self.child_a).finish());
            stack.add_child(ChildView::new(&self.child_b).finish());

            stack.finish()
        }
        fn ui_name() -> &'static str {
            "Container"
        }
    }
    impl Container {
        fn new(ctx: &mut ViewContext<Self>) -> Self {
            let child_a = ctx.add_view(|_| ViewA);
            let child_b = ctx.add_view(|_| ViewB);

            Self { child_a, child_b }
        }
    }

    struct Root {
        child: ViewHandle<Container>,
    }
    impl Entity for Root {
        type Event = ();
    }
    impl View for Root {
        fn render(&self, _: &AppContext) -> Box<dyn Element> {
            ChildView::new(&self.child).finish()
        }
        fn ui_name() -> &'static str {
            "Root"
        }
    }

    impl TypedActionView for Root {
        type Action = ();
    }

    impl Root {
        fn new(ctx: &mut ViewContext<Self>) -> Self {
            let child = ctx.add_view(Container::new);

            Self { child }
        }
    }

    #[derive(Debug)]
    enum Action {
        A,
        EmptyRoot,
        BContainer,
        BLeaf,
        C,
        EmptyB,
    }

    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.register_fixed_bindings(vec![
                FixedBinding::new("a", Action::A, id!("Root")),
                FixedBinding::empty("description", Action::EmptyRoot, id!("Root")),
                FixedBinding::new("b", Action::BContainer, id!("Container")),
                FixedBinding::new("b", Action::BLeaf, id!("ViewA")),
                FixedBinding::new("c", Action::C, id!("ViewB")),
                FixedBinding::empty("other description", Action::EmptyB, id!("ViewB")),
            ]);
        });

        /*
            Builds a View Hierarchy that looks like (with bound keys in parentheses)

                    Root (a)
                     |
                 Container (b)
                  |     |
            (b) ViewA ViewB (c)
        */

        let (window_id, _) = app.add_window(WindowStyle::NotStealFocus, Root::new);
        let view_id_a = app.views_of_type::<ViewA>(window_id).unwrap()[0].id();
        let view_id_b = app.views_of_type::<ViewB>(window_id).unwrap()[0].id();

        // Force an update so the child view hierarchy is processed
        app.views_of_type::<Root>(window_id).unwrap()[0].update(&mut app, |_, ctx| ctx.notify());

        app.update(|ctx| {
            // Binding actions available to ViewA should be 'A', 'BLeaf', and 'EmptyRoot', since
            // the key binding on 'b' overlaps between ViewA and Container, we should only get the
            // highest precedence one ('BLeaf')
            let bindings_a = ctx
                .key_bindings_for_view(window_id, view_id_a)
                .into_iter()
                .map(|binding| format!("{:?}", binding.action))
                .collect::<Vec<_>>();

            assert_eq!(bindings_a, ["BLeaf", "EmptyRoot", "A"]);

            // Binding actions available to ViewB should be 'A', 'EmptyRoot', 'BContainer', 'C',
            // and 'EmptyB', as the only binding overlaps are empty, which aren't treated as
            // overlapping each other
            let bindings_b = ctx
                .key_bindings_for_view(window_id, view_id_b)
                .into_iter()
                .map(|binding| format!("{:?}", binding.action))
                .collect::<Vec<_>>();

            assert_eq!(bindings_b, ["EmptyB", "C", "BContainer", "EmptyRoot", "A"]);
        });
    });
}

#[test]
fn test_await_spawned_future() {
    #[derive(Default)]
    struct Model {
        received_spawn_callback: bool,
    }

    impl Entity for Model {
        type Event = ();
    }

    App::test((), |mut app| async move {
        let model = app.add_model(|_| Model::default());

        model
            .update(&mut app, |_, ctx| {
                let future_handle = ctx.spawn(
                    async move { Timer::after(Duration::from_millis(10)).await },
                    |model, _, _| {
                        model.received_spawn_callback = true;
                    },
                );

                ctx.await_spawned_future(future_handle.future_id())
            })
            .await;

        // Assert that the callback was executed. We do this via a model because we aren't
        // guaranteed that the callback will run if the future is awaited.
        model.read(&app, |model, _| {
            assert!(model.received_spawn_callback);
        });
    });
}

#[test]
fn test_editable_binding_getters() {
    use keymap::EditableBinding;
    struct ViewA;
    struct ViewB;
    struct Container {
        child_a: ViewHandle<ViewA>,
        child_b: ViewHandle<ViewB>,
    }
    struct Root {
        child: ViewHandle<Container>,
    }
    impl Entity for ViewA {
        type Event = ();
    }
    impl Entity for ViewB {
        type Event = ();
    }
    impl Entity for Container {
        type Event = ();
    }
    impl Entity for Root {
        type Event = ();
    }
    impl View for ViewA {
        fn render(&self, _: &AppContext) -> Box<dyn Element> {
            Empty::new().finish()
        }
        fn ui_name() -> &'static str {
            "ViewA"
        }
    }
    impl View for ViewB {
        fn render(&self, _: &AppContext) -> Box<dyn Element> {
            Empty::new().finish()
        }
        fn ui_name() -> &'static str {
            "ViewB"
        }
    }
    impl View for Container {
        fn render(&self, _: &AppContext) -> Box<dyn Element> {
            Stack::new()
                .with_child(ChildView::new(&self.child_a).finish())
                .with_child(ChildView::new(&self.child_b).finish())
                .finish()
        }
        fn ui_name() -> &'static str {
            "Container"
        }
    }
    impl Container {
        fn new(ctx: &mut ViewContext<Self>) -> Self {
            let child_a = ctx.add_view(|_| ViewA);
            let child_b = ctx.add_view(|_| ViewB);

            Self { child_a, child_b }
        }
    }
    impl View for Root {
        fn render(&self, _: &AppContext) -> Box<dyn Element> {
            ChildView::new(&self.child).finish()
        }
        fn ui_name() -> &'static str {
            "Root"
        }
    }

    impl TypedActionView for Root {
        type Action = ();
    }

    impl Root {
        fn new(ctx: &mut ViewContext<Self>) -> Self {
            let child = ctx.add_view(Container::new);

            Self { child }
        }
    }
    #[derive(Debug)]
    struct Action(#[allow(dead_code)] String);

    App::test((), |mut app| async move {
        use crate::keymap::macros::*;
        app.update(|ctx| {
            ctx.register_editable_bindings([
                EditableBinding::new("a", "Action a", Action("a".into()))
                    .with_context_predicate(id!("Root")),
                EditableBinding::new("b-container", "Action b in Container", Action("b".into()))
                    .with_context_predicate(id!("Container")),
                EditableBinding::new("b-leaf", "Action b in Leaf", Action("b".into()))
                    .with_context_predicate(id!("ViewA")),
                EditableBinding::new("c", "Action c", Action("c".into()))
                    .with_context_predicate(id!("ViewB")),
            ]);
        });

        /*
            Builds a View Hierarchy that looks like (with bound keys in parentheses)

                    Root (a)
                     |
                 Container (b)
                  |     |
            (b) ViewA ViewB (c)
        */

        let (window_id, _) = app.add_window(WindowStyle::NotStealFocus, Root::new);
        let view_id_a = app.views_of_type::<ViewA>(window_id).unwrap()[0].id();
        let view_id_b = app.views_of_type::<ViewB>(window_id).unwrap()[0].id();

        // Force an update so the child view hierarchy is processed
        app.views_of_type::<Root>(window_id).unwrap()[0].update(&mut app, |_, ctx| ctx.notify());

        // Editable Bindings available to ViewA should be 'a', 'b-container', and 'b-leaf'
        // since those are all tied to context ViewA or its ancestors
        app.update(|ctx| {
            let actions_a = ctx
                .editable_bindings_for_view(window_id, view_id_a)
                .into_iter()
                .map(|action| action.name.to_owned())
                .collect::<Vec<_>>();
            assert_eq!(actions_a, ["b-leaf", "b-container", "a"]);

            // Editable bindings available to ViewB should be 'a', 'b-container', and 'c' since
            // those are all tied to ViewB or its ancestors
            let actions_b = ctx
                .editable_bindings_for_view(window_id, view_id_b)
                .into_iter()
                .map(|action| action.name.to_owned())
                .collect::<Vec<_>>();
            assert_eq!(actions_b, ["c", "b-container", "a"]);

            // Calling `editable_bindings` should get _all_ editable bindings
            let all_actions = ctx
                .editable_bindings()
                .map(|action| action.name.to_owned())
                .collect::<Vec<_>>();
            assert_eq!(all_actions, ["c", "b-leaf", "b-container", "a"]);
        });
    });
}

#[test]
fn test_unsubscribe_from_model_inside_callback() {
    #[derive(Default)]
    struct SubscriberModel {
        events: Vec<usize>,
    }

    impl Entity for SubscriberModel {
        type Event = ();
    }

    struct EmitterModel;

    impl Entity for EmitterModel {
        type Event = usize;
    }

    App::test((), |mut app| async move {
        let emitter = app.add_model(|_| EmitterModel);
        let emitter_clone = emitter.clone();

        let subscriber = app.add_model(|_| SubscriberModel::default());

        // Subscribe, and unsubscribe from inside the callback.
        subscriber.update(&mut app, |_, ctx| {
            let emitter_for_unsubscribe = emitter_clone.clone();
            ctx.subscribe_to_model(&emitter_clone, move |model, event, ctx| {
                model.events.push(*event);
                // Unsubscribe from inside the callback.
                ctx.unsubscribe_from_model(&emitter_for_unsubscribe);
            });
        });

        // First emit should trigger the callback.
        emitter.update(&mut app, |_, ctx| ctx.emit(1));
        subscriber.read(&app, |model, _| {
            assert_eq!(model.events, vec![1], "first event should be received");
        });

        // Second emit should NOT trigger the callback, since we unsubscribed.
        emitter.update(&mut app, |_, ctx| ctx.emit(2));
        subscriber.read(&app, |model, _| {
            assert_eq!(
                model.events,
                vec![1],
                "second event should NOT be received after unsubscribe"
            );
        });
    });
}

#[test]
fn test_unsubscribe_from_model_inside_callback_with_multiple_subscriptions() {
    // This test verifies that when there are multiple subscriptions from A to B,
    // and one callback calls unsubscribe_from_model:
    // 1. All callbacks for the CURRENT event still fire (unsubscribe affects future events).
    // 2. No callbacks fire for FUTURE events.

    #[derive(Default)]
    struct SubscriberModel {
        events: Vec<&'static str>,
    }

    impl Entity for SubscriberModel {
        type Event = ();
    }

    struct EmitterModel;

    impl Entity for EmitterModel {
        type Event = ();
    }

    App::test((), |mut app| async move {
        let emitter = app.add_model(|_| EmitterModel);
        let emitter_clone1 = emitter.clone();
        let emitter_clone2 = emitter.clone();

        let subscriber = app.add_model(|_| SubscriberModel::default());

        // Add two subscriptions from subscriber to emitter.
        subscriber.update(&mut app, |_, ctx| {
            // First subscription: will call unsubscribe.
            let emitter_for_unsubscribe = emitter_clone1.clone();
            ctx.subscribe_to_model(&emitter_clone1, move |model, _, ctx| {
                model.events.push("first");
                ctx.unsubscribe_from_model(&emitter_for_unsubscribe);
            });

            // Second subscription: should still be called for this event.
            ctx.subscribe_to_model(&emitter_clone2, move |model, _, _| {
                model.events.push("second");
            });
        });

        // Emit an event. BOTH callbacks should fire for the current event.
        emitter.update(&mut app, |_, ctx| ctx.emit(()));
        subscriber.read(&app, |model, _| {
            assert_eq!(
                model.events,
                vec!["first", "second"],
                "all existing callbacks should fire for the current event"
            );
        });

        // Emit again. Neither callback should fire since we unsubscribed.
        emitter.update(&mut app, |_, ctx| ctx.emit(()));
        subscriber.read(&app, |model, _| {
            assert_eq!(
                model.events,
                vec!["first", "second"],
                "no callbacks should be invoked for future events after unsubscribe"
            );
        });
    });
}

#[test]
fn test_unsubscribe_then_resubscribe_from_model_inside_callback_keeps_new_subscription() {
    #[derive(Default)]
    struct SubscriberModel {
        events: Vec<&'static str>,
    }

    impl Entity for SubscriberModel {
        type Event = ();
    }

    struct EmitterModel;

    impl Entity for EmitterModel {
        type Event = ();
    }

    App::test((), |mut app| async move {
        let emitter = app.add_model(|_| EmitterModel);
        let emitter_clone = emitter.clone();

        let subscriber = app.add_model(|_| SubscriberModel::default());

        subscriber.update(&mut app, |_, ctx| {
            let emitter_for_unsubscribe = emitter_clone.clone();
            let emitter_for_resubscribe = emitter_clone.clone();

            ctx.subscribe_to_model(&emitter_clone, move |model, _, ctx| {
                model.events.push("old");

                ctx.unsubscribe_from_model(&emitter_for_unsubscribe);

                ctx.subscribe_to_model(&emitter_for_resubscribe, |model, _, _| {
                    model.events.push("new");
                });
            });
        });

        emitter.update(&mut app, |_, ctx| ctx.emit(()));
        subscriber.read(&app, |model, _| {
            assert_eq!(
                model.events,
                vec!["old"],
                "The new subscription should not fire for the current event."
            );
        });

        emitter.update(&mut app, |_, ctx| ctx.emit(()));
        subscriber.read(&app, |model, _| {
            assert_eq!(
                model.events,
                vec!["old", "new"],
                "The new subscription should survive unsubscribe-then-resubscribe inside a callback."
            );
        });
    });
}

#[test]
fn test_subscribe_then_unsubscribe_from_model_inside_callback_drops_new_subscription() {
    #[derive(Default)]
    struct SubscriberModel {
        events: Vec<&'static str>,
    }

    impl Entity for SubscriberModel {
        type Event = ();
    }

    struct EmitterModel;

    impl Entity for EmitterModel {
        type Event = ();
    }

    App::test((), |mut app| async move {
        let emitter = app.add_model(|_| EmitterModel);
        let emitter_clone = emitter.clone();

        let subscriber = app.add_model(|_| SubscriberModel::default());

        subscriber.update(&mut app, |_, ctx| {
            let emitter_for_subscribe = emitter_clone.clone();
            let emitter_for_unsubscribe = emitter_clone.clone();

            ctx.subscribe_to_model(&emitter_clone, move |model, _, ctx| {
                model.events.push("old");

                ctx.subscribe_to_model(&emitter_for_subscribe, |model, _, _| {
                    model.events.push("new");
                });

                ctx.unsubscribe_from_model(&emitter_for_unsubscribe);
            });
        });

        emitter.update(&mut app, |_, ctx| ctx.emit(()));
        subscriber.read(&app, |model, _| {
            assert_eq!(
                model.events,
                vec!["old"],
                "The new subscription should not fire for the current event."
            );
        });

        emitter.update(&mut app, |_, ctx| ctx.emit(()));
        subscriber.read(&app, |model, _| {
            assert_eq!(
                model.events,
                vec!["old"],
                "The new subscription should be removed when subscribe-then-unsubscribe happens in a callback."
            );
        });
    });
}

#[test]
fn test_view_unsubscribe_to_model_inside_callback() {
    #[derive(Default)]
    struct SubscriberView {
        events: Vec<usize>,
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

    struct EmitterModel;

    impl Entity for EmitterModel {
        type Event = usize;
    }

    App::test((), |mut app| async move {
        let emitter = app.add_model(|_| EmitterModel);
        let emitter_clone = emitter.clone();

        let (_, subscriber) =
            app.add_window(WindowStyle::NotStealFocus, |_| SubscriberView::default());

        // Subscribe, and unsubscribe from inside the callback.
        subscriber.update(&mut app, |_, ctx| {
            let emitter_for_unsubscribe = emitter_clone.clone();
            ctx.subscribe_to_model(&emitter_clone, move |view, _, event, ctx| {
                view.events.push(*event);
                // Unsubscribe from inside the callback.
                ctx.unsubscribe_to_model(&emitter_for_unsubscribe);
            });
        });

        // First emit should trigger the callback.
        emitter.update(&mut app, |_, ctx| ctx.emit(1));
        subscriber.read(&app, |view, _| {
            assert_eq!(view.events, vec![1], "first event should be received");
        });

        // Second emit should NOT trigger the callback, since we unsubscribed.
        emitter.update(&mut app, |_, ctx| ctx.emit(2));
        subscriber.read(&app, |view, _| {
            assert_eq!(
                view.events,
                vec![1],
                "second event should NOT be received after unsubscribe"
            );
        });
    });
}

#[test]
fn test_view_unsubscribe_to_model_inside_callback_with_multiple_subscriptions() {
    // This test verifies that when there are multiple subscriptions from a view to a model,
    // and one callback calls unsubscribe_to_model:
    // 1. All callbacks for the CURRENT event still fire (unsubscribe affects future events).
    // 2. No callbacks fire for FUTURE events.

    #[derive(Default)]
    struct SubscriberView {
        events: Vec<&'static str>,
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

    struct EmitterModel;

    impl Entity for EmitterModel {
        type Event = ();
    }

    App::test((), |mut app| async move {
        let emitter = app.add_model(|_| EmitterModel);
        let emitter_clone1 = emitter.clone();
        let emitter_clone2 = emitter.clone();

        let (_, subscriber) =
            app.add_window(WindowStyle::NotStealFocus, |_| SubscriberView::default());

        // Add two subscriptions from subscriber view to emitter model.
        subscriber.update(&mut app, |_, ctx| {
            // First subscription: will call unsubscribe.
            let emitter_for_unsubscribe = emitter_clone1.clone();
            ctx.subscribe_to_model(&emitter_clone1, move |view, _, _, ctx| {
                view.events.push("first");
                ctx.unsubscribe_to_model(&emitter_for_unsubscribe);
            });

            // Second subscription: should still be called for this event.
            ctx.subscribe_to_model(&emitter_clone2, move |view, _, _, _| {
                view.events.push("second");
            });
        });

        // Emit an event. BOTH callbacks should fire for the current event.
        emitter.update(&mut app, |_, ctx| ctx.emit(()));
        subscriber.read(&app, |view, _| {
            assert_eq!(
                view.events,
                vec!["first", "second"],
                "all existing callbacks should fire for the current event"
            );
        });

        // Emit again. Neither callback should fire since we unsubscribed.
        emitter.update(&mut app, |_, ctx| ctx.emit(()));
        subscriber.read(&app, |view, _| {
            assert_eq!(
                view.events,
                vec!["first", "second"],
                "no callbacks should be invoked for future events after unsubscribe"
            );
        });
    });
}

#[test]
fn test_view_unsubscribe_then_resubscribe_to_model_inside_callback_keeps_new_subscription() {
    #[derive(Default)]
    struct SubscriberView {
        events: Vec<&'static str>,
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

    struct EmitterModel;

    impl Entity for EmitterModel {
        type Event = ();
    }

    App::test((), |mut app| async move {
        let emitter = app.add_model(|_| EmitterModel);
        let emitter_clone = emitter.clone();

        let (_, subscriber) =
            app.add_window(WindowStyle::NotStealFocus, |_| SubscriberView::default());

        subscriber.update(&mut app, |_, ctx| {
            let emitter_for_unsubscribe = emitter_clone.clone();
            let emitter_for_resubscribe = emitter_clone.clone();

            ctx.subscribe_to_model(&emitter_clone, move |view, _, _, ctx| {
                view.events.push("old");

                ctx.unsubscribe_to_model(&emitter_for_unsubscribe);

                ctx.subscribe_to_model(&emitter_for_resubscribe, |view, _, _, _| {
                    view.events.push("new");
                });
            });
        });

        emitter.update(&mut app, |_, ctx| ctx.emit(()));
        subscriber.read(&app, |view, _| {
            assert_eq!(
                view.events,
                vec!["old"],
                "The new subscription should not fire for the current event."
            );
        });

        emitter.update(&mut app, |_, ctx| ctx.emit(()));
        subscriber.read(&app, |view, _| {
            assert_eq!(
                view.events,
                vec!["old", "new"],
                "The new subscription should survive unsubscribe-then-resubscribe inside a callback."
            );
        });
    });
}

#[test]
fn test_view_subscribe_then_unsubscribe_to_model_inside_callback_drops_new_subscription() {
    #[derive(Default)]
    struct SubscriberView {
        events: Vec<&'static str>,
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

    struct EmitterModel;

    impl Entity for EmitterModel {
        type Event = ();
    }

    App::test((), |mut app| async move {
        let emitter = app.add_model(|_| EmitterModel);
        let emitter_clone = emitter.clone();

        let (_, subscriber) =
            app.add_window(WindowStyle::NotStealFocus, |_| SubscriberView::default());

        subscriber.update(&mut app, |_, ctx| {
            let emitter_for_subscribe = emitter_clone.clone();
            let emitter_for_unsubscribe = emitter_clone.clone();

            ctx.subscribe_to_model(&emitter_clone, move |view, _, _, ctx| {
                view.events.push("old");

                ctx.subscribe_to_model(&emitter_for_subscribe, |view, _, _, _| {
                    view.events.push("new");
                });

                ctx.unsubscribe_to_model(&emitter_for_unsubscribe);
            });
        });

        emitter.update(&mut app, |_, ctx| ctx.emit(()));
        subscriber.read(&app, |view, _| {
            assert_eq!(
                view.events,
                vec!["old"],
                "The new subscription should not fire for the current event."
            );
        });

        emitter.update(&mut app, |_, ctx| ctx.emit(()));
        subscriber.read(&app, |view, _| {
            assert_eq!(
                view.events,
                vec!["old"],
                "The new subscription should be removed when subscribe-then-unsubscribe happens in a callback."
            );
        });
    });
}
