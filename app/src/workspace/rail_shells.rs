//! Which of a project's **live** task rows the rail draws when the user has
//! asked it to hide shells that have nothing to do with an agent.
//!
//! A plain `zsh` sitting in a repo is a real tab, but in a rail whose job is
//! "which agent needs me?" a column of them is pure noise. The
//! `rail_hide_shells_without_agents` setting drops those rows — and only
//! those: a row is kept the moment anything agent-related has ever touched its
//! pane (a live CLI agent, a Warp Agent Mode conversation, or a stored/scanned
//! session handle), because a finished agent's tab is exactly the one a user
//! comes back to.
//!
//! Dormant and scanned rows never reach this filter at all: they exist only
//! because a session does, so they are agent rows by construction.
//!
//! Everything here is a pure function of plain data, in the same style as
//! [`rail_triage`](super::rail_triage), so the exemptions can be unit-tested
//! without a renderer. The two exemptions are what stop the filter from being
//! disorienting:
//!
//! - **The active tab is never hidden.** Hiding the row of the tab the user is
//!   looking at would leave the rail disagreeing with the terminal on screen.
//! - **The selected project keeps a row.** If hiding would leave the selected
//!   project with no task rows at all, its most-recently-used tab stays, so the
//!   project the user is standing in never collapses to a bare header.
//!
//! What is dropped is always reported: the caller renders one dim
//! [`hidden_shells_label`] row per project, so a project never appears to have
//! silently lost tabs.
//!
//! A hidden row can never be one that needs the user. `has_agent` is resolved
//! by [`pane_has_agent`](super::tab_title::pane_has_agent) from the same
//! sources a row's *status* comes from, so anything the rail could tint orange,
//! red or green is agent-backed by construction and stays visible.

/// One live task row the filter is deciding about, in rail order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RailLiveRow {
    /// Index into the workspace's tabs — the row's identity everywhere else in
    /// the rail.
    pub tab_index: usize,
    /// Whether anything agent-related is or was attached to this tab: a live
    /// CLI agent session, a Warp Agent Mode conversation, or a stored session
    /// handle for its pane. `false` means a plain shell.
    pub has_agent: bool,
}

/// The inputs the exemptions need, gathered at the call site where they are
/// known.
///
/// `fallback_row` is passed in rather than derived here because recency lives
/// in the workspace's MRU order, not in rail order: the filter should not have
/// to guess which row "most recent" means.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RailShellFilter {
    /// The setting: `false` leaves every row visible.
    pub hide_shells: bool,
    /// The workspace's active tab, which is never hidden.
    pub active_tab: Option<usize>,
    /// The row to keep when the filter would otherwise empty this project —
    /// set only for the selected project, whose most-recently-used tab it is.
    pub fallback_row: Option<usize>,
}

/// What the rail should draw for one project's live rows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RailLiveRowsView {
    /// The tab indices to render, in the order they were given.
    pub visible: Vec<usize>,
    /// How many rows were dropped. Rendered as the single dim "N shells"
    /// summary row, so a project never silently loses tabs.
    pub hidden_shells: usize,
}

/// Applies the shell filter to one project's live rows.
///
/// Order is preserved: the filter only ever removes rows, so the rail's spatial
/// memory survives toggling it on and off.
pub fn visible_live_rows(rows: &[RailLiveRow], filter: RailShellFilter) -> RailLiveRowsView {
    if !filter.hide_shells {
        return RailLiveRowsView {
            visible: rows.iter().map(|row| row.tab_index).collect(),
            hidden_shells: 0,
        };
    }

    let mut visible: Vec<usize> = rows
        .iter()
        .filter(|row| row.has_agent || filter.active_tab == Some(row.tab_index))
        .map(|row| row.tab_index)
        .collect();

    // A project that would keep nothing falls back to one row rather than to
    // none — but only where the user is actually standing, so unselected
    // projects still collapse to their header as intended.
    if visible.is_empty()
        && let Some(fallback) = filter
            .fallback_row
            .filter(|fallback| rows.iter().any(|row| row.tab_index == *fallback))
    {
        visible.push(fallback);
    }

    RailLiveRowsView {
        hidden_shells: rows.len() - visible.len(),
        visible,
    }
}

/// The dim summary row's text, or `None` when nothing was hidden.
///
/// Singular below two so the row reads as a sentence rather than a counter.
pub fn hidden_shells_label(hidden: usize) -> Option<String> {
    match hidden {
        0 => None,
        1 => Some("1 shell".to_owned()),
        many => Some(format!("{many} shells")),
    }
}

#[cfg(test)]
#[path = "rail_shells_tests.rs"]
mod tests;
