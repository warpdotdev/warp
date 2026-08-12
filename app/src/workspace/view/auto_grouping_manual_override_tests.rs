//! Manual override: what happens when the user places a tab somewhere
//! automation would not have (R13), puts a detached tab back where it belongs
//! (R14), or dissolves a whole group (R15).
//!
//! Tracked-ness is derived from placement rather than stored (KTD5), so almost
//! nothing here is new code — these cases pin down that the derivation reads
//! the way the requirements describe. The one exception is the
//! queued-for-placement marker, which outranks placement in that derivation and
//! therefore has to be retired wherever the user places a tab by hand.
//!
//! A terminal pane in a unit test has no shell and therefore no working
//! directory, so the directory each pane reports is injected through
//! [`AutoGroupingState`]'s test map, exactly as in the wiring cases. The
//! directories below are plain non-git paths, so a tab's project key is just
//! its directory and no repository detection has to be faked.

use std::path::PathBuf;

use warp_core::features::FeatureFlag;
use warpui::App;

use super::*;
use crate::pane_group::Event;
use crate::workspace::view::tests::{initialize_app, mock_workspace};

/// Three unrelated non-git directories, none a prefix of another, so
/// `display_name` never qualifies a derived group name.
const SCRATCH: &str = "/home/dev/scratch";
const NOTES: &str = "/home/dev/notes";
const LAB: &str = "/home/dev/lab";

fn path(value: &str) -> PathBuf {
    PathBuf::from(value)
}

/// Turns the mode on for real, through the setting the workspace subscribes to.
fn enable_auto_grouping(app: &mut App) {
    TabSettings::handle(&*app).update(app, |settings, ctx| {
        settings.auto_group_tabs.set_value(true, ctx).unwrap();
    });
}

fn grow_to(workspace: &mut Workspace, total: usize, ctx: &mut ViewContext<Workspace>) {
    while workspace.tab_count() < total {
        workspace.add_terminal_tab(false, ctx);
    }
    assert_eq!(workspace.tab_count(), total);
}

/// The pane a tab's project key follows: the first terminal pane in its layout.
fn anchor_pane(workspace: &Workspace, tab_index: usize, ctx: &ViewContext<Workspace>) -> PaneId {
    let pane_group = workspace.tabs[tab_index].pane_group.clone();
    let group = pane_group.as_ref(ctx);
    group
        .visible_pane_ids()
        .into_iter()
        .find(|pane_id| group.terminal_view_from_pane_id(*pane_id, ctx).is_some())
        .expect("a terminal tab has at least one terminal pane")
}

fn set_pane_directory(workspace: &mut Workspace, pane_id: PaneId, directory: &str) {
    workspace
        .auto_grouping_state
        .test_pane_directories
        .insert(pane_id, path(directory));
}

/// Points the tab's anchor pane at `directory` and delivers the event a real
/// `cd` would.
fn cd(
    workspace: &mut Workspace,
    tab_index: usize,
    directory: &str,
    ctx: &mut ViewContext<Workspace>,
) {
    let anchor = anchor_pane(workspace, tab_index, ctx);
    set_pane_directory(workspace, anchor, directory);
    let pane_group = workspace.tabs[tab_index].pane_group.clone();
    workspace.handle_file_tree_event(pane_group, &Event::AppStateChanged, ctx);
}

/// Delivers a repository-detection answer, the trigger that is deliberately not
/// guarded on a directory delta.
fn repo_changed(workspace: &mut Workspace, tab_index: usize, ctx: &mut ViewContext<Workspace>) {
    let pane_group = workspace.tabs[tab_index].pane_group.clone();
    workspace.handle_file_tree_event(pane_group, &Event::RepoChanged, ctx);
}

fn tab_index_of(workspace: &Workspace, pane_group_id: EntityId) -> usize {
    workspace
        .tabs
        .iter()
        .position(|tab| tab.pane_group.id() == pane_group_id)
        .expect("tab still exists")
}

fn group_of(workspace: &Workspace, pane_group_id: EntityId) -> Option<TabGroupId> {
    workspace.tabs[tab_index_of(workspace, pane_group_id)].group_id
}

fn group_key_of(workspace: &Workspace, pane_group_id: EntityId) -> Option<String> {
    group_of(workspace, pane_group_id)
        .and_then(|group_id| workspace.tab_groups.get(&group_id))
        .and_then(|group| group.project_key.clone())
}

/// The group carrying `key_storage`, if the window has one.
fn group_keyed(workspace: &Workspace, key_storage: &str) -> Option<TabGroupId> {
    workspace
        .tab_groups
        .values()
        .find(|group| group.project_key.as_deref() == Some(key_storage))
        .map(|group| group.id)
}

