//! Unit tests for [`CrossWindowTabDrag`] drag-source state and the
//! placeholder-collapse policy.
//!
//! These focus on [`CrossWindowTabDrag::collapsed_source_placeholder_index`],
//! which decides whether the source window's horizontal tab bar collapses the
//! detached-placeholder slot to zero width. The regression these guard against
//! is the horizontal "fuzzy shake": collapsing the placeholder while the cursor
//! is reordering it back in the source window removed the visible drop zone and
//! made the slot oscillate every frame.

use warpui::geometry::vector::{Vector2F, vec2f};
use warpui::{EntityId, WindowId};

use super::CrossWindowTabDrag;
use crate::workspace::tab_group::TabGroupId;

const SOURCE_TAB_INDEX: usize = 2;

fn begin_multi_tab_drag(
    drag: &mut CrossWindowTabDrag,
    source_window_id: WindowId,
    preview_window_id: WindowId,
) {
    drag.begin_multi_tab_drag(
        source_window_id,
        SOURCE_TAB_INDEX,
        EntityId::from_usize(42),
        Vector2F::zero(),
        vec2f(800.0, 600.0),
        Vector2F::zero(),
        preview_window_id,
        false,
        vec2f(120.0, 34.0),
    );
}

#[test]
fn drag_captures_the_dragged_pane_group_identity() {
    // Source cleanup resolves the tab to remove through this id, so the id has
    // to survive from drag start to drop. The index next to it is deliberately
    // NOT the thing cleanup keys on: it is frozen here and goes stale if the
    // source tab list changes while the drag is in flight.
    let source = WindowId::from_usize(1);
    let preview = WindowId::from_usize(2);
    let dragged = EntityId::from_usize(7);

    let mut drag = CrossWindowTabDrag::new();
    assert_eq!(drag.source_pane_group_id(), None);

    drag.begin_multi_tab_drag(
        source,
        SOURCE_TAB_INDEX,
        dragged,
        Vector2F::zero(),
        vec2f(800.0, 600.0),
        Vector2F::zero(),
        preview,
        false,
        vec2f(120.0, 34.0),
    );
    assert_eq!(drag.source_pane_group_id(), Some(dragged));

    let mut drag = CrossWindowTabDrag::new();
    drag.begin_single_tab_drag(
        source,
        dragged,
        Vector2F::zero(),
        vec2f(800.0, 600.0),
        Vector2F::zero(),
        false,
        vec2f(120.0, 34.0),
    );
    assert_eq!(drag.source_pane_group_id(), Some(dragged));
}

#[test]
fn no_active_drag_keeps_all_slots_full_width() {
    let drag = CrossWindowTabDrag::new();
    assert_eq!(
        drag.collapsed_source_placeholder_index(WindowId::from_usize(1)),
        None
    );
}

#[test]
fn multi_tab_drag_collapses_only_the_source_window_placeholder() {
    let source = WindowId::from_usize(1);
    let preview = WindowId::from_usize(2);
    let other = WindowId::from_usize(3);

    let mut drag = CrossWindowTabDrag::new();
    begin_multi_tab_drag(&mut drag, source, preview);

    // The source window collapses its detached placeholder while the tab is
    // floating in the preview window.
    assert_eq!(
        drag.collapsed_source_placeholder_index(source),
        Some(SOURCE_TAB_INDEX)
    );
    // The preview and unrelated windows never collapse a slot.
    assert_eq!(drag.collapsed_source_placeholder_index(preview), None);
    assert_eq!(drag.collapsed_source_placeholder_index(other), None);
}

#[test]
fn source_reorder_keeps_placeholder_full_width() {
    let source = WindowId::from_usize(1);
    let preview = WindowId::from_usize(2);

    let mut drag = CrossWindowTabDrag::new();
    begin_multi_tab_drag(&mut drag, source, preview);

    // Cursor returns to the source's own tab bar: the placeholder is reordered
    // in place like an in-window drag and must stay full width. Collapsing it
    // here is what produced the horizontal "fuzzy shake".
    drag.set_reordering_in_source_for_test(true);
    assert_eq!(drag.collapsed_source_placeholder_index(source), None);

    // Leaving the source again restores the zero-width collapse.
    drag.set_reordering_in_source_for_test(false);
    assert_eq!(
        drag.collapsed_source_placeholder_index(source),
        Some(SOURCE_TAB_INDEX)
    );
}

