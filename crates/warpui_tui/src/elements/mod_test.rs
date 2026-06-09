use warpui_core::{AppContext, Entity, TuiView};

use super::{TuiElement, TuiRenderOutput};
use crate::{TuiBuffer, TuiConstraint, TuiSize};

#[test]
fn unit_element_is_inert() {
    let mut element = ();
    assert_eq!(
        element.layout(TuiConstraint::tight(TuiSize::new(4, 2))),
        TuiSize::ZERO,
    );
    assert_eq!(element.desired_height(10), 0);
    assert_eq!(
        TuiElement::cursor_position(&(), crate::TuiRect::new(0, 0, 4, 2)),
        None
    );

    // Painting a `()` leaves the buffer untouched.
    let mut buffer = TuiBuffer::new(TuiSize::new(2, 1));
    element.render(crate::TuiRect::new(0, 0, 2, 1), &mut buffer);
    assert_eq!(buffer.to_lines(), vec!["  "]);
}

#[test]
fn unit_element_is_boxable_as_render_output() {
    let boxed: TuiRenderOutput = Box::new(());
    assert_eq!(boxed.desired_height(3), 0);
}

// A minimal view that exists only to prove, at compile time, that the
// `TuiRenderOutput` bridge is a valid `TuiView::RenderOutput` against
// `warpui_core --features tui`. A real view returns its element tree from
// `render_tui`; here we only need the types to line up.
struct ProbeView;

impl Entity for ProbeView {
    type Event = ();
}

impl TuiView for ProbeView {
    type RenderOutput = TuiRenderOutput;

    fn ui_name() -> &'static str {
        "ProbeView"
    }

    fn render_tui(&self, _ctx: &AppContext) -> Self::RenderOutput {
        Box::new(())
    }
}

#[test]
fn render_output_bridge_satisfies_the_core_contract() {
    // Compiles iff `TuiRenderOutput` binds as `ProbeView`'s erased render
    // output, i.e. iff the bridge type is a valid `TuiView::RenderOutput`.
    fn assert_bridge<T: TuiView<RenderOutput = TuiRenderOutput>>(_view: T) {}
    assert_bridge(ProbeView);
}
