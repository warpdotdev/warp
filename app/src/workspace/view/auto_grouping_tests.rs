use std::path::PathBuf;

use warp_core::features::FeatureFlag;
use warpui::App;

use super::*;
use crate::workspace::view::tests::{initialize_app, mock_workspace};

// Project keys used across the cases. Every basename is distinct so
// `display_name` never qualifies a name, which keeps the derived-vs-user-set
// name comparisons readable.
const API: &str = "/work/api/.git";
const WEB: &str = "/work/web/.git";
const DOCS: &str = "/work/docs/.git";
const INFRA: &str = "/work/infra/.git";

fn key(path: &str) -> ProjectKey {
    ProjectKey::from_path(PathBuf::from(path))
}

/// Grows the workspace (which starts with one tab) to `total` tabs.
fn grow_to(workspace: &mut Workspace, total: usize, ctx: &mut ViewContext<Workspace>) {
    while workspace.tab_count() < total {
        workspace.add_terminal_tab(false, ctx);
    }
    assert_eq!(workspace.tab_count(), total);
}

/// Creates a group keyed to `path`, named exactly as automation would name it,
/// holding the tabs at `member_indices` (which must already be a contiguous
/// run — the workspace never lets them be anything else).
fn keyed_group(workspace: &mut Workspace, path: &str, member_indices: &[usize]) -> TabGroupId {
    let project_key = key(path);
    let mut group = TabGroup::new();
    let group_id = group.id;
    group.project_key = Some(project_key.to_storage_string());
    workspace.tab_groups.insert(group_id, group);

    // Derived after insertion so the name sees its own key among the window's
    // keys, exactly as the reconcile pass computes it.
    let derived = workspace.derived_group_name(&project_key);
    if let Some(group) = workspace.tab_groups.get_mut(&group_id) {
        group.name = Some(derived);
    }
    for &index in member_indices {
        workspace.tabs[index].group_id = Some(group_id);
    }
    group_id
}

/// Records the key a tab last resolved to, standing in for the resolver state
/// the event wiring would have accumulated.
fn set_last_resolved_key(workspace: &mut Workspace, tab_index: usize, path: &str) {
    let pane_group_id = workspace.tabs[tab_index].pane_group.id();
    workspace
        .auto_grouping_state
        .record_resolved_key(pane_group_id, key(path));
}

fn group_key(workspace: &Workspace, group_id: TabGroupId) -> Option<String> {
    workspace
        .tab_groups
        .get(&group_id)
        .and_then(|group| group.project_key.clone())
}

fn group_name(workspace: &Workspace, group_id: TabGroupId) -> Option<String> {
    workspace
        .tab_groups
        .get(&group_id)
        .and_then(|group| group.name.clone())
}

fn tab_order(workspace: &Workspace) -> Vec<EntityId> {
    workspace
        .tabs
        .iter()
        .map(|tab| tab.pane_group.id())
        .collect()
}

/// Group members occupying a contiguous run of the tab list is a convention the
/// workspace maintains and its helpers assume; nothing enforces it at runtime,
/// so every case below asserts it explicitly.
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

fn reconcile(
    workspace: &mut Workspace,
    tab_index: usize,
    resolved: Option<&str>,
    ctx: &mut ViewContext<Workspace>,
) {
    let pane_group_id = workspace.tabs[tab_index].pane_group.id();
    workspace.reconcile_tab_auto_group(pane_group_id, resolved.map(key), ctx);
}

