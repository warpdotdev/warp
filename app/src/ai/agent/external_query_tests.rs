use base64::Engine as _;
use prost::Message as _;
use warp_multi_agent_api::external_message::{GitHub, Platform, Slack};
use warp_multi_agent_api::request::input::user_inputs::ExternalQueryToken;
use warp_multi_agent_api::{ExternalMessage, ExternalQuery, ExternalUser};

use super::{container_label, decode_external_query_token, platform_name, sender_display_name};

fn slack_message(channel_name: &str, channel_id: &str) -> ExternalMessage {
    ExternalMessage {
        platform: Some(Platform::Slack(Slack {
            channel_name: channel_name.to_owned(),
            channel_id: channel_id.to_owned(),
            ..Default::default()
        })),
        ..Default::default()
    }
}

#[test]
fn container_label_prefers_slack_channel_name_over_id() {
    assert_eq!(
        container_label(&slack_message("eng", "C123")).as_deref(),
        Some("#eng")
    );
    assert_eq!(
        container_label(&slack_message("", "C123")).as_deref(),
        Some("C123")
    );
    assert_eq!(container_label(&slack_message("", "")), None);
}

#[test]
fn container_label_formats_github_issue_reference() {
    let message = ExternalMessage {
        platform: Some(Platform::Github(GitHub {
            owner: "warpdotdev".to_owned(),
            repo: "warp".to_owned(),
            number: 42,
            is_pull_request: true,
        })),
        ..Default::default()
    };
    assert_eq!(
        container_label(&message).as_deref(),
        Some("warpdotdev/warp#42")
    );
    assert_eq!(platform_name(&message), "GitHub");
}

#[test]
fn sender_display_name_falls_back_through_handle_id_and_platform() {
    let mut message = slack_message("eng", "C123");
    assert_eq!(sender_display_name(&message), "Slack");

    message.sender = Some(ExternalUser {
        id: "U123".to_owned(),
        ..Default::default()
    });
    assert_eq!(sender_display_name(&message), "U123");

    message.sender.as_mut().unwrap().handle = "jane".to_owned();
    assert_eq!(sender_display_name(&message), "jane");

    message.sender.as_mut().unwrap().display_name = "Jane Doe".to_owned();
    assert_eq!(sender_display_name(&message), "Jane Doe");
}

fn encode_token(token: &ExternalQueryToken) -> String {
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(token.encode_to_vec());
    format!("{payload}.unchecked-signature")
}

#[test]
fn decode_external_query_token_returns_payload_query_without_checking_signature() {
    let token = ExternalQueryToken {
        query: Some(ExternalQuery {
            message: Some(ExternalMessage {
                body: "hello from slack".to_owned(),
                ..Default::default()
            }),
            ..Default::default()
        }),
        run_id: "run-1".to_owned(),
        ..Default::default()
    };

    let decoded = decode_external_query_token(&encode_token(&token)).expect("decodes");
    assert_eq!(decoded, token.query.unwrap());
}

#[test]
fn decode_external_query_token_rejects_malformed_input() {
    assert!(decode_external_query_token("").is_err());
    assert!(decode_external_query_token("not base64!.sig").is_err());
    let garbage = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b"\xff\xfe\x00");
    assert!(decode_external_query_token(&garbage).is_err());

    let query_less = ExternalQueryToken {
        run_id: "run-1".to_owned(),
        ..Default::default()
    };
    assert!(decode_external_query_token(&encode_token(&query_less)).is_err());
}
