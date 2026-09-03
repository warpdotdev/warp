use std::collections::HashMap;
use std::sync::Arc;

use regex::Regex;
use serial_test::serial;
use warp_multi_agent_api as api;

use super::redact_inputs;
use crate::ai::agent::{AIAgentInput, UserQueryMode};
use crate::terminal::model::secrets;

const TOKEN: &str = "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";

fn external_query_input() -> AIAgentInput {
    AIAgentInput::ExternalQuery {
        query: Box::new(api::ExternalQuery {
            message: Some(api::ExternalMessage {
                sender: Some(api::ExternalUser {
                    display_name: format!("Jane {TOKEN}"),
                    handle: format!("jane-{TOKEN}"),
                    ..Default::default()
                }),
                body: format!("my token is {TOKEN} please rotate it"),
                permalink: format!("https://slack.example/p1?t={TOKEN}"),
                ..Default::default()
            }),
            ..Default::default()
        }),
        token: Some("opaque.signed".to_owned()),
        context: Arc::new([]),
        referenced_attachments: HashMap::new(),
        user_query_mode: UserQueryMode::Normal,
    }
}

/// Platform message text is user-provided from the model's point of view, so it goes through the
/// same secret redaction as a typed query before being sent to the server.
#[test]
#[serial]
fn redact_inputs_redacts_external_query_text_fields() {
    secrets::set_user_and_enterprise_secret_regexes(
        [&Regex::new(r"\bghp_[A-Za-z0-9_]{36}\b").expect("valid regex")],
        std::iter::empty(),
    );

    let mut inputs = vec![external_query_input()];
    redact_inputs(&mut inputs);

    let AIAgentInput::ExternalQuery { query, token, .. } = &inputs[0] else {
        panic!("variant is preserved");
    };
    let message = query.message.as_ref().expect("message is preserved");
    let sender = message.sender.as_ref().expect("sender is preserved");
    for field in [
        &message.body,
        &message.permalink,
        &sender.display_name,
        &sender.handle,
    ] {
        assert!(
            !field.contains(TOKEN),
            "secret should be redacted from {field:?}"
        );
    }
    assert!(message.body.starts_with("my token is "));
    assert_eq!(token.as_deref(), Some("opaque.signed"));
}
