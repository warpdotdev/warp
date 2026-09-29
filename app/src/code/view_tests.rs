use super::{TabCloseButtonPlacement, tab_close_button_placement};

#[test]
fn close_button_trails_active_or_hovered_tabs_by_default() {
    for (is_active, is_hovered) in [(true, false), (false, true), (true, true)] {
        assert_eq!(
            tab_close_button_placement(is_active, is_hovered, false),
            TabCloseButtonPlacement::Trailing,
            "is_active={is_active}, is_hovered={is_hovered}",
        );
    }
}

#[test]
fn inactive_unhovered_tab_reserves_close_button_space_by_default() {
    assert_eq!(
        tab_close_button_placement(false, false, false),
        TabCloseButtonPlacement::TrailingPlaceholder,
    );
}

#[test]
fn close_button_replaces_icon_on_hover_when_enabled() {
    for is_active in [false, true] {
        assert_eq!(
            tab_close_button_placement(is_active, true, true),
            TabCloseButtonPlacement::IconSlot,
            "is_active={is_active}",
        );
    }
}

#[test]
fn close_button_is_omitted_without_hover_when_enabled() {
    for is_active in [false, true] {
        assert_eq!(
            tab_close_button_placement(is_active, false, true),
            TabCloseButtonPlacement::Omitted,
            "is_active={is_active}",
        );
    }
}