#[test]
fn single_tab_drag_never_collapses_a_slot() {
    let source = WindowId::from_usize(1);

    let mut drag = CrossWindowTabDrag::new();
    // A single-tab window is its own floating preview; there is no separate
    // placeholder to collapse.
    drag.begin_single_tab_drag(
        source,
        EntityId::from_usize(42),
        Vector2F::zero(),
        vec2f(800.0, 600.0),
        Vector2F::zero(),
        false,
        vec2f(120.0, 34.0),
    );

    assert_eq!(drag.collapsed_source_placeholder_index(source), None);
}

fn begin_group_drag(
    drag: &mut CrossWindowTabDrag,
    source_window_id: WindowId,
    members: Vec<EntityId>,
    preview_window_id: Option<WindowId>,
    pinned: bool,
) -> TabGroupId {
    let group_id = TabGroupId::new();
    drag.begin_group_drag(
        source_window_id,
        group_id,
        SOURCE_TAB_INDEX,
        members,
        pinned,
        preview_window_id,
        Vector2F::zero(),
        vec2f(800.0, 600.0),
        Vector2F::zero(),
        false,
        vec2f(240.0, 34.0),
    );
    group_id
}

#[test]
fn group_drag_carries_its_members_and_identity() {
    // Source cleanup and snapshot filtering both resolve through these, so
    // they have to survive from drag start to drop.
    let source = WindowId::from_usize(1);
    let preview = WindowId::from_usize(2);
    let members = vec![EntityId::from_usize(11), EntityId::from_usize(12)];

    let mut drag = CrossWindowTabDrag::new();
    assert_eq!(drag.source_group_id(), None);
    assert!(drag.member_pane_group_ids().is_empty());

    let group_id = begin_group_drag(&mut drag, source, members.clone(), Some(preview), false);
    assert_eq!(drag.source_group_id(), Some(group_id));
    assert_eq!(drag.member_pane_group_ids(), members);
}

#[test]
fn whole_window_group_drag_has_no_dedicated_preview() {
    // A group spanning every tab leaves nothing behind, so the source window
    // IS the preview - the same shape as a single-tab drag. Getting this wrong
    // sends the drop down the multi-tab path, which moves one member and
    // destroys the rest.
    let source = WindowId::from_usize(1);
    let members = vec![EntityId::from_usize(11), EntityId::from_usize(12)];

    let mut drag = CrossWindowTabDrag::new();
    begin_group_drag(&mut drag, source, members, None, false);

    assert!(
        !drag.has_dedicated_preview_window(),
        "a whole-window group has no separate preview to close"
    );
    assert!(
        drag.source_is_own_preview(),
        "the source window must be treated as the preview, so it is closed rather than \
         having tabs removed from it"
    );

    // With a dedicated preview both answers invert.
    let mut drag = CrossWindowTabDrag::new();
    begin_group_drag(
        &mut drag,
        source,
        vec![EntityId::from_usize(11)],
        Some(WindowId::from_usize(2)),
        false,
    );
    assert!(drag.has_dedicated_preview_window());
    assert!(!drag.source_is_own_preview());
}

#[test]
fn group_drag_collapses_the_source_placeholder_like_a_tab_drag() {
    let source = WindowId::from_usize(1);
    let preview = WindowId::from_usize(2);
    let other = WindowId::from_usize(3);

    let mut drag = CrossWindowTabDrag::new();
    begin_group_drag(
        &mut drag,
        source,
        vec![EntityId::from_usize(11), EntityId::from_usize(12)],
        Some(preview),
        false,
    );

    assert_eq!(
        drag.collapsed_source_placeholder_index(source),
        Some(SOURCE_TAB_INDEX)
    );
    assert_eq!(drag.collapsed_source_placeholder_index(preview), None);
    assert_eq!(drag.collapsed_source_placeholder_index(other), None);

    // Back over the source's own tab bar the placeholder stays full width, so
    // the drop zone does not vanish under the cursor.
    drag.set_reordering_in_source_for_test(true);
    assert_eq!(drag.collapsed_source_placeholder_index(source), None);
}

#[test]
fn group_placeholder_index_follows_a_reorder_in_source() {
    // set_source_placeholder_index silently no-opped for groups, so a group
    // reordered back in the source dropped at its drag-start position.
    let source = WindowId::from_usize(1);
    let mut drag = CrossWindowTabDrag::new();
    begin_group_drag(
        &mut drag,
        source,
        vec![EntityId::from_usize(11), EntityId::from_usize(12)],
        Some(WindowId::from_usize(2)),
        false,
    );
    assert_eq!(drag.transferred_tab_index(), Some(SOURCE_TAB_INDEX));

    drag.set_source_placeholder_index(0);
    assert_eq!(
        drag.transferred_tab_index(),
        Some(0),
        "the group's placeholder run start must move with the reorder"
    );
}
