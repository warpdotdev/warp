//! Tests proving that a [`TuiView`] reuses the shared application core exactly
//! as a GUI `View` does: registration + focus/blur hook dispatch, handle
//! update/read, typed-action dispatch through the shared responder chain,
//! model subscription, drop/ref-count cleanup, and TUI rendering — all running
//! additively alongside the full GUI test suite under `--features tui`.

use super::*;
use crate::elements::tui::{
    TuiBuffer, TuiConstraint, TuiElement, TuiLayoutContext, TuiRect, TuiSize,
};
use crate::platform::WindowStyle;

/// A GUI root view hosting TUI views: under the additive design, GUI and TUI
/// views coexist in the same window registry.
#[derive(Default)]
struct RootView {
    pings: usize,
}

impl Entity for RootView {
    type Event = ();
}

impl View for RootView {
    fn ui_name() -> &'static str {
        "RootView"
    }

    fn render(&self, _: &AppContext) -> Box<dyn crate::elements::Element> {
        crate::elements::Empty::new().finish()
    }
}

#[derive(Debug)]
struct RootPing;

impl TypedActionView for RootView {
    type Action = RootPing;

    fn handle_action(&mut self, _action: &RootPing, _ctx: &mut ViewContext<Self>) {
        self.pings += 1;
    }
}

/// A minimal `TuiView` carrying a counter and focus/blur hook recorders.
#[derive(Default)]
struct CounterView {
    count: usize,
    focus_events: usize,
    blur_events: usize,
}

impl Entity for CounterView {
    type Event = ();
}

impl TuiView for CounterView {
    fn ui_name() -> &'static str {
        "CounterView"
    }

    fn render(&self, _: &AppContext) -> Box<dyn TuiElement> {
        Box::new(TuiEmpty)
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, _ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            self.focus_events += 1;
        }
    }

    fn on_blur(&mut self, blur_ctx: &BlurContext, _ctx: &mut ViewContext<Self>) {
        if blur_ctx.is_self_blurred() {
            self.blur_events += 1;
        }
    }
}

#[test]
fn test_add_focus_and_hook_dispatch() {
    App::test((), |mut app| async move {
        let (window_id, root) = app.add_window(WindowStyle::NotStealFocus, |_| RootView::default());

        let tui = app.update(|ctx| ctx.add_tui_view(window_id, |_| CounterView::default()));
        let name = app.read(|ctx| ctx.view_name(window_id, tui.id()).map(str::to_owned));
        assert_eq!(name.as_deref(), Some("CounterView"));

        // Focus the TUI view: the shared focus effect must dispatch its
        // on_focus hook through the unified ViewContext.
        tui.update(&mut app, |_, ctx| ctx.focus_self());
        assert_eq!(app.focused_view_id(window_id), Some(tui.id()));
        assert!(app.read(|ctx| tui.is_focused(ctx)));
        assert_eq!(tui.read(&app, |view, _| view.focus_events), 1);
        assert_eq!(tui.read(&app, |view, _| view.blur_events), 0);

        // Refocus the GUI root: the TUI view's on_blur hook fires.
        root.update(&mut app, |_, ctx| ctx.focus_self());
        assert_eq!(app.focused_view_id(window_id), Some(root.id()));
        assert_eq!(tui.read(&app, |view, _| view.blur_events), 1);
    });
}

#[test]
fn test_update_and_read_via_handle() {
    App::test((), |mut app| async move {
        let (window_id, _root) =
            app.add_window(WindowStyle::NotStealFocus, |_| RootView::default());

        let tui = app.update(|ctx| ctx.add_tui_view(window_id, |_| CounterView::default()));

        tui.update(&mut app, |view, _| view.count = 41);
        tui.update(&mut app, |view, _| view.count += 1);

        assert_eq!(tui.read(&app, |view, _| view.count), 42);
    });
}

