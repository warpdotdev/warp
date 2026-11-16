use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use ui_components::tooltip::{Params as TooltipParams, Tooltip as TooltipComponent};
use ui_components::{Component as _, Options as ComponentOptions};
use warp_core::ui::theme::{AnsiColorIdentifier, Fill as ThemeFill};
use warpui::elements::{
    Border, ChildAnchor, ConstrainedBox, Container, CornerRadius, Element, Hoverable,
    MouseStateHandle, OffsetPositioning, ParentAnchor, ParentElement, ParentOffsetBounds, Radius,
    Stack,
};
use warpui::platform::Cursor;

use crate::appearance::Appearance;
use crate::ui_components::icons::Icon;

const COLOR_DOT_SIZE: f32 = 16.;

pub(crate) const TAB_COLOR_OPTIONS: [AnsiColorIdentifier; 6] = [
    AnsiColorIdentifier::Red,
    AnsiColorIdentifier::Green,
    AnsiColorIdentifier::Yellow,
    AnsiColorIdentifier::Blue,
    AnsiColorIdentifier::Magenta,
    AnsiColorIdentifier::Cyan,
];

/// Renders a hoverable color dot with selection ring, tooltip, and pointer cursor.
/// For the no-color option, pass `is_no_color: true` to show a slash overlay.
/// Returns a `Hoverable` so callers can chain `.on_click(...)` before `.finish()`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_color_dot(
    mouse_state: MouseStateHandle,
    dot_color: ColorU,
    is_selected: bool,
    ring_color: ColorU,
    is_no_color: bool,
    foreground_color: ThemeFill,
    tooltip_text: String,
    appearance: &Appearance,
) -> Hoverable {
    Hoverable::new(mouse_state, move |state| {
        let overlay: Option<Box<dyn Element>> = if is_no_color {
            Some(Icon::SlashCircle.to_warpui_icon(foreground_color).finish())
        } else {
            None
        };

        let dot_element = render_dot_element(dot_color, is_selected, ring_color, overlay);

        if state.is_hovered() {
            let tooltip_element = TooltipComponent.render(
                appearance,
                TooltipParams {
                    label: tooltip_text.clone().into(),
                    options: ComponentOptions::default(appearance),
                },
            );
            Stack::new()
                .with_child(dot_element)
                .with_positioned_child(
                    tooltip_element,
                    OffsetPositioning::offset_from_parent(
                        vec2f(0., -4.),
                        ParentOffsetBounds::Unbounded,
                        ParentAnchor::TopMiddle,
                        ChildAnchor::BottomMiddle,
                    ),
                )
                .finish()
        } else {
            dot_element
        }
    })
    .with_cursor(Cursor::PointingHand)
}

/// Pure visual element: circular dot with optional overlay and selection ring.
fn render_dot_element(
    dot_color: ColorU,
    is_selected: bool,
    ring_color: ColorU,
    overlay: Option<Box<dyn Element>>,
) -> Box<dyn Element> {
    let dot = ConstrainedBox::new(Icon::Ellipse.to_warpui_icon(dot_color.into()).finish())
        .with_width(COLOR_DOT_SIZE)
        .with_height(COLOR_DOT_SIZE)
        .finish();

    let inner = if let Some(overlay_element) = overlay {
        let overlay_sized = ConstrainedBox::new(overlay_element)
            .with_width(COLOR_DOT_SIZE)
            .with_height(COLOR_DOT_SIZE)
            .finish();
        Stack::new()
            .with_child(dot)
            .with_child(overlay_sized)
            .finish()
    } else {
        dot
    };

    let border_color = if is_selected {
        ring_color
    } else {
        ColorU::transparent_black()
    };

    Container::new(inner)
        .with_border(Border::all(2.).with_border_color(border_color))
        .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
        .finish()
}
