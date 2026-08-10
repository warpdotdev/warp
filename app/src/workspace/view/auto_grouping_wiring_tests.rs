//! The event wiring around [`Workspace::reconcile_tab_auto_group`]: which
//! moments call it, which deliberately do not, and the enable sweep.
//!
//! A terminal pane in a unit test has no shell and therefore no working
//! directory, so the two facts the resolver reads from the world — the
//! directory a pane reports and what repository detection knows about it — are
//! injected through [`AutoGroupingState`]'s test maps. The directory map is
//! keyed by *pane*, so anchor selection itself still runs for real.

use std::path::PathBuf;

use warp_core::features::FeatureFlag;
use warpui::App;

use super::*;
use crate::pane_group::Direction;
use crate::tab::TabData;
use crate::workspace::view::TransferredTab;
use crate::workspace::view::tests::{initialize_app, mock_workspace};

/// Two unrelated non-git directories, neither a prefix of the other.
const SCRATCH: &str = "/home/dev/scratch";
const NOTES: &str = "/home/dev/notes";
/// A git working directory and the shared git directory it resolves to.
const API_DIR: &str = "/work/api";
const API_KEY: &str = "/work/api/.git";
const WEB_KEY: &str = "/work/web/.git";

fn path(value: &str) -> PathBuf {
    PathBuf::from(value)
}

fn key(value: &str) -> ProjectKey {
    ProjectKey::from_path(path(value))
}

/// Turns the mode on for real, through the setting the workspace subscribes to.
fn enable_auto_grouping(app: &mut App) {
    TabSettings::handle(&*app).update(app, |settings, ctx| {
        settings.auto_group_tabs.set_value(true, ctx).unwrap();
    });
}

fn disable_auto_grouping(app: &mut App) {
    TabSettings::handle(&*app).update(app, |settings, ctx| {
        settings.auto_group_tabs.set_value(false, ctx).unwrap();
    });
}

fn grow_to(workspace: &mut Workspace, total: usize, ctx: &mut ViewContext<Workspace>) {
    while workspace.tab_count() < total {
        workspace.add_terminal_tab(false, ctx);
    }
    assert_eq!(workspace.tab_count(), total);
}

/// Every terminal pane of a tab, in layout order. The first is what automation
/// anchors to.
fn terminal_panes(
    workspace: &Workspace,
    tab_index: usize,
    ctx: &ViewContext<Workspace>,
) -> Vec<PaneId> {
    let pane_group = workspace.tabs[tab_index].pane_group.clone();
    let group = pane_group.as_ref(ctx);
    group
        .visible_pane_ids()
        .into_iter()
        .filter(|pane_id| group.terminal_view_from_pane_id(*pane_id, ctx).is_some())
        .collect()
}

fn anchor_pane(workspace: &Workspace, tab_index: usize, ctx: &ViewContext<Workspace>) -> PaneId {
    *terminal_panes(workspace, tab_index, ctx)
        .first()
        .expect("a terminal tab has at least one terminal pane")
}

fn set_pane_directory(workspace: &mut Workspace, pane_id: PaneId, directory: &str) {
    workspace
        .auto_grouping_state
        .test_pane_directories
        .insert(pane_id, path(directory));
}

fn set_git_resolution(workspace: &mut Workspace, directory: &str, resolution: GitResolution) {
    workspace
        .auto_grouping_state
        .test_git_resolutions
        .insert(path(directory), resolution);
}

fn fire(
    workspace: &mut Workspace,
    tab_index: usize,
    event: crate::pane_group::Event,
    ctx: &mut ViewContext<Workspace>,
) {
    let pane_group = workspace.tabs[tab_index].pane_group.clone();
    workspace.handle_file_tree_event(pane_group, &event, ctx);
}

fn directory_changed(
    workspace: &mut Workspace,
    tab_index: usize,
    ctx: &mut ViewContext<Workspace>,
) {
    fire(
        workspace,
        tab_index,
        crate::pane_group::Event::AppStateChanged,
        ctx,
    );
}

