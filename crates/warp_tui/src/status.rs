//! Shared status presentation for compact TUI transcript rows.

use warpui_core::elements::tui::TuiStyle;

use crate::tui_builder::TuiUiBuilder;

/// Coarse UI state shared by tool calls and orchestration participants.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TuiStatusState {
    /// Content is still being assembled and may be incomplete.
    Constructing,
    /// Work is queued but has not started.
    Pending,
    /// Work is blocked on user input or approval.
    Blocked,
    /// Work is actively executing.
    Running,
    /// Work completed successfully.
    Succeeded,
    /// Work completed with an error.
    Failed,
    /// Work was cancelled.
    Cancelled,
}

impl TuiStatusState {
    /// The compact leading glyph for this status.
    pub(crate) fn glyph(self) -> &'static str {
        match self {
            Self::Constructing | Self::Pending => "○",
            Self::Blocked | Self::Cancelled => "■",
            Self::Running => "●",
            Self::Succeeded => "✓",
            Self::Failed => "×",
        }
    }

    /// The semantic theme style for this status glyph.
    pub(crate) fn glyph_style(self, builder: &TuiUiBuilder) -> TuiStyle {
        match self {
            Self::Constructing | Self::Pending => builder.dim_text_style(),
            Self::Blocked | Self::Running => builder.attention_glyph_style(),
            Self::Succeeded => builder.success_glyph_style(),
            Self::Failed => builder.error_text_style(),
            Self::Cancelled => builder.muted_text_style(),
        }
    }

    /// The semantic text style paired with this status.
    pub(crate) fn label_style(self, builder: &TuiUiBuilder) -> TuiStyle {
        match self {
            Self::Constructing | Self::Pending => builder.dim_text_style(),
            Self::Blocked | Self::Running | Self::Succeeded | Self::Failed | Self::Cancelled => {
                builder.primary_text_style()
            }
        }
    }
}

#[cfg(test)]
#[path = "status_tests.rs"]
mod tests;
