use prost::Message as _;
use session_sharing_protocol::common::{
    OrderedTerminalEvent, OrderedTerminalEventType, ParticipantId, Role, WindowSize,
};
use session_sharing_protocol::sharer::UpstreamMessage;
use warp_multi_agent_api::client_action::{Action, AddMessagesToTask, BeginTransaction};
use warp_multi_agent_api::message::{AgentOutput, Message as MessageKind};
use warp_multi_agent_api::response_event::{StreamInit, Type};
use warp_multi_agent_api::{ClientAction, ResponseEvent};

use super::{
    bound_ordered_terminal_event, client_actions_event, length_delimited_len,
    ordered_terminal_event_wire_len, placeholder_for, replacement_for_oversized_message,
};
use crate::terminal::shared_session::ai_agent::{
    decode_agent_response_event, encode_agent_response_event,
};

/// A cap small enough that tests can exceed it with tiny payloads.
const TEST_MAX_BYTES: usize = 2_000;

fn agent_output_message(id: &str, text_len: usize) -> warp_multi_agent_api::Message {
    warp_multi_agent_api::Message {
        id: id.to_string(),
        request_id: "request-1".to_string(),
        message: Some(MessageKind::AgentOutput(AgentOutput {
            text: "x".repeat(text_len),
        })),
        ..Default::default()
    }
}

fn add_messages_action(messages: Vec<warp_multi_agent_api::Message>) -> ClientAction {
    ClientAction {
        action: Some(Action::AddMessagesToTask(AddMessagesToTask {
            task_id: "task-1".to_string(),
            messages,
        })),
    }
}

fn begin_transaction_action() -> ClientAction {
    ClientAction {
        action: Some(Action::BeginTransaction(BeginTransaction {})),
    }
}

fn agent_response_event(response: &ResponseEvent) -> OrderedTerminalEventType {
    OrderedTerminalEventType::AgentResponseEvent {
        response_initiator: Some(ParticipantId::new()),
        response_event: encode_agent_response_event(response),
        forked_from_conversation_token: Some("forked-from".to_string()),
    }
}

fn decode_actions(event_type: &OrderedTerminalEventType) -> Vec<ClientAction> {
    let OrderedTerminalEventType::AgentResponseEvent { response_event, .. } = event_type else {
        panic!("expected an AgentResponseEvent, got {event_type:?}");
    };
    match decode_agent_response_event(response_event).unwrap().r#type {
        Some(Type::ClientActions(client_actions)) => client_actions.actions,
        other => panic!("expected ClientActions, got {other:?}"),
    }
}

fn message_ids(actions: &[ClientAction]) -> Vec<String> {
    actions
        .iter()
        .flat_map(|action| {
            let Some(Action::AddMessagesToTask(add)) = &action.action else {
                return Vec::new();
            };
            add.messages.iter().map(|m| m.id.clone()).collect()
        })
        .collect()
}

fn envelope(event_type: &OrderedTerminalEventType) -> (Option<ParticipantId>, Option<String>) {
    let OrderedTerminalEventType::AgentResponseEvent {
        response_initiator,
        forked_from_conversation_token,
        ..
    } = event_type
    else {
        panic!("expected an AgentResponseEvent, got {event_type:?}");
    };
    (
        response_initiator.clone(),
        forked_from_conversation_token.clone(),
    )
}

fn real_wire_len(event_type: &OrderedTerminalEventType) -> usize {
    UpstreamMessage::OrderedTerminalEvent(OrderedTerminalEvent {
        event_no: usize::MAX,
        event_type: event_type.clone(),
    })
    .to_json()
    .unwrap()
    .len()
}

#[test]
fn test_wire_len_matches_real_serialization() {
    let pty = OrderedTerminalEventType::PtyBytesRead {
        bytes: lz4_flex::block::compress_prepend_size(b"hello \"world\"\n"),
    };
    assert_eq!(ordered_terminal_event_wire_len(&pty), real_wire_len(&pty));

    let agent = agent_response_event(&client_actions_event(vec![add_messages_action(vec![
        agent_output_message("m1", 10),
    ])]));
    assert_eq!(
        ordered_terminal_event_wire_len(&agent),
        real_wire_len(&agent)
    );

    let resize = OrderedTerminalEventType::Resize {
        window_size: WindowSize {
            num_rows: 24,
            num_cols: 80,
        },
    };
    assert_eq!(
        ordered_terminal_event_wire_len(&resize),
        real_wire_len(&resize)
    );
}

#[test]
fn test_length_delimited_len_matches_prost() {
    for text_len in [0, 1, 100, 200, 20_000] {
        let action = add_messages_action(vec![agent_output_message("m", text_len)]);
        let actions_len = length_delimited_len(action.encoded_len());
        let event = client_actions_event(vec![action]);
        assert_eq!(length_delimited_len(actions_len), event.encoded_len());
    }
}

