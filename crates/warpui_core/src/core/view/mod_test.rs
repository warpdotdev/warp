//! Tests proving the unified [`View`] reuses the shared application core
//! identically on both backends: window registration + focus, handle
//! update/read, `notify` invalidation, async `spawn`, model subscription,
//! typed-action dispatch, and root-view resolution.
//!
//! Ported from the former `tui_view_tests.rs` (which exercised the deleted
//! `TuiView` mirror); these now run under BOTH the default (GUI) build and
//! `--features tui`, differing only in the [`RenderOutput`] each backend
//! produces.

use std::cell::Cell;
use std::rc::Rc;

use super::{TypedActionView, View, ViewContext};
use crate::platform::WindowStyle;
use crate::test::empty_render_output;
use crate::{AddWindowOptions, App, AppContext, Entity, ModelHandle, RenderOutput, UpdateModel};

fn window_options() -> AddWindowOptions {
    AddWindowOptions {
        window_style: WindowStyle::NotStealFocus,
        ..Default::default()
    }
}

/// A minimal `View` carrying a counter, used by most tests. It is also a
/// `TypedActionView` (with a no-op action) so it can serve as a window root.
#[derive(Default)]
struct CounterView {
    count: usize,
}

impl Entity for CounterView {
    type Event = ();
}

impl View for CounterView {
    fn ui_name() -> &'static str {
        "CounterView"
    }
    fn render(&self, _ctx: &AppContext) -> RenderOutput {
        empty_render_output()
    }
}

impl TypedActionView for CounterView {
    type Action = ();
}

#[test]
fn test_add_and_focus() {
    App::test((), |mut app| async move {
        let (window_id, root) =
            app.update(|ctx| ctx.add_window(window_options(), |_| CounterView::default()));

        // The root is focused on creation.
        assert_eq!(app.focused_view_id(window_id), Some(root.id()));

        // Add a child view and focus it; the focus should move to the child.
        let child = app.update(|ctx| ctx.add_view(window_id, |_| CounterView::default()));
        assert_ne!(child.id(), root.id());

        child.update(&mut app, |_, ctx| ctx.focus_self());

        assert_eq!(app.focused_view_id(window_id), Some(child.id()));
        assert!(app.read(|ctx| child.is_focused(ctx)));
        assert!(!app.read(|ctx| root.is_focused(ctx)));
    });
}

#[test]
fn test_update_and_read() {
    App::test((), |mut app| async move {
        let (_, handle) =
            app.update(|ctx| ctx.add_window(window_options(), |_| CounterView::default()));

        handle.update(&mut app, |view, _| view.count = 41);
        handle.update(&mut app, |view, _| view.count += 1);

        let count = handle.read(&app, |view, _| view.count);
        assert_eq!(count, 42);
    });
}

#[test]
fn test_notify_marks_window_invalidated() {
    App::test((), |mut app| async move {
        let (window_id, handle) =
            app.update(|ctx| ctx.add_window(window_options(), |_| CounterView::default()));

        // Override the window's invalidation callback with our own recorder. This
        // replaces the test harness's auto-build-scene callback (which would clear
        // invalidations), so we can observe the same `window_invalidations` signal
        // the render loop uses without it being immediately drained.
        let invalidated = Rc::new(Cell::new(false));
        let recorder = invalidated.clone();
        app.update(move |ctx| {
            ctx.on_window_invalidated(window_id, move |_, _| recorder.set(true));
        });

        invalidated.set(false);
        handle.update(&mut app, |_, ctx| ctx.notify());

        // The window-invalidated callback fired, and the invalidation signal is set.
        assert!(invalidated.get(), "notify should invalidate the window");
        assert!(app.read(|ctx| ctx.has_window_invalidations(window_id)));
    });
}

#[test]
fn test_async_spawn_runs_on_main_thread() {
    App::test((), |mut app| async move {
        let (_, handle) =
            app.update(|ctx| ctx.add_window(window_options(), |_| CounterView::default()));

        let (tx, rx) = futures::channel::oneshot::channel();
        handle.update(&mut app, move |_, ctx| {
            ctx.spawn(async { 7usize }, move |view, output, _| {
                view.count = output;
                tx.send(()).unwrap();
            })
        });
        rx.await.unwrap();

        assert_eq!(handle.read(&app, |view, _| view.count), 7);
    });
}

/// A backend-agnostic model (plain [`Entity`]/`ModelContext`, no backend
/// parameter), reused by a view through the unified `ViewContext`.
struct CounterModel {
    value: usize,
}

impl Entity for CounterModel {
    type Event = usize;
}

#[derive(Default)]
struct SubscriberView {
    last_seen: usize,
    model: Option<ModelHandle<CounterModel>>,
}

impl Entity for SubscriberView {
    type Event = ();
}

impl View for SubscriberView {
    fn ui_name() -> &'static str {
        "SubscriberView"
    }
    fn render(&self, _ctx: &AppContext) -> RenderOutput {
        empty_render_output()
    }
}

impl TypedActionView for SubscriberView {
    type Action = ();
}

#[test]
fn test_model_reuse_subscribe_and_emit() {
    App::test((), |mut app| async move {
        let (_, handle) = app.update(|ctx| {
            ctx.add_window(window_options(), |vctx| {
                let model = vctx.add_model(|_| CounterModel { value: 0 });
                vctx.subscribe_to_model(&model, |view: &mut SubscriberView, _handle, event, _| {
                    view.last_seen = *event;
                });
                SubscriberView {
                    last_seen: 0,
                    model: Some(model),
                }
            })
        });

        let model = handle.read(&app, |view, _| view.model.clone().unwrap());

        // Mutate the model and emit an event; the view's subscription should react.
        app.update(|ctx| {
            ctx.update_model(&model, |model, mctx| {
                model.value = 99;
                mctx.emit(model.value);
            });
        });

        assert_eq!(handle.read(&app, |view, _| view.last_seen), 99);
    });
}

#[derive(Debug)]
struct Increment(usize);

#[derive(Default)]
struct ActionView {
    total: usize,
}

impl Entity for ActionView {
    type Event = ();
}

impl View for ActionView {
    fn ui_name() -> &'static str {
        "ActionView"
    }
    fn render(&self, _ctx: &AppContext) -> RenderOutput {
        empty_render_output()
    }
}

impl TypedActionView for ActionView {
    type Action = Increment;
    fn handle_action(&mut self, action: &Increment, _ctx: &mut ViewContext<Self>) {
        self.total += action.0;
    }
}

#[test]
fn test_typed_action_dispatch() {
    App::test((), |mut app| async move {
        let (window_id, handle) =
            app.update(|ctx| ctx.add_window(window_options(), |_| ActionView::default()));

        // Dispatch a typed action through the shared dispatch path. The view's
        // `handle_action` should run and mutate it.
        app.dispatch_typed_action(window_id, &[handle.id()], &Increment(5));
        app.dispatch_typed_action(window_id, &[handle.id()], &Increment(3));

        assert_eq!(handle.read(&app, |view, _| view.total), 8);
    });
}

#[test]
fn test_root_view_resolution() {
    // Successor to the former `test_alias_resolves_to_tui_backend`: the same
    // unified `root_view` accessor resolves the typed root handle on whichever
    // backend the build selects.
    App::test((), |mut app| async move {
        let (window_id, root) =
            app.update(|ctx| ctx.add_window(window_options(), |_| CounterView::default()));

        let resolved = app.read(|ctx| ctx.root_view::<CounterView>(window_id).map(|h| h.id()));
        assert_eq!(resolved, Some(root.id()));
    });
}
