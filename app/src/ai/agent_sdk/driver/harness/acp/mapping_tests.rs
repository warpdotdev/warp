use serde_json::json;
use uuid::Uuid;
use warp_multi_agent_api::client_action::Action;
use warp_multi_agent_api::message::Message as MessageKind;
use warp_multi_agent_api::message::tool_call::Tool;
use warp_multi_agent_api::message::tool_call_result::Result as ToolResult;
use warp_multi_agent_api::message::update_todos::Operation;
use warp_multi_agent_api::{
    self as api, ExternalMessage, ExternalUser, McpIntegration, McpServerIdentity, QueryAuthor,
    UserQueryOrigin, apply_file_diffs_result, call_mcp_tool_result, query_author,
    read_files_result, user_query_origin,
};

use super::super::mcp_projection::{KnownMcpServer, KnownMcpServers};
use super::super::protocol::SessionUpdate;
use super::{AcpTurnMapper, TurnEvent};
use crate::ai::agent::BaseUserQuery;
use crate::ai::mcp::builtin::{COMPUTER_USE_MCP_INSTALLATION_UUID, COMPUTER_USE_MCP_SERVER_NAME};

fn update(value: serde_json::Value) -> SessionUpdate {
    serde_json::from_value(value).expect("fixture should deserialize")
}

fn mapper() -> AcpTurnMapper {
    mapper_with_servers(Vec::new())
}

fn mapper_with_servers(servers: Vec<KnownMcpServer>) -> AcpTurnMapper {
    AcpTurnMapper::new(
        "task-1".to_owned(),
        "req-1".to_owned(),
        KnownMcpServers::new(servers),
    )
}

const SLACK_INSTALLATION_ID: Uuid = Uuid::from_u128(0x5a1c);

/// The Slack integration configured under the name `slack`.
fn slack_server() -> KnownMcpServer {
    KnownMcpServer {
        name: "slack".to_owned(),
        installation_id: SLACK_INSTALLATION_ID,
        warp_id: Some("slack".to_owned()),
    }
}

fn slack_identity() -> McpServerIdentity {
    McpServerIdentity {
        display_name: "slack".to_owned(),
        integration: McpIntegration::Slack as i32,
        ..Default::default()
    }
}

fn mcp_result_text(result: &ToolResult) -> &str {
    let ToolResult::CallMcpTool(mcp) = result else {
        panic!("expected CallMcpTool result, got {result:?}");
    };
    let Some(call_mcp_tool_result::Result::Success(success)) = &mcp.result else {
        panic!("expected a successful MCP result, got {mcp:?}");
    };
    match &success.results[0].result {
        Some(call_mcp_tool_result::success::result::Result::Text(text)) => &text.text,
        other => panic!("expected text, got {other:?}"),
    }
}

fn added_messages(action: &warp_multi_agent_api::ClientAction) -> &[warp_multi_agent_api::Message] {
    match &action.action {
        Some(Action::AddMessagesToTask(add)) => &add.messages,
        other => panic!("expected AddMessagesToTask, got {other:?}"),
    }
}

/// The client actions of a single `TurnEvent::Actions` event.
fn only_actions(event: &TurnEvent) -> &[warp_multi_agent_api::ClientAction] {
    match event {
        TurnEvent::Actions(actions) => actions,
        TurnEvent::SegmentBoundary => panic!("expected actions, got a segment boundary"),
    }
}

/// The single `ToolCall` announced by `event`, as `(action id, tool)`.
fn announced_tool(event: &TurnEvent) -> (String, Tool) {
    match &added_messages(&only_actions(event)[0])[0].message {
        Some(MessageKind::ToolCall(call)) => {
            (call.tool_call_id.clone(), call.tool.clone().expect("tool"))
        }
        other => panic!("expected ToolCall, got {other:?}"),
    }
}

/// The single `ToolCallResult` in `event`, as `(action id, result)`.
fn reported_result(event: &TurnEvent) -> (String, ToolResult) {
    match &added_messages(&only_actions(event)[0])[0].message {
        Some(MessageKind::ToolCallResult(result)) => (
            result.tool_call_id.clone(),
            result.result.clone().expect("result"),
        ),
        other => panic!("expected ToolCallResult, got {other:?}"),
    }
}

#[test]
fn initial_actions_create_task_then_echo_user_query() {
    let actions = mapper().initial_actions("hello", None);
    assert_eq!(actions.len(), 2);
    let Some(Action::CreateTask(create)) = &actions[0].action else {
        panic!("expected CreateTask");
    };
    assert_eq!(create.task.as_ref().unwrap().id, "task-1");
    let messages = added_messages(&actions[1]);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].task_id, "task-1");
    assert_eq!(messages[0].request_id, "req-1");
    match &messages[0].message {
        Some(MessageKind::UserQuery(query)) => assert_eq!(query.query, "hello"),
        other => panic!("expected UserQuery, got {other:?}"),
    }
}

#[test]
fn initial_actions_carry_the_opening_query_attribution() {
    let origin = UserQueryOrigin {
        variant: Some(user_query_origin::Variant::WarpClient(
            user_query_origin::WarpClient::default(),
        )),
    };
    let source_message = ExternalMessage {
        body: "fix the flaky test".to_owned(),
        ..Default::default()
    };
    let base = BaseUserQuery::from_proto(api::request::input::UserQuery {
        origin: Some(origin.clone()),
        source_message: Some(source_message.clone()),
        ..Default::default()
    });

    let actions = mapper().initial_actions("<skill>...</skill>\n\nfix the flaky test", Some(&base));
    match &added_messages(&actions[1])[0].message {
        Some(MessageKind::UserQuery(query)) => {
            assert_eq!(query.query, "<skill>...</skill>\n\nfix the flaky test");
            assert_eq!(query.origin, Some(origin));
            assert!(query.author.is_none());
            assert_eq!(query.source_message, Some(source_message));
        }
        other => panic!("expected UserQuery, got {other:?}"),
    }
}

