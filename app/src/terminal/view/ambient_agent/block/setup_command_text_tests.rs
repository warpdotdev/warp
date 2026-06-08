use super::SetupCommandState;

#[test]
fn setup_command_groups_have_independent_visibility() {
    let mut state = SetupCommandState::default();
    let first_group_id = state.current_group_id();

    state.set_should_expand(first_group_id, false);
    state.set_did_execute_a_setup_command(true);

    let second_group_id = state.start_new_group();

    assert!(!state.should_expand(first_group_id));
    assert!(state.should_expand(second_group_id));
    assert!(!state.did_execute_a_setup_command());
}

#[test]
fn setup_command_groups_track_running_group_independently() {
    let mut state = SetupCommandState::default();
    let first_group_id = state.current_group_id();
    assert!(!state.should_suppress_input_sync_for_current_group());
    let second_group_id = state.start_new_group();

    assert!(!state.is_running(first_group_id));
    assert!(state.is_running(second_group_id));
    assert!(state.should_suppress_input_sync_for_current_group());

    state.finish_group(first_group_id);

    assert!(state.is_running(second_group_id));
    assert!(state.should_suppress_input_sync_for_current_group());

    state.finish_group(second_group_id);

    assert!(!state.is_running(second_group_id));
    assert!(!state.should_suppress_input_sync_for_current_group());
}
