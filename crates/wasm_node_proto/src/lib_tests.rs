use super::*;

/// Build an SSE event block (`data:<base64>\n\n`) from a `ResponseEvent`.
fn sse_event(event: &api::ResponseEvent) -> String {
    let bytes = event.encode_to_vec();
    let b64 = base64::engine::general_purpose::URL_SAFE.encode(&bytes);
    format!("data:{b64}\n\n")
}

#[test]
fn decodes_init_client_actions_and_finished_to_a_terminal_result() {
    // StreamInit
    let init = api::ResponseEvent {
        r#type: Some(api::response_event::Type::Init(
            api::response_event::StreamInit {
                conversation_id: "conv-123".to_string(),
                request_id: "req-1".to_string(),
                run_id: "run-9".to_string(),
            },
        )),
    };
    // ClientActions carrying an assistant AgentOutput message.
    let agent_msg = api::Message {
        message: Some(api::message::Message::AgentOutput(
            api::message::AgentOutput {
                text: "Hello from the agent!".to_string(),
            },
        )),
        ..Default::default()
    };
    let actions = api::ResponseEvent {
        r#type: Some(api::response_event::Type::ClientActions(
            api::response_event::ClientActions {
                actions: vec![api::ClientAction {
                    action: Some(api::client_action::Action::AddMessagesToTask(
                        api::client_action::AddMessagesToTask {
                            task_id: "task-1".to_string(),
                            messages: vec![agent_msg],
                        },
                    )),
                }],
            },
        )),
    };
    // StreamFinished::Done (terminal).
    let finished = api::ResponseEvent {
        r#type: Some(api::response_event::Type::Finished(
            api::response_event::StreamFinished {
                reason: Some(api::response_event::stream_finished::Reason::Done(
                    api::response_event::stream_finished::Done {},
                )),
                ..Default::default()
            },
        )),
    };

    let stream = format!(
        "{}{}{}",
        sse_event(&init),
        sse_event(&actions),
        sse_event(&finished)
    );

    let mut acc = StreamAccumulator::new();
    let mut events_log: Vec<String> = Vec::new();
    let mut event_count = 0;
    let mut terminal = false;

    let mut buffer = stream;
    while let Some((block, rest)) = take_complete_event(&buffer) {
        buffer = rest;
        for data in parse_sse_event_data(&block) {
            event_count += 1;
            let event = decode_event(&data).expect("event decodes");
            let (label, t) = accumulate(&event, &mut acc, &mut events_log);
            if let Some(label) = label {
                events_log.push(label);
            }
            if t {
                terminal = true;
            }
        }
    }

    assert!(terminal, "Finished event should be terminal");
    assert_eq!(event_count, 3);
    assert_eq!(acc.conversation_id.as_deref(), Some("conv-123"));
    assert_eq!(acc.request_id.as_deref(), Some("req-1"));
    assert_eq!(acc.run_id.as_deref(), Some("run-9"));
    assert_eq!(acc.text, "Hello from the agent!");
    assert_eq!(acc.finished_reason.as_deref(), Some("done"));
    assert!(acc.ok);
    assert_eq!(acc.status, "success");
}

#[test]
fn decodes_finished_internal_error_as_failure() {
    let finished = api::ResponseEvent {
        r#type: Some(api::response_event::Type::Finished(
            api::response_event::StreamFinished {
                reason: Some(api::response_event::stream_finished::Reason::InternalError(
                    api::response_event::stream_finished::InternalError {
                        message: "boom".to_string(),
                    },
                )),
                ..Default::default()
            },
        )),
    };
    let mut acc = StreamAccumulator::new();
    let mut log = Vec::new();
    let (_, terminal) = accumulate(&finished, &mut acc, &mut log);
    assert!(terminal);
    assert!(!acc.ok);
    assert_eq!(acc.status, "error");
    assert_eq!(acc.finished_reason.as_deref(), Some("internal_error:boom"));
    assert_eq!(
        acc.to_result().error.as_deref(),
        Some("stream finished: internal_error:boom")
    );
}

#[test]
fn detects_tool_call_messages_as_unsupported_capability_signal() {
    let tool_msg = api::Message {
        message: Some(api::message::Message::ToolCall(
            api::message::ToolCall::default(),
        )),
        ..Default::default()
    };
    assert!(message_is_tool_call(&tool_msg));
    let agent_msg = api::Message {
        message: Some(api::message::Message::AgentOutput(
            api::message::AgentOutput {
                text: "hi".to_string(),
            },
        )),
        ..Default::default()
    };
    assert!(!message_is_tool_call(&agent_msg));
    assert_eq!(agent_text_from_message(&agent_msg).as_deref(), Some("hi"));
    assert!(agent_text_from_message(&tool_msg).is_none());
}

#[test]
fn builds_a_valid_request_for_a_hello_prompt() {
    let req = build_request("hello", Some("claude-4-5-sonnet"));
    let input_ref = req.input.as_ref().expect("input present");
    let user_inputs = match input_ref.r#type.as_ref().expect("type present") {
        api::request::input::Type::UserInputs(ui) => ui,
        _ => panic!("expected UserInputs"),
    };
    assert_eq!(user_inputs.inputs.len(), 1);
    let req_bytes = req.encode_to_vec();
    assert!(
        req_bytes.len() > 8,
        "request serializes to non-trivial protobuf"
    );
    // Round-trips through protobuf decode.
    let round = api::Request::decode(req_bytes.as_slice()).expect("round-trips");
    assert!(round.input.is_some());
}

#[test]
fn rejects_missing_api_key_empty_prompt_and_non_http_url() {
    assert_eq!(
        validate_input("", "wk-x", "https://app.warp.dev"),
        Err("missing prompt".to_string())
    );
    assert_eq!(
        validate_input("hi", "", "https://app.warp.dev"),
        Err("missing api_key".to_string())
    );
    assert_eq!(
        validate_input("hi", "wk-x", "ftp://nope"),
        Err("server_root_url must be an http(s) URL".to_string())
    );
    assert!(validate_input("hi", "wk-x", "https://app.warp.dev").is_ok());
    assert!(validate_input("hi", "wk-x", "http://localhost:8080").is_ok());
}
