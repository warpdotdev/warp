use super::*;

#[test]
fn base_layout_key_maps_character_keys() {
    assert_eq!(KeyCode::KeyV.base_layout_key(), Some('v'));
    assert_eq!(KeyCode::KeyZ.base_layout_key(), Some('z'));
    assert_eq!(KeyCode::Digit7.base_layout_key(), Some('7'));
    assert_eq!(KeyCode::Slash.base_layout_key(), Some('/'));
    assert_eq!(KeyCode::Backslash.base_layout_key(), Some('\\'));
}

#[test]
fn base_layout_key_is_none_for_keys_without_a_character() {
    assert_eq!(KeyCode::Enter.base_layout_key(), None);
    assert_eq!(KeyCode::ControlLeft.base_layout_key(), None);
    assert_eq!(KeyCode::Backspace.base_layout_key(), None);
}
