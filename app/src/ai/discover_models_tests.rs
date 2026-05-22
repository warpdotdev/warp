//! Tests for [`super::discover_models`] and [`super::new_model_ids`].
//!
//! Network paths use `mockito::Server` against `http_client::Client::new_for_test()`.

use futures::executor::block_on;
use mockito::Server;

use super::{discover_models, new_model_ids};

fn test_client() -> http_client::Client {
    http_client::Client::new_for_test()
}

#[test]
fn returns_model_ids_on_happy_path() {
    let mut server = Server::new();
    let mock = server
        .mock("GET", "/models")
        .match_header("authorization", "Bearer sk-test")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            r#"{
                "object": "list",
                "data": [
                    {"id": "gpt-4o", "object": "model"},
                    {"id": "gpt-4o-mini", "object": "model"},
                    {"id": "o1-preview", "object": "model"}
                ]
            }"#,
        )
        .create();

    let result = block_on(discover_models(&test_client(), &server.url(), "sk-test")).unwrap();

    mock.assert();
    assert_eq!(result, vec!["gpt-4o", "gpt-4o-mini", "o1-preview"]);
}

#[test]
fn tolerates_trailing_slash_on_base_url() {
    let mut server = Server::new();
    let mock = server
        .mock("GET", "/models")
        .with_status(200)
        .with_body(r#"{"data":[{"id":"only-one"}]}"#)
        .create();

    let url_with_slash = format!("{}/", server.url());
    let result = block_on(discover_models(&test_client(), &url_with_slash, "k")).unwrap();

    mock.assert();
    assert_eq!(result, vec!["only-one"]);
}

#[test]
fn rejects_empty_url() {
    let err = block_on(discover_models(&test_client(), "  ", "k")).unwrap_err();
    assert!(err.to_string().contains("URL is empty"));
}

#[test]
fn rejects_empty_api_key() {
    let err = block_on(discover_models(
        &test_client(),
        "https://example.com/v1",
        "  ",
    ))
    .unwrap_err();
    assert!(err.to_string().contains("API key is empty"));
}

#[test]
fn surfaces_401_unauthorized() {
    let mut server = Server::new();
    let mock = server
        .mock("GET", "/models")
        .with_status(401)
        .with_body("Invalid API key")
        .create();

    let err = block_on(discover_models(&test_client(), &server.url(), "bad-key")).unwrap_err();
    mock.assert();
    let msg = err.to_string();
    assert!(msg.contains("401"), "unexpected error: {msg}");
    assert!(msg.contains("Invalid API key"), "unexpected error: {msg}");
}

#[test]
fn surfaces_404_not_found() {
    let mut server = Server::new();
    let mock = server.mock("GET", "/models").with_status(404).create();

    let err = block_on(discover_models(&test_client(), &server.url(), "k")).unwrap_err();
    mock.assert();
    assert!(err.to_string().contains("404"));
}

#[test]
fn fails_on_non_json_response() {
    let mut server = Server::new();
    let mock = server
        .mock("GET", "/models")
        .with_status(200)
        .with_body("<html>oops, you hit a captive portal</html>")
        .create();

    let err = block_on(discover_models(&test_client(), &server.url(), "k")).unwrap_err();
    mock.assert();
    assert!(err.to_string().contains("OpenAI-compatible JSON"));
}

#[test]
fn fails_on_missing_data_field() {
    let mut server = Server::new();
    let mock = server
        .mock("GET", "/models")
        .with_status(200)
        .with_body(r#"{"models":[{"id":"x"}]}"#)
        .create();

    let err = block_on(discover_models(&test_client(), &server.url(), "k")).unwrap_err();
    mock.assert();
    assert!(err.to_string().contains("OpenAI-compatible JSON"));
}

#[test]
fn fails_on_empty_model_list() {
    let mut server = Server::new();
    let mock = server
        .mock("GET", "/models")
        .with_status(200)
        .with_body(r#"{"data":[]}"#)
        .create();

    let err = block_on(discover_models(&test_client(), &server.url(), "k")).unwrap_err();
    mock.assert();
    assert!(err.to_string().contains("no models"));
}

#[test]
fn filters_empty_ids_but_keeps_nonempty() {
    let mut server = Server::new();
    let mock = server
        .mock("GET", "/models")
        .with_status(200)
        .with_body(r#"{"data":[{"id":""},{"id":"keep-me"},{"id":"   "}]}"#)
        .create();

    let result = block_on(discover_models(&test_client(), &server.url(), "k")).unwrap();
    mock.assert();
    assert_eq!(result, vec!["keep-me"]);
}

#[test]
fn surfaces_network_error_for_unreachable_endpoint() {
    let err = block_on(discover_models(
        &test_client(),
        "http://127.0.0.1:1/v1",
        "k",
    ))
    .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("network error"), "unexpected error: {msg}");
}

#[test]
fn new_model_ids_excludes_existing_case_insensitive() {
    let discovered = vec![
        "GPT-4o".to_string(),
        "gpt-4o-mini".to_string(),
        "o1-preview".to_string(),
    ];
    let existing = vec!["gpt-4o".to_string(), "o1-preview".to_string()];
    let new_ids = new_model_ids(&discovered, &existing);
    assert_eq!(new_ids, vec!["gpt-4o-mini"]);
}

#[test]
fn new_model_ids_dedups_within_discovery() {
    let discovered = vec!["a".to_string(), "A".to_string(), "b".to_string()];
    let existing: Vec<String> = Vec::new();
    let new_ids = new_model_ids(&discovered, &existing);
    assert_eq!(new_ids, vec!["a", "b"]);
}

#[test]
fn new_model_ids_ignores_blank_existing_rows() {
    let discovered = vec!["new-model".to_string()];
    let existing = vec!["".to_string(), "   ".to_string()];
    let new_ids = new_model_ids(&discovered, &existing);
    assert_eq!(new_ids, vec!["new-model"]);
}
