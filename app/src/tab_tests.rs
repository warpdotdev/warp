use std::collections::HashMap;

use super::tab_group_menu_entry_flags;
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
