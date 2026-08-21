//! The reconciliation pass for automatic tab grouping.
//!
//! One entry point — [`Workspace::reconcile_tab_auto_group`] — decides and
//! applies the correct group for a single tab. It deliberately does *not* reuse
//! the manual grouping entry points in `tab_grouping.rs`: those clear pinned
//! flags, force-expand the destination group, and dispatch the inline rename
//! editor, none of which automation may do. It builds on the lower-level
//! primitives those paths share instead.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use ::settings::Setting;
use repo_metadata::repositories::DetectedRepositories;
use warpui::{EntityId, SingletonEntity, ViewContext, ViewHandle};

use super::{Workspace, group_has_single_member, group_member_indices};
use crate::pane_group::{PaneGroup, PaneId};
use crate::tab::SelectedTabColor;
use crate::workspace::project_key::{self, GitResolution, ProjectKey, ProjectKeyInput};
use crate::workspace::tab_group::{TabGroup, TabGroupId, auto_tab_grouping_available};
use crate::workspace::tab_settings::TabSettings;

/// The terminal pane a tab's project key follows, and the directory that pane
/// reported the last time it was looked at.
///
/// The anchor is deliberately *not* the focused pane: focusing a split checked
/// out from another repository must never move the tab (R9).
#[derive(Clone, Debug, PartialEq, Eq)]
struct TabAnchor {
    pane_id: PaneId,
    directory: Option<PathBuf>,
}

/// The resolver state automatic grouping keeps between reconciles.
///
/// In memory only: it merely has to survive between two directory changes, and
/// what has to survive a restart is persisted on the tab and the group instead.
#[derive(Default)]
pub(crate) struct AutoGroupingState {
    /// The project key each tab last resolved to, keyed by pane-group identity.
    ///
    /// Membership is tested against *this* key, never against the tab's current
    /// one: at the instant a tab's directory changes, a tab automation placed
    /// and a tab the user placed are state-identical — both sit in a group whose
    /// key no longer matches. Only the previously resolved key tells them apart.
    last_resolved_keys: HashMap<EntityId, ProjectKey>,
    /// Each tab's anchor pane and the directory it last reported, keyed by
    /// pane-group identity. Re-derived lazily, so a restored tab anchors to the
    /// first terminal pane of its restored pane group.
    anchors: HashMap<EntityId, TabAnchor>,
    /// Test-only stand-in for the anchor pane's working directory. A real
    /// terminal pane in a unit test has no shell and therefore no pwd, so the
    /// directory each pane reports is injected here instead. Keyed by pane so
    /// the anchor selection itself still runs for real.
    #[cfg(test)]
    pub(super) test_pane_directories: HashMap<PaneId, PathBuf>,
    /// Test-only stand-in for what repository detection knows about a
    /// directory. Absent means "no repository, and detection has answered".
    #[cfg(test)]
    pub(super) test_git_resolutions: HashMap<PathBuf, GitResolution>,
}

impl AutoGroupingState {
    fn last_resolved_key(&self, pane_group_id: EntityId) -> Option<&ProjectKey> {
        self.last_resolved_keys.get(&pane_group_id)
    }

    fn record_resolved_key(&mut self, pane_group_id: EntityId, key: ProjectKey) {
        self.last_resolved_keys.insert(pane_group_id, key);
    }

    /// Drops everything the resolver remembers about a tab that is gone for
    /// good.
    ///
    /// Both maps are keyed by pane-group identity and are only ever written on
    /// the way in, so without this they accumulate for the life of the window.
    /// `EntityId` is a monotonic counter, so a stale entry can never be adopted
    /// by a later tab — the cost is memory, not correctness, which is why this
    /// is upkeep rather than a guard.
    pub(super) fn forget(&mut self, pane_group_id: EntityId) {
        self.last_resolved_keys.remove(&pane_group_id);
        self.anchors.remove(&pane_group_id);
    }
}

/// Whether a stored group key came from git identity rather than from a plain
/// directory.
///
/// Nothing records which of the two a key is — the group table stores one
/// string — so it is derived from the shape git guarantees: `<repo>/.git` for a
/// normal checkout and `<repo>.git` for a bare one. A repository whose git
/// directory was relocated somewhere without that suffix reads as a directory
/// key; the only consequence is that it also becomes a prefix candidate for
/// non-git tabs, which no requirement depends on.
fn is_git_project_key(key: &ProjectKey) -> bool {
    key.path()
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".git"))
}

/// The key a group is carrying, rebuilt from the string the group table
/// persists it as.
///
/// `TabGroup::project_key` is a `String` because that is the column's type, so
/// every reader has to go back through [`ProjectKey::from_path`]; this is that
/// step, named once.
fn group_project_key(group: &TabGroup) -> Option<ProjectKey> {
    Some(ProjectKey::from_path(PathBuf::from(
        group.project_key.as_ref()?,
    )))
}