#[test]
fn test_render_tui_view() {
    App::test((), |mut app| async move {
        let (window_id, root) = app.add_window(WindowStyle::NotStealFocus, |_| RootView::default());

        let tui = app.update(|ctx| ctx.add_tui_view(window_id, |_| CounterView::default()));

        // The TUI view renders through the TUI path and is rejected by the GUI path.
        assert!(app.read(|ctx| ctx.render_tui_view(window_id, tui.id()).is_ok()));
        assert!(app.read(|ctx| ctx.render_view(window_id, tui.id()).is_err()));

        // And vice versa for the GUI root.
        assert!(app.read(|ctx| ctx.render_view(window_id, root.id()).is_ok()));
        assert!(app.read(|ctx| ctx.render_tui_view(window_id, root.id()).is_err()));
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

/// An empty element, useful as a placeholder render output.
#[derive(Default)]
pub struct TuiEmpty;

impl TuiElement for TuiEmpty {
    fn layout(&mut self, _constraint: TuiConstraint, _ctx: &mut TuiLayoutContext) -> TuiSize {
        TuiSize::ZERO
    }

    fn render(&self, _area: TuiRect, _buffer: &mut TuiBuffer, _ctx: &mut TuiLayoutContext) {}
}

impl TuiView for ActionView {
    fn ui_name() -> &'static str {
        "ActionView"
    }

    fn render(&self, _: &AppContext) -> Box<dyn TuiElement> {
        Box::new(TuiEmpty)
    }
}

impl TypedActionView for ActionView {
    type Action = Increment;

    fn handle_action(&mut self, action: &Increment, _ctx: &mut ViewContext<Self>) {
        self.total += action.0;
    }
}

#[test]
fn test_typed_action_dispatch_through_shared_responder_chain() {
    App::test((), |mut app| async move {
        let (window_id, root) = app.add_window(WindowStyle::NotStealFocus, |_| RootView::default());

        // Create the TUI view as a structural child of the GUI root, joining
        // the shared view_parents hierarchy.
        let tui = root.update(&mut app, |_, ctx| {
            ctx.add_typed_action_tui_view(|_| ActionView::default())
        });

        // Dispatch typed actions from the TUI leaf: the responder chain is
        // derived from the shared view hierarchy and the handler registered in
        // the shared typed_actions registry runs on the TUI view.
        app.update(|ctx| ctx.dispatch_typed_action_for_view(window_id, tui.id(), &Increment(5)));
        app.update(|ctx| ctx.dispatch_typed_action_for_view(window_id, tui.id(), &Increment(3)));
        assert_eq!(tui.read(&app, |view, _| view.total), 8);

        // An action only the GUI root handles, dispatched from the TUI leaf,
        // traverses the chain through the TUI view up to the GUI parent.
        app.update(|ctx| ctx.dispatch_typed_action_for_view(window_id, tui.id(), &RootPing));
        assert_eq!(root.read(&app, |view, _| view.pings), 1);
    });
}

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

impl TuiView for SubscriberView {
    fn ui_name() -> &'static str {
        "SubscriberView"
    }

    fn render(&self, _: &AppContext) -> Box<dyn TuiElement> {
        Box::new(TuiEmpty)
    }
}

#[test]
fn test_model_subscription_from_tui_view() {
    App::test((), |mut app| async move {
        let (window_id, _root) =
            app.add_window(WindowStyle::NotStealFocus, |_| RootView::default());

        let tui = app.update(|ctx| {
            ctx.add_tui_view(window_id, |vctx| {
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

        let model = tui.read(&app, |view, _| view.model.clone().unwrap());

        app.update(|ctx| {
            ctx.update_model(&model, |model, mctx| {
                model.value = 99;
                mctx.emit(model.value);
            });
        });

        assert_eq!(tui.read(&app, |view, _| view.last_seen), 99);
    });
}

#[test]
fn test_drop_removes_tui_view() {
    App::test((), |mut app| async move {
        let (window_id, _root) =
            app.add_window(WindowStyle::NotStealFocus, |_| RootView::default());

        let tui = app.update(|ctx| ctx.add_tui_view(window_id, |_| CounterView::default()));
        let view_id = tui.id();
        assert!(app.read(|ctx| ctx.view_name(window_id, view_id).is_some()));

        // Dropping the last strong handle removes the TUI view through the
        // shared ref-count/remove_dropped_items path.
        drop(tui);
        app.update(|_| {});
        assert!(app.read(|ctx| ctx.view_name(window_id, view_id).is_none()));
    });
}
