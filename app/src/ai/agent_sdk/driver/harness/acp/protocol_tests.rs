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
