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

#[test]
fn parses_session_capabilities_from_claude_agent_acp() {
    let response: InitializeResponse = serde_json::from_value(json!({
        "protocolVersion": 1,
        "agentInfo": { "name": "@agentclientprotocol/claude-agent-acp", "version": "0.93.0" },
        "agentCapabilities": {
            "loadSession": true,
            "promptCapabilities": { "image": true, "embeddedContext": true },
            "sessionCapabilities": { "resume": {}, "list": {}, "close": {} },
        },
    }))
    .unwrap();

    let capabilities = response.agent_capabilities;
    assert!(capabilities.session_capabilities.supports_resume());
    assert!(!capabilities.extra.contains_key("sessionCapabilities"));
}

#[test]
fn agents_that_announce_no_resume_cannot_resume() {
    for agent_capabilities in [
        json!({}),
        json!({ "sessionCapabilities": {} }),
        json!({ "sessionCapabilities": { "resume": null, "list": {} } }),
    ] {
        let response: InitializeResponse = serde_json::from_value(json!({
            "protocolVersion": 1,
            "agentCapabilities": agent_capabilities,
        }))
        .unwrap();
        assert!(
            !response
                .agent_capabilities
                .session_capabilities
                .supports_resume()
        );
    }
}

#[test]
fn resume_response_becomes_the_session_it_reopened() {
    let response: ResumeSessionResponse = serde_json::from_value(json!({
        "configOptions": [{ "id": "model", "category": "model", "type": "select", "options": [] }],
        "_meta": { "adapter": "data" },
    }))
    .unwrap();

    let session = response.into_session("prior-session".to_owned());

    assert_eq!(session.session_id, "prior-session");
    assert_eq!(session.config_options.len(), 1);
    assert_eq!(session.meta["adapter"], "data");

    let empty: ResumeSessionResponse = serde_json::from_value(json!({})).unwrap();
    assert!(empty.into_session("s".to_owned()).config_options.is_empty());
}

#[test]
fn resume_request_names_the_session_cwd_and_servers() {
    let params = serde_json::to_value(ResumeSessionParams {
        session_id: "prior-session".to_owned(),
        cwd: "/workspace".to_owned(),
        mcp_servers: Vec::new(),
    })
    .unwrap();

    assert_eq!(
        params,
        json!({ "sessionId": "prior-session", "cwd": "/workspace", "mcpServers": [] })
    );
}
