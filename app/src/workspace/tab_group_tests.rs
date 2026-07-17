use std::collections::HashMap;

use warp_core::ui::theme::AnsiColorIdentifier;

use super::{next_group_color, next_unused_group_color, TabGroup, TabGroupId};
use crate::tab::SelectedTabColor;

/// Helper: a fresh group whose color is set to `color`.
fn group_with_color(color: SelectedTabColor) -> TabGroup {
    let mut group = TabGroup::new();
    group.color = color;
    group
}

#[test]
fn test_next_unused_group_color_empty_returns_first_palette_color() {
    assert_eq!(
        next_unused_group_color(&[]),
        SelectedTabColor::Color(AnsiColorIdentifier::Red),
    );
}

#[test]
fn test_next_unused_group_color_returns_first_unused_in_palette_order() {
    // Red is used -> Green is next.
    assert_eq!(
        next_unused_group_color(&[SelectedTabColor::Color(AnsiColorIdentifier::Red)]),
        SelectedTabColor::Color(AnsiColorIdentifier::Green),
    );
    // Red, Green, Yellow used -> Blue is next.
    assert_eq!(
        next_unused_group_color(&[
            SelectedTabColor::Color(AnsiColorIdentifier::Red),
            SelectedTabColor::Color(AnsiColorIdentifier::Green),
            SelectedTabColor::Color(AnsiColorIdentifier::Yellow),
        ]),
        SelectedTabColor::Color(AnsiColorIdentifier::Blue),
    );
}

#[test]
fn test_next_unused_group_color_prefers_earlier_palette_color_even_if_later_one_used() {
    // Only Green is used; Red is still the first unused palette color.
    assert_eq!(
        next_unused_group_color(&[SelectedTabColor::Color(AnsiColorIdentifier::Green)]),
        SelectedTabColor::Color(AnsiColorIdentifier::Red),
    );
    // Only Cyan is used; Red is still the first unused palette color.
    assert_eq!(
        next_unused_group_color(&[SelectedTabColor::Color(AnsiColorIdentifier::Cyan)]),
        SelectedTabColor::Color(AnsiColorIdentifier::Red),
    );
}

#[test]
fn test_next_unused_group_color_ignores_unset_and_cleared() {
    // Unset and Cleared groups don't claim a palette color, so the first
    // palette color (Red) is still available.
    assert_eq!(
        next_unused_group_color(&[SelectedTabColor::Unset, SelectedTabColor::Cleared]),
        SelectedTabColor::Color(AnsiColorIdentifier::Red),
    );
    // Mixed: an Unset group alongside a Red one still leaves Green next.
    assert_eq!(
        next_unused_group_color(&[
            SelectedTabColor::Unset,
            SelectedTabColor::Color(AnsiColorIdentifier::Red),
        ]),
        SelectedTabColor::Color(AnsiColorIdentifier::Green),
    );
}

#[test]
fn test_next_unused_group_color_cycles_when_all_palette_colors_used() {
    let all = [
        SelectedTabColor::Color(AnsiColorIdentifier::Red),
        SelectedTabColor::Color(AnsiColorIdentifier::Green),
        SelectedTabColor::Color(AnsiColorIdentifier::Yellow),
        SelectedTabColor::Color(AnsiColorIdentifier::Blue),
        SelectedTabColor::Color(AnsiColorIdentifier::Magenta),
        SelectedTabColor::Color(AnsiColorIdentifier::Cyan),
    ];
    // Every palette color is used -> cycle back to the first (Red).
    assert_eq!(
        next_unused_group_color(&all),
        SelectedTabColor::Color(AnsiColorIdentifier::Red),
    );
}

#[test]
fn test_next_group_color_empty_map_returns_first_palette_color() {
    let existing: HashMap<TabGroupId, TabGroup> = HashMap::new();
    assert_eq!(
        next_group_color(&existing),
        SelectedTabColor::Color(AnsiColorIdentifier::Red),
    );
}

#[test]
fn test_next_group_color_skips_colors_already_used_by_existing_groups() {
    let mut existing: HashMap<TabGroupId, TabGroup> = HashMap::new();
    existing.insert(
        TabGroupId::new(),
        group_with_color(SelectedTabColor::Color(AnsiColorIdentifier::Red)),
    );
    existing.insert(
        TabGroupId::new(),
        group_with_color(SelectedTabColor::Color(AnsiColorIdentifier::Blue)),
    );
    // Red and Blue are used; Green is the first unused palette color.
    assert_eq!(
        next_group_color(&existing),
        SelectedTabColor::Color(AnsiColorIdentifier::Green),
    );
}

#[test]
fn test_next_group_color_ignores_unset_groups() {
    let mut existing: HashMap<TabGroupId, TabGroup> = HashMap::new();
    existing.insert(TabGroupId::new(), group_with_color(SelectedTabColor::Unset));
    // An unset group doesn't claim a color, so Red is still available.
    assert_eq!(
        next_group_color(&existing),
        SelectedTabColor::Color(AnsiColorIdentifier::Red),
    );
}

#[test]
fn test_next_group_color_cycles_when_all_palette_colors_used() {
    let mut existing: HashMap<TabGroupId, TabGroup> = HashMap::new();
    for color in [
        AnsiColorIdentifier::Red,
        AnsiColorIdentifier::Green,
        AnsiColorIdentifier::Yellow,
        AnsiColorIdentifier::Blue,
        AnsiColorIdentifier::Magenta,
        AnsiColorIdentifier::Cyan,
    ] {
        existing.insert(
            TabGroupId::new(),
            group_with_color(SelectedTabColor::Color(color)),
        );
    }
    // All six palette colors are claimed -> cycle back to Red.
    assert_eq!(
        next_group_color(&existing),
        SelectedTabColor::Color(AnsiColorIdentifier::Red),
    );
}