fn repo_changed(workspace: &mut Workspace, tab_index: usize, ctx: &mut ViewContext<Workspace>) {
    fire(
        workspace,
        tab_index,
        crate::pane_group::Event::RepoChanged,
        ctx,
    );
}

fn group_key_of_tab(workspace: &Workspace, tab_index: usize) -> Option<String> {
    workspace.tabs[tab_index]
        .group_id
        .and_then(|group_id| workspace.tab_groups.get(&group_id))
        .and_then(|group| group.project_key.clone())
}

fn group_name_of_tab(workspace: &Workspace, tab_index: usize) -> Option<String> {
    workspace.tabs[tab_index]
        .group_id
        .and_then(|group_id| workspace.tab_groups.get(&group_id))
        .and_then(|group| group.name.clone())
}

fn tab_index_of(workspace: &Workspace, pane_group_id: EntityId) -> usize {
    workspace
        .tabs
        .iter()
        .position(|tab| tab.pane_group.id() == pane_group_id)
        .expect("tab still exists")
}

/// Group members occupying a contiguous run is a convention nothing enforces at
/// runtime, so every case that moves a tab asserts it.
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

#[test]
fn directory_change_between_two_non_git_directories_reconciles() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        enable_auto_grouping(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            let anchor = anchor_pane(workspace, 0, ctx);
            set_pane_directory(workspace, anchor, SCRATCH);
            directory_changed(workspace, 0, ctx);

            assert_eq!(group_key_of_tab(workspace, 0).as_deref(), Some(SCRATCH));
            assert_eq!(group_name_of_tab(workspace, 0).as_deref(), Some("scratch"));

            // Sideways into an unrelated non-git directory. Repository
            // detection never answers differently for this move, so
            // `RepoChanged` is never emitted — only the directory delta on
            // `AppStateChanged` can carry it.
            set_pane_directory(workspace, anchor, NOTES);
            directory_changed(workspace, 0, ctx);

            assert_eq!(group_key_of_tab(workspace, 0).as_deref(), Some(NOTES));
            assert_eq!(group_name_of_tab(workspace, 0).as_deref(), Some("notes"));
            assert_groups_contiguous(workspace);
        });
    });
}

#[test]
fn repository_change_reconciles_a_tab_whose_key_was_pending() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        enable_auto_grouping(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            let anchor = anchor_pane(workspace, 0, ctx);
            set_pane_directory(workspace, anchor, API_DIR);
            // Detection has not answered for this directory yet.
            set_git_resolution(workspace, API_DIR, GitResolution::Pending);

            directory_changed(workspace, 0, ctx);

            assert!(
                workspace.tabs[0].group_id.is_none(),
                "a pending key must leave the tab alone"
            );
            assert!(
                workspace.tabs[0].placed_by_automation,
                "the tab stays queued for placement rather than reading as ungrouped by hand"
            );

            // Detection answers. The directory has not changed, so the
            // directory-delta guard would swallow this — the repository-change
            // trigger exists exactly for it.
            set_git_resolution(workspace, API_DIR, GitResolution::Resolved(path(API_KEY)));
            repo_changed(workspace, 0, ctx);

            assert_eq!(group_key_of_tab(workspace, 0).as_deref(), Some(API_KEY));
            assert_eq!(group_name_of_tab(workspace, 0).as_deref(), Some("api"));
            assert!(!workspace.tabs[0].placed_by_automation);
            assert_groups_contiguous(workspace);
        });
    });
}

