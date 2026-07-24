//! Unit tests for [`CrossWindowTabDrag`] placeholder-collapse policy.
//!
//! These focus on [`CrossWindowTabDrag::collapsed_source_placeholder_index`],
//! which decides whether the source window's horizontal tab bar collapses the
//! detached-placeholder slot to zero width. The regression these guard against
//! is the horizontal "fuzzy shake": collapsing the placeholder while the cursor
//! is reordering it back in the source window removed the visible drop zone and
//! made the slot oscillate every frame.

use warpui::WindowId;
use warpui::geometry::vector::{Vector2F, vec2f};

use super::CrossWindowTabDrag;

const SOURCE_TAB_INDEX: usize = 2;

fn begin_multi_tab_drag(
    drag: &mut CrossWindowTabDrag,
    source_window_id: WindowId,
    preview_window_id: WindowId,
) {
    drag.begin_multi_tab_drag(
        source_window_id,
        SOURCE_TAB_INDEX,
        Vector2F::zero(),
        vec2f(800.0, 600.0),
        Vector2F::zero(),
        preview_window_id,
        false,
        vec2f(120.0, 34.0),
    );
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
        Vector2F::zero(),
        vec2f(800.0, 600.0),
        Vector2F::zero(),
        false,
        vec2f(120.0, 34.0),
    );

    assert_eq!(drag.collapsed_source_placeholder_index(source), None);
}