/// Whether the color a tab carries is one automation put there rather than one
/// the user chose.
///
/// Derived rather than stored, exactly as a group's name provenance is: a color
/// matching what `previous_key` derives was automation's and may be replaced,
/// and anything else was the user's and may not. `Cleared` is the user
/// deliberately removing a color, so it is protected too; `Unset` is a tab with
/// no color to protect.
///
/// The palette holds six colors, so a user who picks precisely the one their
/// key derives reads as automation and is repainted by the next move. The name
/// heuristic has the identical hole — a rename that happens to match the derived
/// name is also indistinguishable — and the cost is one color the user can set
/// again, which is cheaper than persisting a provenance flag that no other
/// automation state needs.
///
/// `previous_key` is the key of the group the tab sits in *before* the
/// transition being applied. Reading it off the group rather than off a per-tab
/// record of the last resolved key is what makes provenance survive a restart:
/// the group table persists the key, while the resolver's `last_resolved_keys`
/// is in-memory only, so after a restart every colored tab would otherwise read
/// as the user's and never follow its directory again.
///
/// Provenance is a property of the *tab*, not of the project: clearing a tab's
/// color says "not this tab", not "never color this project", so another tab in
/// the same project is still colored.
fn tab_color_is_derived(color: SelectedTabColor, previous_key: Option<&ProjectKey>) -> bool {
    match color {
        SelectedTabColor::Unset => true,
        SelectedTabColor::Cleared => false,
        SelectedTabColor::Color(color) => {
            previous_key.is_some_and(|key| project_key::derived_color(key) == color)
        }
    }
}

impl Workspace {
    /// Reconciles one tab's group membership against the project key its
    /// directory currently resolves to.
    ///
    /// Identified by pane group rather than by tab index on purpose: repository
    /// detection answers an arbitrary number of frames after the directory
    /// changed, so an index captured before the answer arrived can address a
    /// different tab by now. The identity is resolved to an index immediately
    /// before any mutation, and a vanished identity is a no-op.
    ///
    /// `resolved_key` is `None` whenever project identity could not be
    /// established — no directory, or detection still pending. That is not a
    /// manual act, so the tab is left exactly where it is and stays queued for
    /// placement.
    ///
    /// Callers own resolving the key (and therefore which pane anchors it).
    pub fn reconcile_tab_auto_group(
        &mut self,
        pane_group_id: EntityId,
        resolved_key: Option<ProjectKey>,
        ctx: &mut ViewContext<Self>,
    ) {
        if !auto_tab_grouping_available() {
            return;
        }
        let Some(tab_index) = self.tab_index_for_pane_group(pane_group_id) else {
            return;
        };

        // A pinned tab is never grouped and never loses its pin. Skipping it
        // outright is what keeps automation clear of the manual join path,
        // whose only way into a group clears the pin.
        if self.tabs[tab_index].pinned {
            return;
        }

        // Identity is unknown: leave the tab where it is, and leave the marker
        // set so it is still placed once its key does resolve.
        let Some(key) = resolved_key else {
            return;
        };
        let key_storage = key.to_storage_string();

        let current_group_id = self.tabs[tab_index].group_id;
        let current_group_key = current_group_id.and_then(|group_id| {
            self.tab_groups
                .get(&group_id)
                .and_then(|group| group.project_key.clone())
        });

        // Already in the group for its current key. Nothing to move, but the
        // tab is demonstrably under automation again — this is also how a tab
        // the user detached re-attaches by being put back where it belongs.
        if current_group_key.as_deref() == Some(key_storage.as_str()) {
            let was_awaiting_placement =
                std::mem::replace(&mut self.tabs[tab_index].placed_by_automation, false);
            // Colored here too, not only on the paths that move the tab: a tab
            // born straight into its own project's group — inherited from the
            // active tab, or reopened into it — is already where it belongs and
            // reaches no other colouring path. Judged against the group's own
            // key, so a colour the user set on this tab still survives.
            let recolored = self.apply_derived_tab_color(tab_index, Some(&key), Some(&key), ctx);
            self.auto_grouping_state
                .record_resolved_key(pane_group_id, key);
            if was_awaiting_placement || recolored {
                ctx.dispatch_global_action("workspace:save_app", ());
                ctx.notify();
            }
            return;
        }

        // Under automation? Either the group the tab sits in still carries the
        // key the tab last resolved to — which is exactly the state a tracked
        // tab is in the moment its directory changes — or the tab has never
        // been placed at all. Any other mismatch is a placement the user made,
        // and a manual act is never undone.
        let previous_key_storage = self
            .auto_grouping_state
            .last_resolved_key(pane_group_id)
            .map(ProjectKey::to_storage_string);
        let sits_in_previous_keys_group = match (&current_group_key, &previous_key_storage) {
            (Some(group_key), Some(previous_key)) => group_key == previous_key,
            _ => false,
        };
        let under_automation =
            sits_in_previous_keys_group || self.tabs[tab_index].placed_by_automation;

        // Record before any early return: the next reconcile has to compare
        // against the key seen now, including for a tab left alone here, or a
        // detached tab dragged back into the group it originally came from
        // would read as tracked again.
        self.auto_grouping_state
            .record_resolved_key(pane_group_id, key.clone());

        if !under_automation {
            return;
        }

        let active_pane_group_id = self
            .tabs
            .get(self.active_tab_index)
            .map(|tab| tab.pane_group.id());

        if let Some(destination_group_id) = self.group_for_project_key(&key_storage) {
            self.join_keyed_group(tab_index, destination_group_id, ctx);
        } else if let Some(group_id) = current_group_id
            && self
                .tab_groups
                .get(&group_id)
                .is_some_and(|group| group.project_key.is_some())
            && group_has_single_member(&self.tabs, group_id)
        {
            // Re-key the group in place instead of destroying and recreating
            // it, so a group holding a single tab does not flicker on every
            // `cd`. The tab does not move.
            self.rekey_group_in_place(group_id, &key, ctx);
        } else {
            self.create_keyed_group_for_tab(tab_index, &key, ctx);
        }

        // A reorder must not go through the normal activation path, so the
        // active tab is re-seated by identity.
        self.restore_active_tab_index(active_pane_group_id);

        if let Some(previous_group_id) = current_group_id {
            self.prune_empty_tab_group(previous_group_id, ctx);
        }

        // The tab moved, so its index may have changed: clear the marker by
        // identity rather than by the index captured above.
        if let Some(tab_index) = self.tab_index_for_pane_group(pane_group_id) {
            self.tabs[tab_index].placed_by_automation = false;
        }

        ctx.dispatch_global_action("workspace:save_app", ());
        ctx.notify();
    }