#[test]
fn pane_split_fires_the_shared_event_but_does_not_reconcile() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        enable_auto_grouping(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            let anchor = anchor_pane(workspace, 0, ctx);
            set_pane_directory(workspace, anchor, SCRATCH);
            directory_changed(workspace, 0, ctx);
            assert_eq!(group_key_of_tab(workspace, 0).as_deref(), Some(SCRATCH));

            // Arrange for a reconcile to be *visible* if one happens: the same
            // directory now resolves to a repository.
            set_git_resolution(workspace, SCRATCH, GitResolution::Resolved(path(API_KEY)));

            let pane_group = workspace.tabs[0].pane_group.clone();
            pane_group.update(ctx, |pane_group, ctx| {
                pane_group.add_terminal_pane(Direction::Right, None, ctx);
            });
            assert_eq!(
                terminal_panes(workspace, 0, ctx).len(),
                2,
                "the split added a second terminal pane"
            );

            directory_changed(workspace, 0, ctx);

            assert_eq!(
                group_key_of_tab(workspace, 0).as_deref(),
                Some(SCRATCH),
                "a split changes neither the anchor nor its directory, so nothing reconciles"
            );
            assert_eq!(
                anchor_pane(workspace, 0, ctx),
                anchor,
                "the split must not steal the anchor"
            );

            // The control: a trigger that does not guard on the delta does
            // reconcile, proving the assertion above is about the guard and not
            // about the resolver having nothing to say.
            repo_changed(workspace, 0, ctx);
            assert_eq!(group_key_of_tab(workspace, 0).as_deref(), Some(API_KEY));
            assert_groups_contiguous(workspace);
        });
    });
}

#[test]
fn directory_change_in_a_non_anchor_pane_does_not_move_the_tab() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        enable_auto_grouping(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            let anchor = anchor_pane(workspace, 0, ctx);
            set_pane_directory(workspace, anchor, SCRATCH);
            directory_changed(workspace, 0, ctx);
            assert_eq!(group_key_of_tab(workspace, 0).as_deref(), Some(SCRATCH));

            let pane_group = workspace.tabs[0].pane_group.clone();
            pane_group.update(ctx, |pane_group, ctx| {
                pane_group.add_terminal_pane(Direction::Right, None, ctx);
            });
            let split = *terminal_panes(workspace, 0, ctx)
                .iter()
                .find(|pane_id| **pane_id != anchor)
                .expect("the split pane exists");

            // The split is checked out somewhere else entirely, and it is the
            // focused pane after the split. The tab's project follows its
            // anchor, never the focus.
            set_pane_directory(workspace, split, NOTES);
            directory_changed(workspace, 0, ctx);

            assert_eq!(group_key_of_tab(workspace, 0).as_deref(), Some(SCRATCH));
            assert_eq!(workspace.tab_groups.len(), 1);
            assert_groups_contiguous(workspace);
        });
    });
}

#[test]
fn closing_the_anchor_pane_reanchors_and_reconciles() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        enable_auto_grouping(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            let anchor = anchor_pane(workspace, 0, ctx);
            set_pane_directory(workspace, anchor, SCRATCH);
            directory_changed(workspace, 0, ctx);
            assert_eq!(group_key_of_tab(workspace, 0).as_deref(), Some(SCRATCH));

            let pane_group = workspace.tabs[0].pane_group.clone();
            pane_group.update(ctx, |pane_group, ctx| {
                pane_group.add_terminal_pane(Direction::Right, None, ctx);
            });
            let split = *terminal_panes(workspace, 0, ctx)
                .iter()
                .find(|pane_id| **pane_id != anchor)
                .expect("the split pane exists");
            set_pane_directory(workspace, split, NOTES);

            pane_group.update(ctx, |pane_group, ctx| {
                pane_group.close_pane(anchor, ctx);
            });
            directory_changed(workspace, 0, ctx);

            assert_eq!(
                anchor_pane(workspace, 0, ctx),
                split,
                "the next remaining terminal pane becomes the anchor"
            );
            assert_eq!(group_key_of_tab(workspace, 0).as_deref(), Some(NOTES));
            assert_groups_contiguous(workspace);
        });
    });
}

#[test]
fn newly_created_tab_is_auto_grouped() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        enable_auto_grouping(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            grow_to(workspace, 2, ctx);
            // The tab was created before any directory was known, so it is
            // queued for placement rather than read as deliberately ungrouped.
            assert!(workspace.tabs[1].placed_by_automation);

            let anchor = anchor_pane(workspace, 1, ctx);
            set_pane_directory(workspace, anchor, API_DIR);
            set_git_resolution(workspace, API_DIR, GitResolution::Resolved(path(API_KEY)));
            directory_changed(workspace, 1, ctx);

            let grouped = tab_index_of(workspace, workspace.tabs[1].pane_group.id());
            assert_eq!(
                group_key_of_tab(workspace, grouped).as_deref(),
                Some(API_KEY)
            );
            assert!(!workspace.tabs[grouped].placed_by_automation);
            assert_groups_contiguous(workspace);
        });
    });
}