/// Grows the workspace to two tabs sitting in their own automation groups, one
/// per directory, and hands back their identities.
fn two_grouped_tabs(
    workspace: &mut Workspace,
    ctx: &mut ViewContext<Workspace>,
) -> (EntityId, EntityId) {
    grow_to(workspace, 2, ctx);
    let first = workspace.tabs[0].pane_group.id();
    let second = workspace.tabs[1].pane_group.id();
    cd(workspace, 0, SCRATCH, ctx);
    cd(workspace, tab_index_of(workspace, second), NOTES, ctx);
    assert_eq!(group_key_of(workspace, first).as_deref(), Some(SCRATCH));
    assert_eq!(group_key_of(workspace, second).as_deref(), Some(NOTES));
    (first, second)
}

#[test]
fn dragged_tab_stays_in_another_projects_group_across_a_directory_change() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        enable_auto_grouping(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            let (dragged, host) = two_grouped_tabs(workspace, ctx);
            let notes_group = group_of(workspace, host).expect("the second tab was grouped");

            // The user drags the first tab into the second's group — a
            // placement automation would never have made.
            let dragged_index = tab_index_of(workspace, dragged);
            workspace.commit_dragged_tab_group(dragged_index, Some(notes_group), ctx);
            assert_eq!(group_of(workspace, dragged), Some(notes_group));

            // Its directory moves on. A tracked tab would follow it into a
            // group of its own; this one was placed by hand.
            cd(workspace, tab_index_of(workspace, dragged), LAB, ctx);

            assert_eq!(
                group_of(workspace, dragged),
                Some(notes_group),
                "a tab the user dropped somewhere must stop following its directory"
            );
            assert!(
                group_keyed(workspace, LAB).is_none(),
                "no group is created for the directory the detached tab moved to"
            );
            assert_groups_contiguous(workspace);
        });
    });
}

#[test]
fn tab_moved_through_the_menu_stays_in_another_projects_group() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        enable_auto_grouping(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            let (moved, host) = two_grouped_tabs(workspace, ctx);
            let notes_group = group_of(workspace, host).expect("the second tab was grouped");

            // "Move to group" on the tab's own context menu.
            let moved_index = tab_index_of(workspace, moved);
            workspace.move_tab_to_group(moved_index, notes_group, ctx);
            assert_eq!(group_of(workspace, moved), Some(notes_group));

            cd(workspace, tab_index_of(workspace, moved), LAB, ctx);

            assert_eq!(
                group_of(workspace, moved),
                Some(notes_group),
                "the menu path detaches the tab exactly as the drag path does"
            );
            assert!(group_keyed(workspace, LAB).is_none());
            assert_groups_contiguous(workspace);
        });
    });
}

#[test]
fn detached_tab_returned_to_its_own_projects_group_follows_its_directory_again() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        enable_auto_grouping(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            let (anchor_tab, host) = two_grouped_tabs(workspace, ctx);
            let scratch_group = group_of(workspace, anchor_tab).expect("the first tab was grouped");
            let notes_group = group_of(workspace, host).expect("the second tab was grouped");

            // A third tab in the same project as the first, so the SCRATCH
            // group survives the wandering tab leaving it.
            grow_to(workspace, 3, ctx);
            let wanderer = workspace.tabs[2].pane_group.id();
            cd(workspace, tab_index_of(workspace, wanderer), SCRATCH, ctx);
            assert_eq!(group_of(workspace, wanderer), Some(scratch_group));

            // Detach it: the user drags it into the other project's group.
            let wanderer_index = tab_index_of(workspace, wanderer);
            workspace.commit_dragged_tab_group(wanderer_index, Some(notes_group), ctx);

            // Put it back where its own project lives. Nothing is written here
            // — the group simply carries the key the tab resolves to again.
            let wanderer_index = tab_index_of(workspace, wanderer);
            workspace.commit_dragged_tab_group(wanderer_index, Some(scratch_group), ctx);
            repo_changed(workspace, tab_index_of(workspace, wanderer), ctx);

            // Under automation again: its next directory change takes it along.
            cd(workspace, tab_index_of(workspace, wanderer), LAB, ctx);

            assert_eq!(
                group_key_of(workspace, wanderer).as_deref(),
                Some(LAB),
                "a tab put back into the group for its own project is under automation again"
            );
            assert_eq!(
                group_of(workspace, anchor_tab),
                Some(scratch_group),
                "the tab that stayed behind keeps the group"
            );
            assert_groups_contiguous(workspace);
        });
    });
}

#[test]
fn new_group_from_a_detached_tab_adopts_its_project_key() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        enable_auto_grouping(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            let (host, detached) = two_grouped_tabs(workspace, ctx);
            let scratch_group = group_of(workspace, host).expect("the first tab was grouped");

            // The user drags the NOTES tab into the SCRATCH group; its own
            // group is left empty and pruned.
            let detached_index = tab_index_of(workspace, detached);
            workspace.commit_dragged_tab_group(detached_index, Some(scratch_group), ctx);
            assert!(group_keyed(workspace, NOTES).is_none());

            // Then changes their mind and gives it a group of its own.
            let detached_index = tab_index_of(workspace, detached);
            workspace.new_tab_group_from_tab(detached_index, ctx);

            let new_group = group_of(workspace, detached).expect("the tab is in its new group");
            assert_ne!(new_group, scratch_group);
            assert_eq!(
                group_key_of(workspace, detached).as_deref(),
                Some(NOTES),
                "the new group adopts the key of the tab it was created from"
            );

            // And the tab is under automation again: the group follows it.
            cd(workspace, tab_index_of(workspace, detached), LAB, ctx);

            assert_eq!(
                group_of(workspace, detached),
                Some(new_group),
                "a sole member re-keys its group in place rather than flickering"
            );
            assert_eq!(group_key_of(workspace, detached).as_deref(), Some(LAB));
            assert_groups_contiguous(workspace);
        });
    });
}

