// `panic!` causes the app to crash before debug info can be exported. Use `AssertionOutcome::immediate_failure` instead.
#![deny(clippy::panic)]
// `assert!` causes the app to crash before debug info can be exported. Use `integration_assert!` instead.
#![deny(clippy::assertions_on_constants)]

use warp_multi_agent_api as api;
use warpui::integration::{AssertionCallback, AssertionOutcome, TestStep};
use warpui::{EntityId, SingletonEntity as _, integration_assert};

use super::llm_judge::{LLMJudge, LLMJudgeConfig};
use crate::BlocklistAIHistoryModel;
use crate::ai::agent::conversation::{AIConversation, AIConversationId, ConversationStatus};
use crate::ai::agent::todos::AIAgentTodoList;
use crate::ai::agent::{
    AIAgentActionResultType, AIAgentActionType, AIAgentExchange, AIAgentInput,
    AIAgentOutputMessageType, AIAgentOutputStatus, AIAgentTextSection, FileEdit,
    FinishedAIAgentOutput, ReadFilesRequest, TodoOperation,
};
use crate::ai::llms::{LLMId, LLMPreferences};
use crate::integration_testing::view_getters::terminal_view;

type TextAssertion = Box<dyn Fn(&str) -> bool + 'static>;
type ActionAssertion = Box<dyn Fn(&AIAgentActionType) -> bool + 'static>;
type ActionResultAssertion = Box<dyn Fn(&AIAgentActionResultType) -> bool + 'static>;

/// The maximum duration of an individual exchange before we assume it's stalled.
#[allow(unused)]
const MAX_EXCHANGE_DURATION: chrono::Duration = chrono::Duration::minutes(18);

pub fn assert_exchange_succeeds(exchange_index: usize) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = terminal_view(app, window_id, 0, 0);
        BlocklistAIHistoryModel::handle(app).update(app, |history_model, _| {
            exchange_succeeds_with_expected_output(
                None,
                None,
                ConversationTarget::Active,
                terminal_view.id(),
                exchange_index,
                history_model,
            )
        })
    })
}

// Make an assertion on the text output in the exchange at exchange_index.
// This assumes the AI block has completed successfully.
pub fn assert_exchange_text(
    exchange_index: usize,
    assertion: impl Fn(&str) -> bool + 'static + Clone,
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = terminal_view(app, window_id, 0, 0);
        BlocklistAIHistoryModel::handle(app).update(app, |history_model, _| {
            exchange_succeeds_with_expected_output(
                Some(Box::new(assertion.clone())),
                None,
                ConversationTarget::Active,
                terminal_view.id(),
                exchange_index,
                history_model,
            )
        })
    })
}

// Helper function to assert that the exchange text contains a specific string.
pub fn assert_exchange_text_contains(
    exchange_index: usize,
    expected_text: &'static str,
) -> AssertionCallback {
    assert_exchange_text(exchange_index, move |text| text.contains(expected_text))
}

// Make an assertion on the text output in the latest exchange.
// This assumes the AI block has completed successfully.
pub fn assert_latest_exchange_text(
    assertion: impl Fn(&str) -> bool + 'static + Clone,
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = terminal_view(app, window_id, 0, 0);
        BlocklistAIHistoryModel::handle(app).update(app, |history_model, _| {
            let exchange_count = get_exchange_count(terminal_view.id(), history_model);
            exchange_succeeds_with_expected_output(
                Some(Box::new(assertion.clone())),
                None,
                ConversationTarget::Active,
                terminal_view.id(),
                exchange_count - 1,
                history_model,
            )
        })
    })
}

// Make an assertion on the action requested in the exchange at exchange_index.
/// This is private because `AIAgentActionType` is not public outside the warp app crate
/// for use within agent mode evals, so they can't write the `ActionAssertion` directly.
/// We need to define other functions for specific action assertions that don't expose the type.
/// TODO: consider exposing `AIAgentActionType`
fn assert_exchange_action(
    conversation_target: ConversationTarget,
    exchange_index: usize,
    assertion: impl Fn(&AIAgentActionType) -> bool + 'static + Clone,
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = terminal_view(app, window_id, 0, 0);
        BlocklistAIHistoryModel::handle(app).update(app, |history_model, _| {
            exchange_succeeds_with_expected_output(
                None,
                Some(Box::new(assertion.clone())),
                conversation_target,
                terminal_view.id(),
                exchange_index,
                history_model,
            )
        })
    })
}

pub fn assert_exchange_action_result(
    exchange_index: usize,
    assertion: impl Fn(&AIAgentActionResultType) -> bool + 'static + Clone,
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = terminal_view(app, window_id, 0, 0);
        BlocklistAIHistoryModel::handle(app).update(app, |history_model, _| {
            exchange_with_expected_action_result(
                Some(Box::new(assertion.clone())),
                ConversationTarget::Active,
                terminal_view.id(),
                exchange_index,
                history_model,
            )
        })
    })
}

