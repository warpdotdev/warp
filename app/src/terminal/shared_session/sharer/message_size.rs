//! Keeps every websocket message the sharer uploads under the session sharing server's frame
//! size limit.

use std::collections::VecDeque;
use std::io;

use prost::Message as _;
use prost::encoding::encoded_len_varint;
use serde::Serialize;
use session_sharing_protocol::common::{
    OrderedTerminalEvent, OrderedTerminalEventType, ParticipantId,
};
use session_sharing_protocol::sharer::UpstreamMessage;
use warp_multi_agent_api::client_action::{Action, AddMessagesToTask};
use warp_multi_agent_api::response_event::{ClientActions, Type};
use warp_multi_agent_api::{ClientAction, ResponseEvent};

use crate::terminal::shared_session::ai_agent::{
    decode_agent_response_event, encode_agent_response_event,
};

/// Largest websocket message the session sharing server accepts. The client sends each message
/// as a single frame and the server keeps tungstenite's default 16 MiB `max_frame_size`, so it
/// drops the connection on anything larger regardless of its `max_message_size`. Because unacked
/// ordered events are replayed after every reconnect, an oversized one would be resent forever.
pub(super) const SERVER_MAX_WEBSOCKET_MESSAGE_BYTES: usize = 16 * 1024 * 1024;

/// Soft cap on the serialized size of a single `OrderedTerminalEvent` upload, kept at half of
/// [`SERVER_MAX_WEBSOCKET_MESSAGE_BYTES`] so the hard limit is never approached. Events over this
/// cap are split into smaller events or, when indivisible, dropped before they are numbered.
pub(super) const MAX_ORDERED_TERMINAL_EVENT_BYTES: usize = SERVER_MAX_WEBSOCKET_MESSAGE_BYTES / 2;

/// Largest run of raw PTY bytes compressed into a single `PtyBytesRead` event. JSON encodes each
/// compressed byte as up to four characters and LZ4 can slightly expand incompressible input, so
/// this stays a factor of eight below [`MAX_ORDERED_TERMINAL_EVENT_BYTES`].
pub(super) const MAX_PTY_BATCH_RAW_BYTES: usize = MAX_ORDERED_TERMINAL_EVENT_BYTES / 8;

/// A payload that could not be made to fit under the cap and was left out of the upload.
#[derive(Debug)]
pub(super) struct DroppedPayload {
    pub description: String,
    pub bytes: usize,
}

/// The events to upload in place of a single ordered terminal event.
#[derive(Debug)]
pub(super) struct BoundedEvents {
    pub events: Vec<OrderedTerminalEventType>,
    /// The serialized size of the original event when it exceeded the cap and had to be rewritten.
    pub original_bytes: Option<usize>,
    pub dropped: Vec<DroppedPayload>,
}

/// Rewrites `event_type` into events that each serialize to at most `max_bytes` on the wire.
///
/// Only `AgentResponseEvent`s carrying `ClientActions` can be split: actions are packed into as
/// few events as fit, an `AddMessagesToTask` is split one message per action when it does not fit
/// on its own, and a single message that still exceeds the cap is dropped. Every other oversized
/// event is indivisible and dropped whole.
pub(super) fn bound_ordered_terminal_event(
    event_type: OrderedTerminalEventType,
    max_bytes: usize,
) -> BoundedEvents {
    let wire_bytes = ordered_terminal_event_wire_len(&event_type);
    if wire_bytes <= max_bytes {
        return BoundedEvents {
            events: vec![event_type],
            original_bytes: None,
            dropped: Vec::new(),
        };
    }

    let kind = format!("{event_type:?}");
    let mut bounded = match event_type {
        OrderedTerminalEventType::AgentResponseEvent {
            response_initiator,
            response_event,
            forked_from_conversation_token,
        } => split_agent_response_event(
            response_initiator,
            response_event,
            forked_from_conversation_token,
            max_bytes,
        ),
        OrderedTerminalEventType::PtyBytesRead { .. }
        | OrderedTerminalEventType::CommandExecutionStarted { .. }
        | OrderedTerminalEventType::CommandExecutionFinished { .. }
        | OrderedTerminalEventType::Resize { .. }
        | OrderedTerminalEventType::AgentConversationReplayStarted
        | OrderedTerminalEventType::AgentConversationReplayEnded
        | OrderedTerminalEventType::CloudModeSetupPhaseEnded => BoundedEvents {
            events: Vec::new(),
            original_bytes: None,
            dropped: vec![DroppedPayload {
                description: kind,
                bytes: wire_bytes,
            }],
        },
    };
    bounded.original_bytes = Some(wire_bytes);
    bounded
}