#[test]
fn test_small_events_are_unchanged() {
    let events = [
        OrderedTerminalEventType::PtyBytesRead {
            bytes: lz4_flex::block::compress_prepend_size(b"abc"),
        },
        OrderedTerminalEventType::CommandExecutionStarted {
            participant_id: ParticipantId::new(),
            ai_metadata: None,
        },
        OrderedTerminalEventType::AgentConversationReplayStarted,
        agent_response_event(&client_actions_event(vec![add_messages_action(vec![
            agent_output_message("m1", 100),
            agent_output_message("m2", 100),
        ])])),
    ];
    for event_type in events {
        let expected = ordered_terminal_event_wire_len(&event_type);
        let bounded = bound_ordered_terminal_event(event_type, TEST_MAX_BYTES);
        assert_eq!(bounded.events.len(), 1);
        assert!(bounded.original_bytes.is_none());
        assert!(bounded.dropped.is_empty());
        assert_eq!(
            ordered_terminal_event_wire_len(&bounded.events[0]),
            expected
        );
    }
}

#[test]
fn test_oversized_client_actions_are_packed_into_fitting_events() {
    // Each action is ~500 bytes on the wire, so a handful exceed the cap together but a few fit
    // in each chunk. Non-message actions interleaved to check order is preserved.
    let actions = vec![
        begin_transaction_action(),
        add_messages_action(vec![agent_output_message("m1", 500)]),
        add_messages_action(vec![agent_output_message("m2", 500)]),
        add_messages_action(vec![agent_output_message("m3", 500)]),
        add_messages_action(vec![agent_output_message("m4", 500)]),
        add_messages_action(vec![agent_output_message("m5", 500)]),
        begin_transaction_action(),
    ];
    let original = agent_response_event(&client_actions_event(actions.clone()));
    let original_len = ordered_terminal_event_wire_len(&original);
    assert!(original_len > TEST_MAX_BYTES);

    let bounded = bound_ordered_terminal_event(original.clone(), TEST_MAX_BYTES);
    assert_eq!(bounded.original_bytes, Some(original_len));
    assert!(bounded.dropped.is_empty());
    assert!(bounded.events.len() > 1);

    let mut reassembled = Vec::new();
    for event_type in &bounded.events {
        assert!(real_wire_len(event_type) <= TEST_MAX_BYTES);
        assert_eq!(envelope(event_type), envelope(&original));
        reassembled.extend(decode_actions(event_type));
    }
    assert_eq!(reassembled, actions);
}

#[test]
fn test_oversized_add_messages_action_is_split_per_message() {
    let messages = (0..6)
        .map(|i| agent_output_message(&format!("m{i}"), 500))
        .collect::<Vec<_>>();
    let original = agent_response_event(&client_actions_event(vec![add_messages_action(
        messages.clone(),
    )]));

    let bounded = bound_ordered_terminal_event(original, TEST_MAX_BYTES);
    assert!(bounded.dropped.is_empty());
    assert!(bounded.events.len() > 1);

    let mut reassembled = Vec::new();
    for event_type in &bounded.events {
        assert!(real_wire_len(event_type) <= TEST_MAX_BYTES);
        for action in decode_actions(event_type) {
            let Some(Action::AddMessagesToTask(add)) = action.action else {
                panic!("expected AddMessagesToTask");
            };
            assert_eq!(add.task_id, "task-1");
            reassembled.extend(add.messages);
        }
    }
    assert_eq!(reassembled, messages);
}

#[test]
fn test_single_message_over_cap_is_dropped_and_rest_kept() {
    let original = agent_response_event(&client_actions_event(vec![
        add_messages_action(vec![agent_output_message("small-before", 100)]),
        add_messages_action(vec![
            agent_output_message("huge", TEST_MAX_BYTES * 2),
            agent_output_message("small-sibling", 100),
        ]),
        begin_transaction_action(),
    ]));

    let bounded = bound_ordered_terminal_event(original, TEST_MAX_BYTES);
    assert_eq!(bounded.dropped.len(), 1);
    let dropped = &bounded.dropped[0];
    assert!(dropped.description.contains("AddMessagesToTask"));
    assert!(dropped.description.contains("message_ids=[huge]"));
    assert!(dropped.description.contains("request_id=request-1"));
    assert!(dropped.bytes > TEST_MAX_BYTES);

    let reassembled = bounded
        .events
        .iter()
        .flat_map(decode_actions)
        .collect::<Vec<_>>();
    assert_eq!(
        message_ids(&reassembled),
        vec!["small-before".to_string(), "small-sibling".to_string()]
    );
    assert!(matches!(
        reassembled.last().unwrap().action,
        Some(Action::BeginTransaction(_))
    ));
    for event_type in &bounded.events {
        assert!(real_wire_len(event_type) <= TEST_MAX_BYTES);
    }
}