    /// Retires the queued-for-placement marker on a tab the user has just
    /// placed by hand.
    ///
    /// This is the *only* automation state a manual placement writes.
    /// Tracked-ness itself is derived from where the tab sits (KTD5) and is
    /// deliberately never stored, so a drop needs to record nothing for R13 to
    /// hold — with one exception. `placed_by_automation` outranks placement in
    /// [`Workspace::reconcile_tab_auto_group`]'s derivation, because a tab that
    /// has never been placed has no placement worth reading. A tab still
    /// carrying the marker — newly created, arrived from another window,
    /// reopened without its group, or just unpinned — would therefore be pulled
    /// back out of the group the user dropped it into by the first reconcile
    /// that can resolve its key, silently undoing a manual act. Retiring the
    /// marker is what stops that; everything after it is ordinary derivation.
    pub(super) fn note_manual_tab_placement(&mut self, tab_index: usize) {
        if let Some(tab) = self.tabs.get_mut(tab_index) {
            tab.placed_by_automation = false;
        }
    }

    /// Points a group the user just created from a single tab at that tab's
    /// project, per R14.
    ///
    /// Without this the new group carries no key, so it matches nothing the
    /// tab can ever resolve to and the tab stays detached forever — "give this
    /// tab its own group" would be a one-way door out of automation. Adopting
    /// the key instead makes the group the tab's own project's group, which is
    /// exactly the state R14 re-attaches from.
    ///
    /// Only for single-tab creation: a group made from a multi-tab selection
    /// has no one project to adopt, and stays an ordinary manual group.
    ///
    /// `previous_key` is the key of the group the tab left, captured before
    /// creating this one possibly emptied and pruned it. It is what judges the
    /// colour the tab still carries: run on the sole member of its own
    /// project's group, "new group from this tab" replaces one group with
    /// another for the same project, and the tab must keep a colour the user
    /// chose there rather than have automation launder it into its own.
    pub(super) fn adopt_project_key_for_new_group(
        &mut self,
        group_id: TabGroupId,
        pane_group_id: EntityId,
        previous_key: Option<ProjectKey>,
        ctx: &mut ViewContext<Self>,
    ) {
        if !self.auto_grouping_enabled(ctx) {
            return;
        }
        // No project to adopt: the group stays unkeyed — an ordinary manual
        // group — and the tab stays detached. Placing it somewhere keyed is
        // still the way back under automation.
        let Some(key) = self.resolve_project_key_for_tab(pane_group_id, ctx) else {
            return;
        };
        if !self.tab_groups.contains_key(&group_id) {
            return;
        }
        // The project already has a group here — the tab was pulled out of it,
        // not out of nowhere. Adopting the key would leave the window with two
        // groups claiming one project under the same name, and the tab would be
        // pulled straight back into whichever one `group_for_project_key`
        // picks. "Give this tab its own group" is honoured as an ordinary
        // manual group instead, which R14 re-attaches from just the same.
        let existing = self.group_for_project_key(&key.to_storage_string());
        if existing.is_some_and(|existing| existing != group_id) {
            return;
        }

        if let Some(group) = self.tab_groups.get_mut(&group_id) {
            group.project_key = Some(key.to_storage_string());
        }
        // Named after the key is stored so the name is qualified against every
        // key in the window, this one included; then the window's other
        // automatic names are re-derived against the key just added.
        let name = self.derived_group_name(&key);
        if let Some(group) = self.tab_groups.get_mut(&group_id) {
            group.name = Some(name);
        }
        // The tab keeps the colour it walked in with; only its provenance is
        // re-judged, against the key of the group it left. A colour the user set
        // or cleared there survives the replacement, and one automation derived
        // is re-derived for the key this group now holds.
        if let Some(tab_index) = self.tab_index_for_pane_group(pane_group_id) {
            self.apply_derived_tab_color(tab_index, Some(&key), previous_key.as_ref(), ctx);
        }
        self.requalify_derived_group_names();

        // Re-attaches through the ordinary derivation rather than by writing
        // tracked-ness: the group now carries exactly the key the tab resolves
        // to, which reconcile already reads as "already correct". It also
        // records the resolved key, which the next reconcile compares against.
        self.reconcile_tab_auto_group(pane_group_id, Some(key), ctx);
    }

    /// The project key of a group a tab is about to leave, for
    /// [`Workspace::adopt_project_key_for_new_group`] to judge that tab's colour
    /// against.
    ///
    /// Call before the move; leaving may prune the group, and a group that no
    /// longer exists yields `None`.
    pub(crate) fn project_key_of_group(&self, group_id: TabGroupId) -> Option<ProjectKey> {
        group_project_key(self.tab_groups.get(&group_id)?)
    }