#[test]
fn user_query_action_copies_the_base_query_attribution() {
    let origin = UserQueryOrigin {
        variant: Some(user_query_origin::Variant::ServerSynthesized(
            user_query_origin::ServerSynthesized {
                reason: "slack_follow_up".to_owned(),
            },
        )),
    };
    let author = QueryAuthor {
        principal: Some(query_author::Principal::User(api::WarpUser {
            uid: "uid-1".to_owned(),
            email: "viewer@example.com".to_owned(),
            team_uid: String::new(),
        })),
        resolution: api::IdentityResolution::ClientSession.into(),
    };
    let source_message = ExternalMessage {
        sender: Some(ExternalUser {
            handle: "viewer".to_owned(),
            ..Default::default()
        }),
        body: "please also run the tests".to_owned(),
        ..Default::default()
    };
    let base = BaseUserQuery::from_proto(api::request::input::UserQuery {
        query: "ignored: the echo shows the relayed text".to_owned(),
        origin: Some(origin.clone()),
        author: Some(author.clone()),
        source_message: Some(source_message.clone()),
        ..Default::default()
    });

    let attributed = mapper().user_query_action("run the tests", Some(&base));
    match &added_messages(&attributed)[0].message {
        Some(MessageKind::UserQuery(query)) => {
            assert_eq!(query.query, "run the tests");
            assert_eq!(query.origin, Some(origin));
            assert_eq!(query.author, Some(author));
            assert_eq!(query.source_message, Some(source_message));
        }
        other => panic!("expected UserQuery, got {other:?}"),
    }

    let unattributed = mapper().user_query_action("hello", None);
    match &added_messages(&unattributed)[0].message {
        Some(MessageKind::UserQuery(query)) => {
            assert!(query.origin.is_none());
            assert!(query.author.is_none());
            assert!(query.source_message.is_none());
        }
        other => panic!("expected UserQuery, got {other:?}"),
    }
}

#[test]
fn consecutive_message_chunks_append_to_the_same_message() {
    let mut mapper = mapper();
    let first = mapper.map_update(update(json!({
        "sessionUpdate": "agent_message_chunk",
        "content": { "type": "text", "text": "Hel" }
    })));
    let second = mapper.map_update(update(json!({
        "sessionUpdate": "agent_message_chunk",
        "content": { "type": "text", "text": "lo" }
    })));

    let messages = added_messages(&only_actions(&first[0])[0]);
    let message_id = messages[0].id.clone();
    assert!(matches!(
        &messages[0].message,
        Some(MessageKind::AgentOutput(output)) if output.text == "Hel"
    ));

    let Some(Action::AppendToMessageContent(append)) = &only_actions(&second[0])[0].action else {
        panic!("expected AppendToMessageContent, got {:?}", second[0]);
    };
    assert_eq!(append.message.as_ref().unwrap().id, message_id);
    assert_eq!(
        append.mask.as_ref().unwrap().paths,
        vec!["agent_output.text".to_owned()]
    );
}

#[test]
fn a_different_message_id_or_kind_starts_a_new_message() {
    let mut mapper = mapper();
    mapper.map_update(update(json!({
        "sessionUpdate": "agent_message_chunk",
        "messageId": "m1",
        "content": { "type": "text", "text": "one" }
    })));
    let thought = mapper.map_update(update(json!({
        "sessionUpdate": "agent_thought_chunk",
        "content": { "type": "text", "text": "thinking" }
    })));
    let new_message = mapper.map_update(update(json!({
        "sessionUpdate": "agent_message_chunk",
        "messageId": "m2",
        "content": { "type": "text", "text": "two" }
    })));

    assert!(matches!(
        &added_messages(&only_actions(&thought[0])[0])[0].message,
        Some(MessageKind::AgentReasoning(reasoning)) if reasoning.reasoning == "thinking"
    ));
    assert!(matches!(
        &added_messages(&only_actions(&new_message[0])[0])[0].message,
        Some(MessageKind::AgentOutput(output)) if output.text == "two"
    ));
}

#[test]
fn execute_tool_call_maps_to_shell_command_and_its_result_opens_a_new_segment() {
    let mut mapper = mapper();
    let call = mapper.map_update(update(json!({
        "sessionUpdate": "tool_call",
        "toolCallId": "call-1",
        "title": "Run tests",
        "kind": "execute",
        "status": "pending",
        "rawInput": { "command": "cargo test" }
    })));
    assert_eq!(call.len(), 1);
    let messages = added_messages(&only_actions(&call[0])[0]);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].request_id, "req-1");
    let action_id = match &messages[0].message {
        Some(MessageKind::ToolCall(tool_call)) => {
            assert!(matches!(
                &tool_call.tool,
                Some(Tool::RunShellCommand(run)) if run.command == "cargo test"
            ));
            tool_call.tool_call_id.clone()
        }
        other => panic!("expected ToolCall, got {other:?}"),
    };
    assert_ne!(
        action_id, "call-1",
        "the agent's id is not reused as the action id"
    );

    let in_progress = mapper.map_update(update(json!({
        "sessionUpdate": "tool_call_update",
        "toolCallId": "call-1",
        "status": "in_progress"
    })));
    assert!(in_progress.is_empty());

    let completed = mapper.map_update(update(json!({
        "sessionUpdate": "tool_call_update",
        "toolCallId": "call-1",
        "status": "completed",
        "content": [{ "type": "content", "content": { "type": "text", "text": "ok" } }]
    })));
    assert_eq!(completed.len(), 2, "boundary then result");
    assert!(matches!(completed[0], TurnEvent::SegmentBoundary));
    // The runner opens the next stream before applying what follows the boundary.
    mapper.start_segment("req-2".to_owned());
    let messages = added_messages(&only_actions(&completed[1])[0]);
    match &messages[0].message {
        Some(MessageKind::ToolCallResult(result)) => {
            assert_eq!(result.tool_call_id, action_id);
            let Some(ToolResult::RunShellCommand(shell)) = &result.result else {
                panic!("expected shell result");
            };
            assert_eq!(shell.command, "cargo test");
            #[allow(deprecated)]
            {
                assert_eq!(shell.exit_code, 0);
            }
        }
        other => panic!("expected ToolCallResult, got {other:?}"),
    }

    // A second completion for the same call is ignored.
    assert!(
        mapper
            .map_update(update(json!({
                "sessionUpdate": "tool_call_update",
                "toolCallId": "call-1",
                "status": "completed"
            })))
            .is_empty()
    );

    // Text after the results is stamped with the new request id.
    let text = mapper.map_update(update(json!({
        "sessionUpdate": "agent_message_chunk",
        "content": { "type": "text", "text": "done" }
    })));
    assert_eq!(
        added_messages(&only_actions(&text[0])[0])[0].request_id,
        "req-2"
    );
}

