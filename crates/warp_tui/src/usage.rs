//! Reusable token/cost usage display for the TUI.
//!
//! [`TokenCostToggle`] owns the tokens⇄cost display mode and hover state
//! behind the footer's clickable usage entry; the formatting helpers are
//! shared by every surface that renders token usage (the footer entry today,
//! the transcript/loading-indicator usage row next — CODE-1832).

use std::cell::Cell;
use std::rc::Rc;

use warp::tui_export::ConversationUsageTotals;
use warpui_core::elements::tui::{Modifier, TuiElement, TuiHoverable, TuiStyle, TuiText};
use warpui_core::elements::MouseStateHandle;

/// Which unit a usage entry currently displays.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum UsageDisplayMode {
    /// Token count (`4 tok`).
    #[default]
    Tokens,
    /// Dollar cost (`$0.03`).
    Cost,
}

impl UsageDisplayMode {
    fn toggled(self) -> Self {
        match self {
            Self::Tokens => Self::Cost,
            Self::Cost => Self::Tokens,
        }
    }
}

/// How a token count is labeled.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TokenLabelForm {
    /// `4 tok` — the compact footer form.
    Short,
    /// `4 tokens` — the long transcript form.
    // Constructed only by tests today; the transcript usage row (CODE-1832)
    // is the first production consumer, so `expect` would be unfulfilled in
    // test builds.
    #[allow(dead_code)]
    Long,
}

/// The tokens⇄cost toggle behind a clickable usage entry. Owned by the
/// composing view (created once, cloned into render closures) so the display
/// mode and hover state survive element-tree rebuilds.
#[derive(Clone, Default)]
pub(crate) struct TokenCostToggle {
    mode: Rc<Cell<UsageDisplayMode>>,
    /// Hover state for the entry. Owned here (not created inline during
    /// render) so it survives element-tree rebuilds, following the GUI's
    /// `MouseStateHandle` pattern.
    hover_state: MouseStateHandle,
}

impl TokenCostToggle {
    /// Flips between token-count and cost display.
    fn toggle(&self) {
        self.mode.set(self.mode.get().toggled());
    }

    /// The entry's current text: the token count or the dollar cost.
    fn entry_text(&self, totals: ConversationUsageTotals) -> String {
        match self.mode.get() {
            UsageDisplayMode::Tokens => {
                format_token_count(totals.total_tokens, TokenLabelForm::Short)
            }
            UsageDisplayMode::Cost => format_cost(totals.cost_in_cents),
        }
    }

    /// Renders the clickable usage entry (`4 tok` ⇄ `$0.03`), dim like the
    /// rest of the footer metadata and brightened while hovered. A click
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

/// Formats a token count with the label form chosen by the surface:
/// `4 tok` / `4 tokens`, abbreviating large counts (`12.3k tok`, `1.2M tok`).
pub(crate) fn format_token_count(tokens: u64, form: TokenLabelForm) -> String {
    let count = abbreviate_count(tokens);
    let label = match form {
        TokenLabelForm::Short => "tok",
        TokenLabelForm::Long => {
            if tokens == 1 {
                "token"
            } else {
                "tokens"
            }
        }
    };
    format!("{count} {label}")
}

/// Formats an accumulated cost in US cents as dollars (`3.2` cents → `$0.03`).
pub(crate) fn format_cost(cost_in_cents: f64) -> String {
    format!("${:.2}", cost_in_cents / 100.0)
}

/// Abbreviates a count at thousand/million granularity so the footer entry
/// keeps a stable width: raw below 10k (`9999`), then `12.3k` / `1.2M` with a
/// trailing `.0` trimmed (`10k`, `1M`).
fn abbreviate_count(count: u64) -> String {
    const THOUSAND: f64 = 1_000.0;
    const MILLION: f64 = 1_000_000.0;
    let count_f = count as f64;
    if count_f >= MILLION {
        format!(
            "{}M",
            trim_trailing_zero(format!("{:.1}", count_f / MILLION))
        )
    } else if count >= 10_000 {
        format!(
            "{}k",
            trim_trailing_zero(format!("{:.1}", count_f / THOUSAND))
        )
    } else {
        count.to_string()
    }
}

/// Drops a trailing `.0` from a one-decimal formatted number.
fn trim_trailing_zero(value: String) -> String {
    value.strip_suffix(".0").map(str::to_owned).unwrap_or(value)
}

#[cfg(test)]
#[path = "usage_tests.rs"]
mod tests;
