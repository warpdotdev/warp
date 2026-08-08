//! The user's ordered project priority list, and the rail's two-band layout.
//!
//! Priority is *one ordered list*, not a per-project score: rank is the
//! position in that list. Storing the order rather than a map of rank numbers
//! is what makes reordering a splice and makes rank collisions unrepresentable
//! — there is no state in which two projects both claim rank 3.
//!
//! Entries are [`ProjectKey`] storage strings
//! ([`ProjectKey::to_storage_key`]), never raw cwds, so every worktree of a
//! repository shares one rank. That is deliberate: the future consumer is
//! per-project token budgets, which want a budget per repo, not per checkout.
//!
//! [`ProjectId::Other`] collects tabs with no detectable repo or directory. It
//! has no stable identity across restarts, so it can never be ranked and
//! always sits in the unranked band.

use super::project_key::ProjectKey;
use super::project_layout::{ProjectEntry, ProjectId};

/// The user's project priority order, highest first.
///
/// Persisted as a settings value (registered in
/// [`tab_settings`](super::tab_settings)); the stored form is the ordered list
/// of [`ProjectKey::to_storage_key`] strings.
#[derive(
    Default,
    Debug,
    Clone,
    serde::Serialize,
    serde::Deserialize,
    PartialEq,
    Eq,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(description = "Ordered list of prioritized projects, highest priority first.")]
pub struct ProjectPriorities(pub(crate) Vec<String>);

impl ProjectPriorities {
    /// This project's priority rank, or `None` when it is unranked.
    ///
    /// **Rank is the index**: 0 is the highest priority, and the rail renders
    /// `rank + 1` so the user sees a 1-based number.
    ///
    /// This returns the number, not merely an ordering, because rank is a
    /// readable attribute of a project rather than only a sort key: per-project
    /// coding-agent **token budgets** will read the same value to size a
    /// project's allowance (spec §4). Sorting the rail is the first consumer,
    /// not the only one.
    pub fn rank_of(&self, key: &ProjectKey) -> Option<usize> {
        let encoded = key.to_storage_key();
        self.0.iter().position(|entry| *entry == encoded)
    }

    /// The rank of a rail project bucket. [`ProjectId::Other`] is unrankable.
    pub fn rank_of_project(&self, id: &ProjectId) -> Option<usize> {
        match id {
            ProjectId::Key(key) => self.rank_of(key),
            ProjectId::Other => None,
        }
    }

    /// Whether this project is in the priority list at all.
    pub fn contains(&self, id: &ProjectId) -> bool {
        self.rank_of_project(id).is_some()
    }

    /// Returns a copy with `key` at rank 0.
    ///
    /// Any existing entry is removed first, so re-adding an already-ranked
    /// project promotes it to the top instead of duplicating it — which would
    /// otherwise leave a second, permanently unreachable copy in the list.
    pub fn with_added_to_top(&self, key: &ProjectKey) -> Self {
        let encoded = key.to_storage_key();
        let mut entries = self.0.clone();
        entries.retain(|entry| *entry != encoded);
        entries.insert(0, encoded);
        Self(entries)
    }

    /// Returns a copy with `key` removed. A no-op when it is not ranked.
    pub fn with_removed(&self, key: &ProjectKey) -> Self {
        let encoded = key.to_storage_key();
        let mut entries = self.0.clone();
        entries.retain(|entry| *entry != encoded);
        Self(entries)
    }

    /// Returns a copy with `key` swapped one rank towards the top.
    ///
    /// A no-op when the project is unranked or already first: the menu hides
    /// the entry in those cases, but a keyboard/palette dispatch can still
    /// arrive when it does not apply, and silently doing nothing beats
    /// wrapping the top project round to the bottom.
    pub fn with_moved_up(&self, key: &ProjectKey) -> Self {
        match self.rank_of(key) {
            Some(rank) if rank > 0 => {
                let mut entries = self.0.clone();
                entries.swap(rank, rank - 1);
                Self(entries)
            }
            Some(_) | None => self.clone(),
        }
    }

    /// Returns a copy with `key` swapped one rank towards the bottom. A no-op
    /// when the project is unranked or already last.
    pub fn with_moved_down(&self, key: &ProjectKey) -> Self {
        match self.rank_of(key) {
            Some(rank) if rank + 1 < self.0.len() => {
                let mut entries = self.0.clone();
                entries.swap(rank, rank + 1);
                Self(entries)
            }
            Some(_) | None => self.clone(),
        }
    }

    /// Whether `id` can move up — false for unranked projects and the top one.
    pub fn can_move_up(&self, id: &ProjectId) -> bool {
        self.rank_of_project(id).is_some_and(|rank| rank > 0)
    }

    /// Whether `id` can move down — false for unranked projects and the last
    /// one.
    pub fn can_move_down(&self, id: &ProjectId) -> bool {
        self.rank_of_project(id)
            .is_some_and(|rank| rank + 1 < self.0.len())
    }
}

/// One row of the rail's project list, in render order.
///
/// Indices point into the `projects` slice the rows were built from, so the
/// banding stays a pure function of (projects, priorities) and can be tested
/// without a renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RailProjectRow {
    /// A project row. `rank` is `Some(index)` only in the ranked band.
    Project { index: usize, rank: Option<usize> },
    /// The thin "unranked" divider marking the boundary between the bands.
    UnrankedDivider,
}

/// Lays the rail's projects out as ranked band, divider, unranked band.
///
/// The unranked band keeps the incoming first-seen order untouched. Ordering
/// depends only on the priority list, never on agent activity: a sidebar's
/// value is spatial memory, so a project must not jump because one of its
/// agents changed state.
pub fn rail_project_rows(
    projects: &[ProjectEntry],
    priorities: &ProjectPriorities,
) -> Vec<RailProjectRow> {
    let mut ranked: Vec<(usize, usize)> = Vec::new();
    let mut unranked: Vec<usize> = Vec::new();
    for (index, entry) in projects.iter().enumerate() {
        match priorities.rank_of_project(&entry.id) {
            Some(rank) => ranked.push((rank, index)),
            None => unranked.push(index),
        }
    }
    // A list-shaped store cannot produce duplicate ranks, but the sort must
    // still be total; `sort_by_key` is stable, so first-seen order breaks any
    // tie a hand-edited settings file might smuggle in.
    ranked.sort_by_key(|(rank, _)| *rank);

    let mut rows: Vec<RailProjectRow> = ranked
        .into_iter()
        .map(|(rank, index)| RailProjectRow::Project {
            index,
            rank: Some(rank),
        })
        .collect();
    // The divider marks a boundary, so it only earns its row when there is one
    // to mark. With a single band it would read as a section header for a
    // section that is the whole list.
    if !rows.is_empty() && !unranked.is_empty() {
        rows.push(RailProjectRow::UnrankedDivider);
    }
    rows.extend(
        unranked
            .into_iter()
            .map(|index| RailProjectRow::Project { index, rank: None }),
    );
    rows
}

#[cfg(test)]
#[path = "project_priorities_tests.rs"]
mod tests;