#[test]
fn an_agent_reusing_a_finished_tool_call_id_starts_a_new_call() {
    let mut mapper = mapper();
    let first_call = json!({
        "sessionUpdate": "tool_call",
        "toolCallId": "call-1",
        "kind": "execute",
        "status": "completed",
        "rawInput": { "command": "true" }
    });
    let first = mapper.map_update(update(first_call.clone()));
    assert_eq!(first.len(), 3, "tool call, boundary, result");
    mapper.start_segment("req-2".to_owned());

    let second = mapper.map_update(update(first_call));
    assert_eq!(second.len(), 3, "a fresh tool call, boundary, result");
    let first_id = match &added_messages(&only_actions(&first[0])[0])[0].message {
        Some(MessageKind::ToolCall(call)) => call.tool_call_id.clone(),
        other => panic!("expected ToolCall, got {other:?}"),
    };
    match &added_messages(&only_actions(&second[0])[0])[0].message {
        Some(MessageKind::ToolCall(call)) => assert_ne!(call.tool_call_id, first_id),
        other => panic!("expected ToolCall, got {other:?}"),
    }
    match &added_messages(&only_actions(&second[2])[0])[0].message {
        Some(MessageKind::ToolCallResult(result)) => assert_ne!(result.tool_call_id, first_id),
        other => panic!("expected ToolCallResult, got {other:?}"),
    }
}

#[test]
fn results_for_several_tool_calls_share_one_boundary() {
    let mut mapper = mapper();
    for id in ["a", "b"] {
        mapper.map_update(update(json!({
            "sessionUpdate": "tool_call",
            "toolCallId": id,
            "kind": "read",
            "status": "in_progress"
        })));
    }
    let first = mapper.map_update(update(json!({
        "sessionUpdate": "tool_call_update",
        "toolCallId": "a",
        "status": "completed"
    })));
    assert!(matches!(first[0], TurnEvent::SegmentBoundary));
    let second = mapper.map_update(update(json!({
        "sessionUpdate": "tool_call_update",
        "toolCallId": "b",
        "status": "completed"
    })));
    assert_eq!(second.len(), 1);
    assert!(matches!(second[0], TurnEvent::Actions(_)));
}

#[test]
fn unprojectable_tool_calls_use_the_generic_mcp_shape_and_report_failures() {
    let mut mapper = mapper();
    let call = mapper.map_update(update(json!({
        "sessionUpdate": "tool_call",
        "toolCallId": "call-2",
        "title": "Find usages",
        "kind": "search",
        "status": "failed",
        "rawInput": { "pattern": "foo" }
    })));
    assert_eq!(call.len(), 3, "tool call, boundary, failure result");
    match announced_tool(&call[0]).1 {
        Tool::CallMcpTool(mcp) => {
            assert_eq!(mcp.name, "search: Find usages");
            assert!(mcp.args.as_ref().unwrap().fields.contains_key("pattern"));
        }
        other => panic!("expected CallMcpTool, got {other:?}"),
    }
    assert!(matches!(call[1], TurnEvent::SegmentBoundary));
    assert!(matches!(
        reported_result(&call[2]).1,
        ToolResult::CallMcpTool(_)
    ));
}

#[test]
fn generic_tool_calls_always_carry_args() {
    let mut mapper = mapper();
    let without_input = mapper.map_update(update(json!({
        "sessionUpdate": "tool_call",
        "toolCallId": "call-3",
        "kind": "read",
        "status": "pending"
    })));
    let Tool::CallMcpTool(mcp) = announced_tool(&without_input[0]).1 else {
        panic!("expected CallMcpTool");
    };
    assert!(mcp.args.is_some_and(|args| args.fields.is_empty()));

    let non_object_input = mapper.map_update(update(json!({
        "sessionUpdate": "tool_call",
        "toolCallId": "call-4",
        "kind": "fetch",
        "status": "pending",
        "rawInput": "https://example.com"
    })));
    let Tool::CallMcpTool(mcp) = announced_tool(&non_object_input[0]).1 else {
        panic!("expected CallMcpTool");
    };
    assert!(
        mcp.args
            .is_some_and(|args| args.fields.contains_key("input"))
    );
}

/// The message an `UpdateTaskMessage` in `event` rewrites, with its mask paths.
fn updated_message(event: &TurnEvent) -> (&warp_multi_agent_api::Message, &[String]) {
    match &only_actions(event)[0].action {
        Some(Action::UpdateTaskMessage(update)) => (
            update.message.as_ref().expect("message"),
            &update.mask.as_ref().expect("mask").paths,
        ),
        other => panic!("expected UpdateTaskMessage, got {other:?}"),
    }
}