    /// Appends the tab to the end of `group_id`'s contiguous run.
    ///
    /// Unlike the manual join path this leaves the destination group's collapse
    /// state alone: collapsing is a manual act automation must not undo.
    fn join_keyed_group(
        &mut self,
        tab_index: usize,
        group_id: TabGroupId,
        ctx: &mut ViewContext<Self>,
    ) {
        // Computed before membership changes, so the slot is past the group's
        // existing members rather than past the tab we are about to add.
        let target = self.index_after_group(group_id).unwrap_or(self.tabs.len());
        // Recolored before the move, while `tab_index` still addresses this tab:
        // the color depends on which group the tab joins, never on where it
        // lands in the strip.
        let previous_key = self.project_key_of_tabs_group(tab_index);
        let key = self.tab_groups.get(&group_id).and_then(group_project_key);
        self.apply_derived_tab_color(tab_index, key.as_ref(), previous_key.as_ref(), ctx);
        self.assign_tab_to_group(tab_index, Some(group_id), ctx);
        self.move_tab_to_index(tab_index, target, ctx);
    }

    /// The project key of the group the tab at `tab_index` sits in, which is
    /// what any color already on that tab would have been derived from.
    pub(super) fn project_key_of_tabs_group(&self, tab_index: usize) -> Option<ProjectKey> {
        let group_id = self.tabs.get(tab_index)?.group_id?;
        group_project_key(self.tab_groups.get(&group_id)?)
    }

    /// Gives the tab at `tab_index` the color `key` derives, unless the color it
    /// carries now is the user's.
    ///
    /// `key` is `None` when the tab is leaving automation's reach — ungrouped by
    /// hand, or moved into a group carrying no project. Automation then takes
    /// its color back off rather than replacing it, so a detached tab stops
    /// wearing the project it left instead of advertising one it is no longer
    /// in. `Unset` rather than `Cleared`, so the tab falls back through to its
    /// directory color exactly as an untouched tab does.
    ///
    /// `previous_key` is the key the color it carries now would have been
    /// derived from: the key of the group it is leaving, and `None` for a tab
    /// that was in no keyed group.
    /// Returns whether the tab's colour actually changed, so the callers that
    /// are not already saving and notifying for a move can do so only when
    /// there is something to save.
    pub(super) fn apply_derived_tab_color(
        &mut self,
        tab_index: usize,
        key: Option<&ProjectKey>,
        previous_key: Option<&ProjectKey>,
        ctx: &warpui::AppContext,
    ) -> bool {
        if !self.auto_group_colors_enabled(ctx) {
            return false;
        }
        let color = match key {
            Some(key) => SelectedTabColor::Color(project_key::derived_color(key)),
            None => SelectedTabColor::Unset,
        };
        let Some(tab) = self.tabs.get_mut(tab_index) else {
            return false;
        };
        if !tab_color_is_derived(tab.selected_color, previous_key) || tab.selected_color == color {
            return false;
        }
        tab.selected_color = color;
        true
    }

    /// Repaints every member of `group_id` for `key`.
    ///
    /// For the transitions that change what a whole group stands for — a re-key,
    /// or the sweep — rather than the ones that move a single tab between
    /// groups.
    fn apply_derived_color_to_group_members(
        &mut self,
        group_id: TabGroupId,
        key: &ProjectKey,
        previous_key: Option<&ProjectKey>,
        ctx: &warpui::AppContext,
    ) {
        if !self.auto_group_colors_enabled(ctx) {
            return;
        }
        let members: Vec<usize> = group_member_indices(&self.tabs, group_id).collect();
        for tab_index in members {
            self.apply_derived_tab_color(tab_index, Some(key), previous_key, ctx);
        }
    }

    /// Takes automation's color back off a tab that is leaving `group_id`,
    /// leaving a color the user set or cleared alone.
    ///
    /// The manual ungroup paths call this: automation itself never strands a
    /// tab outside a group, so without it the only way to lose a derived color
    /// would be to clear it by hand.
    pub(super) fn clear_derived_tab_color_on_leaving(
        &mut self,
        tab_index: usize,
        group_id: TabGroupId,
        ctx: &warpui::AppContext,
    ) {
        let previous_key = self.tab_groups.get(&group_id).and_then(group_project_key);
        self.apply_derived_tab_color(tab_index, None, previous_key.as_ref(), ctx);
    }

    /// Points an existing group at `key`, keeping a name the user set.
    ///
    /// Name provenance is derived rather than stored: a name that differs from
    /// what the group's own key would have produced was typed by the user and
    /// survives the re-key; one that matches was derived and is replaced. The
    /// group's color follows the same rule, in
    /// [`Workspace::apply_derived_group_color`].
    fn rekey_group_in_place(
        &mut self,
        group_id: TabGroupId,
        key: &ProjectKey,
        ctx: &warpui::AppContext,
    ) {
        let Some(group) = self.tab_groups.get(&group_id) else {
            return;
        };
        let previous_key = group_project_key(group);
        let name_was_derived = match (&group.name, &previous_key) {
            // An unnamed group has nothing of the user's to protect.
            (None, _) => true,
            (Some(name), Some(previous_key)) => project_key::is_derived_name(previous_key, name),
            // A name with no key behind it cannot be shown to be derived.
            (Some(_), None) => false,
        };

        if let Some(group) = self.tab_groups.get_mut(&group_id) {
            group.project_key = Some(key.to_storage_string());
        }
        if name_was_derived {
            // Derived after the re-key so the name is qualified against the
            // window's keys as they stand now.
            let name = self.derived_group_name(key);
            if let Some(group) = self.tab_groups.get_mut(&group_id) {
                group.name = Some(name);
            }
        }
        // Every member follows the key the group now carries: a re-key changes
        // what the whole group stands for, not where any one tab sits.
        self.apply_derived_color_to_group_members(group_id, key, previous_key.as_ref(), ctx);
        self.requalify_derived_group_names();
    }

