//! Display semantics for [`TransientHint`]. Expiry timing and the abort of a
//! superseded notice's timer ride on the framework's `SpawnedFutureHandle`
//! abort semantics, so only the show/supersede state is pinned here.

use warpui_core::elements::tui::{TuiElement, TuiText};
use warpui_core::platform::WindowStyle;
use warpui_core::{
    AddWindowOptions, App, AppContext, Entity, TuiView, TypedActionView, ViewHandle,
};

use super::TransientHint;

/// Minimal view owning a [`TransientHint`], standing in for the session view.
struct HintView {
    hint: TransientHint,
}

impl Entity for HintView {
    type Event = ();
}

impl TuiView for HintView {
    fn ui_name() -> &'static str {
        "HintView"
    }

    fn render(&self, _ctx: &AppContext) -> Box<dyn TuiElement> {
        TuiText::new(String::new()).finish()
    }
}

impl TypedActionView for HintView {
    type Action = ();
}

fn build_view(ctx: &mut AppContext) -> ViewHandle<HintView> {
    let (_window_id, view) = ctx.add_tui_window(
        AddWindowOptions {
            window_style: WindowStyle::NotStealFocus,
            ..Default::default()
        },
        |_| HintView {
            hint: TransientHint::default(),
        },
    );
    view
}

fn show(view: &ViewHandle<HintView>, ctx: &mut AppContext, text: &str) {
    let text = text.to_owned();
    view.update(ctx, |view, ctx| {
        view.hint.show(text, ctx, |view| &mut view.hint);
    });
}

#[test]
fn show_displays_the_notice() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            assert_eq!(view.as_ref(ctx).hint.current(), None);
            show(&view, ctx, "notice");
            assert_eq!(view.as_ref(ctx).hint.current(), Some("notice"));
        });
    });
}

#[test]
fn show_supersedes_the_earlier_notice() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            show(&view, ctx, "first");
            show(&view, ctx, "second");
            assert_eq!(view.as_ref(ctx).hint.current(), Some("second"));
        });
    });
}
