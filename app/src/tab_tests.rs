use std::collections::HashMap;

use warpui::platform::keyboard::KeyCode;

use super::{
    MOVE_TO_GROUP_LABEL, SelectedTabColor, ShortcutModifierKind, TAB_ACTIVATE_BINDING_NAMES,
    TAB_ACTIVATE_LAST_BINDING_NAME, TURN_OFF_AUTO_GROUP_TABS_LABEL, TURN_ON_AUTO_GROUP_TABS_LABEL,
    TabShortcutModifierState, auto_group_tabs_menu_label, next_tab_color,
    tab_activate_binding_name, tab_group_menu_entry_flags, tab_group_menu_items_for,
};
use crate::menu::MenuItem;
use crate::settings_view::{AppearancePageAction, SettingsAction};
use crate::themes::theme::AnsiColorIdentifier;
use crate::ui_components::color_dot::TAB_COLOR_OPTIONS;
use crate::workspace::WorkspaceAction;
use crate::workspace::tab_group::{TabGroup, TabGroupId};

/// Build a `tab_groups` map containing exactly the given group ids.
fn groups(ids: &[TabGroupId]) -> HashMap<TabGroupId, TabGroup> {
    ids.iter()
        .map(|id| {
            let mut group = TabGroup::new();
            group.id = *id;
            (*id, group)
        })
        .collect()
}

/// Stand-in label for a divider, so a single `assert_eq!` on the section can
/// pin both the entries and where the dividers fall between them.
const DIVIDER: &str = "<divider>";

/// The label of every row in a built menu section, in order.
fn labels(items: &[MenuItem<WorkspaceAction>]) -> Vec<&str> {
    items
        .iter()
        .map(|item| match item {
            MenuItem::Item(fields) => fields.label(),
            MenuItem::Separator => DIVIDER,
            MenuItem::ItemsRow { .. } => "<items row>",
            MenuItem::Submenu { fields, .. } => fields.label(),
            MenuItem::Header { fields, .. } => fields.label(),
        })
        .collect()
}

/// The tab index every menu-section test builds for; which tab it is does not
/// matter to the automatic-grouping entry, which is deliberately not tab-scoped.
const TAB_INDEX: usize = 0;

// GH-13073: a tab that is the sole member of its group must NOT be offered
// "New group with tab" (it would just recreate an identical single-tab group);
// it offers "Remove from group" instead.
#[test]
fn sole_member_of_group_hides_new_group_and_offers_remove() {
    let gid = TabGroupId::new();
    let (show_new_group, _show_move_to_group, show_remove_from_group) =
        tab_group_menu_entry_flags(Some(gid), &groups(&[gid]), /* is_only_member */ true);

    assert!(
        !show_new_group,
        "the sole member of a group should not offer 'New group with tab'"
    );
    assert!(
        show_remove_from_group,
        "a tab in a group should offer 'Remove from group'"
    );
}

#[test]
fn tab_shortcut_modifier_state_clear_reports_whether_state_changed() {
    let mut state = TabShortcutModifierState::new();

    assert!(!state.clear_held_keys());

    assert!(state.held_keys.insert(KeyCode::SuperLeft));
    assert!(state.held_kinds().is_empty());
    assert!(state.reveal_key_if_held(KeyCode::SuperLeft));
    assert_eq!(
        state.held_kinds(),
        [ShortcutModifierKind::Super].into_iter().collect()
    );

    assert!(state.clear_held_keys());
    assert!(state.held_kinds().is_empty());
    assert!(!state.clear_held_keys());
}

#[test]
fn tab_shortcut_modifier_state_only_reveals_keys_that_remain_held() {
    let mut state = TabShortcutModifierState::new();

    assert!(!state.reveal_key_if_held(KeyCode::SuperLeft));

    assert!(state.held_keys.insert(KeyCode::SuperLeft));
    assert!(state.held_keys.remove(&KeyCode::SuperLeft));
    assert!(!state.reveal_key_if_held(KeyCode::SuperLeft));
    assert!(state.held_kinds().is_empty());
}

#[test]
fn tab_activate_binding_name_prefers_numbered_binding_for_final_tab() {
    assert_eq!(
        tab_activate_binding_name(2, 3),
        Some(TAB_ACTIVATE_BINDING_NAMES[2])
    );
    assert_eq!(
        tab_activate_binding_name(7, 8),
        Some(TAB_ACTIVATE_BINDING_NAMES[7])
    );
}