    /// Creates a group carrying `key` and moves the tab into it.
    ///
    /// Automation's own creation path: it never dispatches the deferred rename
    /// action both manual entry points end with, which would open the inline
    /// rename editor and steal focus.
    fn create_keyed_group_for_tab(
        &mut self,
        tab_index: usize,
        key: &ProjectKey,
        ctx: &mut ViewContext<Self>,
    ) {
        let previous_group_id = self.tabs[tab_index].group_id;
        // Anchor the new group at the tab's own slot, or — when the tab is
        // leaving a group — just past that group's last remaining member, so
        // the group it leaves stays contiguous. Then clamp past the pinned
        // region: the new group is unpinned and must not land inside it.
        let natural_target = previous_group_id
            .and_then(|group_id| self.index_after_group(group_id))
            .unwrap_or(tab_index);
        let target = self.clamp_to_unpinned_region(&self.tabs, natural_target);

        let mut group = TabGroup::new();
        let group_id = group.id;
        group.project_key = Some(key.to_storage_string());
        self.tab_groups.insert(group_id, group);
        // Named after insertion so the name is qualified against every key in
        // the window, this one included; then the window's other automatic
        // names are re-derived against the key this group just added.
        let name = self.derived_group_name(key);
        if let Some(group) = self.tab_groups.get_mut(&group_id) {
            group.name = Some(name);
        }
        self.requalify_derived_group_names();

        // Recolored before the move, while `tab_index` still addresses this tab.
        // The key it is leaving is what judges the color it carries now: a tab
        // walking from one project's group into a group built for another must
        // give up the first project's color.
        let previous_key = self.project_key_of_tabs_group(tab_index);
        self.apply_derived_tab_color(tab_index, Some(key), previous_key.as_ref(), ctx);
        self.assign_tab_to_group(tab_index, Some(group_id), ctx);
        self.move_tab_to_index(tab_index, target, ctx);
    }

    /// The group carrying `key_storage`, if there is one. Ties break on the
    /// group's first member and then on its id, so the choice never depends on
    /// hash-map iteration order.
    fn group_for_project_key(&self, key_storage: &str) -> Option<TabGroupId> {
        self.tab_groups
            .values()
            .filter(|group| group.project_key.as_deref() == Some(key_storage))
            .min_by_key(|group| {
                let first_member = group_member_indices(&self.tabs, group.id)
                    .next()
                    .unwrap_or(usize::MAX);
                (first_member, group.id.0)
            })
            .map(|group| group.id)
    }

    /// The name automatic grouping would give a group keyed to `key`.
    fn derived_group_name(&self, key: &ProjectKey) -> String {
        let others = self.project_keys_in_window();
        project_key::display_name(key, others.iter())
    }

    /// Re-derives every automatic group name against the window's keys as they
    /// now stand.
    ///
    /// R8 qualifies *both* sides of a collision, and a name is stored once at
    /// keying time: without this, adding `/work/vendor/api` to a window that
    /// already groups `/work/services/api` would read as `api` + `vendor/api`.
    /// A name the user typed falls outside the two forms derivation can
    /// produce, so it is recognised and left alone.
    fn requalify_derived_group_names(&mut self) {
        let keys = self.project_keys_in_window();
        let renames: Vec<(TabGroupId, String)> = self
            .tab_groups
            .values()
            .filter_map(|group| {
                let key = group_project_key(group)?;
                let name = group.name.as_deref()?;
                if !project_key::is_derived_name(&key, name) {
                    return None;
                }
                let requalified = project_key::display_name(&key, keys.iter());
                (requalified != name).then_some((group.id, requalified))
            })
            .collect();

        for (group_id, name) in renames {
            if let Some(group) = self.tab_groups.get_mut(&group_id) {
                group.name = Some(name);
            }
        }
    }

    /// Every project key in use by a group in this window.
    fn project_keys_in_window(&self) -> Vec<ProjectKey> {
        self.tab_groups
            .values()
            .filter_map(group_project_key)
            .collect()
    }

    /// Resolves a tab's stable pane-group identity to its current index.
    ///
    /// Automatic grouping moves tabs between reconcile calls, so any index held
    /// across a call boundary can be stale. Callers that must survive such a
    /// move hold the [`EntityId`] and resolve it here at the point of use.
    pub(crate) fn tab_index_for_pane_group(&self, pane_group_id: EntityId) -> Option<usize> {
        self.tabs
            .iter()
            .position(|tab| tab.pane_group.id() == pane_group_id)
    }

    fn pane_group_for_id(&self, pane_group_id: EntityId) -> Option<ViewHandle<PaneGroup>> {
        self.tabs
            .iter()
            .find(|tab| tab.pane_group.id() == pane_group_id)
            .map(|tab| tab.pane_group.clone())
    }

