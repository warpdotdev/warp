use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::{RoleClient, RunningService};
use rmcp::{ServiceError, ServiceExt};
use serde_json::{Value, json};

use super::*;
use crate::RecordingDestination;
use crate::test_util::{FakeActorLog, FakeRecorder, test_config};

async fn connect(log: &FakeActorLog) -> RunningService<RoleClient, ()> {
    connect_with_config(test_config(), log).await
}

async fn connect_with_config(
    config: ComputerUseMcpConfig,
    log: &FakeActorLog,
) -> RunningService<RoleClient, ()> {
    let (client_stream, server_stream) = tokio::io::duplex(1 << 20);
    let server = ComputerUseMcpServer::with_platform(config, log.factory(), Box::new(FakeRecorder));
    tokio::spawn(async move {
        let running = server.serve(server_stream).await.unwrap();
        running.waiting().await.unwrap();
    });
    ().serve(client_stream).await.unwrap()
}

async fn call(
    client: &RunningService<RoleClient, ()>,
    tool: &'static str,
    arguments: Value,
) -> Result<CallToolResult, ServiceError> {
    let Value::Object(arguments) = arguments else {
        panic!("tool arguments must be an object");
    };
    client
        .call_tool(CallToolRequestParams::new(tool).with_arguments(arguments))
        .await
}

fn text(result: &CallToolResult) -> &str {
    &result.content[0].as_text().expect("text content").text
}

#[tokio::test]
async fn lists_every_tool() {
    let client = connect(&FakeActorLog::default()).await;
    let mut names: Vec<_> = client
        .list_all_tools()
        .await
        .unwrap()
        .into_iter()
        .map(|tool| tool.name.to_string())
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "click",
            "cursor_position",
            "drag",
            "key",
            "mouse_down",
            "mouse_move",
            "mouse_up",
            "screenshot",
            "scroll",
            "type_text",
            "wait",
            "zoom",
        ]
    );
}

#[tokio::test]
async fn recording_tools_are_listed_when_recording_is_enabled() {
    let config = ComputerUseMcpConfig {
        recording: Some(RecordingDestination::Directory(std::env::temp_dir())),
        ..test_config()
    };
    let client = connect_with_config(config, &FakeActorLog::default()).await;
    let names: Vec<_> = client
        .list_all_tools()
        .await
        .unwrap()
        .into_iter()
        .map(|tool| tool.name.to_string())
        .collect();
    assert!(names.contains(&"start_recording".to_owned()));
    assert!(names.contains(&"stop_recording".to_owned()));
}

#[tokio::test]
async fn click_requires_coordinates() {
    let client = connect(&FakeActorLog::default()).await;
    let tools = client.list_all_tools().await.unwrap();
    let click = tools.iter().find(|tool| tool.name == "click").unwrap();
    assert_eq!(click.input_schema.get("required"), Some(&json!(["x", "y"])));
}

#[tokio::test]
async fn screenshot_returns_text_and_a_png() {
    let client = connect(&FakeActorLog::default()).await;
    let result = call(&client, "screenshot", json!({})).await.unwrap();
    assert_eq!(result.is_error, Some(false));
    assert!(text(&result).starts_with("Screenshot is "));
    let image = result.content[1].as_image().expect("image content");
    assert_eq!(image.mime_type, "image/png");
    assert_eq!(STANDARD.decode(&image.data).unwrap(), b"\x89PNG fake");
}

#[tokio::test]
async fn invalid_keys_are_tool_errors() {
    let client = connect(&FakeActorLog::default()).await;
    let result = call(&client, "key", json!({ "keys": "BackSpace BackSpace" }))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    assert!(text(&result).contains("is not a valid key"));
}

#[tokio::test]
async fn out_of_bounds_clicks_are_tool_errors() {
    let client = connect(&FakeActorLog::default()).await;
    let result = call(&client, "click", json!({ "x": 100000, "y": 5 }))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    assert!(text(&result).contains("is outside the"));
}

#[tokio::test]
async fn partial_mouse_down_coordinates_are_tool_errors() {
    let log = FakeActorLog::default();
    let client = connect(&log).await;
    let result = call(&client, "mouse_down", json!({ "x": 5 }))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    assert!(log.batches().is_empty());
}

#[tokio::test]
async fn malformed_arguments_are_tool_errors() {
    let client = connect(&FakeActorLog::default()).await;
    let result = call(&client, "click", json!({ "x": "left", "y": 5 }))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    assert!(text(&result).contains("failed to deserialize parameters"));
}