#[test]
fn tracked_tab_follows_its_key_into_an_existing_group() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            grow_to(workspace, 4, ctx);
            let api_group = keyed_group(workspace, API, &[0, 1]);
            let web_group = keyed_group(workspace, WEB, &[2]);
            set_last_resolved_key(workspace, 0, API);
            set_last_resolved_key(workspace, 1, API);

            let moved = workspace.tabs[0].pane_group.id();
            let stayed = workspace.tabs[1].pane_group.id();

            reconcile(workspace, 0, Some(WEB), ctx);

            // The tab left the API group for the WEB group; the API group
            // still holds its other member, so it survives.
            let moved_index = tab_order(workspace)
                .iter()
                .position(|id| *id == moved)
                .expect("moved tab still exists");
            assert_eq!(workspace.tabs[moved_index].group_id, Some(web_group));
            assert!(workspace.tab_groups.contains_key(&api_group));
            assert_groups_contiguous(workspace);

            // Once the last member leaves too, the emptied group disappears.
            let stayed_index = tab_order(workspace)
                .iter()
                .position(|id| *id == stayed)
                .expect("remaining tab still exists");
            reconcile(workspace, stayed_index, Some(WEB), ctx);

            assert!(!workspace.tab_groups.contains_key(&api_group));
            assert_eq!(workspace.tab_groups.len(), 1);
            assert_eq!(
                workspace
                    .tabs
                    .iter()
                    .filter(|tab| tab.group_id == Some(web_group))
                    .count(),
                3
            );
            assert_groups_contiguous(workspace);
        });
    });
}

#[test]
fn tab_in_a_group_carrying_neither_key_is_left_alone() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            grow_to(workspace, 3, ctx);
            // The tab sits in a group keyed to a third project: neither the key
            // it last resolved to nor the one it resolves to now.
            let docs_group = keyed_group(workspace, DOCS, &[0]);
            set_last_resolved_key(workspace, 0, API);
            let order_before = tab_order(workspace);

            reconcile(workspace, 0, Some(WEB), ctx);

            assert_eq!(workspace.tabs[0].group_id, Some(docs_group));
            assert_eq!(workspace.tab_groups.len(), 1);
            assert_eq!(group_key(workspace, docs_group).as_deref(), Some(DOCS));
            assert_eq!(tab_order(workspace), order_before);
            assert_groups_contiguous(workspace);
        });
    });
}

#[test]
fn tab_awaiting_placement_is_grouped_on_its_first_resolve() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            grow_to(workspace, 2, ctx);
            // No group, no previously resolved key — only the marker says
            // automation has not reached this tab yet. It must still be placed,
            // even though the enable sweep has long since run.
            workspace.tabs[0].placed_by_automation = true;

            reconcile(workspace, 0, Some(API), ctx);

            let group_id = workspace.tabs[0]
                .group_id
                .expect("the tab awaiting placement should have been grouped");
            assert_eq!(group_key(workspace, group_id).as_deref(), Some(API));
            assert_eq!(group_name(workspace, group_id).as_deref(), Some("api"));
            assert!(!workspace.tabs[0].placed_by_automation);
            assert!(workspace.tabs[1].group_id.is_none());
            assert_groups_contiguous(workspace);
        });
    });
}

#[test]
fn sole_member_rekeys_its_group_in_place_when_no_group_exists() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            grow_to(workspace, 2, ctx);
            let api_group = keyed_group(workspace, API, &[0]);
            set_last_resolved_key(workspace, 0, API);

            reconcile(workspace, 0, Some(WEB), ctx);

            // Same group, re-keyed and renamed rather than destroyed and
            // recreated, so the sole-member case doesn't flicker on every `cd`.
            assert_eq!(workspace.tab_groups.len(), 1);
            assert_eq!(workspace.tabs[0].group_id, Some(api_group));
            assert_eq!(group_key(workspace, api_group).as_deref(), Some(WEB));
            assert_eq!(group_name(workspace, api_group).as_deref(), Some("web"));
            assert_groups_contiguous(workspace);
        });
    });
}

#[test]
fn sole_member_joins_the_existing_group_and_its_old_group_disappears() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            grow_to(workspace, 2, ctx);
            let api_group = keyed_group(workspace, API, &[0]);
            let web_group = keyed_group(workspace, WEB, &[1]);
            set_last_resolved_key(workspace, 0, API);
            let moved = workspace.tabs[0].pane_group.id();

            reconcile(workspace, 0, Some(WEB), ctx);

            // An existing group for the new key beats re-keying in place.
            assert!(!workspace.tab_groups.contains_key(&api_group));
            assert_eq!(workspace.tab_groups.len(), 1);
            let moved_index = tab_order(workspace)
                .iter()
                .position(|id| *id == moved)
                .expect("moved tab still exists");
            assert_eq!(workspace.tabs[moved_index].group_id, Some(web_group));
            assert_groups_contiguous(workspace);
        });
    });
}