    /// Whether automatic grouping should act on this window right now.
    ///
    /// [`Workspace::reconcile_tab_auto_group`] gates itself on the feature
    /// flags but deliberately not on the setting, so every entry point in this
    /// file owns that half of the check.
    pub(super) fn auto_grouping_enabled(&self, ctx: &warpui::AppContext) -> bool {
        auto_tab_grouping_available() && *TabSettings::as_ref(ctx).auto_group_tabs.value()
    }

    /// Whether automatic grouping should also be coloring the groups it keys.
    ///
    /// Layered over [`Workspace::auto_grouping_enabled`] rather than read on
    /// its own: a color derived from a project key is meaningless in a window
    /// where nothing is keyed by project.
    pub(super) fn auto_group_colors_enabled(&self, ctx: &warpui::AppContext) -> bool {
        self.auto_grouping_enabled(ctx) && *TabSettings::as_ref(ctx).auto_group_tab_colors.value()
    }

    /// Re-derives the tab's anchor pane and the directory it reports, recording
    /// both. Returns `true` when either changed since the last refresh.
    ///
    /// The anchor is the first terminal pane in the pane group's layout order,
    /// pinned there for the life of the pane: a later split adds panes without
    /// disturbing it, and only the anchor closing re-anchors, to the next
    /// remaining terminal pane. A restored tab has no recorded anchor, so it
    /// re-derives to the first terminal pane of the restored layout.
    ///
    /// "Closing" is tested against the pane group's membership rather than
    /// against visibility. A pane hidden for a move, a job, a temporary
    /// replacement or a child agent is still this tab's anchor; re-anchoring on
    /// a transient hide would move a two-split tab onto whichever repository the
    /// *other* split happens to be checked out from, and that new anchor is then
    /// recorded permanently.
    fn refresh_tab_anchor(&mut self, pane_group_id: EntityId, ctx: &mut ViewContext<Self>) -> bool {
        let Some(pane_group) = self.pane_group_for_id(pane_group_id) else {
            return false;
        };
        let recorded = self
            .auto_grouping_state
            .anchors
            .get(&pane_group_id)
            .map(|anchor| anchor.pane_id);

        let anchor_pane_id = {
            let group = pane_group.as_ref(ctx);
            let recorded_still_present = recorded.is_some_and(|pane_id| {
                group.has_pane_id(pane_id)
                    && group.terminal_view_from_pane_id(pane_id, ctx).is_some()
            });
            if recorded_still_present {
                recorded
            } else {
                // Choosing a *new* anchor still prefers a visible pane: a hidden
                // one has no place being adopted as the tab's project source.
                group
                    .visible_pane_ids()
                    .into_iter()
                    .find(|pane_id| group.terminal_view_from_pane_id(*pane_id, ctx).is_some())
            }
        };

        let Some(anchor_pane_id) = anchor_pane_id else {
            // No terminal pane at all — a settings, notebook or transcript tab.
            // It has no directory and therefore no project (R7).
            return self
                .auto_grouping_state
                .anchors
                .remove(&pane_group_id)
                .is_some();
        };

        let directory = self.anchor_directory(&pane_group, anchor_pane_id, ctx);
        let anchor = TabAnchor {
            pane_id: anchor_pane_id,
            directory,
        };
        let previous = self
            .auto_grouping_state
            .anchors
            .insert(pane_group_id, anchor.clone());
        previous.is_none_or(|previous| previous != anchor)
    }

    /// The anchor pane's canonical working directory, or `None` when it has no
    /// local session.
    fn anchor_directory(
        &self,
        pane_group: &ViewHandle<PaneGroup>,
        anchor_pane_id: PaneId,
        ctx: &warpui::AppContext,
    ) -> Option<PathBuf> {
        #[cfg(test)]
        if let Some(directory) = self
            .auto_grouping_state
            .test_pane_directories
            .get(&anchor_pane_id)
        {
            return Some(directory.clone());
        }

        let terminal_view = pane_group
            .as_ref(ctx)
            .terminal_view_from_pane_id(anchor_pane_id, ctx)?;
        let pwd = terminal_view
            .as_ref(ctx)
            .canonical_session_pwd_if_local(ctx)?;
        Some(pwd.as_path().to_path_buf())
    }

