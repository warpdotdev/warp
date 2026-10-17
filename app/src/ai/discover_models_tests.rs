use std::time::Duration;

use futures::executor::block_on;
use mockito::Server;

use super::*;

#[test]
fn models_url_strips_one_trailing_slash() {
    assert_eq!(
        models_url("https://openrouter.ai/api/v1").unwrap(),
        "https://openrouter.ai/api/v1/models"
    );
    assert_eq!(
        models_url("https://openrouter.ai/api/v1/").unwrap(),
        "https://openrouter.ai/api/v1/models"
    );
    assert_eq!(
        models_url("  https://host/v1/  ").unwrap(),
        "https://host/v1/models"
    );
}

#[test]
fn models_url_rejects_empty_url() {
    assert_eq!(models_url("   "), Err(DiscoverModelsError::EmptyUrl));
    assert_eq!(models_url(""), Err(DiscoverModelsError::EmptyUrl));
}

#[test]
fn discover_models_rejects_empty_key_without_io() {
    block_on(async {
        let client = http_client::Client::new_for_test();
        let err = discover_models(&client, "https://example.com/v1", "  ")
            .await
            .unwrap_err();
        assert_eq!(err, DiscoverModelsError::EmptyKey);
        assert_eq!(err.to_string(), "empty key");
    });
}

fn discovered(id: &str, alias: Option<&str>) -> DiscoveredModel {
    DiscoveredModel {
        id: id.to_string(),
        alias: alias.map(str::to_string),
    }
}

#[test]
fn new_models_skips_existing_case_insensitively_and_blank_rows() {
    let discovered = vec![
        discovered("openai/gpt-5", Some("GPT-5")),
        discovered("anthropic/claude", Some("Claude")),
        discovered("OPENAI/GPT-5", None),
    ];
    let existing = vec!["  ".to_string(), "OpenAI/GPT-5".to_string()];
    let ids: Vec<&str> = new_models(&discovered, &existing)
        .into_iter()
        .map(|model| model.id.as_str())
        .collect();
    assert_eq!(ids, vec!["anthropic/claude"]);
}

#[test]
fn new_models_dedups_discovered_ids() {
    let discovered = vec![
        discovered("a", Some("A")),
        discovered("A", Some("ignored")),
        discovered("b", None),
    ];
    let ids: Vec<&str> = new_models(&discovered, &[])
        .into_iter()
        .map(|model| model.id.as_str())
        .collect();
    assert_eq!(ids, vec!["a", "b"]);
}