#[test]
fn tab_activate_binding_name_uses_last_tab_binding_beyond_numbered_tabs() {
    assert_eq!(
        tab_activate_binding_name(8, 9),
        Some(TAB_ACTIVATE_LAST_BINDING_NAME)
    );
    assert_eq!(
        tab_activate_binding_name(9, 10),
        Some(TAB_ACTIVATE_LAST_BINDING_NAME)
    );
}

#[test]
fn tab_activate_binding_name_omits_unbound_and_out_of_bounds_tabs() {
    assert_eq!(tab_activate_binding_name(8, 10), None);
    assert_eq!(tab_activate_binding_name(10, 10), None);
    assert_eq!(tab_activate_binding_name(0, 0), None);
}

// GH-13073 follow-up: a tab that shares a group with siblings SHOULD still be
// offered "New group with tab" so it can be pulled out into its own new group
// (à la Chrome), and it offers "Remove from group" as well.
#[test]
fn grouped_tab_with_siblings_offers_new_group_and_remove() {
    let gid = TabGroupId::new();
    let (show_new_group, _show_move_to_group, show_remove_from_group) =
        tab_group_menu_entry_flags(Some(gid), &groups(&[gid]), /* is_only_member */ false);

    assert!(
        show_new_group,
        "a grouped tab with siblings should still offer 'New group with tab'"
    );
    assert!(
        show_remove_from_group,
        "a grouped tab should offer 'Remove from group'"
    );
}

// An ungrouped tab always offers "New group with tab" and never offers
// "Remove from group". `is_only_member` is irrelevant when ungrouped.
#[test]
fn ungrouped_tab_offers_new_group_and_hides_remove() {
    let (show_new_group, _show_move_to_group, show_remove_from_group) =
        tab_group_menu_entry_flags(None, &HashMap::new(), /* is_only_member */ false);

    assert!(
        show_new_group,
        "an ungrouped tab should offer 'New group with tab'"
    );
    assert!(
        !show_remove_from_group,
        "an ungrouped tab should not offer 'Remove from group'"
    );
}

// "Move to group" should only appear when a group other than the tab's own
// exists — for both grouped and ungrouped tabs.
#[test]
fn move_to_group_only_shown_when_other_groups_exist() {
    let own = TabGroupId::new();
    let other = TabGroupId::new();

    // Grouped tab whose group is the only one: no other groups to move to.
    let (_n, move_only_own, _r) = tab_group_menu_entry_flags(Some(own), &groups(&[own]), true);
    assert!(!move_only_own);

    // Grouped tab with another group present: offer "Move to group".
    let (_n, move_with_other, _r) =
        tab_group_menu_entry_flags(Some(own), &groups(&[own, other]), true);
    assert!(move_with_other);

    // Ungrouped tab with an existing group: offer "Move to group".
    let (_n, move_ungrouped, _r) = tab_group_menu_entry_flags(None, &groups(&[other]), false);
    assert!(move_ungrouped);
}

#[test]
fn next_tab_color_follows_the_canonical_palette_and_clears_after_the_last_color() {
    assert_eq!(
        next_tab_color(None),
        SelectedTabColor::Color(TAB_COLOR_OPTIONS[0])
    );
    for adjacent_colors in TAB_COLOR_OPTIONS.windows(2) {
        assert_eq!(
            next_tab_color(Some(adjacent_colors[0])),
            SelectedTabColor::Color(adjacent_colors[1])
        );
    }
    let last_color = TAB_COLOR_OPTIONS
        .last()
        .copied()
        .expect("the canonical tab color palette should not be empty");
    assert_eq!(next_tab_color(Some(last_color)), SelectedTabColor::Cleared);
    assert_eq!(
        next_tab_color(SelectedTabColor::Cleared.resolve(None)),
        SelectedTabColor::Color(TAB_COLOR_OPTIONS[0])
    );
    assert_eq!(
        next_tab_color(Some(AnsiColorIdentifier::White)),
        SelectedTabColor::Color(TAB_COLOR_OPTIONS[0])
    );
}

// R19: with the automatic-grouping mode available, the shared per-tab menu
// offers the toggle, and a divider fences it off from the tab-scoped entries
// above it. The full ordering is asserted so a later entry cannot quietly slip
// in between the divider and the toggle and inherit the fence.
#[test]
fn auto_group_tabs_toggle_is_offered_behind_a_divider() {
    let own = TabGroupId::new();
    let other = TabGroupId::new();

    let items = tab_group_menu_items_for(
        TAB_INDEX,
        Some(own),
        &groups(&[own, other]),
        /* is_only_member */ false,
        /* auto_group_tabs */ Some(false),
    );

    assert_eq!(
        labels(&items),
        vec![
            "New group with tab",
            MOVE_TO_GROUP_LABEL,
            "Remove from group",
            DIVIDER,
            TURN_ON_AUTO_GROUP_TABS_LABEL,
        ],
        "the automatic-grouping toggle should close the section, behind a divider"
    );
}