fn split_agent_response_event(
    response_initiator: Option<ParticipantId>,
    response_event: String,
    forked_from_conversation_token: Option<String>,
    max_bytes: usize,
) -> BoundedEvents {
    let mut dropped = Vec::new();
    let actions = match decode_agent_response_event(&response_event).map(|event| event.r#type) {
        Ok(Some(Type::ClientActions(ClientActions { actions }))) => actions,
        Ok(Some(Type::Init(_))) | Ok(Some(Type::Finished(_))) | Ok(None) | Err(_) => {
            dropped.push(DroppedPayload {
                description: "AgentResponseEvent(indivisible or undecodable)".to_string(),
                bytes: response_event.len(),
            });
            return BoundedEvents {
                events: Vec::new(),
                original_bytes: None,
                dropped,
            };
        }
    };

    let make_event = |actions| OrderedTerminalEventType::AgentResponseEvent {
        response_initiator: response_initiator.clone(),
        response_event: encode_agent_response_event(&client_actions_event(actions)),
        forked_from_conversation_token: forked_from_conversation_token.clone(),
    };
    // Every chunk shares the same envelope; only the base64 payload varies, and base64 never
    // needs JSON escaping, so the wire size of a chunk is exactly envelope + payload length.
    let envelope_bytes =
        ordered_terminal_event_wire_len(&OrderedTerminalEventType::AgentResponseEvent {
            response_initiator: response_initiator.clone(),
            response_event: String::new(),
            forked_from_conversation_token: forked_from_conversation_token.clone(),
        });
    let event_bytes = |actions_proto_bytes: usize| {
        let proto_bytes = length_delimited_len(actions_proto_bytes);
        envelope_bytes.saturating_add(base64::encoded_len(proto_bytes, false).unwrap_or(usize::MAX))
    };

    let mut events = Vec::new();
    let mut current = Vec::new();
    let mut current_bytes = 0;
    let mut pending = VecDeque::from(actions);
    while let Some(action) = pending.pop_front() {
        let action_bytes = length_delimited_len(action.encoded_len());
        if event_bytes(current_bytes + action_bytes) <= max_bytes {
            current.push(action);
            current_bytes += action_bytes;
            continue;
        }
        if !current.is_empty() {
            events.push(make_event(std::mem::take(&mut current)));
            current_bytes = 0;
        }
        if event_bytes(action_bytes) <= max_bytes {
            current.push(action);
            current_bytes = action_bytes;
            continue;
        }
        let mut parts = split_action(action);
        if parts.len() > 1 {
            for part in parts.into_iter().rev() {
                pending.push_front(part);
            }
        } else if let Some(action) = parts.pop() {
            dropped.push(DroppedPayload {
                description: describe_action(&action),
                bytes: action_bytes,
            });
        }
    }
    if !current.is_empty() {
        events.push(make_event(current));
    }

    BoundedEvents {
        events,
        original_bytes: None,
        dropped,
    }
}

/// Splits an `AddMessagesToTask` carrying several messages into one action per message, which
/// viewers apply identically. Any other action is returned unchanged as the only element.
fn split_action(action: ClientAction) -> Vec<ClientAction> {
    match action.action {
        Some(Action::AddMessagesToTask(AddMessagesToTask { task_id, messages }))
            if messages.len() > 1 =>
        {
            messages
                .into_iter()
                .map(|message| ClientAction {
                    action: Some(Action::AddMessagesToTask(AddMessagesToTask {
                        task_id: task_id.clone(),
                        messages: vec![message],
                    })),
                })
                .collect()
        }
        action => vec![ClientAction { action }],
    }
}

fn describe_action(action: &ClientAction) -> String {
    match &action.action {
        Some(Action::AddMessagesToTask(add)) => {
            let message_ids = add
                .messages
                .iter()
                .map(|message| message.id.as_str())
                .collect::<Vec<_>>()
                .join(",");
            let request_id = add
                .messages
                .first()
                .map(|message| message.request_id.as_str())
                .unwrap_or_default();
            format!(
                "AddMessagesToTask(task_id={}, request_id={request_id}, message_ids=[{message_ids}])",
                add.task_id
            )
        }
        Some(Action::CreateTask(_)) => "CreateTask".to_string(),
        Some(Action::UpdateTaskMessage(_)) => "UpdateTaskMessage".to_string(),
        Some(Action::AppendToMessageContent(_)) => "AppendToMessageContent".to_string(),
        Some(Action::ShowSuggestions(_)) => "ShowSuggestions".to_string(),
        Some(Action::UpdateTaskSummary(_)) => "UpdateTaskSummary".to_string(),
        Some(Action::UpdateTaskDescription(_)) => "UpdateTaskDescription".to_string(),
        Some(Action::BeginTransaction(_)) => "BeginTransaction".to_string(),
        Some(Action::CommitTransaction(_)) => "CommitTransaction".to_string(),
        Some(Action::RollbackTransaction(_)) => "RollbackTransaction".to_string(),
        Some(Action::StartNewConversation(_)) => "StartNewConversation".to_string(),
        Some(Action::UpdateTaskServerData(_)) => "UpdateTaskServerData".to_string(),
        Some(Action::MoveMessagesToNewTask(_)) => "MoveMessagesToNewTask".to_string(),
        None => "EmptyClientAction".to_string(),
    }
}

