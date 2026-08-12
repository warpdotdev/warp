use std::path::PathBuf;

use warp_core::features::FeatureFlag;
use warp_core::ui::theme::AnsiColorIdentifier;
use warpui::App;

use super::*;
use crate::ui_components::color_dot::TAB_COLOR_OPTIONS;
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

/// Makes tab `tab_index` resolve to `key_path` through the real resolver: its
/// anchor pane reports the checkout directory, and detection answers with the
/// shared git directory that is the key.
///
/// Needed by the cases that call a path which resolves the key itself, rather
/// than being handed one the way [`reconcile`] is.
fn set_test_key(
    workspace: &mut Workspace,
    tab_index: usize,
    key_path: &str,
    ctx: &ViewContext<Workspace>,
) {
    let pane_group = workspace.tabs[tab_index].pane_group.clone();
    let anchor: PaneId = {
        let group = pane_group.as_ref(ctx);
        group
            .visible_pane_ids()
            .into_iter()
            .find(|pane_id| group.terminal_view_from_pane_id(*pane_id, ctx).is_some())
            .expect("a terminal tab has at least one terminal pane")
    };
    let directory = PathBuf::from(key_path)
        .parent()
        .expect("a project key has a parent directory")
        .to_path_buf();

    workspace
        .auto_grouping_state
        .test_pane_directories
        .insert(anchor, directory.clone());
    workspace
        .auto_grouping_state
        .test_git_resolutions
        .insert(directory, GitResolution::Resolved(PathBuf::from(key_path)));
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

// R8 qualifies both sides of a name collision. The second project's group is
// created long after the first one was named, so the fix has to reach back and
// re-qualify the name already stored on the older group.
#[test]
fn a_new_group_whose_name_collides_qualifies_the_group_it_collides_with() {
    const SERVICES_API: &str = "/work/services/api/.git";
    const VENDOR_API: &str = "/work/vendor/api/.git";

    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            grow_to(workspace, 2, ctx);
            // Both tabs are new and awaiting placement, which is what makes
            // reconcile create a group for each of them.
            workspace.tabs[0].placed_by_automation = true;
            workspace.tabs[1].placed_by_automation = true;

            reconcile(workspace, 0, Some(SERVICES_API), ctx);
            let services_group = workspace.tabs[0]
                .group_id
                .expect("the first tab should be grouped");
            assert_eq!(
                group_name(workspace, services_group).as_deref(),
                Some("api"),
                "with nothing to collide with, the name is unqualified"
            );

            reconcile(workspace, 1, Some(VENDOR_API), ctx);
            let vendor_group = workspace
                .tabs
                .iter()
                .find_map(|tab| tab.group_id.filter(|id| *id != services_group))
                .expect("the second tab should be in its own group");

            assert_eq!(
                group_name(workspace, vendor_group).as_deref(),
                Some("vendor/api")
            );
            assert_eq!(
                group_name(workspace, services_group).as_deref(),
                Some("services/api"),
                "the older group has to be re-qualified too, or the two read as `api` and `vendor/api`"
            );
            assert_groups_contiguous(workspace);
        });
    });
}