/// The shape claude-code-acp reports an MCP call in: `other` kind, `mcp__<server>__<tool>` as
/// the name and title, and the tool's own arguments as the input, which may arrive only on a
/// later update.
#[test]
fn mcp_tool_calls_name_their_tool_and_server_and_take_input_that_arrives_late() {
    let slack = slack_identity();
    let mut mapper = mapper_with_servers(vec![slack_server()]);
    let announced = mapper.map_update(update(json!({
        "sessionUpdate": "tool_call",
        "toolCallId": "slack-1",
        "title": "mcp__slack__send_message_to_thread",
        "kind": "other",
        "status": "pending",
        "rawInput": {}
    })));
    let message_id = added_messages(&only_actions(&announced[0])[0])[0]
        .id
        .clone();
    let (action_id, tool) = announced_tool(&announced[0]);
    let Tool::CallMcpTool(mcp) = tool else {
        panic!("expected CallMcpTool, got {tool:?}");
    };
    assert_eq!(mcp.name, "send_message_to_thread");
    assert_eq!(mcp.server_id, SLACK_INSTALLATION_ID.to_string());
    assert_eq!(mcp.server_identity, Some(slack.clone()));

    let input = mapper.map_update(update(json!({
        "sessionUpdate": "tool_call_update",
        "toolCallId": "slack-1",
        "rawInput": { "channel_id": "C1", "message": "Done" }
    })));
    assert_eq!(input.len(), 1, "{input:?}");
    let (message, paths) = updated_message(&input[0]);
    assert_eq!(message.id, message_id);
    assert_eq!(paths, ["tool_call"]);
    let Some(MessageKind::ToolCall(call)) = &message.message else {
        panic!("expected ToolCall, got {:?}", message.message);
    };
    assert_eq!(call.tool_call_id, action_id);
    let Some(Tool::CallMcpTool(mcp)) = &call.tool else {
        panic!("expected CallMcpTool, got {:?}", call.tool);
    };
    assert_eq!(mcp.name, "send_message_to_thread");
    assert_eq!(mcp.server_identity, Some(slack));
    assert!(mcp.args.as_ref().unwrap().fields.contains_key("message"));

    let unchanged = mapper.map_update(update(json!({
        "sessionUpdate": "tool_call_update",
        "toolCallId": "slack-1",
        "status": "in_progress",
        "rawInput": { "channel_id": "C1", "message": "Done" }
    })));
    assert!(unchanged.is_empty(), "{unchanged:?}");
}

#[test]
fn a_shell_command_that_arrives_late_rewrites_the_announced_call() {
    let mut mapper = mapper();
    let announced = mapper.map_update(update(json!({
        "sessionUpdate": "tool_call",
        "toolCallId": "shell-1",
        "title": "Terminal",
        "kind": "execute",
        "status": "pending",
        "rawInput": {}
    })));
    let (_, tool) = announced_tool(&announced[0]);
    assert!(matches!(tool, Tool::RunShellCommand(run) if run.command == "Terminal"));

    let completed = mapper.map_update(update(json!({
        "sessionUpdate": "tool_call_update",
        "toolCallId": "shell-1",
        "status": "completed",
        "rawInput": { "command": "gh pr view" }
    })));
    // The rewritten call, boundary, its result.
    assert_eq!(completed.len(), 3, "{completed:?}");
    let Some(MessageKind::ToolCall(call)) = &updated_message(&completed[0]).0.message else {
        panic!("expected ToolCall");
    };
    assert!(matches!(
        &call.tool,
        Some(Tool::RunShellCommand(run)) if run.command == "gh pr view"
    ));
    assert!(matches!(completed[1], TurnEvent::SegmentBoundary));
    assert!(matches!(
        reported_result(&completed[2]).1,
        ToolResult::RunShellCommand(result) if result.command == "gh pr view"
    ));
}

#[test]
fn mcp_tool_names_match_servers_by_their_normalized_names() {
    let factory = McpServerIdentity {
        display_name: "warp factory".to_owned(),
        ..Default::default()
    };
    let nested = McpServerIdentity {
        display_name: "warp factory__beta".to_owned(),
        ..Default::default()
    };
    let mut mapper = mapper_with_servers(vec![
        KnownMcpServer {
            name: "warp factory".to_owned(),
            installation_id: Uuid::from_u128(1),
            warp_id: None,
        },
        KnownMcpServer {
            name: "warp factory__beta".to_owned(),
            installation_id: Uuid::from_u128(2),
            warp_id: None,
        },
    ]);
    let mut announce = |id: &str, title: &str| {
        let events = mapper.map_update(update(json!({
            "sessionUpdate": "tool_call",
            "toolCallId": id,
            "title": title,
            "kind": "other",
            "status": "pending"
        })));
        match announced_tool(&events[0]).1 {
            Tool::CallMcpTool(mcp) => (mcp.name, mcp.server_identity),
            other => panic!("expected CallMcpTool, got {other:?}"),
        }
    };

    assert_eq!(
        announce("a", "mcp__warp_factory__send_task"),
        ("send_task".to_owned(), Some(factory))
    );
    assert_eq!(
        announce("b", "mcp__warp_factory__beta__send_task"),
        ("send_task".to_owned(), Some(nested))
    );
    assert_eq!(
        announce("c", "mcp__github__create_issue"),
        ("tool: mcp__github__create_issue".to_owned(), None)
    );
}