fn client_actions_event(actions: Vec<ClientAction>) -> ResponseEvent {
    ResponseEvent {
        r#type: Some(Type::ClientActions(ClientActions { actions })),
    }
}

/// The encoded size of a length-delimited protobuf field (one-byte key) whose body is `len` bytes.
/// Both `ResponseEvent.client_actions` and each `ClientActions.actions` element encode this way.
fn length_delimited_len(len: usize) -> usize {
    1 + encoded_len_varint(len as u64) + len
}

/// A same-numbered stand-in for an event whose payload cannot be delivered. Viewers and the
/// session sharing server's conversation classifier treat the stand-in as a no-op, so it can
/// occupy the original event number without disturbing the sequence. `None` for events that carry
/// no payload and therefore cannot be oversized.
pub(super) fn placeholder_for(
    event_type: &OrderedTerminalEventType,
) -> Option<OrderedTerminalEventType> {
    match event_type {
        OrderedTerminalEventType::PtyBytesRead { .. } => {
            Some(OrderedTerminalEventType::PtyBytesRead {
                bytes: lz4_flex::block::compress_prepend_size(&[]),
            })
        }
        OrderedTerminalEventType::AgentResponseEvent {
            response_initiator,
            forked_from_conversation_token,
            ..
        } => Some(OrderedTerminalEventType::AgentResponseEvent {
            response_initiator: response_initiator.clone(),
            response_event: encode_agent_response_event(&client_actions_event(Vec::new())),
            forked_from_conversation_token: forked_from_conversation_token.clone(),
        }),
        OrderedTerminalEventType::CommandExecutionStarted { .. }
        | OrderedTerminalEventType::CommandExecutionFinished { .. }
        | OrderedTerminalEventType::Resize { .. }
        | OrderedTerminalEventType::AgentConversationReplayStarted
        | OrderedTerminalEventType::AgentConversationReplayEnded
        | OrderedTerminalEventType::CloudModeSetupPhaseEnded => None,
    }
}

/// What to send instead of an upstream message that exceeds the server's frame limit. Ordered
/// terminal events must keep their event number, so they are replaced by a placeholder; every
/// other message is simply not sent.
pub(super) fn replacement_for_oversized_message(
    message: &UpstreamMessage,
) -> Option<UpstreamMessage> {
    match message {
        UpstreamMessage::OrderedTerminalEvent(event) => {
            placeholder_for(&event.event_type).map(|event_type| {
                UpstreamMessage::OrderedTerminalEvent(OrderedTerminalEvent {
                    event_no: event.event_no,
                    event_type,
                })
            })
        }
        UpstreamMessage::Initialize(_)
        | UpstreamMessage::Ping { .. }
        | UpstreamMessage::EndSession { .. }
        | UpstreamMessage::ExtendSessionRetention { .. }
        | UpstreamMessage::UpdateActivePrompt(_)
        | UpstreamMessage::UpdateUniversalDeveloperInputContext(_)
        | UpstreamMessage::Reconnect(_)
        | UpstreamMessage::UpdateSelection(_)
        | UpstreamMessage::UpdateRole { .. }
        | UpstreamMessage::UpdateUserRole { .. }
        | UpstreamMessage::UpdatePendingUserRole { .. }
        | UpstreamMessage::RespondToRoleRequest { .. }
        | UpstreamMessage::UpdateAllRolesToReader { .. }
        | UpstreamMessage::UpdateInput(_)
        | UpstreamMessage::RejectInputUpdate { .. }
        | UpstreamMessage::RejectCommandExecutionRequest { .. }
        | UpstreamMessage::RejectWriteToPtyRequest { .. }
        | UpstreamMessage::RejectAgentPromptRequest { .. }
        | UpstreamMessage::RejectControlActionRequest { .. }
        | UpstreamMessage::UpdateLinkAccessLevel { .. }
        | UpstreamMessage::UpdateTeamAccessLevel { .. }
        | UpstreamMessage::AddGuests { .. }
        | UpstreamMessage::RemoveGuest { .. }
        | UpstreamMessage::RemovePendingGuest { .. } => None,
    }
}

