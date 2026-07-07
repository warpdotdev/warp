use super::*;

/// Locks in [`Harness::config_name`] / [`Harness::from_config_name`] as a true inverse pair
/// for every variant that maps to a real, server-recognized harness. If a new variant is
/// added without a matching `from_config_name` arm, this round-trip test will fail.
#[test]
fn harness_config_name_round_trips_for_known_variants() {
    for harness in [
        Harness::Oz,
        Harness::Claude,
        Harness::OpenCode,
        Harness::Gemini,
        Harness::Codex,
    ] {
        assert_eq!(
            Harness::from_config_name(harness.config_name()),
            Some(harness),
            "round-trip failed for {harness:?}",
        );
    }
}

#[test]
fn harness_from_config_name_returns_none_for_unrecognized() {
    assert_eq!(Harness::from_config_name(""), None);
    assert_eq!(Harness::from_config_name("not-a-real-harness"), None);
}

#[test]
fn harness_from_config_name_round_trips_unknown() {
    assert_eq!(
        Harness::from_config_name(Harness::Unknown.config_name()),
        Some(Harness::Unknown),
    );
}
