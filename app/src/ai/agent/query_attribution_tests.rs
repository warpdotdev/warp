use super::*;

fn external_attribution() -> api::UserQueryAttribution {
    api::UserQueryAttribution {
        origin: Some(api::UserQueryOrigin {
            variant: Some(api::user_query_origin::Variant::ExternalPlatform(
                api::user_query_origin::ExternalPlatform {},
            )),
        }),
        author: Some(api::QueryAuthor {
            principal: Some(api::query_author::Principal::User(api::WarpUser {
                uid: "external-author".into(),
                email: "author@example.com".into(),
                team_uid: "author-team".into(),
            })),
            resolution: api::IdentityResolution::ExternalAccountBinding.into(),
        }),
        source_message: Some(api::ExternalMessage {
            body: "new message".into(),
            platform: Some(api::external_message::Platform::Slack(
                api::external_message::Slack {
                    channel_id: "channel".into(),
                    thread_ts: "thread".into(),
                    ..Default::default()
                },
            )),
            ..Default::default()
        }),
    }
}

#[test]
fn envelope_survives_protocol_json_and_protobuf_echo_without_sender_substitution() {
    let expected = external_attribution();
    let encoded = base64::engine::general_purpose::STANDARD.encode(expected.encode_to_vec());
    let request: session_sharing_protocol::common::AgentPromptRequest =
        serde_json::from_value(serde_json::json!({
            "id": "request", "prompt": "formatted prompt",
            "user_query_attribution_b64": encoded,
        }))
        .unwrap();
    let sharer = ProfileData {
        firebase_uid: "execution-owner".into(),
        email: Some("owner@example.com".into()),
        ..Default::default()
    };
    let captured = UserQueryAttribution::from_shared_session(
        request.user_query_attribution_b64.as_deref(),
        Some(&sharer),
    );
    let echoed = captured.request_fields();
    assert_eq!(echoed.origin, expected.origin);
    assert_eq!(echoed.author, expected.author);
    assert_eq!(echoed.source_message, expected.source_message);
}

#[test]
fn legacy_viewer_prompt_uses_requester_identity_without_a_team() {
    let request: session_sharing_protocol::common::AgentPromptRequest =
        serde_json::from_value(serde_json::json!({"id": "request", "prompt": "viewer text"}))
            .unwrap();
    let viewer = ProfileData {
        firebase_uid: "viewer".into(),
        email: Some("viewer@example.com".into()),
        ..Default::default()
    };
    let captured = UserQueryAttribution::from_shared_session(
        request.user_query_attribution_b64.as_deref(),
        Some(&viewer),
    )
    .request_fields();
    assert!(matches!(
        captured.origin.unwrap().variant,
        Some(api::user_query_origin::Variant::WarpClient(_))
    ));
    let author = captured.author.unwrap();
    assert_eq!(
        author.resolution,
        api::IdentityResolution::ClientSession as i32
    );
    assert_eq!(
        author.principal,
        Some(api::query_author::Principal::User(api::WarpUser {
            uid: "viewer".into(),
            email: "viewer@example.com".into(),
            team_uid: String::new(),
        }))
    );
    assert!(captured.source_message.is_none());
}

fn assert_unavailable(attribution: UserQueryAttribution, expected_reason: &str) {
    let fields = attribution.request_fields();
    let Some(api::user_query_origin::Variant::ServerSynthesized(origin)) =
        fields.origin.unwrap().variant
    else {
        panic!("expected explicit unavailable attribution origin");
    };
    assert_eq!(origin.reason, expected_reason);
    assert!(fields.author.is_none());
    assert!(fields.source_message.is_none());
}

#[test]
fn malformed_or_empty_envelope_never_falls_back_to_execution_owner() {
    let owner = ProfileData {
        firebase_uid: "execution-owner".into(),
        ..Default::default()
    };
    for encoded in ["not base64!", "/w==", ""] {
        assert_unavailable(
            UserQueryAttribution::from_shared_session(Some(encoded), Some(&owner)),
            "attribution_unavailable",
        );
    }
}

#[test]
fn missing_viewer_identity_is_explicitly_unavailable() {
    for profile in [None, Some(&ProfileData::default())] {
        assert_unavailable(
            UserQueryAttribution::from_shared_session(None, profile),
            "shared_session_author_unavailable",
        );
    }
}

#[test]
fn historical_messages_without_metadata_remain_unattributed() {
    assert!(UserQueryAttribution::from_message(&api::message::UserQuery::default()).is_none());
}

#[test]
fn unresolved_sender_and_provider_history_are_preserved() {
    let mut expected = external_attribution();
    expected.author = Some(api::QueryAuthor {
        principal: None,
        resolution: api::IdentityResolution::Unresolved.into(),
    });
    let source = expected.source_message.as_mut().unwrap();
    if let Some(api::external_message::Platform::Slack(slack)) = source.platform.as_mut() {
        slack
            .thread_history
            .push(api::external_message::slack::ThreadMessage {
                text: "earlier context".into(),
                ..Default::default()
            });
    }
    let message = api::message::UserQuery {
        origin: expected.origin.clone(),
        author: expected.author.clone(),
        source_message: expected.source_message.clone(),
        ..Default::default()
    };
    let restored = UserQueryAttribution::from_message(&message).unwrap();
    assert_eq!(restored.0.as_ref(), &expected);
}

#[test]
fn debug_output_does_not_include_source_contents_or_identity() {
    let metadata = UserQueryAttribution(Arc::new(external_attribution()));
    assert_eq!(format!("{metadata:?}"), "UserQueryAttribution { .. }");
}

#[test]
fn fresh_local_query_marks_origin_without_claiming_a_principal() {
    let captured = UserQueryAttribution::fresh_local().request_fields();
    assert!(matches!(
        captured.origin.unwrap().variant,
        Some(api::user_query_origin::Variant::WarpClient(_))
    ));
    assert!(captured.author.is_none());
    assert!(captured.source_message.is_none());
}

#[test]
fn canned_query_protobuf_round_trip_retains_the_complete_envelope() {
    let expected = external_attribution();
    let attribution = UserQueryAttribution(Arc::new(expected.clone()));
    let query = api::request::input::QueryWithCannedResponse {
        query: "canned query".into(),
        attribution: Some(attribution.envelope()),
        ..Default::default()
    };
    let decoded =
        api::request::input::QueryWithCannedResponse::decode(query.encode_to_vec().as_slice())
            .unwrap();
    assert_eq!(decoded.attribution, Some(expected));
    let legacy = api::request::input::QueryWithCannedResponse::decode(&[][..]).unwrap();
    assert!(legacy.attribution.is_none());
}
