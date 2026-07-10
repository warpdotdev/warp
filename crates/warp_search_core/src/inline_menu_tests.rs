use super::{InitialSelection, InlineMenuSelection};

#[test]
fn reset_selects_first_or_last_enabled_item() {
    let enabled = [false, true, false, true];
    let mut selection = InlineMenuSelection::default();

    assert_eq!(
        selection.reset(enabled.len(), InitialSelection::First, |index| enabled
            [index]),
        Some(1)
    );
    assert_eq!(
        selection.reset(enabled.len(), InitialSelection::Last, |index| enabled
            [index]),
        Some(3)
    );
}

#[test]
fn reset_clears_selection_when_no_item_is_enabled() {
    let mut selection = InlineMenuSelection::default();
    selection.reset(1, InitialSelection::First, |_| true);

    assert_eq!(selection.reset(3, InitialSelection::Last, |_| false), None);
    assert_eq!(selection.selected_index(), None);
}

#[test]
fn next_and_previous_wrap_and_skip_disabled_items() {
    let enabled = [true, false, true, false];
    let mut selection = InlineMenuSelection::default();
    selection.reset(enabled.len(), InitialSelection::First, |index| {
        enabled[index]
    });

    assert_eq!(
        selection.select_next(enabled.len(), |index| enabled[index]),
        Some(2)
    );
    assert_eq!(
        selection.select_next(enabled.len(), |index| enabled[index]),
        Some(0)
    );
    assert_eq!(
        selection.select_previous(enabled.len(), |index| enabled[index]),
        Some(2)
    );
}

#[test]
fn navigation_from_no_selection_uses_directional_edge() {
    let enabled = [true, false, true];
    let mut selection = InlineMenuSelection::default();

    assert_eq!(
        selection.select_next(enabled.len(), |index| enabled[index]),
        Some(0)
    );
    selection.clear();
    assert_eq!(
        selection.select_previous(enabled.len(), |index| enabled[index]),
        Some(2)
    );
}

#[test]
fn direct_selection_rejects_invalid_or_disabled_indices_without_moving() {
    let enabled = [true, false];
    let mut selection = InlineMenuSelection::default();
    selection.reset(enabled.len(), InitialSelection::First, |index| {
        enabled[index]
    });

    assert_eq!(
        selection.select(1, enabled.len(), |index| enabled[index]),
        None
    );
    assert_eq!(selection.selected_index(), Some(0));
    assert_eq!(
        selection.select(2, enabled.len(), |index| enabled[index]),
        None
    );
    assert_eq!(selection.selected_index(), Some(0));
}

#[test]
fn empty_navigation_clears_stale_selection() {
    let mut selection = InlineMenuSelection::default();
    selection.reset(1, InitialSelection::First, |_| true);

    assert_eq!(selection.select_next(0, |_| true), None);
    assert_eq!(selection.selected_index(), None);
}
