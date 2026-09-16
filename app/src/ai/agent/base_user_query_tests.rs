use base64::Engine as _;
use prost::Message as _;
use warp_multi_agent_api as api;
use warp_multi_agent_api::AgentType;

use super::BaseUserQuery;
use crate::ai::agent::UserQueryMode;

fn encode(query: &api::request::input::UserQuery) -> String {
    base64::engine::general_purpose::STANDARD.encode(query.encode_to_vec())
}

fn base(query: api::request::input::UserQuery) -> BaseUserQuery {
    BaseUserQuery::from_proto(query)
}

fn plan_mode() -> api::UserQueryMode {
    api::UserQueryMode {
        r#type: Some(api::user_query_mode::Type::Plan(())),
    }
}

fn normal_mode() -> api::UserQueryMode {
    api::UserQueryMode { r#type: None }
}

#[test]
fn decodes_a_serialized_request_user_query() {
    let query = api::request::input::UserQuery {
        query: "take a look at the failing test".to_string(),
        intended_agent: AgentType::Cli.into(),
        origin: Some(api::UserQueryOrigin::default()),
        ..Default::default()
    };

    let decoded = BaseUserQuery::decode_b64(&encode(&query)).expect("valid payload decodes");

    assert_eq!(decoded.to_proto(), query);
}

#[test]
fn rejects_a_payload_that_is_not_base64() {
    assert!(BaseUserQuery::decode_b64("not-base64!").is_none());
}

#[test]
fn rejects_a_payload_that_is_not_a_user_query() {
    // Field 1, length-delimited, with a truncated length varint.
    let truncated = base64::engine::general_purpose::STANDARD.encode([0x0a, 0xff]);
    assert!(BaseUserQuery::decode_b64(&truncated).is_none());
}

#[test]
fn debug_output_reports_shape_without_content() {
    let query = api::request::input::UserQuery {
        query: "the secret launch codes are 1234".to_string(),
        origin: Some(api::UserQueryOrigin::default()),
        ..Default::default()
    };

    let debug = format!("{:?}", BaseUserQuery::from_proto(query));

    assert!(!debug.contains("secret"), "{debug}");
    assert!(debug.contains("query_len: 32"), "{debug}");
    assert!(debug.contains("has_origin: true"), "{debug}");
    assert!(debug.contains("has_author: false"), "{debug}");
}

#[test]
fn accessors_report_unset_fields_as_none() {
    let unset = base(Default::default());
    assert_eq!(unset.query(), None);
    assert_eq!(unset.user_query_mode(), None);
    assert_eq!(unset.intended_agent(), None);

    let set = base(api::request::input::UserQuery {
        query: "look at the failing test".to_string(),
        mode: Some(plan_mode()),
        intended_agent: AgentType::Cli.into(),
        ..Default::default()
    });
    assert_eq!(set.query(), Some("look at the failing test"));
    assert_eq!(set.user_query_mode(), Some(UserQueryMode::Plan));
    assert_eq!(set.intended_agent(), Some(AgentType::Cli));

    // An explicitly Normal mode is a classification, not an unset field.
    let normal = base(api::request::input::UserQuery {
        mode: Some(normal_mode()),
        ..Default::default()
    });
    assert_eq!(normal.user_query_mode(), Some(UserQueryMode::Normal));
}

#[test]
fn seed_uses_the_server_text_and_normalizes_a_prefix_the_server_did_not_classify() {
    let seeded = base(api::request::input::UserQuery {
        query: "/plan from the server".to_string(),
        intended_agent: AgentType::Cli.into(),
        ..Default::default()
    })
    .seed_input_fields(
        "from the prompt".to_string(),
        UserQueryMode::Normal,
        Some(AgentType::Primary),
    );

    assert_eq!(
        seeded,
        (
            "from the server".to_string(),
            UserQueryMode::Plan,
            Some(AgentType::Cli)
        )
    );
}

#[test]
fn seed_falls_back_to_the_client_for_fields_the_server_left_unset() {
    let seeded = base(Default::default()).seed_input_fields(
        "from the prompt".to_string(),
        UserQueryMode::Plan,
        Some(AgentType::Primary),
    );

    assert_eq!(
        seeded,
        (
            "from the prompt".to_string(),
            UserQueryMode::Plan,
            Some(AgentType::Primary)
        )
    );
}

#[test]
fn seed_prefers_the_server_mode_when_it_sent_no_text() {
    let seeded = base(api::request::input::UserQuery {
        mode: Some(plan_mode()),
        ..Default::default()
    })
    .seed_input_fields("from the prompt".to_string(), UserQueryMode::Normal, None);

    assert_eq!(
        seeded,
        ("from the prompt".to_string(), UserQueryMode::Plan, None)
    );
}

#[test]
fn seed_strips_a_prefix_that_agrees_with_the_server_mode() {
    let seeded = base(api::request::input::UserQuery {
        query: "/plan foo".to_string(),
        mode: Some(plan_mode()),
        ..Default::default()
    })
    .seed_input_fields(String::new(), UserQueryMode::Normal, None);

    assert_eq!(seeded, ("foo".to_string(), UserQueryMode::Plan, None));
}

#[test]
fn seed_keeps_the_text_verbatim_when_the_server_classified_the_mode_differently() {
    let seeded = base(api::request::input::UserQuery {
        query: "/plan foo".to_string(),
        mode: Some(normal_mode()),
        ..Default::default()
    })
    .seed_input_fields(String::new(), UserQueryMode::Plan, None);

    assert_eq!(
        seeded,
        ("/plan foo".to_string(), UserQueryMode::Normal, None)
    );
}