// A name the user typed is outside the two forms derivation can produce, so
// re-qualification must leave it alone even when a collision appears.
#[test]
fn requalification_leaves_a_user_named_group_alone() {
    const SERVICES_API: &str = "/work/services/api/.git";
    const VENDOR_API: &str = "/work/vendor/api/.git";

    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            grow_to(workspace, 2, ctx);
            // Both tabs are new and awaiting placement, which is what makes
            // reconcile create a group for each of them.
            workspace.tabs[0].placed_by_automation = true;
            workspace.tabs[1].placed_by_automation = true;

            reconcile(workspace, 0, Some(SERVICES_API), ctx);
            let services_group = workspace.tabs[0]
                .group_id
                .expect("the first tab should be grouped");
            if let Some(group) = workspace.tab_groups.get_mut(&services_group) {
                group.name = Some("Backend".to_string());
            }

            reconcile(workspace, 1, Some(VENDOR_API), ctx);

            assert_eq!(
                group_name(workspace, services_group).as_deref(),
                Some("Backend")
            );
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

// Derived tab colors. The decision table is the same one the name rules use —
// automation may replace what it put there and nothing else — so the cases live
// beside them rather than in a module of their own. What automation colors is
// the group's *members*; the group container itself is left to the user.

/// Turns the mode and its coloring on through the settings the workspace reads.
fn enable_auto_group_colors(app: &mut App) {
    TabSettings::handle(&*app).update(app, |settings, ctx| {
        settings.auto_group_tabs.set_value(true, ctx).unwrap();
        settings.auto_group_tab_colors.set_value(true, ctx).unwrap();
    });
}

/// Turns the mode on with its coloring left off.
fn enable_auto_grouping_only(app: &mut App) {
    TabSettings::handle(&*app).update(app, |settings, ctx| {
        settings.auto_group_tabs.set_value(true, ctx).unwrap();
    });
}

fn tab_color(workspace: &Workspace, tab_index: usize) -> SelectedTabColor {
    workspace.tabs[tab_index].selected_color
}

/// The color of a tab a move has re-indexed, read back by the identity the tab
/// keeps across the move.
fn tab_color_by_identity(workspace: &Workspace, pane_group_id: EntityId) -> SelectedTabColor {
    let tab_index = workspace
        .tab_index_for_pane_group(pane_group_id)
        .expect("the tab still exists");
    tab_color(workspace, tab_index)
}

fn group_color(workspace: &Workspace, group_id: TabGroupId) -> SelectedTabColor {
    workspace
        .tab_groups
        .get(&group_id)
        .expect("group still exists")
        .color
}

fn derived(path: &str) -> SelectedTabColor {
    SelectedTabColor::Color(project_key::derived_color(&key(path)))
}

/// Paints a tab exactly as automation would have, so a later case can show what
/// happens to a color automation owns.
fn paint_as_automation(workspace: &mut Workspace, tab_index: usize, path: &str) {
    workspace.tabs[tab_index].selected_color = derived(path);
}

fn set_tab_color(workspace: &mut Workspace, tab_index: usize, color: SelectedTabColor) {
    workspace.tabs[tab_index].selected_color = color;
}

/// A color automation would never have derived for `path`, for the cases that
/// need a color only the user could have chosen.
fn color_the_user_would_have_picked(path: &str) -> AnsiColorIdentifier {
    let derived = project_key::derived_color(&key(path));
    TAB_COLOR_OPTIONS
        .into_iter()
        .find(|color| *color != derived)
        .expect("the palette holds more than one color")
}

#[test]
fn a_tab_automation_groups_takes_its_projects_color() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        enable_auto_group_colors(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            workspace.tabs[0].placed_by_automation = true;

            reconcile(workspace, 0, Some(API), ctx);

            assert_eq!(tab_color(workspace, 0), derived(API));
        });
    });
}

// The point of the whole arrangement: the colour lands on the tabs, and the
// group container it sits in is left for the user to colour or not.
#[test]
fn automation_never_colors_the_group_itself() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        enable_auto_group_colors(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            workspace.tabs[0].placed_by_automation = true;

            reconcile(workspace, 0, Some(API), ctx);

            let group_id = workspace.tabs[0].group_id.expect("the tab was grouped");
            assert_eq!(group_color(workspace, group_id), SelectedTabColor::Unset);
        });
    });
}

// The case this arrangement exists for: a tab that changes project takes the new
// project's colour with it into the group it moves to.
#[test]
fn a_tab_moving_between_projects_takes_the_new_projects_color() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        enable_auto_group_colors(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            grow_to(workspace, 2, ctx);
            keyed_group(workspace, API, &[0]);
            let web_group = keyed_group(workspace, WEB, &[1]);
            paint_as_automation(workspace, 0, API);
            set_last_resolved_key(workspace, 0, API);
            let moved = workspace.tabs[0].pane_group.id();

            reconcile(workspace, 0, Some(WEB), ctx);

            let moved_index = workspace
                .tab_index_for_pane_group(moved)
                .expect("the tab still exists");
            assert_eq!(workspace.tabs[moved_index].group_id, Some(web_group));
            assert_eq!(tab_color_by_identity(workspace, moved), derived(WEB));
        });
    });
}

