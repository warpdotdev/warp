use super::resolve_playback_speed_multiplier;

const DEFAULT: f32 = 1.5;

#[test]
fn absent_falls_back_to_client_default() {
    // The server never specified a value at all (e.g. an old server
    // build); the client falls back to its own default.
    assert_eq!(resolve_playback_speed_multiplier(None, DEFAULT), DEFAULT);
}

#[test]
fn explicit_zero_is_honored_as_real_time_not_the_default() {
    // Regression test: an explicit request for real-time must not be
    // silently converted back to the client's default speed.
    assert_eq!(resolve_playback_speed_multiplier(Some(0.0), DEFAULT), 1.0);
}

#[test]
fn explicit_one_is_honored_as_real_time_not_the_default() {
    assert_eq!(resolve_playback_speed_multiplier(Some(1.0), DEFAULT), 1.0);
}

#[test]
fn explicit_value_above_one_is_used_as_is() {
    assert_eq!(resolve_playback_speed_multiplier(Some(2.0), DEFAULT), 2.0);
}

#[test]
fn explicit_non_finite_value_degrades_to_real_time() {
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert_eq!(
            resolve_playback_speed_multiplier(Some(value), DEFAULT),
            1.0,
            "non-finite value {value} should degrade to real-time, not the default"
        );
    }
}

#[test]
fn explicit_absurd_value_is_clamped() {
    assert_eq!(
        resolve_playback_speed_multiplier(Some(f32::MAX), DEFAULT),
        computer_use::MAX_PLAYBACK_SPEED_MULTIPLIER
    );
}