    /// What repository detection currently knows about `directory`.
    ///
    /// The lookup is the two-hop, I/O-free path from an already-canonical
    /// working directory to the repository's shared git directory — it runs on
    /// every directory change, so it must never touch the filesystem.
    ///
    /// A cache miss is ambiguous on its own, because detection only ever
    /// records repositories it *found*: there is no negative cache anywhere in
    /// `repo_metadata`, and no per-pane record of which directory the last
    /// answer was for. The pane's own last settled answer is the best available
    /// discriminator:
    ///
    /// - it was in a repository, and this directory is not under it (or the
    ///   lookup would have hit) — so the pane has moved somewhere detection has
    ///   not answered for yet, and `RepoChanged` is still coming. `Pending`.
    /// - it was in no repository — the answer for the directory it just left
    ///   was "not a repository", so treat this one the same. This is the case
    ///   R6 lives in: a non-git to non-git move emits no `RepoChanged` at all
    ///   (the detected root did not change), so waiting for one would leave
    ///   every such tab ungrouped forever.
    ///
    /// The second branch is wrong for exactly one shape — the first visit to an
    /// undetected repository from a non-git directory — and it self-corrects:
    /// the tab is placed on its directory key, stays tracked against that key,
    /// and the `RepoChanged` that follows re-keys it onto the shared git
    /// directory. It is never read as a manual placement, which is the property
    /// R21 protects.
    fn git_resolution(
        &self,
        pane_group: &ViewHandle<PaneGroup>,
        anchor_pane_id: PaneId,
        directory: &Path,
        ctx: &warpui::AppContext,
    ) -> GitResolution {
        #[cfg(test)]
        if let Some(resolution) = self.auto_grouping_state.test_git_resolutions.get(directory) {
            return resolution.clone();
        }

        if let Some(repository) = DetectedRepositories::as_ref(ctx)
            .get_local_watched_repo_for_canonical_path(directory, ctx)
        {
            let common_git_dir = repository.read(ctx, |repository, _| repository.common_git_dir());
            return GitResolution::Resolved(common_git_dir);
        }

        let settled_answer_was_a_repository = pane_group
            .as_ref(ctx)
            .terminal_view_from_pane_id(anchor_pane_id, ctx)
            .is_some_and(|terminal_view| terminal_view.as_ref(ctx).current_repo_path().is_some());

        if settled_answer_was_a_repository {
            GitResolution::Pending
        } else {
            GitResolution::NotARepository
        }
    }

    /// The project key the tab's anchor pane currently resolves to.
    ///
    /// Re-derives the anchor first, so a tab whose anchor pane has closed
    /// resolves against the pane that replaced it.
    pub(super) fn resolve_project_key_for_tab(
        &mut self,
        pane_group_id: EntityId,
        ctx: &mut ViewContext<Self>,
    ) -> Option<ProjectKey> {
        self.refresh_tab_anchor(pane_group_id, ctx);
        self.project_key_for_recorded_anchor(pane_group_id, ctx)
    }

    /// Whether automation will refuse to touch this tab whatever its project
    /// turns out to be.
    ///
    /// A pinned tab is never grouped, so resolving its key — a walk of the
    /// repository cache plus a freshly built list of every group's key — only
    /// produces a value [`Self::reconcile_tab_auto_group`] discards. Its anchor
    /// is still tracked, so unpinning resolves against current state.
    fn tab_is_never_auto_grouped(&self, pane_group_id: EntityId) -> bool {
        self.tab_index_for_pane_group(pane_group_id)
            .is_none_or(|tab_index| self.tabs[tab_index].pinned)
    }

    fn project_key_for_recorded_anchor(
        &self,
        pane_group_id: EntityId,
        ctx: &warpui::AppContext,
    ) -> Option<ProjectKey> {
        let anchor = self.auto_grouping_state.anchors.get(&pane_group_id)?;
        let directory = anchor.directory.clone()?;
        let pane_group = self.pane_group_for_id(pane_group_id)?;
        let git = self.git_resolution(&pane_group, anchor.pane_id, &directory, ctx);
        let existing_non_git_keys: Vec<ProjectKey> = self
            .project_keys_in_window()
            .into_iter()
            .filter(|key| !is_git_project_key(key))
            .collect();
        let home_dir = dirs::home_dir();

        project_key::resolve(&ProjectKeyInput {
            directory: Some(&directory),
            git,
            existing_non_git_keys: &existing_non_git_keys,
            home_dir: home_dir.as_deref(),
        })
    }

    /// The primary trigger: the anchor pane reported a different directory.
    ///
    /// `AppStateChanged` also fires on pane splits, pane closes, session
    /// changes and title updates, so the directory delta is what separates a
    /// move between projects from the rest. A change of anchor counts as a
    /// delta too: re-anchoring after the anchor pane closed can change the
    /// tab's project even when the new anchor's directory happens to match
    /// nothing that moved.
    pub(crate) fn reconcile_tab_auto_group_after_directory_change(
        &mut self,
        pane_group_id: EntityId,
        ctx: &mut ViewContext<Self>,
    ) {
        if !self.auto_grouping_enabled(ctx) {
            return;
        }
        if !self.refresh_tab_anchor(pane_group_id, ctx) {
            return;
        }
        if self.tab_is_never_auto_grouped(pane_group_id) {
            return;
        }
        let key = self.project_key_for_recorded_anchor(pane_group_id, ctx);
        self.reconcile_tab_auto_group(pane_group_id, key, ctx);
    }

    /// The secondary trigger: repository detection answered.
    ///
    /// Deliberately not guarded on a directory delta. Detection resolves
    /// frames after the directory changed, and the whole point of this trigger
    /// is the reconcile that could not be decided when the directory moved.
    pub(crate) fn reconcile_tab_auto_group_after_repo_change(
        &mut self,
        pane_group_id: EntityId,
        ctx: &mut ViewContext<Self>,
    ) {
        if !self.auto_grouping_enabled(ctx) {
            return;
        }
        self.refresh_tab_anchor(pane_group_id, ctx);
        if self.tab_is_never_auto_grouped(pane_group_id) {
            return;
        }
        let key = self.project_key_for_recorded_anchor(pane_group_id, ctx);
        self.reconcile_tab_auto_group(pane_group_id, key, ctx);
    }