// Asserts on the todo operations in the latest exchange.
// Will check continuously until the assertion succeeds or it times out.
// The assertion is run against all todo operations in the exchange.
pub fn assert_todo_operation(
    assertion: impl Fn(&TodoOperation) -> bool + 'static + Clone,
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = terminal_view(app, window_id, 0, 0);
        BlocklistAIHistoryModel::handle(app).update(app, |history_model, _| {
            match get_exchange_todo_operations(
                ConversationTarget::Active,
                terminal_view.id(),
                get_exchange_count(terminal_view.id(), history_model) - 1,
                history_model,
            ) {
                Ok(todo_operations) => {
                    if todo_operations.is_empty() {
                        return AssertionOutcome::failure(
                            "Exchange output has no todo operations".to_owned(),
                        );
                    };

                    // Run assertion against all todo operations
                    for todo_operation in &todo_operations {
                        if assertion(todo_operation) {
                            return AssertionOutcome::Success;
                        }
                    }

                    AssertionOutcome::failure(format!(
                        "No todo operations match assertion. Found operations: {todo_operations:?}"
                    ))
                }
                Err(outcome) => outcome,
            }
        })
    })
}

pub fn assert_any_exchange_text(
    assertion: impl Fn(&str) -> bool + 'static + Clone,
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = terminal_view(app, window_id, 0, 0);
        BlocklistAIHistoryModel::handle(app).update(app, |history_model, _| {
            let exchange_count = get_exchange_count(terminal_view.id(), history_model);
            (0..exchange_count)
                .map(|exchange_index| {
                    exchange_succeeds_with_expected_output(
                        Some(Box::new(assertion.clone())),
                        None,
                        ConversationTarget::Active,
                        terminal_view.id(),
                        exchange_index,
                        history_model,
                    )
                })
                .find(|outcome| matches!(outcome, AssertionOutcome::Success))
                .unwrap_or(AssertionOutcome::failure(
                    "No exchanges match assertion".to_owned(),
                ))
        })
    })
}

pub fn assert_exchange_action_is_grep(exchange_index: usize) -> AssertionCallback {
    assert_exchange_action(ConversationTarget::Active, exchange_index, |action| {
        matches!(action, AIAgentActionType::Grep { .. })
    })
}

pub fn assert_exchange_action_reads_file(
    exchange_index: usize,
    file_path_assertion: impl Fn(&str) -> bool + 'static + Clone,
) -> AssertionCallback {
    assert_exchange_action(ConversationTarget::Active, exchange_index, move |action| {
        let AIAgentActionType::ReadFiles(ReadFilesRequest { locations, .. }) = action else {
            return false;
        };
        locations
            .iter()
            .any(|location| file_path_assertion(&location.name))
    })
}

pub fn assert_exchange_requests_command(
    exchange_index: usize,
    command_assertion: impl Fn(&str) -> bool + 'static + Clone,
) -> AssertionCallback {
    assert_exchange_action(
        ConversationTarget::Active,
        exchange_index,
        Box::new(move |action: &AIAgentActionType| {
            let AIAgentActionType::RequestCommandOutput { command, .. } = action else {
                return false;
            };
            command_assertion(command)
        }),
    )
}

pub fn assert_exchange_requests_file_edit(
    conversation_target: ConversationTarget,
    exchange_index: usize,
) -> AssertionCallback {
    assert_exchange_action(conversation_target, exchange_index, move |action| {
        let AIAgentActionType::RequestFileEdits { .. } = action else {
            return false;
        };
        true
    })
}

/// Asserts on the last requested command input in the conversation.
/// Searches backwards through all exchanges to find the most recent one that contains a RequestCommandOutput action.
/// Errors if no requested command is found in the conversation.
pub fn assert_last_requested_command(
    command_assertion: impl Fn(&str) -> bool + 'static + Clone,
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = terminal_view(app, window_id, 0, 0);
        BlocklistAIHistoryModel::handle(app).update(app, |history_model, _| {
            let conversation = match get_conversation(
                ConversationTarget::Active,
                terminal_view.id(),
                history_model,
            ) {
                Ok(conversation) => conversation,
                Err(assertion) => return assertion,
            };

            let exchanges = conversation.all_exchanges();
            if exchanges.is_empty() {
                return AssertionOutcome::immediate_failure(
                    "No exchanges found in conversation".to_owned(),
                );
            }

            // Search backwards through exchanges to find the last requested command
            for exchange in exchanges.iter().rev() {
                let AIAgentOutputStatus::Finished { finished_output } = &exchange.output_status
                else {
                    continue; // Skip unfinished exchanges
                };

                match finished_output {
                    FinishedAIAgentOutput::Success { output } => {
                        let ai_output = output.get();
                        // Look for RequestCommandOutput action in this exchange
                        for action in ai_output.actions() {
                            if let AIAgentActionType::RequestCommandOutput { command, .. } =
                                &action.action
                            {
                                // Found the last requested command, apply the assertion
                                integration_assert!(
                                    command_assertion(command),
                                    "Last requested command does not match assertion: {command}"
                                );
                                return AssertionOutcome::Success;
                            }
                        }
                    }
                    _ => continue, // Skip failed or cancelled exchanges
                }
            }

            // No command found in any exchange
            AssertionOutcome::immediate_failure(
                "No requested command found in conversation".to_owned(),
            )
        })
    })
}

