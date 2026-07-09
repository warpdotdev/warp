//! [`tui_collapsible`]: a disclosure section — a clickable text header with a
//! chevron over a body that shows only when expanded.
//!
//! This is a plain composition of existing primitives: a [`TuiFlex`] column
//! whose first child is the header (a [`TuiText`] of the label followed by a
//! chevron reflecting the state, wrapped in a [`TuiHoverable`] for the click
//! and hover tracking) and whose second child — present only when expanded —
//! is the body. State is owned by the caller: `collapsed` and the hover state
//! on `mouse_state` are read at composition time and `on_toggle` fires on a
//! header click, leaving the caller to flip its own state and re-render.

use super::{TuiElement, TuiEventContext, TuiFlex, TuiHoverable, TuiStyle, TuiText};
use crate::elements::MouseStateHandle;
use crate::AppContext;

/// Disclosure glyph shown when the section is collapsed.
const CHEVRON_COLLAPSED: &str = "▸";
/// Disclosure glyph shown when the section is expanded.
const CHEVRON_EXPANDED: &str = "▾";

/// Returns the disclosure glyph for a collapsed or expanded section.
pub fn tui_disclosure_chevron(collapsed: bool) -> &'static str {
    if collapsed {
        CHEVRON_COLLAPSED
    } else {
        CHEVRON_EXPANDED
    }
}

/// Composes a collapsible section: a clickable `label` header (suffixed with a
/// state chevron) over `body`, which is included only when `collapsed` is
/// `false`. `on_toggle` runs when the header is clicked. The header is styled
/// with `header_hover_style` while `mouse_state` reports it hovered and
/// `header_style` otherwise; hover transitions are recorded on `mouse_state`,
/// which the caller owns so it survives re-renders.
pub fn tui_collapsible(
    collapsed: bool,
    label: impl Into<String>,
    header_style: TuiStyle,
    header_hover_style: TuiStyle,
    mouse_state: MouseStateHandle,
    body: Box<dyn TuiElement>,
    on_toggle: impl FnMut(&mut TuiEventContext, &AppContext) + 'static,
) -> Box<dyn TuiElement> {
    let chevron = tui_disclosure_chevron(collapsed);
    let style = if mouse_state.lock().unwrap().is_hovered() {
        header_hover_style
    } else {
        header_style
    };
    let header = TuiText::new(format!("{} {chevron}", label.into()))
        .with_style(style)
        .truncate()
        .finish();
    tui_collapsible_with_header(collapsed, mouse_state, header, body, on_toggle)
}

/// Composes a collapsible section from a caller-rendered header element. This
/// is the rich-header counterpart to [`tui_collapsible`]: callers can provide
/// styled spans (for example, a colored status glyph plus label and chevron)
/// while reusing the same persistent hover target, click handling, and
/// conditional body composition.
pub fn tui_collapsible_with_header(
    collapsed: bool,
    mouse_state: MouseStateHandle,
    header: Box<dyn TuiElement>,
    body: Box<dyn TuiElement>,
    on_toggle: impl FnMut(&mut TuiEventContext, &AppContext) + 'static,
) -> Box<dyn TuiElement> {
    let header = TuiHoverable::new(mouse_state, header).on_click(on_toggle);

    let mut column = TuiFlex::column().child(header.finish());
    if !collapsed {
        column = column.child(body);
    }
    column.finish()
}

#[cfg(test)]
#[path = "collapsible_tests.rs"]
mod tests;