/// The shape codex-acp reports an MCP call in: `execute` kind, `mcp.<server>.<tool>` title, the
/// invocation as the input, and `{result, error}` as the output.
#[test]
fn codex_mcp_calls_become_identified_mcp_calls_instead_of_shell_commands() {
    let mut mapper = mapper_with_servers(vec![slack_server()]);
    let call = mapper.map_update(update(json!({
        "sessionUpdate": "tool_call",
        "toolCallId": "call-1",
        "title": "mcp.slack.send_message_to_thread",
        "kind": "execute",
        "status": "in_progress",
        "rawInput": {
            "server": "slack",
            "tool": "send_message_to_thread",
            "arguments": { "channel_id": "C1", "thread_ts": "1.2", "message": "On it" }
        }
    })));
    let (action_id, tool) = announced_tool(&call[0]);
    let Tool::CallMcpTool(mcp) = tool else {
        panic!("expected CallMcpTool, got {tool:?}");
    };
    assert_eq!(mcp.name, "send_message_to_thread");
    assert_eq!(mcp.server_id, SLACK_INSTALLATION_ID.to_string());
    assert_eq!(mcp.server_identity, Some(slack_identity()));
    let args = mcp.args.expect("args");
    assert_eq!(
        args.fields["message"].kind,
        Some(prost_types::value::Kind::StringValue("On it".to_owned()))
    );
    assert!(!args.fields.contains_key("arguments"));

    let done = mapper.map_update(update(json!({
        "sessionUpdate": "tool_call_update",
        "toolCallId": "call-1",
        "status": "completed",
        "rawOutput": {
            "result": { "content": [{ "type": "text", "text": "{\"ok\":true}" }], "_meta": null },
            "error": null
        }
    })));
    let (result_id, result) = reported_result(&done[1]);
    assert_eq!(result_id, action_id);
    assert_eq!(mcp_result_text(&result), "{\"ok\":true}");
}

#[test]
fn mcp_errors_in_the_result_envelope_become_mcp_errors() {
    let mut mapper = mapper_with_servers(vec![slack_server()]);
    let call = mapper.map_update(update(json!({
        "sessionUpdate": "tool_call",
        "toolCallId": "call-1",
        "title": "mcp.slack.send_message_to_thread",
        "kind": "execute",
        "status": "failed",
        "rawInput": { "server": "slack", "tool": "send_message_to_thread", "arguments": {} },
        "rawOutput": { "result": null, "error": { "message": "channel_not_found" } }
    })));
    let ToolResult::CallMcpTool(mcp) = reported_result(&call[2]).1 else {
        panic!("expected CallMcpTool result");
    };
    match mcp.result {
        Some(call_mcp_tool_result::Result::Error(error)) => {
            assert_eq!(error.message, "channel_not_found");
        }
        other => panic!("expected an MCP error, got {other:?}"),
    }
}

/// claude-code-acp also names the tool in `name` and `_meta.claudeCode.toolName`; either
/// identifies the call when the title does not.
#[test]
fn claude_mcp_calls_are_recognized_from_name_or_meta() {
    for fixture in [
        json!({ "name": "mcp__slack__read_thread", "title": "Read thread" }),
        json!({ "title": "Read thread", "_meta": { "claudeCode": { "toolName": "mcp__slack__read_thread" } } }),
    ] {
        let mut mapper = mapper_with_servers(vec![slack_server()]);
        let mut call = json!({
            "sessionUpdate": "tool_call",
            "toolCallId": "toolu_1",
            "kind": "other",
            "status": "pending",
            "rawInput": { "channel_id": "C1", "thread_ts": "1.2" }
        });
        call.as_object_mut()
            .unwrap()
            .extend(fixture.as_object().unwrap().clone());
        let events = mapper.map_update(update(call));
        let Tool::CallMcpTool(mcp) = announced_tool(&events[0]).1 else {
            panic!("expected CallMcpTool for {fixture}");
        };
        assert_eq!(mcp.name, "read_thread", "{fixture}");
        assert_eq!(mcp.server_identity, Some(slack_identity()), "{fixture}");
    }
}

/// The shapes pi-mcp-adapter exposes MCP tools in: a direct `<server>_<tool>` tool, a
/// per-server `mcp__<server>` proxy, and the single `mcp` proxy, the latter two taking
/// `{tool, args}`.
#[test]
fn pi_direct_and_proxy_mcp_calls_resolve_to_the_same_call() {
    for (name, input) in [
        (
            "slack_send_message_to_thread",
            json!({ "channel_id": "C1", "message": "hi" }),
        ),
        (
            "mcp__slack",
            json!({ "tool": "send_message_to_thread", "args": { "channel_id": "C1", "message": "hi" } }),
        ),
        (
            "mcp",
            json!({ "tool": "slack_send_message_to_thread", "args": { "channel_id": "C1", "message": "hi" } }),
        ),
    ] {
        let mut mapper = mapper_with_servers(vec![slack_server()]);
        let call = mapper.map_update(update(json!({
            "sessionUpdate": "tool_call",
            "toolCallId": "pi-1",
            "title": name,
            "kind": "other",
            "status": "pending",
            "rawInput": input
        })));
        let Tool::CallMcpTool(mcp) = announced_tool(&call[0]).1 else {
            panic!("expected CallMcpTool for {name}");
        };
        assert_eq!(mcp.name, "send_message_to_thread", "{name}");
        assert_eq!(mcp.server_id, SLACK_INSTALLATION_ID.to_string(), "{name}");
        let args = mcp.args.expect("args");
        assert!(args.fields.contains_key("message"), "{name}: {args:?}");
        assert!(!args.fields.contains_key("args"), "{name}: {args:?}");
    }
}