#[test]
fn a_color_the_user_chose_survives_a_move_between_projects() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        enable_auto_group_colors(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            grow_to(workspace, 2, ctx);
            keyed_group(workspace, API, &[0]);
            keyed_group(workspace, WEB, &[1]);
            let user_color = color_the_user_would_have_picked(API);
            set_tab_color(workspace, 0, SelectedTabColor::Color(user_color));
            set_last_resolved_key(workspace, 0, API);
            let moved = workspace.tabs[0].pane_group.id();

            reconcile(workspace, 0, Some(WEB), ctx);

            assert_eq!(
                tab_color_by_identity(workspace, moved),
                SelectedTabColor::Color(user_color),
                "a colour the user put on this tab must survive it changing project"
            );
        });
    });
}

#[test]
fn a_rekeyed_groups_members_take_the_new_projects_color() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        enable_auto_group_colors(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            grow_to(workspace, 2, ctx);
            let api_group = keyed_group(workspace, API, &[0]);
            paint_as_automation(workspace, 0, API);
            set_last_resolved_key(workspace, 0, API);

            reconcile(workspace, 0, Some(WEB), ctx);

            // Re-keyed in place, so the same group now reads as the new project.
            assert_eq!(group_key(workspace, api_group).as_deref(), Some(WEB));
            assert_eq!(tab_color(workspace, 0), derived(WEB));
        });
    });
}

#[test]
fn a_color_the_user_chose_survives_a_rekey() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        enable_auto_group_colors(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            grow_to(workspace, 2, ctx);
            let api_group = keyed_group(workspace, API, &[0]);
            let user_color = color_the_user_would_have_picked(API);
            set_tab_color(workspace, 0, SelectedTabColor::Color(user_color));
            set_last_resolved_key(workspace, 0, API);

            reconcile(workspace, 0, Some(WEB), ctx);

            assert_eq!(group_key(workspace, api_group).as_deref(), Some(WEB));
            assert_eq!(tab_color(workspace, 0), SelectedTabColor::Color(user_color));
        });
    });
}

#[test]
fn a_color_the_user_cleared_stays_cleared() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        enable_auto_group_colors(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            grow_to(workspace, 2, ctx);
            keyed_group(workspace, API, &[0]);
            set_tab_color(workspace, 0, SelectedTabColor::Cleared);
            set_last_resolved_key(workspace, 0, API);

            reconcile(workspace, 0, Some(WEB), ctx);

            assert_eq!(tab_color(workspace, 0), SelectedTabColor::Cleared);
        });
    });
}

#[test]
fn tabs_stay_uncolored_while_the_setting_is_off() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        enable_auto_grouping_only(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            workspace.tabs[0].placed_by_automation = true;

            reconcile(workspace, 0, Some(API), ctx);

            assert!(workspace.tabs[0].group_id.is_some(), "the tab was grouped");
            assert_eq!(tab_color(workspace, 0), SelectedTabColor::Unset);
        });
    });
}

// Leaving a group by hand: automation takes back the colour it derived, so a
// detached tab stops advertising a project it is no longer grouped under.

#[test]
fn leaving_a_group_takes_automations_color_off_the_tab() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        enable_auto_group_colors(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            grow_to(workspace, 2, ctx);
            keyed_group(workspace, API, &[0, 1]);
            paint_as_automation(workspace, 0, API);
            paint_as_automation(workspace, 1, API);
            let left = workspace.tabs[0].pane_group.id();

            workspace.remove_tab_from_group(0, ctx);

            assert_eq!(
                tab_color_by_identity(workspace, left),
                SelectedTabColor::Unset,
                "the tab that left keeps no colour of the project it left"
            );
            let stayed = workspace
                .tabs
                .iter()
                .position(|tab| tab.group_id.is_some())
                .expect("the other member is still grouped");
            assert_eq!(
                tab_color(workspace, stayed),
                derived(API),
                "the member that stayed keeps its project's colour"
            );
        });
    });
}

