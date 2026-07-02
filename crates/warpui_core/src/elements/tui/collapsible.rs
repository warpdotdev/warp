//! [`tui_collapsible`]: a disclosure section — a clickable text header with a
//! chevron over a body that shows only when expanded.
//!
//! This is a plain composition of existing primitives: a [`TuiFlex`] column
//! whose first child is the header (a [`TuiText`] of the label followed by a chevron
//! reflecting the state, wrapped in a [`TuiEventHandler`] for the click) and
//! whose second child — present only when expanded — is the body. State is
//! owned by the caller: `collapsed` is read at composition time and
//! `on_toggle` fires on a header click, leaving the caller to flip its own
//! state and re-render.

use super::{TuiElement, TuiEventContext, TuiEventHandler, TuiFlex, TuiStyle, TuiText};
use crate::AppContext;

/// Disclosure glyph shown when the section is collapsed.
const CHEVRON_COLLAPSED: &str = "▸";
/// Disclosure glyph shown when the section is expanded.
const CHEVRON_EXPANDED: &str = "▾";

/// Composes a collapsible section: a clickable `label` header (styled with
/// `header_style` and suffixed with a state chevron) over `body`, which is
/// included only when `collapsed` is `false`. `on_toggle` runs when the header
/// is clicked.
pub fn tui_collapsible(
    collapsed: bool,
    label: impl Into<String>,
    header_style: TuiStyle,
    body: Box<dyn TuiElement>,
    on_toggle: impl FnMut(&mut TuiEventContext, &AppContext) + 'static,
) -> Box<dyn TuiElement> {
    let chevron = if collapsed {
        CHEVRON_COLLAPSED
    } else {
        CHEVRON_EXPANDED
    };
    let header = TuiEventHandler::new(
        TuiText::new(format!("{} {chevron}", label.into()))
            .with_style(header_style)
            .truncate()
            .finish(),
    )
    .on_click(on_toggle);

    let mut column = TuiFlex::column().child(header.finish());
    if !collapsed {
        column = column.child(body);
    }
    column.finish()
}

#[cfg(test)]
#[path = "collapsible_tests.rs"]
mod tests;
