//! Shared width allocation and text formatting for two-column TUI rows.

use warpui_core::elements::tui::{text_width, truncate_with_ellipsis};

/// Width policy for a group of two-column rows.
#[derive(Clone, Copy)]
pub(crate) struct TuiTwoColumnConstraints {
    /// Stable first-column width used when both columns fit comfortably.
    pub(crate) preferred_first_columns: usize,
    /// Smallest useful first-column width, including the trailing gap.
    pub(crate) minimum_first_columns: usize,
    /// Smallest useful second-column width.
    pub(crate) minimum_second_columns: usize,
    /// Width to reserve for the second column before growing the first.
    pub(crate) preferred_maximum_second_columns: usize,
    /// Blank columns separating the first and second columns.
    pub(crate) gap_columns: usize,
}

/// Shared column widths for a group of rows.
#[derive(Clone, Copy)]
pub(crate) struct TuiTwoColumnLayout {
    pub(crate) available_columns: usize,
    pub(crate) first_columns: usize,
    pub(crate) show_second: bool,
    gap_columns: usize,
}
impl TuiTwoColumnLayout {
    /// Uses the shared widths for a row that may omit its second value.
    pub(crate) fn with_second_visible(mut self, show_second: bool) -> Self {
        self.show_second = show_second;
        self
    }
}

/// Allocates shared widths for a group of two-column rows.
///
/// The second column is hidden when there are no complete rows or the
/// available width cannot satisfy both minimums. Otherwise, width is reserved
/// for a useful second column before surplus columns are given to long values
/// in the first column.
pub(crate) fn tui_two_column_layout<'a>(
    available_columns: usize,
    rows: impl IntoIterator<Item = (&'a str, &'a str)>,
    constraints: TuiTwoColumnConstraints,
) -> TuiTwoColumnLayout {
    let mut longest_first_columns: Option<usize> = None;
    let mut longest_second_columns: Option<usize> = None;
    for (first, second) in rows {
        longest_first_columns = Some(
            longest_first_columns
                .unwrap_or_default()
                .max(usize::from(text_width(first))),
        );
        longest_second_columns = Some(
            longest_second_columns
                .unwrap_or_default()
                .max(usize::from(text_width(second))),
        );
    }

    let single_column_layout = || TuiTwoColumnLayout {
        available_columns,
        first_columns: available_columns,
        show_second: false,
        gap_columns: 0,
    };
    let Some((longest_first_columns, longest_second_columns)) =
        longest_first_columns.zip(longest_second_columns)
    else {
        return single_column_layout();
    };
    let minimum_two_column_width =
        constraints.minimum_first_columns + constraints.minimum_second_columns;
    if available_columns < minimum_two_column_width {
        return single_column_layout();
    }

    let preferred_first_columns = constraints
        .preferred_first_columns
        .max(longest_first_columns.saturating_add(constraints.gap_columns));
    let preferred_second_columns = longest_second_columns.clamp(
        constraints.minimum_second_columns,
        constraints.preferred_maximum_second_columns,
    );
    let baseline_width = constraints.preferred_first_columns + preferred_second_columns;
    let first_columns = if available_columns >= baseline_width {
        let growth_columns = available_columns - baseline_width;
        preferred_first_columns.min(
            constraints
                .preferred_first_columns
                .saturating_add(growth_columns),
        )
    } else {
        constraints
            .preferred_first_columns
            .min(available_columns.saturating_sub(constraints.minimum_second_columns))
    };

    TuiTwoColumnLayout {
        available_columns,
        first_columns: first_columns.max(constraints.minimum_first_columns),
        show_second: true,
        gap_columns: constraints.gap_columns,
    }
}

/// Ellipsizes the first value when needed and pads it to the shared width.
pub(crate) fn format_tui_first_column(text: &str, layout: TuiTwoColumnLayout) -> String {
    let (first_columns, gap_columns) = if layout.show_second {
        (layout.first_columns, layout.gap_columns)
    } else {
        (layout.available_columns, 0)
    };
    let content_columns = first_columns.saturating_sub(gap_columns);
    let mut formatted = truncate_with_ellipsis(text, content_columns);
    if layout.show_second {
        let formatted_columns = usize::from(text_width(&formatted));
        formatted.push_str(&" ".repeat(first_columns - formatted_columns));
    }
    formatted
}

#[cfg(test)]
#[path = "tui_column_layout_tests.rs"]
mod tests;