    /// Marks a tab as awaiting automatic placement and reconciles it at once.
    ///
    /// This is the "treat it as newly created" entry point: a tab that has just
    /// been created, one arriving from another window (R25), one reopened whose
    /// stored group no longer exists (R26), and one being unpinned (R23). The
    /// marker is what carries the intent across the asynchronous window in
    /// which the tab's key is not resolvable yet — the reconcile below is often
    /// a no-op, and the first one that can resolve a key does the placing.
    pub(crate) fn place_tab_by_auto_grouping(
        &mut self,
        pane_group_id: EntityId,
        ctx: &mut ViewContext<Self>,
    ) {
        if !self.auto_grouping_enabled(ctx) {
            return;
        }
        let Some(tab_index) = self.tab_index_for_pane_group(pane_group_id) else {
            return;
        };
        self.tabs[tab_index].placed_by_automation = true;

        let key = self.resolve_project_key_for_tab(pane_group_id, ctx);
        self.reconcile_tab_auto_group(pane_group_id, key, ctx);
    }

    /// Places a new tab that inherited the active tab's group.
    ///
    /// Inheriting a *keyed* group is positional rather than a placement: the
    /// tab is still brand new, and is queued for placement like any other so it
    /// follows its own project as soon as its key resolves.
    ///
    /// Inheriting a group the user made by hand is a placement, and the
    /// queued-for-placement marker outranks placement (see
    /// [`Workspace::note_manual_tab_placement`]) — setting it would have the
    /// first reconcile pull the tab straight back out of the group it was
    /// deliberately born into. Such a tab is only reconciled, which records the
    /// key it resolves to without claiming it for automation.
    pub(crate) fn place_tab_born_into_group(
        &mut self,
        pane_group_id: EntityId,
        group_id: TabGroupId,
        ctx: &mut ViewContext<Self>,
    ) {
        if !self.auto_grouping_enabled(ctx) {
            return;
        }
        let born_into_manual_group = self
            .tab_groups
            .get(&group_id)
            .is_some_and(|group| group.project_key.is_none());
        if !born_into_manual_group {
            self.place_tab_by_auto_grouping(pane_group_id, ctx);
            return;
        }
        let key = self.resolve_project_key_for_tab(pane_group_id, ctx);
        self.reconcile_tab_auto_group(pane_group_id, key, ctx);
    }

    /// The enable sweep: the single moment automation touches tabs that are
    /// already ungrouped.
    ///
    /// Tabs sitting in a group were put there by the user (or by an earlier
    /// run of automation) and are left exactly as they are; pinned tabs are
    /// never grouped. Turning the mode *off* runs nothing at all — no dissolve,
    /// no rename, no reorder (R3).
    pub(crate) fn sweep_tabs_for_auto_grouping(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.auto_grouping_enabled(ctx) {
            return;
        }
        let candidates: Vec<EntityId> = self
            .tabs
            .iter()
            .filter(|tab| tab.group_id.is_none() && !tab.pinned)
            .map(|tab| tab.pane_group.id())
            .collect();

        for pane_group_id in candidates {
            self.place_tab_by_auto_grouping(pane_group_id, ctx);
        }
    }

    /// The color equivalent of the enable sweep: paints the members of the
    /// automatic groups the window already has, which are otherwise only
    /// colored at the moment their tab joins or their group is keyed.
    ///
    /// Turning the setting back off runs nothing, exactly as turning the mode
    /// itself off dissolves nothing (R3): what automation put there stays, and
    /// the user can clear or change any of it by hand.
    pub(crate) fn sweep_auto_tab_colors(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.auto_group_colors_enabled(ctx) {
            return;
        }
        let keyed: Vec<(TabGroupId, ProjectKey)> = self
            .tab_groups
            .values()
            .filter_map(|group| Some((group.id, group_project_key(group)?)))
            .collect();

        for (group_id, key) in keyed {
            // The group's current key is the only basis for provenance available
            // here, and that is lossy in one direction. A tab that moved between
            // projects while the setting was off still wears a color derived
            // from the key it used to sit under; nothing records that key, so
            // the color reads as the user's and is left alone. The tab then
            // keeps another project's color permanently — no later pass revisits
            // it, and the only way back under automation is to clear the color
            // by hand. Erring this way is deliberate: the alternative mistakes a
            // user's color for automation's, which R16 forbids outright.
            self.apply_derived_color_to_group_members(group_id, &key, Some(&key), ctx);
        }

        ctx.dispatch_global_action("workspace:save_app", ());
        ctx.notify();
    }
}

/// Asserts that every group's members occupy a contiguous run of the tab list.
///
/// The workspace maintains that convention and its grouping helpers assume it,
/// but nothing enforces it at runtime, so each reconcile case re-checks it.
/// All three `auto_grouping` test modules share this one copy: three separate
/// ones would let a weakened assertion silently disable the check in a single
/// file.
#[cfg(test)]
fn assert_groups_contiguous(workspace: &Workspace) {
    for group_id in workspace.tab_groups.keys() {
        let indices: Vec<usize> = group_member_indices(&workspace.tabs, *group_id).collect();
        let Some(&first) = indices.first() else {
            continue;
        };
        let last = indices[indices.len() - 1];
        assert_eq!(
            last - first + 1,
            indices.len(),
            "group {group_id:?} members are not a contiguous run: {indices:?}"
        );
    }
}

#[cfg(test)]
#[path = "auto_grouping_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "auto_grouping_wiring_tests.rs"]
mod wiring_tests;

#[cfg(test)]
#[path = "auto_grouping_manual_override_tests.rs"]
mod manual_override_tests;
