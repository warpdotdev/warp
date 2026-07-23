use super::runner_controls_enabled_for_state;

#[test]
fn runner_controls_require_both_feature_flag_and_experiment_arm() {
    assert!(!runner_controls_enabled_for_state(false, false));
    assert!(!runner_controls_enabled_for_state(false, true));
    assert!(!runner_controls_enabled_for_state(true, false));
    assert!(runner_controls_enabled_for_state(true, true));
}