#[test]
fn tab_created_while_the_mode_is_off_is_not_queued_for_placement() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            grow_to(workspace, 2, ctx);
            assert!(!workspace.tabs[1].placed_by_automation);

            let anchor = anchor_pane(workspace, 1, ctx);
            set_pane_directory(workspace, anchor, SCRATCH);
            directory_changed(workspace, 1, ctx);

            assert!(workspace.tabs[1].group_id.is_none());
            assert!(workspace.tab_groups.is_empty());
        });
    });
}

#[test]
fn reopened_tab_is_auto_grouped_only_when_its_stored_group_is_gone() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        enable_auto_grouping(&mut app);
        let workspace = mock_workspace(&mut app);
        let donor = mock_workspace(&mut app);
        let donor_pane_group = donor.update(&mut app, |donor, _| donor.tabs[0].pane_group.clone());
        let second_donor = mock_workspace(&mut app);
        let second_pane_group =
            second_donor.update(&mut app, |donor, _| donor.tabs[0].pane_group.clone());

        workspace.update(&mut app, |workspace, ctx| {
            // A group that survived the tab's close, keyed to a different
            // project than the tab's own directory.
            let mut survivor = TabGroup::new();
            let survivor_id = survivor.id;
            survivor.project_key = Some(key(WEB_KEY).to_storage_string());
            workspace.tab_groups.insert(survivor_id, survivor);
            workspace.tabs[0].group_id = Some(survivor_id);

            // Its group is gone: treated as newly created.
            let mut orphaned = TabData::new(donor_pane_group.clone());
            orphaned.group_id = Some(TabGroupId::new());
            workspace.restore_closed_tab(1, orphaned, ctx);
            let orphaned_index = tab_index_of(workspace, donor_pane_group.id());
            assert!(workspace.tabs[orphaned_index].placed_by_automation);

            // Its group survived: its restored placement stands.
            let mut rejoined = TabData::new(second_pane_group.clone());
            rejoined.group_id = Some(survivor_id);
            workspace.restore_closed_tab(2, rejoined, ctx);
            let rejoined_index = tab_index_of(workspace, second_pane_group.id());
            assert!(!workspace.tabs[rejoined_index].placed_by_automation);

            // Every index is re-derived from the pane group here: each restore
            // inserts a tab and shifts the ones after it.
            let orphaned_index = tab_index_of(workspace, donor_pane_group.id());
            let orphaned_anchor = anchor_pane(workspace, orphaned_index, ctx);
            set_pane_directory(workspace, orphaned_anchor, API_DIR);
            set_git_resolution(workspace, API_DIR, GitResolution::Resolved(path(API_KEY)));
            let rejoined_index = tab_index_of(workspace, second_pane_group.id());
            let rejoined_anchor = anchor_pane(workspace, rejoined_index, ctx);
            set_pane_directory(workspace, rejoined_anchor, SCRATCH);

            let orphaned_index = tab_index_of(workspace, donor_pane_group.id());
            directory_changed(workspace, orphaned_index, ctx);
            let rejoined_index = tab_index_of(workspace, second_pane_group.id());
            directory_changed(workspace, rejoined_index, ctx);

            let orphaned_index = tab_index_of(workspace, donor_pane_group.id());
            assert_eq!(
                group_key_of_tab(workspace, orphaned_index).as_deref(),
                Some(API_KEY)
            );

            let rejoined_index = tab_index_of(workspace, second_pane_group.id());
            assert_eq!(
                workspace.tabs[rejoined_index].group_id,
                Some(survivor_id),
                "a reopened tab whose group survived keeps the placement it was closed with"
            );
            assert_groups_contiguous(workspace);
        });
    });
}