/// Searches through all exchanges to find a requested command that matches the assertion.
/// Errors if no matching requested command is found in the conversation.
pub fn assert_any_requested_command(
    command_assertion: impl Fn(&str) -> bool + 'static + Clone,
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = terminal_view(app, window_id, 0, 0);
        BlocklistAIHistoryModel::handle(app).update(app, |history_model, _| {
            let conversation = match get_conversation(
                ConversationTarget::Active,
                terminal_view.id(),
                history_model,
            ) {
                Ok(conversation) => conversation,
                Err(assertion) => return assertion,
            };

            let exchanges = conversation.all_exchanges();
            if exchanges.is_empty() {
                return AssertionOutcome::immediate_failure(
                    "No exchanges found in conversation".to_owned(),
                );
            }

            // Search through exchanges to find a matching requested command
            if exchanges.iter().any(|exchange| {
                let AIAgentOutputStatus::Finished { finished_output } = &exchange.output_status
                else {
                    return false;
                };
                let FinishedAIAgentOutput::Success { output } = finished_output else {
                    return false;
                };
                output.get().actions().any(|action| {
                    let AIAgentActionType::RequestCommandOutput { command, .. } = &action.action
                    else {
                        return false;
                    };
                    command_assertion(command)
                })
            }) {
                AssertionOutcome::Success
            } else {
                AssertionOutcome::immediate_failure(
                    "No matching requested command found in conversation".to_owned(),
                )
            }
        })
    })
}

/// Fails if any requested command in the conversation matches the given predicate.
/// Succeeds if no commands match (or if there are no commands).
pub fn assert_no_requested_command(
    forbidden_predicate: impl Fn(&str) -> bool + 'static + Clone,
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = terminal_view(app, window_id, 0, 0);
        BlocklistAIHistoryModel::handle(app).update(app, |history_model, _| {
            let conversation = match get_conversation(
                ConversationTarget::Active,
                terminal_view.id(),
                history_model,
            ) {
                Ok(conversation) => conversation,
                Err(assertion) => return assertion,
            };

            let exchanges = conversation.all_exchanges();

            // Search through exchanges to find any command matching the forbidden predicate
            for exchange in exchanges.iter() {
                let AIAgentOutputStatus::Finished { finished_output } = &exchange.output_status
                else {
                    continue;
                };
                let FinishedAIAgentOutput::Success { output } = finished_output else {
                    continue;
                };
                for action in output.get().actions() {
                    if let AIAgentActionType::RequestCommandOutput { command, .. } = &action.action
                        && forbidden_predicate(command)
                    {
                        return AssertionOutcome::immediate_failure(format!(
                            "Found forbidden command: {command}"
                        ));
                    }
                }
            }

            AssertionOutcome::Success
        })
    })
}

/// Assert that any file edit across exchanges matches a given assertion.
pub fn assert_any_exchange_requests_file_edit(
    file_edits_assertion: impl Fn(&Vec<FileEdit>) -> bool + 'static + Clone,
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = terminal_view(app, window_id, 0, 0);
        BlocklistAIHistoryModel::handle(app).update(app, |history_model, _| {
            let conversation = match get_conversation(
                ConversationTarget::Active,
                terminal_view.id(),
                history_model,
            ) {
                Ok(conversation) => conversation,
                Err(assertion) => return assertion,
            };

            let exchanges = conversation.all_exchanges();
            if exchanges.is_empty() {
                return AssertionOutcome::immediate_failure(
                    "No exchanges found in conversation".to_owned(),
                );
            }

            // Collect all file edits from all exchanges
            let mut all_file_edits = Vec::new();
            for exchange in exchanges.iter() {
                let AIAgentOutputStatus::Finished { finished_output } = &exchange.output_status
                else {
                    continue; // Skip unfinished exchanges
                };
                let FinishedAIAgentOutput::Success { output } = finished_output else {
                    continue; // Skip failed or cancelled exchanges
                };

                for action in output.get().actions() {
                    if let AIAgentActionType::RequestFileEdits { file_edits, .. } = &action.action {
                        all_file_edits.extend(file_edits.clone());
                    }
                }
            }

            if file_edits_assertion(&all_file_edits) {
                AssertionOutcome::Success
            } else {
                AssertionOutcome::immediate_failure(format!(
                    "File edits do not match assertion. Found {} file edits",
                    all_file_edits.len()
                ))
            }
        })
    })
}

/// Asserts on the todo items in the active conversation's todo list.
pub fn assert_todo_list(
    assertion: impl Fn(&AIAgentTodoList) -> bool + 'static + Clone,
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = terminal_view(app, window_id, 0, 0);
        BlocklistAIHistoryModel::handle(app).update(app, |history_model, _| {
            let Some(conversation) = history_model.active_conversation(terminal_view.id()) else {
                return AssertionOutcome::failure("No active conversation".to_owned());
            };
            let Some(todo_list) = conversation.active_todo_list() else {
                return AssertionOutcome::failure("No todo list".to_owned());
            };

            if assertion(todo_list) {
                AssertionOutcome::Success
            } else {
                AssertionOutcome::failure(format!(
                    "Todo items do not match assertion: {todo_list:?}"
                ))
            }
        })
    })
}

pub fn assert_exchange_has_trigger_suggest_prompt(exchange_index: usize) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = terminal_view(app, window_id, 0, 0);
        BlocklistAIHistoryModel::handle(app).update(app, |history_model, _| {
            let exchange = get_exchange_index_from_conversation(
                ConversationTarget::Only,
                terminal_view.id(),
                exchange_index,
                history_model,
            );
            let Ok(exchange) = exchange else {
                return AssertionOutcome::immediate_failure("Exchange not found".to_string());
            };
            match exchange.input.last() {
                Some(AIAgentInput::TriggerPassiveSuggestion { .. }) => AssertionOutcome::Success,
                Some(_) => AssertionOutcome::immediate_failure(
                    "Exchange input is not a trigger suggest prompt".to_string(),
                ),
                None => AssertionOutcome::immediate_failure("Exchange input is empty".to_string()),
            }
        })
    })
}

