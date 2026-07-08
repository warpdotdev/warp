//! Reusable usage display for the TUI.
//!
//! [`UsageToggle`] owns the credits⇄cost display mode and hover state behind
//! the footer's clickable usage entry; the helpers are shared by every
//! surface that renders usage (the footer entry today, the
//! transcript/loading-indicator usage row next — CODE-1832).

use std::cell::Cell;
use std::rc::Rc;

use warp::tui_export::{format_credits, ConversationUsageTotals};
use warpui_core::elements::tui::{Modifier, TuiElement, TuiHoverable, TuiStyle, TuiText};
use warpui_core::elements::MouseStateHandle;

/// Which unit a usage entry currently displays.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum UsageDisplayMode {
    /// Credits spent (`2.5 credits`) — the same number the GUI's usage
    /// footer and conversation details panel show.
    #[default]
    Credits,
    /// Provider dollar cost (`$0.03`).
    Cost,
}

impl UsageDisplayMode {
    fn toggled(self) -> Self {
        match self {
            Self::Credits => Self::Cost,
            Self::Cost => Self::Credits,
        }
    }
}

/// The credits⇄cost toggle behind a clickable usage entry. Owned by the
/// composing view (created once, cloned into render closures) so the display
/// mode and hover state survive element-tree rebuilds.
#[derive(Clone, Default)]
pub(crate) struct UsageToggle {
    mode: Rc<Cell<UsageDisplayMode>>,
    /// Hover state for the entry. Owned here (not created inline during
    /// render) so it survives element-tree rebuilds, following the GUI's
    /// `MouseStateHandle` pattern.
    hover_state: MouseStateHandle,
}

impl UsageToggle {
    /// Flips between credits and cost display.
    fn toggle(&self) {
        self.mode.set(self.mode.get().toggled());
    }

    /// The entry's current text: the GUI-consistent credits total (formatted
    /// with the GUI's own `format_credits`) or the provider dollar cost.
    fn entry_text(&self, totals: ConversationUsageTotals) -> String {
        match self.mode.get() {
            UsageDisplayMode::Credits => format_credits(totals.credits_spent),
            UsageDisplayMode::Cost => format_cost(totals.cost_in_cents),
        }
    }

    /// Renders the clickable usage entry (`2.5 credits` ⇄ `$0.03`), dim like
    /// the rest of the footer metadata and brightened while hovered. A click
    /// flips the display mode and re-renders the owning view.
    pub(crate) fn render_entry(&self, totals: ConversationUsageTotals) -> Box<dyn TuiElement> {
        let is_hovered = self
            .hover_state
            .lock()
            .is_ok_and(|state| state.is_hovered());
        let mut style = TuiStyle::default();
        if !is_hovered {
            style = style.add_modifier(Modifier::DIM);
        }
        let toggle = self.clone();
        TuiHoverable::new(
            self.hover_state.clone(),
            TuiText::new(self.entry_text(totals))
                .with_style(style)
                .truncate()
                .finish(),
        )
        .on_click(move |event_ctx, _| {
            toggle.toggle();
            event_ctx.notify();
        })
        .finish()
    }
}

/// Formats an accumulated cost in US cents as dollars (`3.2` cents → `$0.03`).
pub(crate) fn format_cost(cost_in_cents: f64) -> String {
    format!("${:.2}", cost_in_cents / 100.0)
}

#[cfg(test)]
#[path = "usage_tests.rs"]
mod tests;