/// A short, payload-free label for log messages.
pub(super) fn upstream_message_label(message: &UpstreamMessage) -> String {
    match message {
        UpstreamMessage::OrderedTerminalEvent(event) => {
            format!(
                "OrderedTerminalEvent({:?}, event_no={})",
                event.event_type, event.event_no
            )
        }
        UpstreamMessage::Initialize(_) => "Initialize".to_string(),
        UpstreamMessage::Ping { .. } => "Ping".to_string(),
        UpstreamMessage::EndSession { .. } => "EndSession".to_string(),
        UpstreamMessage::ExtendSessionRetention { .. } => "ExtendSessionRetention".to_string(),
        UpstreamMessage::UpdateActivePrompt(_) => "UpdateActivePrompt".to_string(),
        UpstreamMessage::UpdateUniversalDeveloperInputContext(_) => {
            "UpdateUniversalDeveloperInputContext".to_string()
        }
        UpstreamMessage::Reconnect(_) => "Reconnect".to_string(),
        UpstreamMessage::UpdateSelection(_) => "UpdateSelection".to_string(),
        UpstreamMessage::UpdateRole { .. } => "UpdateRole".to_string(),
        UpstreamMessage::UpdateUserRole { .. } => "UpdateUserRole".to_string(),
        UpstreamMessage::UpdatePendingUserRole { .. } => "UpdatePendingUserRole".to_string(),
        UpstreamMessage::RespondToRoleRequest { .. } => "RespondToRoleRequest".to_string(),
        UpstreamMessage::UpdateAllRolesToReader { .. } => "UpdateAllRolesToReader".to_string(),
        UpstreamMessage::UpdateInput(_) => "UpdateInput".to_string(),
        UpstreamMessage::RejectInputUpdate { .. } => "RejectInputUpdate".to_string(),
        UpstreamMessage::RejectCommandExecutionRequest { .. } => {
            "RejectCommandExecutionRequest".to_string()
        }
        UpstreamMessage::RejectWriteToPtyRequest { .. } => "RejectWriteToPtyRequest".to_string(),
        UpstreamMessage::RejectAgentPromptRequest { .. } => "RejectAgentPromptRequest".to_string(),
        UpstreamMessage::RejectControlActionRequest { .. } => {
            "RejectControlActionRequest".to_string()
        }
        UpstreamMessage::UpdateLinkAccessLevel { .. } => "UpdateLinkAccessLevel".to_string(),
        UpstreamMessage::UpdateTeamAccessLevel { .. } => "UpdateTeamAccessLevel".to_string(),
        UpstreamMessage::AddGuests { .. } => "AddGuests".to_string(),
        UpstreamMessage::RemoveGuest { .. } => "RemoveGuest".to_string(),
        UpstreamMessage::RemovePendingGuest { .. } => "RemovePendingGuest".to_string(),
    }
}

/// The number of bytes `UpstreamMessage::OrderedTerminalEvent` serializes to for `event_type`,
/// measured with the widest possible event number so the result is an upper bound for any number
/// the event is eventually assigned.
pub(super) fn ordered_terminal_event_wire_len(event_type: &OrderedTerminalEventType) -> usize {
    serialized_len(&UpstreamMessageMirror::OrderedTerminalEvent(
        OrderedTerminalEventMirror {
            event_no: usize::MAX,
            event_type,
        },
    ))
}

/// Borrowing mirrors of the protocol types that serialize byte-for-byte identically, so an event
/// can be measured without cloning it into an owned `UpstreamMessage`.
#[derive(Serialize)]
enum UpstreamMessageMirror<'a> {
    OrderedTerminalEvent(OrderedTerminalEventMirror<'a>),
}

#[derive(Serialize)]
struct OrderedTerminalEventMirror<'a> {
    event_no: usize,
    event_type: &'a OrderedTerminalEventType,
}

/// Serialized JSON length without allocating the output. A value that fails to serialize could
/// never be sent, so it is reported as infinitely large.
fn serialized_len<T: Serialize>(value: &T) -> usize {
    let mut counter = ByteCounter(0);
    match serde_json::to_writer(&mut counter, value) {
        Ok(()) => counter.0,
        Err(_) => usize::MAX,
    }
}

struct ByteCounter(usize);

impl io::Write for ByteCounter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0 += buf.len();
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
#[path = "message_size_tests.rs"]
mod tests;