#[test]
fn tab_inserted_from_another_window_is_auto_grouped() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        enable_auto_grouping(&mut app);
        let workspace = mock_workspace(&mut app);
        let donor = mock_workspace(&mut app);
        let donor_pane_group = donor.update(&mut app, |donor, _| donor.tabs[0].pane_group.clone());

        workspace.update(&mut app, |workspace, ctx| {
            workspace.insert_transferred_tab_at_index(
                TransferredTab {
                    pane_group: donor_pane_group.clone(),
                    color: None,
                    custom_title: None,
                    left_panel_open: false,
                    vertical_tabs_panel_open: false,
                    right_panel_open: false,
                    is_right_panel_maximized: false,
                    draggable_state: Default::default(),
                },
                1,
                ctx,
            );

            let arrived = tab_index_of(workspace, donor_pane_group.id());
            assert!(
                workspace.tabs[arrived].placed_by_automation,
                "a transferred tab arrives with no group and must read as new, not as ungrouped by hand"
            );

            let anchor = anchor_pane(workspace, arrived, ctx);
            set_pane_directory(workspace, anchor, API_DIR);
            set_git_resolution(
                workspace,
                API_DIR,
                GitResolution::Resolved(path(API_KEY)),
            );
            directory_changed(workspace, arrived, ctx);

            let arrived = tab_index_of(workspace, donor_pane_group.id());
            assert_eq!(group_key_of_tab(workspace, arrived).as_deref(), Some(API_KEY));
            assert_groups_contiguous(workspace);
        });
    });
}

#[test]
fn pinning_removes_a_tab_from_its_group_and_unpinning_regroups_it() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);
    let _pinned_tabs_guard = FeatureFlag::PinnedTabs.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        enable_auto_grouping(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            grow_to(workspace, 2, ctx);
            let anchor = anchor_pane(workspace, 0, ctx);
            let pane_group_id = workspace.tabs[0].pane_group.id();
            set_pane_directory(workspace, anchor, SCRATCH);
            directory_changed(workspace, 0, ctx);

            let grouped = tab_index_of(workspace, pane_group_id);
            let group_id = workspace.tabs[grouped]
                .group_id
                .expect("the tab was grouped");

            workspace.pin_tab(grouped, ctx);

            let pinned = tab_index_of(workspace, pane_group_id);
            assert!(workspace.tabs[pinned].pinned);
            assert!(
                workspace.tabs[pinned].group_id.is_none(),
                "pinning takes the tab out of its automation group"
            );
            assert!(
                !workspace.tab_groups.contains_key(&group_id),
                "the emptied group is pruned"
            );

            workspace.unpin_tab(pinned, ctx);

            let unpinned = tab_index_of(workspace, pane_group_id);
            assert!(!workspace.tabs[unpinned].pinned);
            assert_eq!(
                group_key_of_tab(workspace, unpinned).as_deref(),
                Some(SCRATCH),
                "unpinning reconciles the tab as if it were newly created, so pin/unpin round-trips"
            );
            assert_groups_contiguous(workspace);
        });
    });
}

#[test]
fn enabling_the_mode_sweeps_ungrouped_unpinned_tabs_only() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);
    let _pinned_tabs_guard = FeatureFlag::PinnedTabs.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let workspace = mock_workspace(&mut app);

        // Three tabs, all created with the mode off: one ungrouped, one in a
        // manual group, one pinned.
        let manual_group_id = workspace.update(&mut app, |workspace, ctx| {
            grow_to(workspace, 3, ctx);
            for tab_index in 0..3 {
                let anchor = anchor_pane(workspace, tab_index, ctx);
                set_pane_directory(workspace, anchor, SCRATCH);
            }

            let manual = TabGroup::new();
            let manual_group_id = manual.id;
            workspace.tab_groups.insert(manual_group_id, manual);
            workspace.tabs[1].group_id = Some(manual_group_id);
            workspace.tabs[2].pinned = true;
            manual_group_id
        });

        enable_auto_grouping(&mut app);

        workspace.update(&mut app, |workspace, _ctx| {
            assert_eq!(
                group_key_of_tab(workspace, 0).as_deref(),
                Some(SCRATCH),
                "the ungrouped, unpinned tab is placed"
            );
            let manual_index = workspace
                .tabs
                .iter()
                .position(|tab| tab.group_id == Some(manual_group_id))
                .expect("the manual group still holds its member");
            assert!(
                workspace.tab_groups[&manual_group_id].project_key.is_none(),
                "a manual group is never keyed by the sweep"
            );
            assert!(!workspace.tabs[manual_index].pinned);

            let pinned_index = workspace
                .tabs
                .iter()
                .position(|tab| tab.pinned)
                .expect("the pinned tab is still pinned");
            assert!(
                workspace.tabs[pinned_index].group_id.is_none(),
                "a pinned tab is never grouped"
            );
            assert_groups_contiguous(workspace);
        });
    });
}