/// Computer use calls carry the built-in server's name and installation id, which is what
/// transcript viewers recognize them by, and their screenshots never reach the result as base64.
#[test]
fn computer_use_calls_are_identified_and_keep_screenshots_out_of_the_result_text() {
    const SCREENSHOT: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk";
    let claude = (
        json!({
            "title": "mcp__warp-computer-use__click",
            "kind": "other",
            "rawInput": { "x": 10, "y": 20 }
        }),
        json!({
            "content": [
                { "type": "content", "content": { "type": "text", "text": "Left click at (10, 20)." } },
                { "type": "content", "content": { "type": "image", "data": SCREENSHOT, "mimeType": "image/png" } }
            ]
        }),
    );
    let codex = (
        json!({
            "title": "mcp.warp-computer-use.click",
            "kind": "execute",
            "rawInput": { "server": "warp-computer-use", "tool": "click", "arguments": { "x": 10, "y": 20 } }
        }),
        json!({
            "rawOutput": {
                "result": {
                    "content": [
                        { "type": "text", "text": "Left click at (10, 20)." },
                        { "type": "image", "data": SCREENSHOT, "mimeType": "image/png" }
                    ]
                },
                "error": null
            }
        }),
    );
    for (harness, (call, finish)) in [("claude", claude), ("codex", codex)] {
        let mut mapper = mapper_with_servers(vec![KnownMcpServer {
            name: COMPUTER_USE_MCP_SERVER_NAME.to_owned(),
            installation_id: COMPUTER_USE_MCP_INSTALLATION_UUID,
            warp_id: None,
        }]);
        let mut announce = json!({
            "sessionUpdate": "tool_call",
            "toolCallId": "cu-1",
            "status": "in_progress"
        });
        announce
            .as_object_mut()
            .unwrap()
            .extend(call.as_object().unwrap().clone());
        let announced = mapper.map_update(update(announce));
        let Tool::CallMcpTool(mcp) = announced_tool(&announced[0]).1 else {
            panic!("expected CallMcpTool for {harness}");
        };
        assert_eq!(mcp.name, "click", "{harness}");
        assert_eq!(
            mcp.server_id,
            COMPUTER_USE_MCP_INSTALLATION_UUID.to_string(),
            "{harness}"
        );
        assert_eq!(
            mcp.server_identity,
            Some(McpServerIdentity {
                display_name: "warp-computer-use".to_owned(),
                ..Default::default()
            }),
            "{harness}"
        );
        assert!(
            mcp.args.expect("args").fields.contains_key("x"),
            "{harness}"
        );

        let mut done = json!({
            "sessionUpdate": "tool_call_update",
            "toolCallId": "cu-1",
            "status": "completed"
        });
        done.as_object_mut()
            .unwrap()
            .extend(finish.as_object().unwrap().clone());
        let finished = mapper.map_update(update(done));
        let (_, result) = reported_result(&finished[1]);
        let text = mcp_result_text(&result);
        assert!(
            text.starts_with("Left click at (10, 20)."),
            "{harness}: {text}"
        );
        assert!(!text.contains(SCREENSHOT), "{harness}: {text}");
    }
}

#[test]
fn mcp_shaped_calls_for_servers_the_run_did_not_configure_stay_generic_or_shell() {
    let mut mapper = mapper_with_servers(vec![slack_server()]);
    let other_server = mapper.map_update(update(json!({
        "sessionUpdate": "tool_call",
        "toolCallId": "call-1",
        "title": "mcp.github.create_issue",
        "kind": "execute",
        "status": "pending",
        "rawInput": { "server": "github", "tool": "create_issue", "arguments": {} }
    })));
    assert!(matches!(
        announced_tool(&other_server[0]).1,
        Tool::RunShellCommand(_)
    ));

    // A command that merely starts with a configured server's name is still a command.
    let command = mapper.map_update(update(json!({
        "sessionUpdate": "tool_call",
        "toolCallId": "call-2",
        "title": "slack_cli --version",
        "kind": "execute",
        "status": "pending",
        "rawInput": { "command": "slack_cli --version" }
    })));
    match announced_tool(&command[0]).1 {
        Tool::RunShellCommand(shell) => assert_eq!(shell.command, "slack_cli --version"),
        other => panic!("expected RunShellCommand, got {other:?}"),
    }

    let mut without_servers = self::mapper();
    let unconfigured = without_servers.map_update(update(json!({
        "sessionUpdate": "tool_call",
        "toolCallId": "call-3",
        "title": "mcp__slack__send_message_to_thread",
        "kind": "other",
        "status": "pending",
        "rawInput": { "message": "hi" }
    })));
    match announced_tool(&unconfigured[0]).1 {
        Tool::CallMcpTool(mcp) => {
            assert_eq!(mcp.name, "tool: mcp__slack__send_message_to_thread");
            assert!(mcp.server_identity.is_none());
        }
        other => panic!("expected a generic CallMcpTool, got {other:?}"),
    }
}

#[test]
fn read_tool_calls_with_a_path_project_to_read_files() {
    let mut mapper = mapper();
    let call = mapper.map_update(update(json!({
        "sessionUpdate": "tool_call",
        "toolCallId": "read-1",
        "title": "Read config",
        "kind": "read",
        "status": "pending",
        "rawInput": { "file_path": "/tmp/config.json", "offset": 10, "limit": 5 }
    })));
    let (action_id, tool) = announced_tool(&call[0]);
    let Tool::ReadFiles(read) = tool else {
        panic!("expected ReadFiles, got {tool:?}");
    };
    assert_eq!(read.files.len(), 1);
    assert_eq!(read.files[0].name, "/tmp/config.json");
    assert_eq!(read.files[0].line_ranges.len(), 1);
    assert_eq!(read.files[0].line_ranges[0].start, 10);
    assert_eq!(read.files[0].line_ranges[0].end, 14);

    let done = mapper.map_update(update(json!({
        "sessionUpdate": "tool_call_update",
        "toolCallId": "read-1",
        "status": "completed",
        "content": [{ "type": "content", "content": { "type": "text", "text": "{}" } }]
    })));
    let (result_id, result) = reported_result(&done[1]);
    assert_eq!(result_id, action_id);
    let ToolResult::ReadFiles(read) = result else {
        panic!("expected ReadFiles result, got {result:?}");
    };
    match read.result {
        Some(read_files_result::Result::TextFilesSuccess(success)) => {
            assert_eq!(success.files.len(), 1);
            assert_eq!(success.files[0].file_path, "/tmp/config.json");
            assert_eq!(success.files[0].content, "{}");
            assert_eq!(
                success.files[0].line_range.as_ref().map(|r| r.start),
                Some(10)
            );
        }
        other => panic!("expected TextFilesSuccess, got {other:?}"),
    }
}

