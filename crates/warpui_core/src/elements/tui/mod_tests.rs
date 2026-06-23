use super::{TuiElement, TuiRenderOutput};
use crate::elements::tui::{TuiBuffer, TuiBufferExt, TuiConstraint, TuiRect, TuiSize, TuiText};
use crate::platform::WindowStyle;
use crate::{AddWindowOptions, App, AppContext, Entity, TuiView, TypedActionView};

#[test]
fn unit_element_is_inert() {
    let mut element = ();
    assert_eq!(
        element.layout(TuiConstraint::tight(TuiSize::new(4, 2))),
        TuiSize::ZERO,
    );
    assert_eq!(element.desired_height(10), 0);
    assert_eq!(
        TuiElement::cursor_position(&(), TuiRect::new(0, 0, 4, 2)),
        None
    );

    // Painting a `()` leaves the buffer untouched.
    let mut buffer = TuiBuffer::empty(TuiRect::new(0, 0, 2, 1));
    element.render(TuiRect::new(0, 0, 2, 1), &mut buffer);
    assert_eq!(buffer.to_lines(), vec!["  "]);
}

#[test]
fn unit_element_is_boxable_as_render_output() {
    let boxed: TuiRenderOutput = Box::new(());
    assert_eq!(boxed.desired_height(3), 0);
}

/// A minimal view proving the typed render contract: a [`TuiView`] returns a
/// `Box<dyn TuiElement>` and the core renders it back fully typed.
struct ProbeView;

impl Entity for ProbeView {
    type Event = ();
}

impl TuiView for ProbeView {
    fn ui_name() -> &'static str {
        "ProbeView"
    }

    fn render(&self, _: &AppContext) -> Box<dyn TuiElement> {
        Box::new(TuiText::new("PROBE"))
    }
}

impl TypedActionView for ProbeView {
    type Action = ();
}

#[test]
fn typed_render_output_round_trips_through_the_core_without_downcasts() {
    App::test((), |mut app| async move {
        let (window_id, root) = app.update(|ctx| {
            ctx.add_tui_window(
                AddWindowOptions {
                    window_style: WindowStyle::NotStealFocus,
                    ..Default::default()
                },
                |_| ProbeView,
            )
        });

        // The core hands back the typed element tree — no `Any`, no downcast.
        let mut element = app
            .read(|ctx| ctx.render_tui_view(window_id, root.id()))
            .expect("the TUI root view renders");
        let size = TuiSize::new(5, 1);
        element.layout(TuiConstraint::tight(size));
        let mut buffer = TuiBuffer::empty(TuiRect::new(0, 0, size.width, size.height));
        element.render(TuiRect::new(0, 0, size.width, size.height), &mut buffer);
        assert_eq!(buffer.to_lines(), vec!["PROBE"]);
    });
}
