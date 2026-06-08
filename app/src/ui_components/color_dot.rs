use std::sync::Arc;

use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use ui_components::tooltip::{Params as TooltipParams, Tooltip as TooltipComponent};
use ui_components::{Component as _, Options as ComponentOptions};
use warp_core::ui::theme::{AnsiColorIdentifier, Fill as ThemeFill};
use warpui::elements::{
    Border, ChildAnchor, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Element,
    Flex, Hoverable, MainAxisAlignment, MainAxisSize, MouseStateHandle, OffsetPositioning,
    ParentAnchor, ParentElement, ParentOffsetBounds, Radius, Stack,
};
use warpui::platform::Cursor;
use warpui::Action;

use crate::appearance::Appearance;
use crate::menu::{MenuAction, MenuItem, MenuItemFields};
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

/// Builds a single custom menu item holding a row of clickable color dots: a
/// "no color" option followed by `TAB_COLOR_OPTIONS`. The currently `selected`
/// color is ringed. Clicking a dot dispatches the action produced by
/// `on_select(chosen)` (where `None` means "clear color") and then closes the
/// menu. Shared by the per-tab and per-tab-group color pickers.
pub(crate) fn color_dot_picker_menu_item<A, F>(
    selected: Option<AnsiColorIdentifier>,
    on_select: F,
) -> MenuItem<A>
where
    A: Action + Clone + 'static,
    F: Fn(Option<AnsiColorIdentifier>) -> A + Clone + 'static,
{
    let mouse_states: Vec<MouseStateHandle> = (0..TAB_COLOR_OPTIONS.len() + 1)
        .map(|_| MouseStateHandle::default())
        .collect();

    MenuItem::Item(
        MenuItemFields::new_with_custom_label(
            Arc::new(move |_is_selected, _is_hovered, appearance, _app| {
                let theme = appearance.theme();
                let terminal_colors = theme.terminal_colors().normal;
                let ring_color: ColorU = theme.accent().into();

                let mut row = Flex::row()
                    .with_main_axis_alignment(MainAxisAlignment::SpaceEvenly)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_main_axis_size(MainAxisSize::Max);

                for (ansi_id, mouse_state) in std::iter::once(None)
                    .chain(TAB_COLOR_OPTIONS.iter().copied().map(Some))
                    .zip(mouse_states.iter().cloned())
                {
                    let is_selected = match ansi_id {
                        None => selected.is_none(),
                        Some(id) => selected == Some(id),
                    };
                    let dot_color: ColorU = match ansi_id {
                        None => ColorU::transparent_black(),
                        Some(id) => id.to_ansi_color(&terminal_colors).into(),
                    };
                    let tooltip = match ansi_id {
                        None => "Default (no color)".to_string(),
                        Some(id) => id.to_string(),
                    };

                    let on_select = on_select.clone();
                    let dot = render_color_dot(
                        mouse_state,
                        dot_color,
                        is_selected,
                        ring_color,
                        ansi_id.is_none(),
                        theme.foreground(),
                        tooltip,
                        appearance,
                    )
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(on_select(ansi_id));
                        ctx.dispatch_typed_action(MenuAction::Close(true));
                    });

                    row.add_child(dot.finish());
                }

                row.finish()
            }),
            None,
        )
        .no_highlight_on_hover()
        .with_no_interaction_on_hover(),
    )
}

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
