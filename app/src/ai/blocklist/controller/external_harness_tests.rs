use std::time::SystemTime;

use uuid::Uuid;
use warp_multi_agent_api::client_action::{AddMessagesToTask, CreateTask};
use warp_multi_agent_api::message::tool_call::{RunShellCommand, Tool};
use warp_multi_agent_api::message::tool_call_result::Result as ToolResult;
use warp_multi_agent_api::message::{
    AgentOutput, Message as MessageKind, ToolCall, ToolCallResult, UserQuery,
};
use warp_multi_agent_api::response_event::{StreamFinished, stream_finished};
use warp_multi_agent_api::{
    ClientAction, Message, ResponseEvent, RunShellCommandResult, ShellCommandFinished, Task,
    client_action, response_event, run_shell_command_result,
};
use warpui::{App, AppContext};

use super::*;
use crate::ai::agent::conversation::{ConversationDriver, ConversationStatus};
use crate::ai::agent::{AIAgentActionId, AIAgentInput};
use crate::ai::blocklist::QueuedQueryModel;
use crate::ai::blocklist::controller::ParticipantId;
use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};

fn done() -> ResponseEvent {
    ResponseEvent {
        r#type: Some(response_event::Type::Finished(StreamFinished {
            reason: Some(stream_finished::Reason::Done(stream_finished::Done {})),
            ..Default::default()
        })),
    }
}

fn client_actions(actions: Vec<ClientAction>) -> ResponseEvent {
    ResponseEvent {
        r#type: Some(response_event::Type::ClientActions(
            response_event::ClientActions { actions },
        )),
    }
}

fn action(action: client_action::Action) -> ClientAction {
    ClientAction {
        action: Some(action),
    }
}

fn message(task_id: &str, request_id: &str, kind: MessageKind) -> Message {
    Message {
        id: Uuid::new_v4().to_string(),
        task_id: task_id.to_owned(),
        request_id: request_id.to_owned(),
        timestamp: Some(prost_types::Timestamp::from(SystemTime::now())),
        message: Some(kind),
        ..Default::default()
    }
}

fn add_messages(task_id: &str, messages: Vec<Message>) -> ClientAction {
    action(client_action::Action::AddMessagesToTask(
        AddMessagesToTask {
            task_id: task_id.to_owned(),
            messages,
        },
    ))
}

fn create_task(task_id: &str) -> ClientAction {
    action(client_action::Action::CreateTask(CreateTask {
        task: Some(Task {
            id: task_id.to_owned(),
            ..Default::default()
        }),
    }))
}

fn shell_tool_call(task_id: &str, request_id: &str, tool_call_id: &str) -> Message {
    message(
        task_id,
        request_id,
        MessageKind::ToolCall(ToolCall {
            tool_call_id: tool_call_id.to_owned(),
            tool: Some(Tool::RunShellCommand(RunShellCommand {
                command: "ls".to_owned(),
                ..Default::default()
            })),
        }),
    )
}

fn shell_tool_call_result(task_id: &str, request_id: &str, tool_call_id: &str) -> Message {
    #[allow(deprecated)]
    let result = RunShellCommandResult {
        command: "ls".to_owned(),
        output: "README.md".to_owned(),
        exit_code: 0,
        result: Some(run_shell_command_result::Result::CommandFinished(
            ShellCommandFinished {
                output: "README.md".to_owned(),
                exit_code: 0,
                command_id: tool_call_id.to_owned(),
                start_ts: None,
                finish_ts: None,
            },
        )),
    };
    message(
        task_id,
        request_id,
        MessageKind::ToolCallResult(ToolCallResult {
            tool_call_id: tool_call_id.to_owned(),
            context: None,
            result: Some(ToolResult::RunShellCommand(result)),
        }),
    )
}

fn root_task_id(ctx: &AppContext, id: AIConversationId) -> String {
    BlocklistAIHistoryModel::as_ref(ctx)
        .conversation(&id)
        .unwrap()
        .get_root_task_id()
        .to_string()
}

fn status(ctx: &AppContext, id: AIConversationId) -> ConversationStatus {
    BlocklistAIHistoryModel::as_ref(ctx)
        .conversation(&id)
        .unwrap()
        .status()
        .clone()
}

#[test]
fn shared_session_injections_go_to_the_prompt_sink_instead_of_the_queue() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let (sink, prompts) = async_channel::unbounded();
        terminal.update(&mut app, |terminal, ctx| {
            terminal.ai_controller().update(ctx, |controller, ctx| {
                let id = controller.bind_native_prompt_conversation(None, ctx);
                controller.set_external_harness_prompt_sink(sink);
                controller.execute_warp_agent_prompt_from_shared_session_injection(
                    "follow-up".into(),
                    None,
                    vec![],
                    ParticipantId::new(),
                    None,
                    ctx,
                );
                assert_eq!(prompts.try_recv().unwrap(), "follow-up");
                assert!(!QueuedQueryModel::as_ref(ctx).has_queue(id));
                assert_eq!(
                    BlocklistAIHistoryModel::as_ref(ctx)
                        .conversation(&id)
                        .unwrap()
                        .exchange_count(),
                    0,
                    "the harness owns the turn; nothing is sent to Warp's agent"
                );

                controller.unbind_native_prompt_conversation(ctx);
                assert!(
                    prompts.is_closed(),
                    "unbinding must drop the sink so the harness sees the channel close"
                );
            });
        });
    });
}