#[test]
fn test_event_with_only_oversized_message_produces_no_events() {
    let original = agent_response_event(&client_actions_event(vec![add_messages_action(vec![
        agent_output_message("huge", TEST_MAX_BYTES * 2),
    ])]));
    let bounded = bound_ordered_terminal_event(original, TEST_MAX_BYTES);
    assert!(bounded.events.is_empty());
    assert!(bounded.original_bytes.is_some());
    assert_eq!(bounded.dropped.len(), 1);
}

#[test]
fn test_oversized_indivisible_events_are_dropped() {
    let init = agent_response_event(&ResponseEvent {
        r#type: Some(Type::Init(StreamInit {
            conversation_id: "c".repeat(TEST_MAX_BYTES),
            request_id: "r".to_string(),
            run_id: String::new(),
        })),
    });
    let bounded = bound_ordered_terminal_event(init, TEST_MAX_BYTES);
    assert!(bounded.events.is_empty());
    assert_eq!(bounded.dropped.len(), 1);
    assert!(
        bounded.dropped[0]
            .description
            .contains("AgentResponseEvent")
    );

    let undecodable = OrderedTerminalEventType::AgentResponseEvent {
        response_initiator: None,
        response_event: "!".repeat(TEST_MAX_BYTES),
        forked_from_conversation_token: None,
    };
    let bounded = bound_ordered_terminal_event(undecodable, TEST_MAX_BYTES);
    assert!(bounded.events.is_empty());
    assert_eq!(bounded.dropped.len(), 1);

    let pty = OrderedTerminalEventType::PtyBytesRead {
        bytes: vec![255; TEST_MAX_BYTES],
    };
    let pty_len = ordered_terminal_event_wire_len(&pty);
    let bounded = bound_ordered_terminal_event(pty, TEST_MAX_BYTES);
    assert!(bounded.events.is_empty());
    assert_eq!(bounded.original_bytes, Some(pty_len));
    assert_eq!(bounded.dropped.len(), 1);
    assert_eq!(bounded.dropped[0].description, "PtyBytesRead");
    assert_eq!(bounded.dropped[0].bytes, pty_len);
}

#[test]
fn test_placeholders_are_no_ops_that_keep_the_envelope() {
    let initiator = Some(ParticipantId::new());
    let agent = OrderedTerminalEventType::AgentResponseEvent {
        response_initiator: initiator.clone(),
        response_event: "payload".to_string(),
        forked_from_conversation_token: Some("forked-from".to_string()),
    };
    let Some(OrderedTerminalEventType::AgentResponseEvent {
        response_initiator,
        response_event,
        forked_from_conversation_token,
    }) = placeholder_for(&agent)
    else {
        panic!("expected an AgentResponseEvent placeholder");
    };
    assert_eq!(response_initiator, initiator);
    assert_eq!(
        forked_from_conversation_token.as_deref(),
        Some("forked-from")
    );
    let decoded = decode_agent_response_event(&response_event).unwrap();
    assert!(matches!(
        decoded.r#type,
        Some(Type::ClientActions(actions)) if actions.actions.is_empty()
    ));

    let pty = OrderedTerminalEventType::PtyBytesRead {
        bytes: vec![1, 2, 3],
    };
    let Some(OrderedTerminalEventType::PtyBytesRead { bytes }) = placeholder_for(&pty) else {
        panic!("expected a PtyBytesRead placeholder");
    };
    assert!(
        lz4_flex::block::decompress_size_prepended(&bytes)
            .unwrap()
            .is_empty()
    );

    assert!(placeholder_for(&OrderedTerminalEventType::AgentConversationReplayEnded).is_none());
    assert!(
        placeholder_for(&OrderedTerminalEventType::Resize {
            window_size: WindowSize::default(),
        })
        .is_none()
    );
}

#[test]
fn test_replacement_keeps_event_number_and_skips_other_messages() {
    let message = UpstreamMessage::OrderedTerminalEvent(OrderedTerminalEvent {
        event_no: 42,
        event_type: OrderedTerminalEventType::PtyBytesRead {
            bytes: vec![1, 2, 3],
        },
    });
    let Some(UpstreamMessage::OrderedTerminalEvent(replacement)) =
        replacement_for_oversized_message(&message)
    else {
        panic!("expected an ordered terminal event replacement");
    };
    assert_eq!(replacement.event_no, 42);
    assert!(matches!(
        replacement.event_type,
        OrderedTerminalEventType::PtyBytesRead { .. }
    ));

    assert!(
        replacement_for_oversized_message(&UpstreamMessage::AddGuests {
            emails: vec![],
            role: Role::Reader,
        })
        .is_none()
    );
}