pub fn assert_exchange_action_is_suggested_prompt(exchange_index: usize) -> AssertionCallback {
    assert_exchange_action(ConversationTarget::Only, exchange_index, move |action| {
        let AIAgentActionType::SuggestPrompt { .. } = action else {
            return false;
        };
        true
    })
}

/// Asserts that no exchange that finished successfully contains the SuggestPrompt action.
pub fn assert_no_suggested_prompt() -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = terminal_view(app, window_id, 0, 0);
        BlocklistAIHistoryModel::handle(app).update(app, |history_model, _| {
            let mut exchanges =
                history_model.all_live_root_task_exchanges_for_terminal_surface(terminal_view.id());

            if exchanges.any(|exchange| {
                let AIAgentOutputStatus::Finished { finished_output } = &exchange.output_status
                else {
                    return false;
                };
                let FinishedAIAgentOutput::Success { output } = finished_output else {
                    return false;
                };
                output.get().actions().any(|action| {
                    let AIAgentActionType::SuggestPrompt { .. } = &action.action else {
                        return false;
                    };
                    true
                })
            }) {
                AssertionOutcome::immediate_failure(
                    "Suggested prompt found in conversation".to_owned(),
                )
            } else {
                AssertionOutcome::Success
            }
        })
    })
}

/// Specifies which conversations to check for task completion.
#[derive(Debug, Clone, Copy)]
pub enum ConversationTarget {
    /// Check only the active conversation in the terminal view
    Active,
    /// Get the only conversation in this terminal view (active/passive) or throw if multiple conversations were found
    Only,
    /// Check a specific conversation by ID in the terminal view
    ById(AIConversationId),
}

/// Fails immediately on any error response.
pub fn assert_turned_finish_with_success(
    conversation_target: ConversationTarget,
    max_number_of_turns: Option<usize>,
) -> AssertionCallback {
    assert_latest_task_is_done(max_number_of_turns, conversation_target, |status| {
        matches!(status, ConversationStatus::Success)
    })
}

/// Fails immediately on any error response.
pub fn assert_latest_task_succeeds_or_blocked(
    conversation_target: ConversationTarget,
    max_number_of_turns: Option<usize>,
) -> AssertionCallback {
    assert_latest_task_is_done(max_number_of_turns, conversation_target, |status| {
        matches!(status, ConversationStatus::Success)
            || matches!(status, ConversationStatus::Blocked { .. })
    })
}

/// Asserts that the conversation is blocked (waiting for user approval on an action).
/// Fails immediately on any error response.
pub fn assert_task_is_blocked(conversation_target: ConversationTarget) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = terminal_view(app, window_id, 0, 0);
        BlocklistAIHistoryModel::handle(app).update(app, |history_model, _| {
            let conversation =
                match get_conversation(conversation_target, terminal_view.id(), history_model) {
                    Ok(conversation) => conversation,
                    Err(assertion) => return assertion,
                };

            // Check if the latest exchange ended with an API error
            if let Some(api_error_outcome) = check_for_api_error_in_latest_exchange(conversation) {
                return api_error_outcome;
            }

            let status = conversation.status();
            if status.is_blocked() {
                AssertionOutcome::Success
            } else if status.is_in_progress() {
                AssertionOutcome::failure("Task is still in progress".to_owned())
            } else {
                AssertionOutcome::immediate_failure(format!(
                    "Expected task to be blocked, but status is {status:?}"
                ))
            }
        })
    })
}

/// Check if a conversation has ended with an API error in its latest exchange.
fn check_for_api_error_in_latest_exchange(
    conversation: &AIConversation,
) -> Option<AssertionOutcome> {
    if let Some(latest_exchange) = conversation.latest_exchange()
        && let AIAgentOutputStatus::Finished {
            finished_output: FinishedAIAgentOutput::Error { error, .. },
        } = &latest_exchange.output_status
    {
        return Some(AssertionOutcome::immediate_failure(format!(
            "Conversation ended with API error: {error:?}"
        )));
    }
    None
}

fn get_conversation(
    conversation_target: ConversationTarget,
    terminal_view_id: EntityId,
    history_model: &BlocklistAIHistoryModel,
) -> Result<&AIConversation, AssertionOutcome> {
    match &conversation_target {
        ConversationTarget::Active => {
            if let Some(active_conv) = history_model.active_conversation(terminal_view_id) {
                Ok(active_conv)
            } else {
                Err(AssertionOutcome::failure(
                    "No active conversation".to_owned(),
                ))
            }
        }
        ConversationTarget::Only => {
            // Get all conversations (including passive ones)
            let mut conversations: Vec<_> = history_model
                .all_live_conversations_for_terminal_surface(terminal_view_id)
                .collect();
            match conversations.len() {
                1 => conversations.pop().ok_or(AssertionOutcome::failure(
                    "Expected one conversation, but vector was empty".to_string(),
                )),
                0 => Err(AssertionOutcome::failure(
                    "No conversations were found".to_string(),
                )),
                _ => Err(AssertionOutcome::failure(
                    "Multiple conversations were found".to_string(),
                )),
            }
        }
        ConversationTarget::ById(conversation_id) => {
            if let Some(conversation) = history_model.conversation(conversation_id) {
                Ok(conversation)
            } else {
                Err(AssertionOutcome::failure(format!(
                    "Conversation with ID {conversation_id:?} not found in terminal view"
                )))
            }
        }
    }
}

