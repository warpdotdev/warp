use super::WindowBackdrop;

#[test]
fn default_value_is_none() {
    assert_eq!(WindowBackdrop::default(), WindowBackdrop::None);
}

#[test]
fn legacy_enabled_value_maps_to_acrylic() {
    assert_eq!(
        serde_json::from_str::<WindowBackdrop>("true").unwrap(),
        WindowBackdrop::Acrylic
    );
}

#[test]
fn legacy_disabled_value_maps_to_none() {
    assert_eq!(
        serde_json::from_str::<WindowBackdrop>("false").unwrap(),
        WindowBackdrop::None
    );
}

#[test]
fn named_values_round_trip() {
    let expected = ["none", "auto", "mica", "acrylic", "mica_alt"];

    for (backdrop, expected_name) in WindowBackdrop::ALL.into_iter().zip(expected) {
        let serialized = serde_json::to_string(&backdrop).unwrap();
        assert_eq!(serialized, format!("\"{expected_name}\""));
        assert_eq!(
            serde_json::from_str::<WindowBackdrop>(&serialized).unwrap(),
            backdrop
        );
    }
}