#[test]
fn leaving_a_group_keeps_a_color_the_user_chose() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        enable_auto_group_colors(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            grow_to(workspace, 2, ctx);
            keyed_group(workspace, API, &[0, 1]);
            let user_color = color_the_user_would_have_picked(API);
            set_tab_color(workspace, 0, SelectedTabColor::Color(user_color));
            let left = workspace.tabs[0].pane_group.id();

            workspace.remove_tab_from_group(0, ctx);

            assert_eq!(
                tab_color_by_identity(workspace, left),
                SelectedTabColor::Color(user_color)
            );
        });
    });
}

#[test]
fn ungrouping_takes_automations_color_off_every_member() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        enable_auto_group_colors(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            grow_to(workspace, 2, ctx);
            let api_group = keyed_group(workspace, API, &[0, 1]);
            paint_as_automation(workspace, 0, API);
            let user_color = color_the_user_would_have_picked(API);
            set_tab_color(workspace, 1, SelectedTabColor::Color(user_color));

            workspace.ungroup_tabs(api_group, ctx);

            assert_eq!(tab_color(workspace, 0), SelectedTabColor::Unset);
            assert_eq!(
                tab_color(workspace, 1),
                SelectedTabColor::Color(user_color),
                "ungrouping is not licence to drop a colour the user chose"
            );
        });
    });
}

#[test]
fn the_sweep_colors_the_tabs_the_mode_already_grouped() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        enable_auto_group_colors(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            grow_to(workspace, 2, ctx);
            keyed_group(workspace, API, &[0]);
            keyed_group(workspace, WEB, &[1]);

            workspace.sweep_auto_tab_colors(ctx);

            assert_eq!(tab_color(workspace, 0), derived(API));
            assert_eq!(tab_color(workspace, 1), derived(WEB));
        });
    });
}

#[test]
fn the_sweep_colors_every_member_of_a_shared_project() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        enable_auto_group_colors(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            grow_to(workspace, 3, ctx);
            keyed_group(workspace, API, &[0, 1, 2]);

            workspace.sweep_auto_tab_colors(ctx);

            assert_eq!(tab_color(workspace, 0), derived(API));
            assert_eq!(tab_color(workspace, 1), derived(API));
            assert_eq!(tab_color(workspace, 2), derived(API));
        });
    });
}

#[test]
fn the_sweep_leaves_a_color_the_user_chose() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        enable_auto_group_colors(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            keyed_group(workspace, API, &[0]);
            let user_color = color_the_user_would_have_picked(API);
            set_tab_color(workspace, 0, SelectedTabColor::Color(user_color));

            workspace.sweep_auto_tab_colors(ctx);

            assert_eq!(tab_color(workspace, 0), SelectedTabColor::Color(user_color));
        });
    });
}

#[test]
fn the_sweep_never_colors_a_tab_in_a_group_the_user_made() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        enable_auto_group_colors(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            // No project key: an ordinary manual group, whose members have no
            // project to take a color from.
            let group = TabGroup::new();
            let group_id = group.id;
            workspace.tab_groups.insert(group_id, group);
            workspace.tabs[0].group_id = Some(group_id);

            workspace.sweep_auto_tab_colors(ctx);

            assert_eq!(tab_color(workspace, 0), SelectedTabColor::Unset);
        });
    });
}

#[test]
fn the_sweep_does_nothing_while_the_setting_is_off() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        enable_auto_grouping_only(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            keyed_group(workspace, API, &[0]);

            workspace.sweep_auto_tab_colors(ctx);

            assert_eq!(tab_color(workspace, 0), SelectedTabColor::Unset);
        });
    });
}

// Turning the coloring off is not an undo: what automation painted stays, and
// the user can clear or change any of it by hand.
#[test]
fn colors_automation_set_outlive_the_setting_being_turned_off() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        enable_auto_group_colors(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            keyed_group(workspace, API, &[0]);
            workspace.sweep_auto_tab_colors(ctx);
            assert_eq!(tab_color(workspace, 0), derived(API));
        });

        TabSettings::handle(&app).update(&mut app, |settings, ctx| {
            settings
                .auto_group_tab_colors
                .set_value(false, ctx)
                .unwrap();
        });

        workspace.update(&mut app, |workspace, _ctx| {
            assert_eq!(tab_color(workspace, 0), derived(API));
        });
    });
}