#[test]
fn tab_unresolved_during_the_sweep_is_grouped_when_its_key_first_resolves() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let workspace = mock_workspace(&mut app);

        // No directory is known for this tab when the sweep runs.
        enable_auto_grouping(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            assert!(workspace.tabs[0].group_id.is_none());
            assert!(
                workspace.tabs[0].placed_by_automation,
                "the sweep leaves an unresolvable tab queued rather than skipping it forever"
            );

            let anchor = anchor_pane(workspace, 0, ctx);
            set_pane_directory(workspace, anchor, API_DIR);
            set_git_resolution(workspace, API_DIR, GitResolution::Resolved(path(API_KEY)));
            directory_changed(workspace, 0, ctx);

            assert_eq!(group_key_of_tab(workspace, 0).as_deref(), Some(API_KEY));
            assert!(!workspace.tabs[0].placed_by_automation);
            assert_groups_contiguous(workspace);
        });
    });
}

#[test]
fn disabling_the_mode_changes_nothing() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        enable_auto_grouping(&mut app);
        let workspace = mock_workspace(&mut app);

        let before = workspace.update(&mut app, |workspace, ctx| {
            grow_to(workspace, 2, ctx);
            let first = anchor_pane(workspace, 0, ctx);
            set_pane_directory(workspace, first, SCRATCH);
            directory_changed(workspace, 0, ctx);
            let second = anchor_pane(workspace, 1, ctx);
            set_pane_directory(workspace, second, NOTES);
            directory_changed(workspace, 1, ctx);

            assert_eq!(workspace.tab_groups.len(), 2);
            (
                workspace
                    .tabs
                    .iter()
                    .map(|tab| (tab.pane_group.id(), tab.group_id))
                    .collect::<Vec<_>>(),
                workspace
                    .tab_groups
                    .iter()
                    .map(|(id, group)| (*id, group.name.clone(), group.project_key.clone()))
                    .collect::<Vec<_>>(),
            )
        });

        disable_auto_grouping(&mut app);

        workspace.update(&mut app, |workspace, _ctx| {
            let after_tabs: Vec<_> = workspace
                .tabs
                .iter()
                .map(|tab| (tab.pane_group.id(), tab.group_id))
                .collect();
            let mut after_groups: Vec<_> = workspace
                .tab_groups
                .iter()
                .map(|(id, group)| (*id, group.name.clone(), group.project_key.clone()))
                .collect();
            let mut before_groups = before.1.clone();
            after_groups.sort_by_key(|entry| entry.0.0);
            before_groups.sort_by_key(|entry| entry.0.0);

            assert_eq!(after_tabs, before.0, "no tab is dissolved or reordered");
            assert_eq!(
                after_groups, before_groups,
                "no group is renamed or dropped"
            );
        });

        // And nothing reconciles afterwards either.
        workspace.update(&mut app, |workspace, ctx| {
            let first = anchor_pane(workspace, 0, ctx);
            set_pane_directory(workspace, first, NOTES);
            directory_changed(workspace, 0, ctx);

            assert_eq!(group_key_of_tab(workspace, 0).as_deref(), Some(SCRATCH));
        });
    });
}
