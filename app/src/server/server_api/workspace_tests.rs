use futures::executor::block_on;

use super::*;
use crate::ChannelState;

/// A current-window overage refresh must send the team's raw UID in `X-Warp-Team-Uid` so the
/// server returns that team's overages instead of an arbitrary one.
#[test]
fn refresh_ai_overages_sends_the_team_header_when_a_team_is_given() {
    let team_uid = ServerId::from_string_lossy(format!("{:0>22}", "team-uid-1234567890"));
    let team_uid_header_value = team_uid.to_string();

    let mock = {
        let mut server = ChannelState::mock_server();
        server
            .mock("POST", "/graphql/v2")
            .match_header(
                warp_server_client::base_client::TEAM_UID_HEADER,
                team_uid_header_value.as_str(),
            )
            .with_status(200)
            .with_body(r#"{"data":null,"errors":[{"message":"stub response"}]}"#)
            .create()
    };

    let server_api = ServerApi::new_for_test();
    let _ = block_on(WorkspaceClient::refresh_ai_overages(
        &server_api,
        Some(team_uid),
    ));

    mock.assert();
}

/// The legacy, unscoped refresh (no window to infer a team from) must not send a team header.
#[test]
fn refresh_ai_overages_sends_no_team_header_without_a_team() {
    let mock = {
        let mut server = ChannelState::mock_server();
        server
            .mock("POST", "/graphql/v2")
            .match_header(
                warp_server_client::base_client::TEAM_UID_HEADER,
                mockito::Matcher::Missing,
            )
            .with_status(200)
            .with_body(r#"{"data":null,"errors":[{"message":"stub response"}]}"#)
            .create()
    };

    let server_api = ServerApi::new_for_test();
    let _ = block_on(WorkspaceClient::refresh_ai_overages(&server_api, None));

    mock.assert();
}