#[test]
fn a_turn_without_tool_calls_settles_when_its_stream_finishes() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |terminal, ctx| {
            terminal.ai_controller().update(ctx, |controller, ctx| {
                let id = controller.bind_native_prompt_conversation(None, ctx);
                let turn = controller
                    .begin_external_harness_turn(id, Some("run-1".into()), ctx)
                    .unwrap();
                assert_eq!(status(ctx, id), ConversationStatus::InProgress);

                let task_id = root_task_id(ctx, id);
                controller.apply_external_harness_event(
                    &turn.stream_id,
                    client_actions(vec![
                        create_task(&task_id),
                        add_messages(
                            &task_id,
                            vec![message(
                                &task_id,
                                &turn.request_id,
                                MessageKind::UserQuery(UserQuery {
                                    query: "hello".into(),
                                    ..Default::default()
                                }),
                            )],
                        ),
                        add_messages(
                            &task_id,
                            vec![message(
                                &task_id,
                                &turn.request_id,
                                MessageKind::AgentOutput(AgentOutput {
                                    text: "hi there".into(),
                                }),
                            )],
                        ),
                    ]),
                    ctx,
                );
                assert_eq!(status(ctx, id), ConversationStatus::InProgress);

                controller.apply_external_harness_event(&turn.stream_id, done(), ctx);
                assert_eq!(status(ctx, id), ConversationStatus::Success);

                let conversation = BlocklistAIHistoryModel::as_ref(ctx)
                    .conversation(&id)
                    .unwrap();
                let queries: Vec<_> = conversation
                    .root_task_exchanges()
                    .flat_map(|exchange| exchange.input.iter())
                    .filter_map(|input| match input {
                        AIAgentInput::UserQuery { query, .. } => Some(query.as_str()),
                        _ => None,
                    })
                    .collect();
                assert_eq!(queries, vec!["hello"]);
                assert_eq!(conversation.driver(), ConversationDriver::ExternalHarness);
            });
        });
    });
}

#[test]
fn tool_call_results_are_applied_as_finished_and_settle_in_the_next_stream() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |terminal, ctx| {
            terminal.ai_controller().update(ctx, |controller, ctx| {
                let id = controller.bind_native_prompt_conversation(None, ctx);
                let first = controller
                    .begin_external_harness_turn(id, None, ctx)
                    .unwrap();
                let task_id = root_task_id(ctx, id);
                controller.apply_external_harness_event(
                    &first.stream_id,
                    client_actions(vec![
                        create_task(&task_id),
                        add_messages(
                            &task_id,
                            vec![
                                message(
                                    &task_id,
                                    &first.request_id,
                                    MessageKind::UserQuery(UserQuery {
                                        query: "list files".into(),
                                        ..Default::default()
                                    }),
                                ),
                                shell_tool_call(&task_id, &first.request_id, "call-1"),
                            ],
                        ),
                    ]),
                    ctx,
                );
                controller.apply_external_harness_event(&first.stream_id, done(), ctx);
                assert_eq!(
                    status(ctx, id),
                    ConversationStatus::InProgress,
                    "a stream that announced a tool call leaves the turn open"
                );

                // The harness ran the tool itself; the result arrives in the next stream, as it
                // does from the MAA server.
                let second = controller
                    .begin_external_harness_turn(id, None, ctx)
                    .unwrap();
                controller.apply_external_harness_event(
                    &second.stream_id,
                    client_actions(vec![add_messages(
                        &task_id,
                        vec![
                            shell_tool_call_result(&task_id, &second.request_id, "call-1"),
                            message(
                                &task_id,
                                &second.request_id,
                                MessageKind::AgentOutput(AgentOutput {
                                    text: "There is one file.".into(),
                                }),
                            ),
                        ],
                    )]),
                    ctx,
                );
                assert!(
                    controller
                        .action_model
                        .as_ref(ctx)
                        .get_action_result(&AIAgentActionId::from("call-1".to_owned()))
                        .is_some(),
                    "the harness-supplied result must be recorded so the action model never \
                     tries to run the command locally"
                );

                controller.apply_external_harness_event(&second.stream_id, done(), ctx);
                assert_eq!(status(ctx, id), ConversationStatus::Success);
            });
        });
    });
}