#[test]
fn rekey_replaces_a_derived_name_and_preserves_a_user_set_one() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            grow_to(workspace, 2, ctx);
            let derived_group = keyed_group(workspace, API, &[0]);
            let renamed_group = keyed_group(workspace, WEB, &[1]);
            if let Some(group) = workspace.tab_groups.get_mut(&renamed_group) {
                group.name = Some("Client work".to_string());
            }
            set_last_resolved_key(workspace, 0, API);
            set_last_resolved_key(workspace, 1, WEB);

            reconcile(workspace, 0, Some(DOCS), ctx);
            reconcile(workspace, 1, Some(INFRA), ctx);

            // A name that matched what the old key derived was automation's, so
            // it is replaced; one that differed was the user's, so it survives.
            assert_eq!(group_key(workspace, derived_group).as_deref(), Some(DOCS));
            assert_eq!(
                group_name(workspace, derived_group).as_deref(),
                Some("docs")
            );
            assert_eq!(group_key(workspace, renamed_group).as_deref(), Some(INFRA));
            assert_eq!(
                group_name(workspace, renamed_group).as_deref(),
                Some("Client work")
            );
            assert_groups_contiguous(workspace);
        });
    });
}

#[test]
fn manually_placed_tab_is_left_alone_even_though_its_own_group_exists() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            grow_to(workspace, 3, ctx);
            let api_group = keyed_group(workspace, API, &[0]);
            let web_group = keyed_group(workspace, WEB, &[1, 2]);
            // The user dragged this tab out of the API group into the WEB one;
            // its key never changed, so the WEB group carries neither its
            // previous nor its current key.
            set_last_resolved_key(workspace, 2, API);
            let order_before = tab_order(workspace);

            reconcile(workspace, 2, Some(API), ctx);

            assert_eq!(workspace.tabs[2].group_id, Some(web_group));
            assert_eq!(workspace.tabs[0].group_id, Some(api_group));
            assert_eq!(tab_order(workspace), order_before);
            assert_groups_contiguous(workspace);
        });
    });
}

#[test]
fn ungrouped_tab_is_left_alone_outside_the_enable_sweep() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            grow_to(workspace, 2, ctx);
            let api_group = keyed_group(workspace, API, &[1]);
            // Ungrouped, no marker: the user put it here deliberately.
            assert!(!workspace.tabs[0].placed_by_automation);

            reconcile(workspace, 0, Some(API), ctx);

            assert!(workspace.tabs[0].group_id.is_none());
            assert_eq!(
                workspace
                    .tabs
                    .iter()
                    .filter(|tab| tab.group_id == Some(api_group))
                    .count(),
                1
            );
            assert_groups_contiguous(workspace);
        });
    });
}

#[test]
fn pinned_tab_is_never_grouped_and_keeps_its_pin() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            grow_to(workspace, 2, ctx);
            keyed_group(workspace, API, &[1]);
            workspace.tabs[0].pinned = true;
            // Even a tab automation has not placed yet is skipped while pinned.
            workspace.tabs[0].placed_by_automation = true;

            reconcile(workspace, 0, Some(API), ctx);

            assert!(workspace.tabs[0].group_id.is_none());
            assert!(workspace.tabs[0].pinned);
            assert!(workspace.tabs[0].placed_by_automation);
            assert_groups_contiguous(workspace);
        });
    });
}