#[test]
fn edit_tool_calls_with_a_diff_project_to_apply_file_diffs() {
    let mut mapper = mapper();
    let call = mapper.map_update(update(json!({
        "sessionUpdate": "tool_call",
        "toolCallId": "edit-1",
        "title": "Edit main.rs",
        "kind": "edit",
        "status": "completed",
        "content": [{
            "type": "diff",
            "path": "src/main.rs",
            "oldText": "fn main() {}",
            "newText": "fn main() { run(); }"
        }]
    })));
    assert_eq!(call.len(), 3, "tool call, boundary, result");
    let (action_id, tool) = announced_tool(&call[0]);
    let Tool::ApplyFileDiffs(edits) = tool else {
        panic!("expected ApplyFileDiffs, got {tool:?}");
    };
    assert_eq!(edits.summary, "Edit main.rs");
    assert_eq!(edits.diffs.len(), 1);
    assert_eq!(edits.diffs[0].file_path, "src/main.rs");
    assert_eq!(edits.diffs[0].search, "fn main() {}");
    assert_eq!(edits.diffs[0].replace, "fn main() { run(); }");
    assert!(edits.new_files.is_empty());

    let (result_id, result) = reported_result(&call[2]);
    assert_eq!(result_id, action_id);
    let ToolResult::ApplyFileDiffs(applied) = result else {
        panic!("expected ApplyFileDiffs result, got {result:?}");
    };
    match applied.result {
        Some(apply_file_diffs_result::Result::Success(success)) => assert!(
            success.updated_files_v2.is_empty(),
            "a hunk is not the file's content, so no updated file is reported"
        ),
        other => panic!("expected Success, got {other:?}"),
    }
}

#[test]
fn a_created_file_reports_its_full_content() {
    let mut mapper = mapper();
    let call = mapper.map_update(update(json!({
        "sessionUpdate": "tool_call",
        "toolCallId": "edit-new",
        "kind": "edit",
        "status": "completed",
        "content": [{ "type": "diff", "path": "NEW.md", "newText": "# New" }]
    })));
    let ToolResult::ApplyFileDiffs(applied) = reported_result(&call[2]).1 else {
        panic!("expected ApplyFileDiffs result");
    };
    match applied.result {
        Some(apply_file_diffs_result::Result::Success(success)) => {
            assert_eq!(success.updated_files_v2.len(), 1);
            let file = success.updated_files_v2[0].file.as_ref().unwrap();
            assert_eq!(file.file_path, "NEW.md");
            assert_eq!(file.content, "# New");
        }
        other => panic!("expected Success, got {other:?}"),
    }
}

#[test]
fn absurd_read_limits_do_not_produce_a_line_range() {
    let mut mapper = mapper();
    let call = mapper.map_update(update(json!({
        "sessionUpdate": "tool_call",
        "toolCallId": "read-huge",
        "kind": "read",
        "status": "pending",
        "rawInput": { "path": "/tmp/a", "offset": 1, "limit": u64::MAX }
    })));
    let Tool::ReadFiles(read) = announced_tool(&call[0]).1 else {
        panic!("expected ReadFiles");
    };
    assert!(read.files[0].line_ranges.is_empty());
}

#[test]
fn a_new_file_edit_projects_to_new_files_and_a_failure_to_an_error() {
    let mut mapper = mapper();
    let call = mapper.map_update(update(json!({
        "sessionUpdate": "tool_call",
        "toolCallId": "edit-2",
        "kind": "edit",
        "status": "failed",
        "content": [{ "type": "diff", "path": "NEW.md", "newText": "# New" }]
    })));
    let Tool::ApplyFileDiffs(edits) = announced_tool(&call[0]).1 else {
        panic!("expected ApplyFileDiffs");
    };
    assert!(edits.diffs.is_empty());
    assert_eq!(edits.new_files.len(), 1);
    assert_eq!(edits.new_files[0].file_path, "NEW.md");
    assert_eq!(edits.new_files[0].content, "# New");
    let ToolResult::ApplyFileDiffs(applied) = reported_result(&call[2]).1 else {
        panic!("expected ApplyFileDiffs result");
    };
    assert!(matches!(
        applied.result,
        Some(apply_file_diffs_result::Result::Error(_))
    ));
}

#[test]
fn an_edit_whose_diff_arrives_late_is_reannounced_as_a_file_edit() {
    let mut mapper = mapper();
    let announced = mapper.map_update(update(json!({
        "sessionUpdate": "tool_call",
        "toolCallId": "edit-3",
        "title": "Edit lib.rs",
        "kind": "edit",
        "status": "pending"
    })));
    let (generic_id, generic_tool) = announced_tool(&announced[0]);
    assert!(matches!(generic_tool, Tool::CallMcpTool(_)));

    let updated = mapper.map_update(update(json!({
        "sessionUpdate": "tool_call_update",
        "toolCallId": "edit-3",
        "status": "completed",
        "content": [{ "type": "diff", "path": "src/lib.rs", "oldText": "a", "newText": "b" }]
    })));
    // Boundary, cancel of the generic call, the file-edit call, boundary, its result.
    assert_eq!(updated.len(), 5, "{updated:?}");
    assert!(matches!(updated[0], TurnEvent::SegmentBoundary));
    mapper.start_segment("req-2".to_owned());
    let (cancelled_id, cancelled) = reported_result(&updated[1]);
    assert_eq!(cancelled_id, generic_id);
    assert!(matches!(cancelled, ToolResult::Cancel(())));
    let (edit_id, edit_tool) = announced_tool(&updated[2]);
    assert_ne!(edit_id, generic_id);
    let Tool::ApplyFileDiffs(edits) = edit_tool else {
        panic!("expected ApplyFileDiffs, got {edit_tool:?}");
    };
    assert_eq!(edits.summary, "Edit lib.rs", "title carries over");
    assert_eq!(edits.diffs[0].file_path, "src/lib.rs");
    assert!(matches!(updated[3], TurnEvent::SegmentBoundary));
    let (result_id, result) = reported_result(&updated[4]);
    assert_eq!(result_id, edit_id);
    assert!(matches!(result, ToolResult::ApplyFileDiffs(_)));
    assert!(mapper.finish_events().is_empty(), "nothing left pending");
}

