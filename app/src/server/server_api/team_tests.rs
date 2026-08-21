use futures::executor::block_on;

use super::*;
use crate::ChannelState;

/// `transfer_team_ownership` explicitly targets a team, so it must send that team's raw UID
/// in `X-Warp-Team-Uid` rather than inferring scope from a window or view.
#[test]
fn transfer_team_ownership_sends_the_explicit_target_team_uid_header() {
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
    let _ = block_on(TeamClient::transfer_team_ownership(
        &server_api,
        team_uid,
        "new-owner@example.com".to_string(),
    ));

    mock.assert();
}
