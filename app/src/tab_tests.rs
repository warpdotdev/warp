use std::collections::HashMap;

use warpui::App;

use super::TabData;
use crate::features::FeatureFlag;
use crate::menu::MenuItem;
use crate::workspace::tab_group::{TabGroup, TabGroupId};
use crate::workspace::view::tests::{initialize_app, mock_workspace};
use crate::workspace::WorkspaceAction;

const NEW_GROUP_WITH_TAB_LABEL: &str = "New group with tab";

/// True when `items` contains the "New group with tab" affordance, recognised by
/// both its `NewTabGroupFromTab` action and its label.
fn contains_new_group_with_tab(items: &[MenuItem<WorkspaceAction>]) -> bool {
    items.iter().any(|item| {
        matches!(
            item.item_on_select_action(),
            Some(WorkspaceAction::NewTabGroupFromTab(_))
        )
    })
}

/// True when any menu item is labelled "New group with tab", regardless of its
/// action. Used to double-check the label-side affordance is gone too.
fn contains_new_group_with_tab_label(items: &[MenuItem<WorkspaceAction>]) -> bool {
    items.iter().any(|item| match item {
        MenuItem::Item(fields) => fields.label() == NEW_GROUP_WITH_TAB_LABEL,
        _ => false,
    })
}

/// Regression test for the vertical-tab context menu offering "New group with
/// tab" on a tab that is already in a group (see the gate added in
/// `TabData::tab_group_menu_items`).
///
/// - A grouped tab (`group_id` is `Some`) must NOT offer "New group with tab".
/// - An ungrouped tab (`group_id` is `None`) must still offer it.
///
/// The ungrouped assertion is the one that fails against the old, unconditional
/// code; the grouped assertion guards the fix.
#[test]
fn new_group_with_tab_hidden_when_tab_already_grouped() {
    // `tab_group_menu_items` early-returns empty unless this flag is on, so we
    // must hold the override for the duration of the test to exercise the real
    // path rather than the disabled-feature short circuit.
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, _ctx| {
            // A real pane group from the mock workspace's first tab. Only
            // `group_id` is read by `tab_group_menu_items`; the pane group is
            // just what `TabData::new` needs to exist.
            let pane_group = workspace
                .get_pane_group_view(0)
                .expect("mock workspace should have a tab at index 0")
                .clone();

            // No other groups exist, so "Move to group" stays absent and the
            // only group-related entry under test is "New group with tab".
            let tab_groups: HashMap<TabGroupId, TabGroup> = HashMap::new();

            // Ungrouped tab: the item must be present.
            let mut tab = TabData::new(pane_group.clone());
            tab.group_id = None;
            let ungrouped_items = tab.tab_group_menu_items(0, &tab_groups);
            assert!(
                contains_new_group_with_tab(&ungrouped_items),
                "an ungrouped tab should still offer \"New group with tab\""
            );
            assert!(
                contains_new_group_with_tab_label(&ungrouped_items),
                "the ungrouped \"New group with tab\" item should carry its label"
            );

            // Grouped tab: the item must be hidden (the regression).
            let group_id = TabGroupId::new();
            tab.group_id = Some(group_id);
            let grouped_items = tab.tab_group_menu_items(0, &tab_groups);
            assert!(
                !contains_new_group_with_tab(&grouped_items),
                "a tab already in a group must not offer \"New group with tab\""
            );
            assert!(
                !contains_new_group_with_tab_label(&grouped_items),
                "a grouped tab must not carry the \"New group with tab\" label either"
            );
        });
    });
}