/// Fails immediately on any error response.
pub fn assert_latest_task_is_done(
    max_number_of_turns: Option<usize>,
    conversation_target: ConversationTarget,
    match_fn: impl Fn(&ConversationStatus) -> bool + 'static,
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = terminal_view(app, window_id, 0, 0);
        BlocklistAIHistoryModel::handle(app).update(app, |history_model, _| {
            let conversation = match get_conversation(conversation_target, terminal_view.id(), history_model) {
                Ok(conversation) => conversation,
                Err(assertion) => return assertion,
            };

            if let Some(max_number_of_turns) = max_number_of_turns {
                integration_assert!(
                    conversation.all_exchanges().len() <= max_number_of_turns,
                    "Conversation exceeded max number of turns before task succeeded: {max_number_of_turns}"
                );
            }

            // Check if the latest exchange ended with an API error
            if let Some(api_error_outcome) = check_for_api_error_in_latest_exchange(conversation) {
                return api_error_outcome;
            }

            // If the task is done and matches our criteria, return success
            if !conversation.status().is_in_progress() {
                let status = conversation.status();
                if match_fn(status) {
                    return AssertionOutcome::Success;
                } else {
                    return AssertionOutcome::failure(format!(
                        "Conversation is done but does not match expected status - is {status:?}",
                    ));
                }
            }

            // If we get here, no conversations had completed tasks matching our criteria
            AssertionOutcome::failure("Task is not done in target conversation".to_owned())
        })
    })
}

pub fn hydrate_ai_conversation_assertion(tasks: Vec<api::Task>) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = terminal_view(app, window_id, 0, 0);
        let tasks = tasks.clone();

        terminal_view.update(app, |terminal_view, ctx| {
            let task_list = api::ConversationData {
                tasks,
                ..Default::default()
            };

            terminal_view.load_conversation_from_tasks(task_list, ctx);
            AssertionOutcome::Success
        })
    })
}

pub fn assert_llm_judge_whole_conversation_passes(
    judge_config: LLMJudgeConfig,
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = terminal_view(app, window_id, 0, 0);
        BlocklistAIHistoryModel::handle(app).update(app, |history_model, _| {
            // Async assertions to wait until this exchange is finished.
            let Some(conversation) = history_model.active_conversation(terminal_view.id()) else {
                return AssertionOutcome::immediate_failure(
                    "Failed to get conversation".to_owned(),
                );
            };

            let judge = LLMJudge::new(judge_config.clone());
            let result = judge.judge(conversation);
            match result {
                Ok(judge_result) => {
                    if !judge_result.pass {
                        AssertionOutcome::immediate_failure(format!(
                            "LLM judged conversation to fail: {}",
                            judge_result.critique
                        ))
                    } else {
                        // TODO: we want to record critiques on success
                        println!("LLM judged conversation to pass: {}", judge_result.critique);
                        AssertionOutcome::Success
                    }
                }
                Err(e) => AssertionOutcome::immediate_failure(format!(
                    "Failed to judge conversation: {e:?}"
                )),
            }
        })
    })
}

/// Passes the text of the final exchange in the active conversation to `f` and
/// returns `f`'s result. Returns an immediate failure if no exchanges are found
/// or the output cannot be retrieved. Intended for eval assertion callbacks
/// that need to inspect the agent's last message without accessing the
/// conversation model directly.
pub fn assert_on_final_exchange_text(
    f: impl Fn(&str) -> AssertionOutcome + 'static,
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = terminal_view(app, window_id, 0, 0);
        BlocklistAIHistoryModel::handle(app).update(app, |history_model, _| {
            let exchange_count = get_exchange_count(terminal_view.id(), history_model);
            if exchange_count == 0 {
                return AssertionOutcome::immediate_failure(
                    "No exchanges found in conversation".to_owned(),
                );
            }
            let (text, _) = match get_exchange_output(
                ConversationTarget::Active,
                terminal_view.id(),
                exchange_count - 1,
                history_model,
            ) {
                Ok(output) => output,
                Err(outcome) => return outcome,
            };
            f(&text)
        })
    })
}

/// Returns a test step that asserts `model` is a valid agent-mode LLM ID.
/// Using a step rather than panicking inside `set_preferred_agent_mode_llm`
/// keeps a misconfigured judge run inspectable (the harness can still export
/// debug output) rather than aborting with a panic.
pub fn validate_agent_mode_llm_step(model: &'static str) -> TestStep {
    let llm_id = LLMId::from(model);
    TestStep::new(&format!("Validate judge model '{model}'")).add_named_assertion(
        "Judge model is an available agent-mode LLM",
        move |app, _window_id| {
            let llm_id = llm_id.clone();
            let is_available = LLMPreferences::handle(app).read(app, |llm_preferences, _| {
                llm_preferences.is_available_agent_mode_llm(&llm_id)
            });
            if is_available {
                AssertionOutcome::Success
            } else {
                AssertionOutcome::immediate_failure(format!(
                    "Judge model '{llm_id}' is not a valid agent mode LLM"
                ))
            }
        },
    )
}

