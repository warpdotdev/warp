use vec1::Vec1;
use warp_multi_agent_api as api;

use super::*;
use crate::ai::agent::conversation::{AIConversationId, MAX_RESTORED_COMMAND_BLOCKS};

/// Sets an explicit timestamp on a message so `RunShellCommand` extraction can order
/// commands deterministically across multiple conversations (see
/// `extract_command_blocks_from_messages`'s `start_ts` fallback to the tool call
/// message's own timestamp when `ShellCommandFinished.start_ts` is absent).
fn timestamped(mut message: api::Message, seconds: i64) -> api::Message {
    message.timestamp = Some(prost_types::Timestamp { seconds, nanos: 0 });
    message
}

/// Builds a restored conversation with `n` sequential, explicitly-timestamped
/// `RunShellCommand` tool calls/results (`echo 0`, `echo 1`, ... `echo {n-1}`), with the
/// i-th command timestamped at `base_seconds + i`.
fn conversation_with_timestamped_run_shell_commands(base_seconds: i64, n: usize) -> AIConversation {
    let mut messages = Vec::with_capacity(n * 2);
    for i in 0..n {
        let tool_call_id = format!("call-{i}");
        let command = format!("echo {i}");
        messages.push(timestamped(
            api::Message {
                fetched_memories: vec![],
                id: format!("tool-call-{i}"),
                task_id: "root-task".to_string(),
                server_message_data: String::new(),
                citations: vec![],
                message: Some(api::message::Message::ToolCall(api::message::ToolCall {
                    tool_call_id: tool_call_id.clone(),
                    tool: Some(api::message::tool_call::Tool::RunShellCommand(
                        api::message::tool_call::RunShellCommand {
                            command: command.clone(),
                            is_read_only: false,
                            uses_pager: false,
                            citations: vec![],
                            is_risky: false,
                            wait_until_complete_value: None,
                            risk_category: 0,
                        },
                    )),
                })),
                request_id: "req".to_string(),
                timestamp: None,
            },
            base_seconds + i as i64,
        ));
        messages.push(api::Message {
            fetched_memories: vec![],
            id: format!("tool-result-{i}"),
            task_id: "root-task".to_string(),
            server_message_data: String::new(),
            citations: vec![],
            message: Some(api::message::Message::ToolCallResult(
                api::message::ToolCallResult {
                    tool_call_id,
                    context: None,
                    result: Some(api::message::tool_call_result::Result::RunShellCommand(
                        #[allow(deprecated)]
                        api::RunShellCommandResult {
                            command: command.clone(),
                            output: i.to_string(),
                            exit_code: 0,
                            result: Some(api::run_shell_command_result::Result::CommandFinished(
                                api::ShellCommandFinished {
                                    command_id: format!("command-{i}"),
                                    output: i.to_string(),
                                    exit_code: 0,
                                    start_ts: None,
                                    finish_ts: None,
                                },
                            )),
                        },
                    )),
                },
            )),
            request_id: "req".to_string(),
            timestamp: None,
        });
    }

    AIConversation::new_restored(
        AIConversationId::new(),
        vec![api::Task {
            id: "root-task".to_string(),
            messages,
            dependencies: None,
            description: String::new(),
            summary: String::new(),
            server_data: String::new(),
        }],
        None,
    )
    .unwrap()
}

/// Regression test for an APP-5428 review finding: the restored-command-block cap must
/// be enforced once, in aggregate, across every conversation restored for one terminal
/// surface at startup -- not independently per conversation. Two conversations here are
/// each individually well under `MAX_RESTORED_COMMAND_BLOCKS`, so a per-conversation cap
/// would never trigger for either one; only an aggregate cap bounds their combined total.
#[test]
fn terminal_view_restored_blocks_applies_one_aggregate_cap_across_startup_conversations() {
    let per_conversation_commands = 300;
    assert!(
        per_conversation_commands < MAX_RESTORED_COMMAND_BLOCKS,
        "each conversation alone must stay under the cap, so only the aggregate cap can \
         be responsible for any truncation seen in this test"
    );

    // Chronologically disjoint: every command in `older` predates every command in `newer`.
    let older = conversation_with_timestamped_run_shell_commands(0, per_conversation_commands);
    let newer = conversation_with_timestamped_run_shell_commands(10_000, per_conversation_commands);

    let conversations = Vec1::try_from_vec(vec![older, newer]).unwrap();
    let restoration = Some(ConversationRestorationInNewPaneType::Startup {
        conversations,
        active_conversation_id: None,
    });

    let items = terminal_view_restored_blocks(None, &restoration)
        .expect("startup restoration with commands should produce restored blocks");

    let combined_total = per_conversation_commands * 2;
    let expected_truncated = combined_total - MAX_RESTORED_COMMAND_BLOCKS;

    // +1 for the synthetic aggregate truncation notice.
    assert_eq!(
        items.len(),
        MAX_RESTORED_COMMAND_BLOCKS + 1,
        "the combined command count across both conversations ({combined_total}) must be \
         capped once, not per conversation"
    );

    let SerializedBlockListItem::Command { block: notice } = &items[0];
    let notice_text = String::from_utf8_lossy(&notice.stylized_command).into_owned();
    assert!(
        notice_text.contains(&format!("{expected_truncated} earlier command")),
        "got: {notice_text:?}"
    );
    // The aggregate notice isn't scoped to either individual conversation's agent view.
    assert!(
        notice.agent_view_visibility.is_none(),
        "an aggregate, multi-conversation truncation notice must not be scoped to a \
         single conversation's agent view"
    );

    // The oldest retained command is from the "older" conversation, at the exact point
    // where the aggregate cap cut in -- not "all of conversation A", which is what the
    // old, purely-per-conversation cap would have produced (since 300 < the cap).
    let SerializedBlockListItem::Command { block: first_kept } = &items[1];
    let first_kept_command = String::from_utf8_lossy(&first_kept.stylized_command).into_owned();
    assert!(
        first_kept_command.contains(&format!("echo {expected_truncated}")),
        "got: {first_kept_command:?}"
    );

    // The newest retained command is the "newer" conversation's last command.
    let SerializedBlockListItem::Command { block: last_kept } = items.last().unwrap();
    let last_kept_command = String::from_utf8_lossy(&last_kept.stylized_command).into_owned();
    assert!(
        last_kept_command.contains(&format!("echo {}", per_conversation_commands - 1)),
        "got: {last_kept_command:?}"
    );
}