#[test]
fn joining_a_collapsed_group_leaves_it_collapsed() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            grow_to(workspace, 2, ctx);
            keyed_group(workspace, API, &[0]);
            let web_group = keyed_group(workspace, WEB, &[1]);
            if let Some(group) = workspace.tab_groups.get_mut(&web_group) {
                group.collapsed = true;
            }
            set_last_resolved_key(workspace, 0, API);
            let moved = workspace.tabs[0].pane_group.id();

            reconcile(workspace, 0, Some(WEB), ctx);

            let moved_index = tab_order(workspace)
                .iter()
                .position(|id| *id == moved)
                .expect("moved tab still exists");
            assert_eq!(workspace.tabs[moved_index].group_id, Some(web_group));
            assert!(
                workspace.tab_groups[&web_group].collapsed,
                "automation must not expand a group the user collapsed"
            );
            assert_groups_contiguous(workspace);
        });
    });
}

#[test]
fn tab_with_no_resolvable_key_is_left_where_it_is() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            grow_to(workspace, 2, ctx);
            let api_group = keyed_group(workspace, API, &[0]);
            set_last_resolved_key(workspace, 0, API);
            workspace.tabs[0].placed_by_automation = true;
            let order_before = tab_order(workspace);

            reconcile(workspace, 0, None, ctx);

            assert_eq!(workspace.tabs[0].group_id, Some(api_group));
            assert_eq!(group_key(workspace, api_group).as_deref(), Some(API));
            assert_eq!(tab_order(workspace), order_before);
            // The marker stays set so the tab is still placed once its key does
            // resolve; an unresolved key is not a manual act.
            assert!(workspace.tabs[0].placed_by_automation);
            assert_groups_contiguous(workspace);
        });
    });
}

#[test]
fn moving_a_tab_out_of_the_middle_of_a_run_keeps_both_groups_contiguous() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            grow_to(workspace, 5, ctx);
            let web_group = keyed_group(workspace, WEB, &[0, 1]);
            let api_group = keyed_group(workspace, API, &[2, 3, 4]);
            set_last_resolved_key(workspace, 3, API);
            let moved = workspace.tabs[3].pane_group.id();

            reconcile(workspace, 3, Some(WEB), ctx);

            let moved_index = tab_order(workspace)
                .iter()
                .position(|id| *id == moved)
                .expect("moved tab still exists");
            assert_eq!(workspace.tabs[moved_index].group_id, Some(web_group));
            assert_eq!(
                workspace
                    .tabs
                    .iter()
                    .filter(|tab| tab.group_id == Some(api_group))
                    .count(),
                2
            );
            assert_groups_contiguous(workspace);
        });
    });
}

#[test]
fn active_tab_is_unchanged_by_a_reorder_that_moves_a_background_tab() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            grow_to(workspace, 4, ctx);
            keyed_group(workspace, API, &[0]);
            keyed_group(workspace, WEB, &[1]);
            set_last_resolved_key(workspace, 0, API);

            // Activate a tab that the reorder will shift to a different index.
            workspace.activate_tab(1, ctx);
            let active = workspace.tabs[workspace.active_tab_index()].pane_group.id();

            reconcile(workspace, 0, Some(WEB), ctx);

            assert_eq!(
                workspace.tabs[workspace.active_tab_index()].pane_group.id(),
                active,
                "the active tab must be re-seated by identity, not left on an index"
            );
            assert_eq!(workspace.active_tab_index(), 0);
            assert_groups_contiguous(workspace);
        });
    });
}

#[test]
fn reconciling_an_identity_that_no_longer_exists_is_a_no_op() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            grow_to(workspace, 2, ctx);
            let api_group = keyed_group(workspace, API, &[0]);
            let order_before = tab_order(workspace);

            // The pane group resolved frames ago and is gone by the time the
            // key arrives.
            workspace.reconcile_tab_auto_group(
                EntityId::from_usize(usize::MAX),
                Some(key(WEB)),
                ctx,
            );

            assert_eq!(tab_order(workspace), order_before);
            assert_eq!(workspace.tab_groups.len(), 1);
            assert_eq!(group_key(workspace, api_group).as_deref(), Some(API));
            assert_groups_contiguous(workspace);
        });
    });
}