/// Assert that the target conversation contains no actions.
pub fn assert_conversation_contains_no_actions(
    conversation_target: ConversationTarget,
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = terminal_view(app, window_id, 0, 0);
        BlocklistAIHistoryModel::handle(app).update(app, |history_model, _| {
            let conversation =
                match get_conversation(conversation_target, terminal_view.id(), history_model) {
                    Ok(conversation) => conversation,
                    Err(assertion) => return assertion,
                };

            for exchange in conversation.all_exchanges().into_iter() {
                if let AIAgentOutputStatus::Finished {
                    finished_output: FinishedAIAgentOutput::Success { output },
                } = &exchange.output_status
                    && output.get().actions().next().is_some()
                {
                    return AssertionOutcome::immediate_failure(
                        "Expected no actions in conversation, but found at least one".to_owned(),
                    );
                }
            }

            AssertionOutcome::Success
        })
    })
}

/// Assert that any exchange in the target conversation performed a ReadFiles action
/// whose file path matches the provided predicate.
pub fn assert_conversation_reads_file(
    conversation_target: ConversationTarget,
    file_path_assertion: impl Fn(&str) -> bool + 'static + Clone,
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = terminal_view(app, window_id, 0, 0);
        BlocklistAIHistoryModel::handle(app).update(app, |history_model, _| {
            let conversation =
                match get_conversation(conversation_target, terminal_view.id(), history_model) {
                    Ok(conversation) => conversation,
                    Err(assertion) => return assertion,
                };

            for exchange in conversation.all_exchanges().into_iter() {
                if let AIAgentOutputStatus::Finished {
                    finished_output: FinishedAIAgentOutput::Success { output },
                } = &exchange.output_status
                {
                    for action in output.get().actions() {
                        if let AIAgentActionType::ReadFiles(ReadFilesRequest {
                            locations, ..
                        }) = &action.action
                        {
                            for location in locations.iter() {
                                if file_path_assertion(&location.name) {
                                    return AssertionOutcome::Success;
                                }
                            }
                        }
                    }
                }
            }

            AssertionOutcome::failure(
                "No ReadFiles action matched the file path assertion in conversation".to_owned(),
            )
        })
    })
}

/// Assert that the exchange at exchange_index finishes successfully.
/// Optionally make assertions on the text and action of the output.
fn exchange_succeeds_with_expected_output(
    text_assertion: Option<TextAssertion>,
    action_assertion: Option<ActionAssertion>,
    conversation_target: ConversationTarget,
    terminal_view_id: EntityId,
    exchange_index: usize,
    history_model: &BlocklistAIHistoryModel,
) -> AssertionOutcome {
    match get_exchange_output(
        conversation_target,
        terminal_view_id,
        exchange_index,
        history_model,
    ) {
        Ok((text, action_type)) => {
            if let Some(text_assertion) = &text_assertion {
                integration_assert!(
                    text_assertion(&text),
                    "Exchange output text does not match assertion: {text}"
                );
            }
            if let Some(action_assertion) = &action_assertion {
                let Some(action_type) = action_type else {
                    return AssertionOutcome::immediate_failure(
                        "Exchange output has no actions".to_owned(),
                    );
                };
                integration_assert!(
                    action_assertion(&action_type),
                    "Exchange action does not match assertion: {action_type:?}"
                );
            }
            AssertionOutcome::Success
        }
        Err(outcome) => outcome,
    }
}

/// Get all todo operations from the exchange at exchange_index.
/// Returns an error if the exchange is not finished.
fn get_exchange_todo_operations(
    conversation_target: ConversationTarget,
    terminal_view_id: EntityId,
    exchange_index: usize,
    history_model: &BlocklistAIHistoryModel,
) -> Result<Vec<TodoOperation>, AssertionOutcome> {
    let exchange = get_exchange_index_from_conversation(
        conversation_target,
        terminal_view_id,
        exchange_index,
        history_model,
    )?;

    let AIAgentOutputStatus::Finished { finished_output } = &exchange.output_status else {
        return Err(AssertionOutcome::failure(
            "Exchange is not finished".to_owned(),
        ));
    };

    // Once the output is finished, we make plain assertions to immediately fail the test on failure, instead
    // of waiting for something to change.
    match finished_output {
        FinishedAIAgentOutput::Success { output } => {
            let ai_output = output.get();
            let todo_operations = ai_output.todo_operations().cloned().collect::<Vec<_>>();

            Ok(todo_operations)
        }
        FinishedAIAgentOutput::Error { error, .. } => Err(AssertionOutcome::immediate_failure(
            format!("Exchange failed with error: {error:?}"),
        )),
        FinishedAIAgentOutput::Cancelled { .. } => Err(AssertionOutcome::immediate_failure(
            "Exchange was cancelled".to_owned(),
        )),
    }
}

