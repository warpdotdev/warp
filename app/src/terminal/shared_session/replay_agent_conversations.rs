use std::collections::HashMap;

use api::response_event::stream_finished as stream_finished_event;
use api::{client_action as api_client_action, response_event as api_response_event};
use warp_multi_agent_api::{self as api, ResponseEvent};

use crate::ai::agent::conversation::AIConversation;
use crate::ai::agent::task::TaskId;
use crate::ai::agent::{AIAgentExchange, MessageId};

// Reconstructs all response events from conversations for use in session sharing.
// These messages are used to replay conversations as if they were happening live.
pub fn reconstruct_response_events_from_conversations(
    conversations: &[AIConversation],
) -> Vec<ResponseEvent> {
    let mut events = vec![];

    // Build a map of message_id -> (task_id, message, conversation) for quick lookup
    let mut message_map: HashMap<MessageId, (&TaskId, &api::Message, &AIConversation)> =
        HashMap::new();
    for conversation in conversations {
        for task in conversation.all_tasks() {
            for message in task.messages() {
                message_map.insert(
                    MessageId::new(message.id.clone()),
                    (task.id(), message, conversation),
                );
            }
        }
    }

    // Collect all exchanges from all conversations and sort by start time
    let mut all_exchanges: Vec<(&AIConversation, &AIAgentExchange)> = conversations
        .iter()
        .flat_map(|conv| {
            conv.all_exchanges()
                .into_iter()
                .map(move |exchange| (conv, exchange))
        })
        .collect();
    all_exchanges.sort_by_key(|(_, exchange)| exchange.start_time);

    // Track which conversations have had their tasks created.
    // We need to send CreateTask on the first exchange to upgrade local task IDs
    // to server task IDs (required for AddMessagesToTask to find the correct task).
    let mut initialized_conversations = std::collections::HashSet::new();

    // For each exchange (in chronological order), emit events
    for (conversation, exchange) in all_exchanges {
        // Collect messages for this exchange in chronological order
        let mut exchange_messages: Vec<(&TaskId, &api::Message)> = exchange
            .added_message_ids
            .iter()
            .filter_map(|msg_id| {
                message_map
                    .get(msg_id)
                    .map(|(task_id, msg, _)| (*task_id, *msg))
            })
            .collect();

        if exchange_messages.is_empty() {
            continue;
        }

        // Sort by timestamp to ensure chronological order
        exchange_messages.sort_by_key(|(_, message)| {
            message.timestamp.as_ref().map(|ts| (ts.seconds, ts.nanos))
        });

        // Use the server conversation token if it's available.
        // Otherwise, fall back to the id that this conversation was forked from.
        // This ensures viewers can properly group historical exchanges together.
        let token = conversation
            .server_conversation_token()
            .or_else(|| conversation.forked_from_server_conversation_token())
            .map(|t| t.as_str().to_string())
            .unwrap_or_default();
        let request_id = exchange_messages
            .first()
            .map(|(_, msg)| msg.request_id.clone())
            .unwrap_or_default();

        // Start this exchange
        events.push(ResponseEvent {
            r#type: Some(api_response_event::Type::Init(
                api_response_event::StreamInit {
                    request_id,
                    conversation_id: token.clone(),
                    // Shared session replays don't need a run_id; the empty
                    // string is filtered to None by initialize_output_for_response_stream.
                    run_id: String::new(),
                },
            )),
        });

        // On the first exchange of each conversation, send CreateTask events to upgrade
        // local task IDs to server task IDs. We construct a task with empty messages
        // because the messages will be added via AddMessagesToTask below - including them
        // in CreateTask would cause duplicate content in the exchange.
        let conversation_id = conversation.id();
        let is_first_exchange = !initialized_conversations.contains(&conversation_id);
        if is_first_exchange {
            initialized_conversations.insert(conversation_id);
            for task in conversation.all_tasks() {
                if let Some(task_source) = task.source() {
                    let task_without_messages = api::Task {
                        id: task_source.id.clone(),
                        description: task_source.description.clone(),
                        dependencies: task_source.dependencies.clone(),
                        messages: vec![], // Empty - messages added via AddMessagesToTask
                        summary: task_source.summary.clone(),
                        server_data: task_source.server_data.clone(),
                    };
                    events.push(wrap_action_in_event(api_client_action::Action::CreateTask(
                        api_client_action::CreateTask {
                            task: Some(task_without_messages),
                        },
                    )));
                }
            }
        }

        // Send all messages for this exchange
        for (task_id, message) in exchange_messages {
            events.push(wrap_action_in_event(
                api_client_action::Action::AddMessagesToTask(
                    api_client_action::AddMessagesToTask {
                        task_id: task_id.to_string(),
                        messages: vec![message.clone()],
                    },
                ),
            ));
        }

        // Finish this exchange — but ONLY if it actually finished. If an
        // exchange is still in-flight when the scrollback is built, emitting a
        // synthetic Finished here corrupts the late-joining viewer's stream:
        // the viewer clears `current_response_id` and then drops every live
        // ClientAction that arrives for the same in-flight stream. Skipping
        // the synthetic Finished lets the live wire's real Finished close the
        // stream naturally for the viewer.
        if exchange.output_status.is_finished() {
            events.push(create_finished_event_from_conversation(conversation));
        }
    }

    events
}

/// Wrap a ClientAction in a ResponseEvent.
fn wrap_action_in_event(action: api_client_action::Action) -> ResponseEvent {
    ResponseEvent {
        r#type: Some(api_response_event::Type::ClientActions(
            api_response_event::ClientActions {
                actions: vec![api::ClientAction {
                    action: Some(action),
                }],
            },
        )),
    }
}

/// Creates a StreamFinished event from a conversation.
fn create_finished_event_from_conversation(conversation: &AIConversation) -> ResponseEvent {
    // Build conversation usage metadata from the conversation's metadata
    let usage_metadata = Some(
        api_response_event::stream_finished::ConversationUsageMetadata {
            context_window_usage: conversation.context_window_usage(),
            total_input_tokens: 0,
            credits_spent: conversation.inference_credits_spent(),
            platform_credits_spent: conversation.platform_credits_spent(),
            summarized: conversation.was_summarized(),
            #[allow(deprecated)]
            token_usage: conversation
                .token_usage()
                .iter()
                .map(|u| u.to_proto_combined())
                .collect(),
            tool_usage_metadata: Some(conversation.tool_usage_metadata().into()),
            warp_token_usage: conversation
                .token_usage()
                .iter()
                .filter_map(|u| u.to_proto_warp_usage())
                .collect(),
            byok_token_usage: conversation
                .token_usage()
                .iter()
                .filter_map(|u| u.to_proto_byok_usage())
                .collect(),
            custom_endpoint_token_usage: conversation
                .token_usage()
                .iter()
                .filter_map(|u| u.to_proto_custom_endpoint_usage())
                .collect(),
            context_window_segments: conversation
                .context_window_segments()
                .iter()
                .map(Into::into)
                .collect(),
        },
    );

    ResponseEvent {
        r#type: Some(api_response_event::Type::Finished(
            api_response_event::StreamFinished {
                reason: Some(stream_finished_event::Reason::Done(
                    stream_finished_event::Done {},
                )),
                conversation_usage_metadata: usage_metadata,
                token_usage: vec![],
                should_refresh_model_config: false,
                request_cost: None,
            },
        )),
    }
}