#[test]
fn ungrouping_leaves_every_member_ungrouped_and_detached() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        enable_auto_grouping(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            grow_to(workspace, 2, ctx);
            let placed = workspace.tabs[0].pane_group.id();
            let queued = workspace.tabs[1].pane_group.id();

            cd(workspace, 0, SCRATCH, ctx);
            let group_id = group_of(workspace, placed).expect("the first tab was grouped");

            // The second member is still queued for placement — its key has
            // never resolved. Only clearing that marker makes the ungroup
            // stick for it.
            let queued_index = tab_index_of(workspace, queued);
            workspace.tabs[queued_index].group_id = Some(group_id);
            workspace.tabs[queued_index].placed_by_automation = true;

            workspace.ungroup_tabs(group_id, ctx);

            assert!(workspace.tab_groups.is_empty());
            for pane_group_id in [placed, queued] {
                let index = tab_index_of(workspace, pane_group_id);
                assert!(workspace.tabs[index].group_id.is_none());
                assert!(
                    !workspace.tabs[index].placed_by_automation,
                    "a former member of an ungrouped group is detached, not queued"
                );
            }

            // Neither is reclaimed when its directory next resolves.
            cd(workspace, tab_index_of(workspace, placed), LAB, ctx);
            cd(workspace, tab_index_of(workspace, queued), NOTES, ctx);

            assert!(group_of(workspace, placed).is_none());
            assert!(group_of(workspace, queued).is_none());
            assert!(
                workspace.tab_groups.is_empty(),
                "ungrouped tabs stay ungrouped until the user places them somewhere"
            );
        });
    });
}

#[test]
fn restored_tab_whose_group_disagrees_with_its_key_is_not_moved() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        enable_auto_grouping(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            // What a restart hands back: a group carrying a key, a tab inside
            // it, a cleared marker — and no resolver state at all, because the
            // last resolved keys live in memory only.
            let restored = workspace.tabs[0].pane_group.id();
            let mut group = TabGroup::new();
            let group_id = group.id;
            group.project_key = Some(ProjectKey::from_path(path(NOTES)).to_storage_string());
            group.name = Some("notes".to_string());
            workspace.tab_groups.insert(group_id, group);
            workspace.tabs[0].group_id = Some(group_id);
            workspace.tabs[0].placed_by_automation = false;

            // The tab's own directory disagrees with the group it was restored
            // into — indistinguishable from a placement the user made before
            // the restart, so it is left exactly where it is.
            cd(workspace, 0, SCRATCH, ctx);

            assert_eq!(group_of(workspace, restored), Some(group_id));
            assert_eq!(group_key_of(workspace, restored).as_deref(), Some(NOTES));
            assert!(group_keyed(workspace, SCRATCH).is_none());
            assert_eq!(workspace.tab_groups.len(), 1);
            assert_groups_contiguous(workspace);
        });
    });
}

#[test]
fn tab_dropped_into_a_group_before_its_key_resolves_stays_there() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        enable_auto_grouping(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            grow_to(workspace, 2, ctx);
            let host = workspace.tabs[0].pane_group.id();
            // The second tab is brand new: no directory has been reported for
            // it, so it is still queued for placement rather than read as
            // deliberately ungrouped. It sits directly below the group, which
            // is where a drag would have left it.
            let fresh = workspace.tabs[1].pane_group.id();
            assert!(workspace.tabs[1].placed_by_automation);

            cd(workspace, 0, SCRATCH, ctx);
            let scratch_group = group_of(workspace, host).expect("the first tab was grouped");

            // The user drops it into a group before its key ever resolves. The
            // marker outranks placement in the tracked-ness derivation, so
            // leaving it set would let the first reconcile undo this drop.
            let fresh_index = tab_index_of(workspace, fresh);
            workspace.commit_dragged_tab_group(fresh_index, Some(scratch_group), ctx);
            assert!(
                !workspace.tabs[tab_index_of(workspace, fresh)].placed_by_automation,
                "the drop retires the queued-for-placement marker"
            );

            // Now its key resolves, to a project that is not this group's.
            cd(workspace, tab_index_of(workspace, fresh), NOTES, ctx);

            assert_eq!(
                group_of(workspace, fresh),
                Some(scratch_group),
                "the first reconcile must not undo a drop the user made while the key was pending"
            );
            assert!(group_keyed(workspace, NOTES).is_none());
            assert_groups_contiguous(workspace);
        });
    });
}
