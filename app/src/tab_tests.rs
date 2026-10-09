use std::collections::HashMap;

use pathfinder_color::ColorU;
use settings_value::SettingsValue as _;
use warpui::platform::keyboard::KeyCode;

use super::{
    SelectedTabColor, ShortcutModifierKind, TAB_ACTIVATE_BINDING_NAMES,
    TAB_ACTIVATE_LAST_BINDING_NAME, TabColor, TabShortcutModifierState, next_tab_color,
    tab_activate_binding_name, tab_group_menu_entry_flags,
};
use crate::themes::theme::AnsiColorIdentifier;
use crate::ui_components::color_dot::TAB_COLOR_OPTIONS;
use crate::workspace::tab_group::{TabGroup, TabGroupId};
use crate::workspace::tab_settings::DirectoryTabColor;

const ORANGE: TabColor = TabColor::Custom(ColorU {
    r: 255,
    g: 136,
    b: 0,
    a: 255,
});

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
        SelectedTabColor::Color(TAB_COLOR_OPTIONS[0].into())
    );
    for adjacent_colors in TAB_COLOR_OPTIONS.windows(2) {
        assert_eq!(
            next_tab_color(Some(adjacent_colors[0].into())),
            SelectedTabColor::Color(adjacent_colors[1].into())
        );
    }
    let last_color = TAB_COLOR_OPTIONS
        .last()
        .copied()
        .expect("the canonical tab color palette should not be empty");
    assert_eq!(
        next_tab_color(Some(last_color.into())),
        SelectedTabColor::Cleared
    );
    assert_eq!(
        next_tab_color(SelectedTabColor::Cleared.resolve(None)),
        SelectedTabColor::Color(TAB_COLOR_OPTIONS[0].into())
    );
    assert_eq!(
        next_tab_color(Some(AnsiColorIdentifier::White.into())),
        SelectedTabColor::Color(TAB_COLOR_OPTIONS[0].into())
    );
}

#[test]
fn tab_color_parses_names_and_hex_and_serializes_as_a_string() {
    let red = TabColor::Ansi(AnsiColorIdentifier::Red);
    assert_eq!("Red".parse(), Ok(red));
    assert_eq!("#FF8800".parse(), Ok(ORANGE));
    assert_eq!(serde_json::to_value(red).unwrap(), "red");
    assert_eq!(serde_json::to_value(ORANGE).unwrap(), "#ff8800");
    for invalid in ["#ff88", "#gg0000", "chartreuse", "ff8800", ""] {
        assert!(
            invalid.parse::<TabColor>().is_err(),
            "`{invalid}` should be rejected"
        );
    }
    assert_eq!(
        next_tab_color(Some(ORANGE)),
        SelectedTabColor::Color(TAB_COLOR_OPTIONS[0].into())
    );
}

#[test]
fn selected_tab_color_yaml_keeps_the_legacy_format_and_round_trips_custom_colors() {
    // The sqlite column format written before custom colors existed, for a red tab.
    let legacy = "---\nColor: red\n";
    let red = SelectedTabColor::Color(AnsiColorIdentifier::Red.into());
    assert_eq!(
        serde_yaml::from_str::<SelectedTabColor>(legacy).unwrap(),
        red
    );
    assert_eq!(serde_yaml::to_string(&red).unwrap(), legacy);

    let custom = SelectedTabColor::Color(ORANGE);
    let yaml = serde_yaml::to_string(&custom).unwrap();
    assert_eq!(
        serde_yaml::from_str::<SelectedTabColor>(&yaml).unwrap(),
        custom
    );
}

#[test]
fn directory_tab_color_setting_accepts_names_and_hex() {
    let from =
        |color: &str| DirectoryTabColor::from_file_value(&serde_json::json!({ "color": color }));
    assert_eq!(
        from("red"),
        Some(DirectoryTabColor::Color(AnsiColorIdentifier::Red.into()))
    );
    assert_eq!(from("#ff8800"), Some(DirectoryTabColor::Color(ORANGE)));
    assert_eq!(from("#ff88"), None);
}
