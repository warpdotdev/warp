use super::{TabCloseButtonPlacement, tab_close_button_placement};

#[test]
fn active_tab_shows_trailing_close_button_by_default() {
    let placement = tab_close_button_placement(true, false, false);

    assert_eq!(placement, TabCloseButtonPlacement::Trailing);
}

#[test]
fn hovered_tab_shows_trailing_close_button_by_default() {
    let placement = tab_close_button_placement(false, true, false);

    assert_eq!(placement, TabCloseButtonPlacement::Trailing);
}

#[test]
fn hovered_active_tab_shows_trailing_close_button_by_default() {
    let placement = tab_close_button_placement(true, true, false);

    assert_eq!(placement, TabCloseButtonPlacement::Trailing);
}

#[test]
fn inactive_unhovered_tab_reserves_close_button_space_by_default() {
    let placement = tab_close_button_placement(false, false, false);

    assert_eq!(placement, TabCloseButtonPlacement::TrailingPlaceholder);
}

#[test]
fn hovered_tab_shows_close_button_in_icon_slot_when_enabled() {
    let placement = tab_close_button_placement(false, true, true);

    assert_eq!(placement, TabCloseButtonPlacement::IconSlot);
}

#[test]
fn hovered_active_tab_shows_close_button_in_icon_slot_when_enabled() {
    let placement = tab_close_button_placement(true, true, true);

    assert_eq!(placement, TabCloseButtonPlacement::IconSlot);
}

#[test]
fn unhovered_active_tab_omits_close_button_when_enabled() {
    let placement = tab_close_button_placement(true, false, true);

    assert_eq!(placement, TabCloseButtonPlacement::Omitted);
}

#[test]
fn inactive_unhovered_tab_omits_close_button_when_enabled() {
    let placement = tab_close_button_placement(false, false, true);

    assert_eq!(placement, TabCloseButtonPlacement::Omitted);
}
