use serde_json::json;

use super::*;

#[test]
fn parses_pi_acp_initialize_response() {
    let response: InitializeResponse = serde_json::from_value(json!({
        "protocolVersion": 1,
        "agentInfo": { "name": "pi-acp", "title": "Pi", "version": "0.0.34" },
        "agentCapabilities": {
            "loadSession": true,
            "promptCapabilities": { "image": true, "audio": false, "embeddedContext": false },
            "mcpCapabilities": { "http": true, "sse": false },
        },
        "authMethods": [],
    }))
    .unwrap();

    assert_eq!(response.protocol_version, 1);
    assert_eq!(response.agent_info.unwrap().name, "pi-acp");
    let capabilities = response.agent_capabilities;
    assert!(capabilities.load_session);
    assert_eq!(
        capabilities.prompt_capabilities,
        PromptCapabilities {
            image: true,
            audio: false,
            embedded_context: false,
        }
    );
    assert_eq!(capabilities.extra["mcpCapabilities"]["http"], true);
}

#[test]
fn missing_capabilities_default_to_a_baseline_agent() {
    let response: InitializeResponse =
        serde_json::from_value(json!({ "protocolVersion": 1 })).unwrap();
    assert!(!response.agent_capabilities.load_session);
    assert_eq!(
        response.agent_capabilities.prompt_capabilities,
        PromptCapabilities::default()
    );
    assert!(response.agent_capabilities.extra.is_empty());

    let partial: InitializeResponse = serde_json::from_value(json!({
        "protocolVersion": 1,
        "agentCapabilities": { "promptCapabilities": { "image": true } },
    }))
    .unwrap();
    assert!(partial.agent_capabilities.prompt_capabilities.image);
    assert!(
        !partial
            .agent_capabilities
            .prompt_capabilities
            .embedded_context
    );
}

#[test]
fn steering_is_read_from_the_initialize_meta() {
    let advertised: InitializeResponse = serde_json::from_value(json!({
        "protocolVersion": 1,
        "agentCapabilities": {},
        "_meta": { "steering": { "supported": true } },
    }))
    .unwrap();
    assert!(advertised.supports_steering());

    let declined: InitializeResponse = serde_json::from_value(json!({
        "protocolVersion": 1,
        "_meta": { "steering": { "supported": false } },
    }))
    .unwrap();
    assert!(!declined.supports_steering());

    let silent: InitializeResponse =
        serde_json::from_value(json!({ "protocolVersion": 1 })).unwrap();
    assert!(!silent.supports_steering());

    // Inside `agentCapabilities` is not where the adapters put it.
    let misplaced: InitializeResponse = serde_json::from_value(json!({
        "protocolVersion": 1,
        "agentCapabilities": { "_meta": { "steering": { "supported": true } } },
    }))
    .unwrap();
    assert!(!misplaced.supports_steering());
}

#[test]
fn steering_outcomes_parse_with_a_fallback_for_unknown_ones() {
    let parse = |outcome: &str| {
        serde_json::from_value::<SteeringResponse>(json!({ "outcome": outcome }))
            .unwrap()
            .outcome
    };
    assert_eq!(parse("injected"), SteeringOutcome::Injected);
    assert_eq!(parse("startedNewTurn"), SteeringOutcome::StartedNewTurn);
    assert_eq!(parse("promptRequired"), SteeringOutcome::PromptRequired);
    assert_eq!(parse("failed"), SteeringOutcome::Failed);
    assert_eq!(parse("queued"), SteeringOutcome::Unknown);

    let params = serde_json::to_value(SteeringParams {
        session_id: "sess".to_owned(),
        prompt: vec![ContentBlock::Text {
            text: "hi".to_owned(),
        }],
        meta: SteeringMeta {
            steering: SteeringOptions {
                idle_behavior: SteeringIdleBehavior::PromptRequired,
            },
        },
    })
    .unwrap();
    assert_eq!(params["sessionId"], "sess");
    assert_eq!(
        params["_meta"]["steering"]["idleBehavior"],
        "promptRequired"
    );
}

#[test]
fn embedded_resource_round_trips_in_wire_shape() {
    let block: ContentBlock = serde_json::from_value(json!({
        "type": "resource",
        "resource": { "uri": "file:///tmp/a.txt", "mimeType": "text/plain", "text": "hi" },
    }))
    .unwrap();
    assert_eq!(block.as_text(), "hi");
    let ContentBlock::Resource { resource } = &block else {
        panic!("expected a resource block");
    };
    assert_eq!(resource.uri, "file:///tmp/a.txt");
    assert_eq!(
        serde_json::to_value(&block).unwrap()["resource"]["mimeType"],
        "text/plain"
    );

    let blob: ContentBlock = serde_json::from_value(json!({
        "type": "resource",
        "resource": { "uri": "file:///tmp/a.bin", "blob": "AAEC" },
    }))
    .unwrap();
    assert_eq!(blob.as_text(), "file:///tmp/a.bin");
}