#[test]
fn fetch_returns_ids_and_sends_bearer_auth() {
    block_on(async {
        let mut server = Server::new();
        let mock = server
            .mock("GET", "/v1/models")
            .match_header("authorization", "Bearer test-key")
            .match_header("accept", "application/json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"object":"list","data":[{"id":"openai/gpt-5","owned_by":"openai"},{"id":"  "},{"id":"openai/gpt-5"},{"id":"anthropic/claude"}]}"#)
            .create();

        let client = http_client::Client::new_for_test();
        let models = discover_models(&client, &format!("{}/v1", server.url()), "test-key")
            .await
            .unwrap();

        mock.assert();
        assert_eq!(
            models,
            vec![
                discovered("openai/gpt-5", None),
                discovered("anthropic/claude", None)
            ]
        );
    });
}

#[test]
fn fetch_treats_trailing_slash_as_the_same_models_path() {
    block_on(async {
        let mut server = Server::new();
        let mock = server
            .mock("GET", "/v1/models")
            .with_status(200)
            .with_body(r#"{"data":[{"id":"one"}]}"#)
            .create();

        let client = http_client::Client::new_for_test();
        let models = discover_models(&client, &format!("{}/v1/", server.url()), "key")
            .await
            .unwrap();

        mock.assert();
        assert_eq!(models, vec![discovered("one", None)]);
    });
}

#[test]
fn fetch_uses_display_name_then_name_as_alias_when_distinct_from_id() {
    block_on(async {
        let mut server = Server::new();
        server
            .mock("GET", "/models")
            .with_status(200)
            .with_body(
                r#"{
                    "data": [
                        {"id": "openai/gpt-4", "name": "GPT-4"},
                        {"id": "gpt-5.6-sol", "name": "gpt-5.6-sol", "display_name": "GPT 5.6 Sol"},
                        {"id": "same", "name": "same"},
                        {"id": "bare"}
                    ]
                }"#,
            )
            .create();
        let client = http_client::Client::new_for_test();
        let models = discover_models(&client, &server.url(), "key")
            .await
            .unwrap();
        assert_eq!(
            models,
            vec![
                discovered("openai/gpt-4", Some("GPT-4")),
                discovered("gpt-5.6-sol", Some("GPT 5.6 Sol")),
                discovered("same", None),
                discovered("bare", None),
            ]
        );
    });
}

#[test]
fn fetch_maps_401_and_403_to_unauthorized() {
    block_on(async {
        let mut server = Server::new();
        server
            .mock("GET", "/models")
            .with_status(401)
            .with_body(r#"{"error":"nope"}"#)
            .create();
        let client = http_client::Client::new_for_test();
        let err = discover_models(&client, &server.url(), "key")
            .await
            .unwrap_err();
        assert_eq!(err, DiscoverModelsError::Unauthorized);
        assert!(!err.to_string().contains("nope"));
        assert!(!err.to_string().contains("key"));
    });
    block_on(async {
        let mut server = Server::new();
        server.mock("GET", "/models").with_status(403).create();
        let client = http_client::Client::new_for_test();
        assert_eq!(
            discover_models(&client, &server.url(), "secret-token")
                .await
                .unwrap_err(),
            DiscoverModelsError::Unauthorized
        );
    });
}

#[test]
fn fetch_maps_404_to_not_found() {
    block_on(async {
        let mut server = Server::new();
        server.mock("GET", "/models").with_status(404).create();
        let client = http_client::Client::new_for_test();
        assert_eq!(
            discover_models(&client, &server.url(), "key")
                .await
                .unwrap_err(),
            DiscoverModelsError::NotFound
        );
    });
}

#[test]
fn fetch_maps_non_json_and_missing_data_to_unexpected_response() {
    block_on(async {
        let mut server = Server::new();
        server
            .mock("GET", "/models")
            .with_status(200)
            .with_body("<html>oops</html>")
            .create();
        let client = http_client::Client::new_for_test();
        let err = discover_models(&client, &server.url(), "key")
            .await
            .unwrap_err();
        assert_eq!(err, DiscoverModelsError::UnexpectedResponse);
        assert!(!err.to_string().contains("html"));
    });
    block_on(async {
        let mut server = Server::new();
        server
            .mock("GET", "/models")
            .with_status(200)
            .with_body(r#"{"object":"list"}"#)
            .create();
        let client = http_client::Client::new_for_test();
        assert_eq!(
            discover_models(&client, &server.url(), "key")
                .await
                .unwrap_err(),
            DiscoverModelsError::UnexpectedResponse
        );
    });
}

#[test]
fn fetch_maps_empty_data_to_no_models() {
    block_on(async {
        let mut server = Server::new();
        server
            .mock("GET", "/models")
            .with_status(200)
            .with_body(r#"{"data":[{"id":"  "}]}"#)
            .create();
        let client = http_client::Client::new_for_test();
        assert_eq!(
            discover_models(&client, &server.url(), "key")
                .await
                .unwrap_err(),
            DiscoverModelsError::NoModels
        );
    });
}

#[test]
fn fetch_maps_oversized_body_to_unexpected_response() {
    block_on(async {
        let mut server = Server::new();
        let oversized = "x".repeat((1024 * 1024) + 1);
        server
            .mock("GET", "/models")
            .with_status(200)
            .with_header("content-length", &oversized.len().to_string())
            .with_body(oversized)
            .create();
        let client = http_client::Client::new_for_test();
        assert_eq!(
            discover_models(&client, &server.url(), "key")
                .await
                .unwrap_err(),
            DiscoverModelsError::UnexpectedResponse
        );
    });
}

#[test]
fn fetch_maps_unreachable_host_to_network_error() {
    block_on(async {
        let client = http_client::Client::new_for_test();
        assert_eq!(
            discover_models_with_timeout(
                &client,
                "http://127.0.0.1:1",
                "key",
                Duration::from_millis(200),
            )
            .await
            .unwrap_err(),
            DiscoverModelsError::Network
        );
    });
}

#[test]
fn error_display_never_includes_request_secrets() {
    for err in [
        DiscoverModelsError::EmptyUrl,
        DiscoverModelsError::EmptyKey,
        DiscoverModelsError::Unauthorized,
        DiscoverModelsError::NotFound,
        DiscoverModelsError::Network,
        DiscoverModelsError::UnexpectedResponse,
        DiscoverModelsError::NoModels,
    ] {
        let text = err.to_string();
        assert!(!text.contains("Bearer"));
        assert!(!text.contains("sk-"));
        assert!(!text.contains("http"));
    }
}
