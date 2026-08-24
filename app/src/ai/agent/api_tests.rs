use warp_core::channel::{Channel, ChannelState};

use super::{RequestParams, ServerConversationToken};
use crate::ai::agent::ServerOutputId;

#[test]
fn debugging_payload_is_link_on_dogfood_channels() {
    let token = ServerConversationToken::new("conversation-token".to_owned());
    let request_id = ServerOutputId::new("request-id".to_owned());
    let expected_link = format!(
        "{}/debug/maa/conversation-token",
        ChannelState::server_root_url()
    );

    for channel in [Channel::Dev, Channel::Local] {
        assert_eq!(
            token.debugging_payload_for_channel(None, channel),
            expected_link
        );
        assert_eq!(
            token.debugging_payload_for_channel(Some(&request_id), channel),
            format!("{expected_link}?request=request-id")
        );
    }
}

#[test]
fn debugging_payload_is_id_on_non_dogfood_channels() {
    let token = ServerConversationToken::new("conversation-token".to_owned());
    let request_id = ServerOutputId::new("request-id".to_owned());

    for channel in [
        Channel::Stable,
        Channel::Preview,
        Channel::Integration,
        Channel::Oss,
    ] {
        assert_eq!(
            token.debugging_payload_for_channel(None, channel),
            "{\"conversation_id\":\"conversation-token\"}"
        );
        assert_eq!(
            token.debugging_payload_for_channel(Some(&request_id), channel),
            "{\"request_id\":\"request-id\",\"conversation_id\":\"conversation-token\"}"
        );
    }
}

/// [`RequestParams::new`] resolves `member_byo_credentials_allowed` from a real scope; this
/// test-only constructor has none to give, so it has to default to the safe answer for a
/// credential gate: not permitted. The scope-driven behaviour itself --
/// `UserWorkspaces::are_member_byo_keys_allowed`/`are_member_byo_endpoints_allowed` gating
/// `RequestParams::api_keys`/`custom_model_providers` by the requesting window's team, with
/// AWS Bedrock/GEAP credentials surviving regardless -- is covered where that policy lives: the
/// `member_byo_policy_*` tests in `workspaces::user_workspaces::user_workspaces_tests`.
#[test]
fn request_params_do_not_allow_member_credentials_until_the_policy_has_been_applied() {
    assert!(!RequestParams::new_for_test().member_byo_credentials_allowed);
}