// The divider is still drawn when only one tab-scoped entry precedes the
// toggle, which is the common case for an ungrouped tab.
#[test]
fn auto_group_tabs_toggle_is_divided_from_a_lone_tab_scoped_entry() {
    let items = tab_group_menu_items_for(
        TAB_INDEX,
        /* group_id */ None,
        &HashMap::new(),
        /* is_only_member */ false,
        /* auto_group_tabs */ Some(false),
    );

    assert_eq!(
        labels(&items),
        vec!["New group with tab", DIVIDER, TURN_ON_AUTO_GROUP_TABS_LABEL,],
    );
}

// With the mode unavailable (feature flags off) the entry must not exist at
// all — not disabled, not present-but-inert — and it must not leave a dangling
// divider at the end of the section either.
#[test]
fn auto_group_tabs_toggle_is_absent_when_the_mode_is_unavailable() {
    let own = TabGroupId::new();
    let other = TabGroupId::new();

    let items = tab_group_menu_items_for(
        TAB_INDEX,
        Some(own),
        &groups(&[own, other]),
        /* is_only_member */ false,
        /* auto_group_tabs */ None,
    );

    assert_eq!(
        labels(&items),
        vec![
            "New group with tab",
            MOVE_TO_GROUP_LABEL,
            "Remove from group",
        ],
        "with the mode unavailable the section should hold only tab-scoped entries"
    );
}

// The label has to carry the entry's whole meaning: which way activating it
// flips the mode, and that the flip is not scoped to this tab or this window.
#[test]
fn auto_group_tabs_toggle_label_reflects_state_and_window_wide_scope() {
    assert_eq!(
        auto_group_tabs_menu_label(false),
        TURN_ON_AUTO_GROUP_TABS_LABEL
    );
    assert_eq!(
        auto_group_tabs_menu_label(true),
        TURN_OFF_AUTO_GROUP_TABS_LABEL
    );
    assert_ne!(
        auto_group_tabs_menu_label(true),
        auto_group_tabs_menu_label(false),
        "the label must read differently with the mode on vs off"
    );

    for (enabled, expected_verb) in [(false, "Turn on"), (true, "Turn off")] {
        let label = auto_group_tabs_menu_label(enabled);
        assert!(
            label.starts_with(expected_verb),
            "{label:?} should say what activating it does"
        );
        assert!(
            label.contains("all windows"),
            "{label:?} should name the window-wide scope, unlike its tab-scoped neighbours"
        );
    }
}

// The label the menu actually renders tracks the setting, so the entry never
// invites the user to turn on a mode that is already on.
#[test]
fn auto_group_tabs_toggle_entry_label_tracks_the_setting() {
    for (enabled, expected) in [
        (false, TURN_ON_AUTO_GROUP_TABS_LABEL),
        (true, TURN_OFF_AUTO_GROUP_TABS_LABEL),
    ] {
        let items = tab_group_menu_items_for(
            TAB_INDEX,
            /* group_id */ None,
            &HashMap::new(),
            /* is_only_member */ false,
            Some(enabled),
        );
        assert_eq!(labels(&items).last(), Some(&expected));
    }
}

// The toggle must not become a second writer of `appearance.tabs.auto_group_tabs`:
// it dispatches the same settings-page action the Settings switch and the
// keybinding use, so all three share one write path and one telemetry event.
#[test]
fn auto_group_tabs_toggle_routes_through_the_settings_page_toggle() {
    let items = tab_group_menu_items_for(
        TAB_INDEX,
        /* group_id */ None,
        &HashMap::new(),
        /* is_only_member */ false,
        /* auto_group_tabs */ Some(false),
    );

    let action = items
        .last()
        .expect("section should end with the toggle")
        .item_on_select_action()
        .expect("the toggle should dispatch an action");

    assert!(
        matches!(
            action,
            WorkspaceAction::DispatchToSettingsTab(SettingsAction::AppearancePageToggle(
                AppearancePageAction::ToggleAutoGroupTabs
            ))
        ),
        "expected the existing settings toggle, got {action:?}"
    );
}
