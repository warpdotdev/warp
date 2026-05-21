use warp::integration_testing::{
    self,
    assertions::{
        assert_websocket_has_not_started, assert_websocket_has_started, create_a_personal_workflow,
        join_a_workspace,
    },
    terminal::wait_until_bootstrapped_single_pane_for_tab,
};

use crate::Builder;

use super::{new_builder, TEST_ONLY_ASSETS};

/// With no objects and no teams, the websocket should not begin
pub fn test_websocket_does_not_begin_on_startup() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(assert_websocket_has_not_started())
}

/// With objects read from sqlite (i.e., objects that are not welcome objects), the websocket should begin
pub fn test_websocket_begins_on_startup() -> Builder {
    new_builder()
        .with_setup(|_utils| {
            integration_testing::create_file_from_assets(
                TEST_ONLY_ASSETS,
                "cloud_objects.sqlite",
                &integration_testing::persistence::database_file_path_for_scope(
                    &integration_testing::persistence::PersistenceScope::App,
                ),
            );
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(assert_websocket_has_started())
}

/// The websocket should start only after joining a team
pub fn test_websocket_begins_after_joining_a_team() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(assert_websocket_has_not_started())
        .with_step(join_a_workspace())
        .with_step(assert_websocket_has_started())
}

/// The websocket should start only after an object is created
pub fn test_websocket_begins_after_creating_an_object() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(assert_websocket_has_not_started())
        .with_step(create_a_personal_workflow())
        .with_step(assert_websocket_has_started())
}