// The wiring, not the sweep: turning the setting on has to reach
// `sweep_auto_tab_colors` through the subscription the workspace really uses.
// Every case above calls the sweep directly, so a broken dispatch would not show
// up in any of them.
#[test]
fn turning_the_setting_on_paints_through_the_settings_subscription() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        enable_auto_grouping_only(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, _ctx| {
            keyed_group(workspace, API, &[0]);
            assert_eq!(tab_color(workspace, 0), SelectedTabColor::Unset);
        });

        TabSettings::handle(&app).update(&mut app, |settings, ctx| {
            settings.auto_group_tab_colors.set_value(true, ctx).unwrap();
        });

        workspace.update(&mut app, |workspace, _ctx| {
            assert_eq!(tab_color(workspace, 0), derived(API));
        });
    });
}

// The compound gate: colouring is meaningless without the mode, and the command
// palette can flip it while the mode is off, since the Settings row is the only
// surface that hides.
#[test]
fn colors_stay_unpainted_while_the_mode_itself_is_off() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        TabSettings::handle(&app).update(&mut app, |settings, ctx| {
            settings.auto_group_tab_colors.set_value(true, ctx).unwrap();
        });
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            keyed_group(workspace, API, &[0]);

            workspace.sweep_auto_tab_colors(ctx);

            assert_eq!(tab_color(workspace, 0), SelectedTabColor::Unset);
        });
    });
}

// Replacing a group with another for the same project — "new group from this
// tab" on a sole member — must not launder the user's colour into automation's.

/// Stands in for `new_tab_group_from_tab`'s core: a fresh group takes the tab,
/// the emptied one is pruned, and the new group adopts the key. The key of the
/// group being left is read first, because pruning is what makes it
/// unrecoverable.
fn replace_group_from_sole_member(
    workspace: &mut Workspace,
    tab_index: usize,
    ctx: &mut ViewContext<Workspace>,
) -> TabGroupId {
    let previous_group_id = workspace.tabs[tab_index].group_id;
    let pane_group_id = workspace.tabs[tab_index].pane_group.id();

    let group = TabGroup::new();
    let group_id = group.id;
    workspace.tab_groups.insert(group_id, group);
    workspace.tabs[tab_index].group_id = Some(group_id);

    let previous_key = previous_group_id.and_then(|gid| workspace.project_key_of_group(gid));
    if let Some(previous_group_id) = previous_group_id {
        workspace.prune_empty_tab_group(previous_group_id, ctx);
    }

    workspace.adopt_project_key_for_new_group(group_id, pane_group_id, previous_key, ctx);
    group_id
}

#[test]
fn a_group_replacing_another_for_the_same_project_keeps_the_project_color() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        enable_auto_group_colors(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            let api_group = keyed_group(workspace, API, &[0]);
            paint_as_automation(workspace, 0, API);
            set_test_key(workspace, 0, API, ctx);

            let replacement = replace_group_from_sole_member(workspace, 0, ctx);

            assert!(!workspace.tab_groups.contains_key(&api_group));
            assert_eq!(group_key(workspace, replacement).as_deref(), Some(API));
            assert_eq!(tab_color(workspace, 0), derived(API));
        });
    });
}

#[test]
fn a_group_replacing_another_keeps_the_color_the_user_chose() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        enable_auto_group_colors(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            keyed_group(workspace, API, &[0]);
            let user_color = color_the_user_would_have_picked(API);
            set_tab_color(workspace, 0, SelectedTabColor::Color(user_color));
            set_test_key(workspace, 0, API, ctx);

            replace_group_from_sole_member(workspace, 0, ctx);

            assert_eq!(
                tab_color(workspace, 0),
                SelectedTabColor::Color(user_color),
                "the colour the user gave this tab must survive the replacement"
            );
        });
    });
}

#[test]
fn a_group_replacing_another_keeps_a_color_the_user_cleared() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        enable_auto_group_colors(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            keyed_group(workspace, API, &[0]);
            set_tab_color(workspace, 0, SelectedTabColor::Cleared);
            set_test_key(workspace, 0, API, ctx);

            replace_group_from_sole_member(workspace, 0, ctx);

            assert_eq!(
                tab_color(workspace, 0),
                SelectedTabColor::Cleared,
                "clearing this tab's colour must not be undone by replacing its group"
            );
        });
    });
}