/// Get the text and action type of the exchange at exchange_index.
/// Returns an error if the exchange is not finished.
fn get_exchange_output(
    conversation_target: ConversationTarget,
    terminal_view_id: EntityId,
    exchange_index: usize,
    history_model: &BlocklistAIHistoryModel,
) -> Result<(String, Option<AIAgentActionType>), AssertionOutcome> {
    let exchange = get_exchange_index_from_conversation(
        conversation_target,
        terminal_view_id,
        exchange_index,
        history_model,
    )?;

    let AIAgentOutputStatus::Finished { finished_output } = &exchange.output_status else {
        return Err(AssertionOutcome::failure(
            "Exchange is not finished".to_owned(),
        ));
    };

    // Once the output is finished, we make plain assertions to immediately fail the test on failure, instead
    // of waiting for something to change.
    match finished_output {
        FinishedAIAgentOutput::Success { output } => {
            let ai_output = output.get();
            let text = ai_output
                .messages
                .iter()
                .filter_map(|message| match &message.message {
                    AIAgentOutputMessageType::Text(text) => Some(text),
                    AIAgentOutputMessageType::Summarization { text, .. } => Some(text),
                    _ => None,
                })
                .flat_map(|text| text.sections.iter())
                .flat_map(|section| match section {
                    AIAgentTextSection::PlainText { text } => Some(text.text().to_string()),
                    AIAgentTextSection::Code { code, language, .. } => {
                        let language_str = language
                            .as_ref()
                            .map_or("text".to_string(), |lang| lang.to_string());
                        Some(format!("```{language_str}\n{code}\n```"))
                    }
                    AIAgentTextSection::Table { table } => Some(table.markdown_source.clone()),
                    AIAgentTextSection::Image { image } => Some(image.markdown_source.clone()),
                    AIAgentTextSection::MermaidDiagram { diagram } => {
                        Some(diagram.markdown_source.clone())
                    }
                })
                .collect::<Vec<_>>()
                .join("");
            let action_type = ai_output
                .actions()
                .next()
                .map(|action| action.action.clone());

            Ok((text, action_type))
        }
        FinishedAIAgentOutput::Error { error, .. } => Err(AssertionOutcome::immediate_failure(
            format!("Exchange failed with error: {error:?}"),
        )),
        FinishedAIAgentOutput::Cancelled { .. } => Err(AssertionOutcome::immediate_failure(
            "Exchange was cancelled".to_owned(),
        )),
    }
}

/// Assert that the exchange at exchange_index contains the expected action result.
#[allow(dead_code)]
fn exchange_with_expected_action_result(
    action_result_assertion: Option<ActionResultAssertion>,
    conversation_target: ConversationTarget,
    terminal_view_id: EntityId,
    exchange_index: usize,
    history_model: &BlocklistAIHistoryModel,
) -> AssertionOutcome {
    match get_exchange_action_result_input(
        conversation_target,
        terminal_view_id,
        exchange_index,
        history_model,
    ) {
        Ok(action_result) => {
            if let Some(action_result_assertion) = &action_result_assertion {
                integration_assert!(
                    action_result_assertion(&action_result),
                    "Exchange input does not match assertion: {action_result:?}"
                );
            }
            AssertionOutcome::Success
        }
        Err(outcome) => outcome,
    }
}

fn get_exchange_index_from_conversation(
    conversation_target: ConversationTarget,
    terminal_view_id: EntityId,
    exchange_index: usize,
    history_model: &BlocklistAIHistoryModel,
) -> Result<&AIAgentExchange, AssertionOutcome> {
    // Async assertions to wait until this exchange is finished.
    let conversation = get_conversation(conversation_target, terminal_view_id, history_model)?;
    match conversation.root_task_exchanges().nth(exchange_index) {
        Some(res) => Ok(res),
        None => {
            // If the exchange doesn't exist and the task is done (no more exchanges will be created), immediately fail
            if !conversation.status().is_in_progress() {
                return Err(AssertionOutcome::immediate_failure(format!(
                    "Exchange at index {exchange_index} does not exist and task is done"
                )));
            };
            // Otherwise, async assertion to wait for the exchange to be created.
            Err(AssertionOutcome::failure(format!(
                "Failed to get exchange at index {exchange_index}"
            )))
        }
    }
}

/// Get the action result at exchange_index.
fn get_exchange_action_result_input(
    conversation_target: ConversationTarget,
    terminal_view_id: EntityId,
    exchange_index: usize,
    history_model: &BlocklistAIHistoryModel,
) -> Result<AIAgentActionResultType, AssertionOutcome> {
    let exchange = get_exchange_index_from_conversation(
        conversation_target,
        terminal_view_id,
        exchange_index,
        history_model,
    )?;
    match exchange.input.last() {
        Some(AIAgentInput::ActionResult { result, .. }) => Ok(result.result.clone()),
        Some(_) => Err(AssertionOutcome::immediate_failure(
            "Exchange input is not an action result".to_string(),
        )),
        None => Err(AssertionOutcome::immediate_failure(
            "Exchange input is empty".to_string(),
        )),
    }
}

fn get_exchange_count(
    terminal_view_id: EntityId,
    history_model: &BlocklistAIHistoryModel,
) -> usize {
    history_model
        .active_conversation(terminal_view_id)
        .map_or(0, |c| c.root_task_exchanges().count())
}

/// Assert that the active conversation was summarized due to context window limits.
pub fn assert_conversation_was_summarized() -> AssertionCallback {
    Box::new(|app, window_id| {
        let terminal_view = terminal_view(app, window_id, 0, 0);
        BlocklistAIHistoryModel::handle(app).update(app, |history_model, _| {
            let Some(conversation) = history_model.active_conversation(terminal_view.id()) else {
                return AssertionOutcome::failure("No active conversation".to_owned());
            };

            if conversation.was_summarized() {
                AssertionOutcome::Success
            } else {
                AssertionOutcome::immediate_failure(
                    "Expected conversation to be summarized due to large context window, but it was not".to_owned(),
                )
            }
        })
    })
}