#[test]
fn think_tool_calls_stream_as_reasoning_instead_of_a_tool() {
    let mut mapper = mapper();
    let first = mapper.map_update(update(json!({
        "sessionUpdate": "tool_call",
        "toolCallId": "think-1",
        "title": "Thinking",
        "kind": "think",
        "status": "in_progress",
        "rawInput": { "thought": "Let me check" }
    })));
    assert_eq!(first.len(), 1);
    let messages = added_messages(&only_actions(&first[0])[0]);
    let message_id = messages[0].id.clone();
    assert!(matches!(
        &messages[0].message,
        Some(MessageKind::AgentReasoning(reasoning)) if reasoning.reasoning == "Let me check"
    ));

    // The agent resends the whole thought; only the new tail is appended.
    let second = mapper.map_update(update(json!({
        "sessionUpdate": "tool_call_update",
        "toolCallId": "think-1",
        "status": "completed",
        "rawInput": { "thought": "Let me check the tests." }
    })));
    let Some(Action::AppendToMessageContent(append)) = &only_actions(&second[0])[0].action else {
        panic!("expected AppendToMessageContent, got {second:?}");
    };
    assert_eq!(append.message.as_ref().unwrap().id, message_id);
    assert!(matches!(
        &append.message.as_ref().unwrap().message,
        Some(MessageKind::AgentReasoning(reasoning)) if reasoning.reasoning == " the tests."
    ));
    assert!(
        mapper.finish_events().is_empty(),
        "a thought is never a pending tool call"
    );

    // Output after a finished thought starts a fresh message.
    let text = mapper.map_update(update(json!({
        "sessionUpdate": "agent_message_chunk",
        "content": { "type": "text", "text": "Done." }
    })));
    assert!(matches!(
        &only_actions(&text[0])[0].action,
        Some(Action::AddMessagesToTask(_))
    ));
}

#[test]
fn finish_cancels_unfinished_tool_calls_only() {
    let mut mapper = mapper();
    mapper.map_update(update(json!({
        "sessionUpdate": "tool_call",
        "toolCallId": "done",
        "kind": "execute",
        "status": "completed",
        "rawInput": { "command": "true" }
    })));
    mapper.start_segment("req-2".to_owned());
    let dangling = mapper.map_update(update(json!({
        "sessionUpdate": "tool_call",
        "toolCallId": "dangling",
        "kind": "search",
        "status": "in_progress"
    })));
    let dangling_id = match &added_messages(&only_actions(&dangling[0])[0])[0].message {
        Some(MessageKind::ToolCall(call)) => call.tool_call_id.clone(),
        other => panic!("expected ToolCall, got {other:?}"),
    };

    let finish = mapper.finish_events();
    assert_eq!(finish.len(), 2, "boundary then cancelled result");
    assert!(matches!(finish[0], TurnEvent::SegmentBoundary));
    let messages = added_messages(&only_actions(&finish[1])[0]);
    assert_eq!(messages.len(), 1);
    match &messages[0].message {
        Some(MessageKind::ToolCallResult(result)) => {
            assert_eq!(result.tool_call_id, dangling_id);
            assert!(matches!(result.result, Some(ToolResult::Cancel(()))));
        }
        other => panic!("expected ToolCallResult, got {other:?}"),
    }
    assert!(mapper.finish_events().is_empty());
}

#[test]
fn plan_creates_a_todo_list_then_marks_completed_entries() {
    let mut mapper = mapper();
    let created = mapper.map_update(update(json!({
        "sessionUpdate": "plan",
        "entries": [
            { "content": "Read code", "priority": "high", "status": "pending" },
            { "content": "Fix bug", "priority": "medium", "status": "pending" }
        ]
    })));
    match &added_messages(&only_actions(&created[0])[0])[0].message {
        Some(MessageKind::UpdateTodos(todos)) => match &todos.operation {
            Some(Operation::CreateTodoList(list)) => {
                assert_eq!(list.initial_todos.len(), 2);
                assert_eq!(list.initial_todos[0].title, "Read code");
            }
            other => panic!("expected CreateTodoList, got {other:?}"),
        },
        other => panic!("expected UpdateTodos, got {other:?}"),
    }

    let progressed = mapper.map_update(update(json!({
        "sessionUpdate": "plan",
        "entries": [
            { "content": "Read code", "status": "completed" },
            { "content": "Fix bug", "status": "in_progress" }
        ]
    })));
    match &added_messages(&only_actions(&progressed[0])[0])[0].message {
        Some(MessageKind::UpdateTodos(todos)) => match &todos.operation {
            Some(Operation::MarkTodosCompleted(done)) => {
                assert_eq!(done.todo_ids, vec!["acp-todo-0".to_owned()])
            }
            other => panic!("expected MarkTodosCompleted, got {other:?}"),
        },
        other => panic!("expected UpdateTodos, got {other:?}"),
    }
}

#[test]
fn unknown_updates_and_tool_kinds_are_tolerated() {
    let mut mapper = mapper();
    assert!(
        mapper
            .map_update(update(
                json!({ "sessionUpdate": "available_commands_update" })
            ))
            .is_empty()
    );
    let call = mapper.map_update(update(json!({
        "sessionUpdate": "tool_call",
        "toolCallId": "call-3",
        "title": "Mystery",
        "kind": "some_future_kind",
        "status": "completed"
    })));
    assert_eq!(call.len(), 3, "tool call, boundary, result");
}
