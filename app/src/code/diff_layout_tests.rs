use warp_core::features::FeatureFlag;

use super::DiffLayout;

#[test]
fn serializes_as_snake_case() {
    assert_eq!(
        serde_json::to_string(&DiffLayout::Inline).unwrap(),
        "\"inline\""
    );
    assert_eq!(
        serde_json::to_string(&DiffLayout::SideBySide).unwrap(),
        "\"side_by_side\""
    );
    assert_eq!(
        serde_json::from_str::<DiffLayout>("\"side_by_side\"").unwrap(),
        DiffLayout::SideBySide
    );
    assert_eq!(
        serde_json::from_str::<DiffLayout>("\"inline\"").unwrap(),
        DiffLayout::Inline
    );
}

#[test]
fn defaults_to_inline() {
    assert_eq!(DiffLayout::default(), DiffLayout::Inline);
}

#[test]
fn effective_layout_is_inline_when_flag_disabled() {
    let _guard = FeatureFlag::SideBySideDiffLayout.override_enabled(false);
    assert_eq!(
        DiffLayout::effective(DiffLayout::SideBySide),
        DiffLayout::Inline
    );
}

#[test]
fn effective_layout_follows_stored_value_when_flag_enabled() {
    let _guard = FeatureFlag::SideBySideDiffLayout.override_enabled(true);
    assert_eq!(
        DiffLayout::effective(DiffLayout::SideBySide),
        DiffLayout::SideBySide
    );
    assert_eq!(
        DiffLayout::effective(DiffLayout::Inline),
        DiffLayout::Inline
    );
}

#[test]
fn toggled_flips_both_ways() {
    assert_eq!(DiffLayout::Inline.toggled(), DiffLayout::SideBySide);
    assert_eq!(DiffLayout::SideBySide.toggled(), DiffLayout::Inline);
}
