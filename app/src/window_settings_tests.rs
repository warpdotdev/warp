use super::ZoomLevel;

#[test]
fn parse_zoom_level_percentage() {
    for (input, expected) in [
        ("50", Some(50)),
        ("100", Some(100)),
        ("115", Some(115)),
        ("142", Some(142)),
        ("150", Some(150)),
        ("175", Some(175)),
        ("300", Some(300)),
        ("350", Some(350)),
        ("50%", Some(50)),
        ("175%", Some(175)),
        (" 100 ", Some(100)),
        (" 100% ", Some(100)),
        ("", None),
        ("abc", None),
        ("12.5", None),
        ("-1", None),
        ("0", None),
        ("49", None),
        ("351", None),
        ("400", None),
        ("50%%", None),
        ("5 0", None),
        ("50%0", None),
    ] {
        assert_eq!(
            ZoomLevel::parse_percentage(input),
            expected,
            "unexpected parse result for {input:?}"
        );
    }
}

#[test]
fn steps_to_neighboring_presets_from_custom_zoom() {
    assert_eq!(ZoomLevel::next_preset(137, true), 150);
    assert_eq!(ZoomLevel::next_preset(137, false), 125);
}

#[test]
fn preserves_preset_zoom_stepping_and_bounds() {
    assert_eq!(ZoomLevel::next_preset(100, true), 110);
    assert_eq!(ZoomLevel::next_preset(100, false), 90);
    assert_eq!(ZoomLevel::next_preset(350, true), 350);
    assert_eq!(ZoomLevel::next_preset(50, false), 50);
}
