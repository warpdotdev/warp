//! Which of a project rail's **live** task rows the "clear shells" button
//! closes, and what it says before it does.
//!
//! A plain `zsh` sitting in a repo is a real tab, but in a rail whose job is
//! "which agent needs me?" a column of them is pure noise. The rail used to
//! *hide* those rows behind a persisted funnel toggle; it now closes them
//! instead. The reason is that a filter and a close both mean "get these out of
//! my way", and shipping both is one control too many — but only one of them
//! actually reclaims the tab, the process and the memory. Hiding left the
//! shells running behind a switch the user then had to remember they had
//! flipped.
//!
//! Because the action is destructive it is **confirm-then-close**, not a
//! toggle: a mis-click on a filter costs a second click to undo, a mis-click
//! here costs tabs. The dialog names the count and the projects so the user can
//! recognise what is about to disappear.
//!
//! Everything here is a pure function of plain data, in the same style as
//! [`rail_triage`](super::rail_triage), so the exemptions can be unit-tested
//! without a renderer or a live workspace. The exemptions are what make a bulk
//! close safe:
//!
//! - **The active tab is never closed.** Closing the tab the user is looking at
//!   yanks the terminal out from under them, and it is also the one tab they
//!   demonstrably still want.
//! - **A busy pane is never closed.** A long-running command is work in
//!   progress; ending it is the user's call, made deliberately, not a side
//!   effect of tidying the rail.
//! - **Anything agent-backed is never closed.** `has_agent` is resolved by
//!   [`pane_has_agent`](super::tab_title::pane_has_agent) from the same sources
//!   a row's *status* comes from, so anything the rail could tint orange, red
//!   or green is agent-backed by construction — a row that could be nagging you
//!   can never be the one that is closed.
//!
//! Dormant and scanned rows are not candidates at all: they have no open tab,
//! so there is nothing to close.

/// One live task row the action is deciding about, in rail order.
///
/// Every field is a reason *not* to close, resolved at the call site where the
/// signal lives. Keeping them separate rather than collapsing them into one
/// `eligible` bool is what lets the tests state each exemption on its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RailShellRow {
    /// Index into the workspace's tabs — the row's identity everywhere else in
    /// the rail.
    pub tab_index: usize,
    /// Whether anything agent-related is or was attached to this tab: a live
    /// CLI agent session, a non-passive Warp Agent Mode conversation, or a
    /// stored session handle for its pane. `false` means a plain shell.
    pub has_agent: bool,
    /// Whether the tab is running a long-running command right now, from the
    /// same summary the quit warning counts processes with.
    pub is_busy: bool,
    /// Whether the tab is one terminal pane and nothing else. A split, a code
    /// pane or an off-tree child agent pane makes this `false`: closing such a
    /// tab would take a sibling with it that no exemption above has vouched
    /// for.
    pub is_lone_terminal: bool,
    /// Whether the session is being shared with someone else. Closing it ends
    /// their session too, so it is never swept up in a bulk tidy.
    pub is_shared: bool,
    /// The project this row is listed under, for the confirmation text.
    pub project: String,
}

/// The rows the "clear shells" action would close, in rail order.
///
/// `active_tab` is passed in rather than derived because the rail renders rows
/// for every project while "active" is a workspace-wide fact.
pub fn shells_to_clear(rows: &[RailShellRow], active_tab: Option<usize>) -> Vec<usize> {
    rows.iter()
        .filter(|row| {
            row.is_lone_terminal
                && !row.has_agent
                && !row.is_busy
                && !row.is_shared
                && active_tab != Some(row.tab_index)
        })
        .map(|row| row.tab_index)
        .collect()
}

/// The distinct projects the given rows belong to, in rail order.
///
/// Used only for the confirmation's second line, so duplicates are collapsed
/// but the rail's ordering is kept: the user reads the dialog against the list
/// they are looking at.
pub fn projects_of(rows: &[RailShellRow], tab_indices: &[usize]) -> Vec<String> {
    let mut projects: Vec<String> = Vec::new();
    for row in rows
        .iter()
        .filter(|row| tab_indices.contains(&row.tab_index))
    {
        if !projects.contains(&row.project) {
            projects.push(row.project.clone());
        }
    }
    projects
}

/// How many projects the confirmation names before it gives up and counts.
///
/// Three fits on one line at the rail's width; past that the list stops being
/// something you can take in at a glance, which is the only job it has.
const MAX_NAMED_PROJECTS: usize = 3;

/// The confirmation's headline, e.g. `Close 23 shells with no agent?`.
///
/// Singular below two so the question reads as a sentence rather than a
/// counter.
pub fn clear_shells_prompt(count: usize) -> String {
    if count == 1 {
        "Close 1 shell with no agent?".to_owned()
    } else {
        format!("Close {count} shells with no agent?")
    }
}

/// The confirmation's detail line: which projects lose tabs, and the promise
/// that the exemptions held.
pub fn clear_shells_detail(projects: &[String]) -> String {
    let named = if projects.len() > MAX_NAMED_PROJECTS {
        format!(
            "{} and {} more",
            projects[..MAX_NAMED_PROJECTS].join(", "),
            projects.len() - MAX_NAMED_PROJECTS
        )
    } else {
        projects.join(", ")
    };
    if named.is_empty() {
        "The active tab, anything with an agent, and anything running a command stay open."
            .to_owned()
    } else {
        format!(
            "In {named}. The active tab, anything with an agent, and anything running a command stay open."
        )
    }
}

/// What the toast says once the shells are gone.
pub fn cleared_shells_label(count: usize) -> String {
    if count == 1 {
        "Closed 1 shell".to_owned()
    } else {
        format!("Closed {count} shells")
    }
}

#[cfg(test)]
#[path = "rail_clear_shells_tests.rs"]
mod tests;