/// Assert that the active conversation did not use prior-conversation
/// context. From the LLM's perspective this is the `search_conversation_history`
/// tool; on the client it materializes as a `FetchConversation` action followed
/// by regular file tools pointed at `/tmp/warp_conversation_search`.
pub fn assert_no_prior_conversation_context_usage() -> AssertionCallback {
    Box::new(|app, window_id| {
        let terminal_view = terminal_view(app, window_id, 0, 0);
        BlocklistAIHistoryModel::handle(app).update(app, |history_model, _| {
            let Some(conversation) = history_model.active_conversation(terminal_view.id()) else {
                return AssertionOutcome::failure("No active conversation".to_owned());
            };

            for (exchange_index, exchange) in conversation.all_exchanges().into_iter().enumerate() {
                let AIAgentOutputStatus::Finished {
                    finished_output: FinishedAIAgentOutput::Success { output },
                } = &exchange.output_status
                else {
                    continue;
                };

                for action in output.get().actions() {
                    if matches!(action.action, AIAgentActionType::FetchConversation { .. }) {
                        return AssertionOutcome::immediate_failure(format!(
                            "Implementation conversation used prior-conversation context via search_conversation_history in exchange {exchange_index}: {action}"
                        ));
                    }

                    let action_text = action.to_string();
                    if action_text.contains("/tmp/warp_conversation_search") {
                        return AssertionOutcome::immediate_failure(format!(
                            "Implementation conversation read materialized prior-conversation context in exchange {exchange_index}: {action_text}"
                        ));
                    }
                }
            }

            AssertionOutcome::Success
        })
    })
}

/// Asserts that no exchanges exist in the history model (which also means no conversations).
pub fn assert_no_exchanges() -> AssertionCallback {
    Box::new(|app, window_id| {
        let terminal_view = terminal_view(app, window_id, 0, 0);
        BlocklistAIHistoryModel::handle(app).update(app, |history_model, _| {
            let exchange_count = history_model
                .all_live_root_task_exchanges_for_terminal_surface(terminal_view.id())
                .count();

            if exchange_count == 0 {
                AssertionOutcome::Success
            } else {
                AssertionOutcome::failure(format!(
                    "Expected no exchanges but found {exchange_count}"
                ))
            }
        })
    })
}

/// Asserts that no file edits to .md files were requested in the active conversation.
pub fn assert_no_md_file_edits() -> AssertionCallback {
    Box::new(|app, window_id| {
        let terminal_view = terminal_view(app, window_id, 0, 0);
        BlocklistAIHistoryModel::handle(app).update(app, |history_model, _| {
            let Some(conversation) = history_model.active_conversation(terminal_view.id()) else {
                return AssertionOutcome::failure("No active conversation".to_owned());
            };

            // Check all exchanges for file edit requests to .md files
            for exchange in conversation.all_exchanges() {
                if let AIAgentOutputStatus::Finished {
                    finished_output: FinishedAIAgentOutput::Success { output },
                } = &exchange.output_status
                {
                    for action in output.get().actions() {
                        if let AIAgentActionType::RequestFileEdits { file_edits, .. } =
                            &action.action
                        {
                            // Check if any file edit is for a .md file
                            for file_edit in file_edits {
                                if let Some(file_path) = file_edit.file()
                                    && file_path.ends_with(".md")
                                {
                                    return AssertionOutcome::immediate_failure(format!(
                                        "Found .md file edit request: {}",
                                        file_path
                                    ));
                                }
                            }
                        }
                    }
                }
            }

            AssertionOutcome::Success
        })
    })
}

// Asserts that the input of the latest exchange contains a CreateDocuments tool call result.
// If `should_autoexecute` is true, asserts that the exchange output contains a tool call.
// If `should_autoexecute` is false, asserts that the exchange output contains NO tool call.
pub fn assert_created_plan(should_autoexecute: bool) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = terminal_view(app, window_id, 0, 0);
        BlocklistAIHistoryModel::handle(app).update(app, |history_model, _| {
            let exchange_count = get_exchange_count(terminal_view.id(), history_model);
            if exchange_count == 0 {
                return AssertionOutcome::immediate_failure("No exchanges found".to_owned());
            }

            let exchange = match get_exchange_index_from_conversation(
                ConversationTarget::Active,
                terminal_view.id(),
                exchange_count - 1,
                history_model,
            ) {
                Ok(exchange) => exchange,
                Err(outcome) => return outcome,
            };

            // Check if any input in the exchange is an ActionResult for CreateDocuments
            let has_create_docs_result = exchange.input.iter().any(|input| {
                if let AIAgentInput::ActionResult { result, .. } = input {
                    matches!(
                        result.result,
                        AIAgentActionResultType::CreateDocuments(_)
                    )
                } else {
                    false
                }
            });

            if !has_create_docs_result {
                return AssertionOutcome::failure(
                    "Latest exchange input does not contain CreateDocuments tool call result"
                        .to_owned(),
                );
            }

            match get_exchange_output(
                ConversationTarget::Active,
                terminal_view.id(),
                exchange_count - 1,
                history_model,
            ) {
                Ok((_, action_type)) => {
                    if should_autoexecute {
                        if action_type.is_some() {
                            AssertionOutcome::Success
                        } else {
                            AssertionOutcome::immediate_failure(
                                "Expected tool call in output (auto-execute), but found none"
                                    .to_owned(),
                            )
                        }
                    } else if action_type.is_none() {
                        AssertionOutcome::Success
                    } else {
                        AssertionOutcome::immediate_failure(format!(
                            "Expected no tool calls in output (no auto-execute), but found: {action_type:?}"
                        ))
                    }
                }
                Err(outcome) => outcome,
            }
        })
    })
}
