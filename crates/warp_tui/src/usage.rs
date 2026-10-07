//! Reusable usage display for the TUI.
//!
//! [`UsageToggle`] owns the hover state behind the footer's clickable usage
//! entry; the credits⇄cost display mode itself is the file-backed, TUI-only
//! `agents.usage_display_mode` setting ([`TuiUsageDisplayMode`]), so the
//! choice persists across TUI sessions. The toggle is only offered to a viewer
//! whose tier charges usage in cents; a tier charged in credits always sees the
//! credits total. The helpers are shared by every surface that renders usage
//! (the footer entry today, the transcript/loading-indicator usage row next —
//! CODE-1832).

use warp::settings::TuiUsageDisplayMode;
use warp::tui_export::{ChargeUnit, ConversationUsageTotals, format_credits, format_dollars};
use warpui_core::AppContext;
use warpui_core::elements::MouseStateHandle;
use warpui_core::elements::tui::{TuiElement, TuiEventContext, TuiHoverable, TuiText};

use crate::tui_builder::TuiUiBuilder;

/// The clickable usage entry (`2.5 credits` ⇄ `$0.03`). Owned by the
/// composing view (created once, cloned into render closures) so the hover
/// state survives element-tree rebuilds.
#[derive(Clone, Default)]
pub(crate) struct UsageToggle {
    /// Hover state for the entry. Owned here (not created inline during
    /// render) so it survives element-tree rebuilds, following the GUI's
    /// `MouseStateHandle` pattern.
    hover_state: MouseStateHandle,
}

impl UsageToggle {
    /// Renders the clickable usage entry (`2.5 credits` ⇄ `$0.03`), dim like
    /// the rest of the footer metadata and brightened while hovered.
    /// `on_click` runs on a left click; the composing view uses it to
    /// dispatch the typed action that flips the persisted display-mode
    /// setting (the element pass only has an immutable [`AppContext`]).
    ///
    /// When the tier's `charge_unit` is credits, the entry is a static,
    /// non-interactive credits total: the persisted display mode is ignored so
    /// the dollar cost is never surfaced (`mode` and `on_click` are unused in
    /// that case).
    pub(crate) fn render_entry(
        &self,
        mode: TuiUsageDisplayMode,
        totals: ConversationUsageTotals,
        charge_unit: ChargeUnit,
        app: &AppContext,
        on_click: impl FnMut(&mut TuiEventContext, &AppContext) + 'static,
    ) -> Box<dyn TuiElement> {
        let builder = TuiUiBuilder::from_app(app);
        match charge_unit {
            ChargeUnit::Cents => {}
            ChargeUnit::Credits => {
                return TuiText::new(entry_text(TuiUsageDisplayMode::Credits, totals))
                    .with_style(builder.muted_text_style())
                    .truncate()
                    .finish();
            }
        }
        let is_hovered = self
            .hover_state
            .lock()
            .is_ok_and(|state| state.is_hovered());
        let style = if is_hovered {
            builder.primary_text_style()
        } else {
            builder.muted_text_style()
        };
        TuiHoverable::new(
            self.hover_state.clone(),
            TuiText::new(entry_text(mode, totals))
                .with_style(style)
                .truncate()
                .finish(),
        )
        .on_click(on_click)
        .finish()
    }
}

/// The entry's text for `mode`: the GUI-consistent credits total (formatted
/// with the GUI's own `format_credits`) or the billed dollar cost.
fn entry_text(mode: TuiUsageDisplayMode, totals: ConversationUsageTotals) -> String {
    match mode {
        TuiUsageDisplayMode::Credits => format_credits(totals.credits_spent),
        TuiUsageDisplayMode::Cost => totals
            .total_cost_in_cents()
            .map(format_dollars)
            .unwrap_or_else(|| "Cost unavailable".to_owned()),
    }
}

#[cfg(test)]
#[path = "usage_tests.rs"]
mod tests;
