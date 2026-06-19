use std::collections::HashSet;
use std::fs::read;
use std::io::{Cursor, Write};
use std::path::Path;
use std::time::Duration;

use command::blocking::Command;
use prost::Message;
use warpui::integration::{AssertionOutcome, TestStep};
use warpui::{async_assert, SingletonEntity};

use crate::ai::agent::conversation::AIConversation;
use crate::ai::agent::{AIAgentActionType, AIAgentOutputStatus, FinishedAIAgentOutput};
use crate::ai::execution_profiles::profiles::AIExecutionProfilesModel;
use crate::ai::execution_profiles::ActionPermission;
use crate::ai::llms::{LLMId, LLMPreferences};
use crate::integration_testing::agent_mode::{
    assert_latest_task_succeeds_or_blocked, assert_task_is_blocked, ConversationTarget,
};
use crate::integration_testing::step::{
    new_step_with_default_assertions, new_step_with_default_assertions_for_pane,
};
use crate::integration_testing::terminal::assert_input_is_focused;
use crate::integration_testing::view_getters::terminal_view;
use crate::BlocklistAIHistoryModel;

pub const AGENT_MODE_RUNNING_STEP_GROUP_NAME: &str = "Agent mode running";

/// Where `capture_impl_artifacts` persists the implementation conversation's
/// git diff. Falls back to a fixed /tmp path when unset.
pub const IMPL_CODE_DIFF_OUTPUT_FILE_ENV_VAR: &str = "IMPL_CODE_DIFF_OUTPUT_FILE";

use super::hydrate_ai_conversation_assertion;

/// Assumes that the terminal input is currently not in AI input mode.
pub fn enter_agent_view() -> TestStep {
    new_step_with_default_assertions("Enter Agent View")
        .with_keystrokes(&["ctrl-shift-enter"])
        .add_named_assertion(
            "Assert that we are in Agent View and AI input mode",
            move |app, window_id| {
                let terminal_view = terminal_view(app, window_id, 0, 0);
                terminal_view.read(app, |terminal_view, app| {
                    let is_ai_input_mode = terminal_view
                        .input()
                        .read(app, |input, app| input.input_type(app).is_ai());
                    let agent_view_state = {
                        let model = terminal_view.model.lock();
                        model.block_list().agent_view_state().clone()
                    };
                    async_assert!(
                        is_ai_input_mode && agent_view_state.is_active(),
                        "Expected fullscreen Agent View + AI input mode, got agent_view_state={agent_view_state:?}, is_ai_input_mode={is_ai_input_mode}"
                    )
                })
            },
        )
}

/// Assumes that the terminal input is currently in AI input mode.
pub fn exit_agent_view() -> TestStep {
    new_step_with_default_assertions("Exit Agent View")
        .with_keystrokes(&["escape"])
        .add_named_assertion(
            "Assert that we exited Agent View and are not in AI input mode",
            move |app, window_id| {
                let terminal_view = terminal_view(app, window_id, 0, 0);
                terminal_view.read(app, |terminal_view, app| {
                    let is_ai_input_mode = terminal_view
                        .input()
                        .read(app, |input, app| input.input_type(app).is_ai());
                    let agent_view_state = {
                        let model = terminal_view.model.lock();
                        model.block_list().agent_view_state().clone()
                    };
                    async_assert!(
                        !is_ai_input_mode && !agent_view_state.is_active(),
                        "Expected inactive Agent View + non-AI input mode, got agent_view_state={agent_view_state:?}, is_ai_input_mode={is_ai_input_mode}"
                    )
                })
            },
        )
}

/// Hydrates a conversation from a protobuf file.
/// File should be generated into the `input_data` directory.
/// See the agent_mode_eval README for more details.
pub fn hydrate_ai_conversation(file_name: &str) -> TestStep {
    let file_bytes = get_input_data(file_name);
    let Ok(request) = warp_multi_agent_api::Request::decode(file_bytes) else {
        panic!("Failed to decode request from protobuf");
    };

    let tasks = request
        .task_context
        .map(|ctx| ctx.tasks)
        .unwrap_or_default();

    new_step_with_default_assertions("Hydrate AI conversation").add_named_assertion(
        "Assert that conversation was hydrated successfully",
        hydrate_ai_conversation_assertion(tasks),
    )
}

/// Attach the latest block in the blocklist (command + output) to the AI query.
pub fn attach_recent_block_as_context() -> TestStep {
    TestStep::new("Attach last block as context").add_named_assertion(
        "Attach last block as context",
        |app, window_id| {
            let terminal_view = terminal_view(app, window_id, 0, 0);
            terminal_view.update(app, |view, ctx| {
                let last_index = {
                    let model = view.model.lock();
                    model.block_list().last_non_hidden_block_by_index()
                };
                if let Some(idx) = last_index {
                    view.integration_test_change_block_selection_to_single(idx, ctx);
                }
            });

            terminal_view.read(app, |view, ctx| {
                let count = view
                    .ai_context_model()
                    .as_ref(ctx)
                    .pending_context_block_ids()
                    .len();
                async_assert!(
                    count == 1,
                    "Expected exactly 1 attached context block, got {count}"
                )
            })
        },
    )
}

// This will fail immediately on any error responses.
pub fn submit_ai_query_and_wait_until_done(query: &str, timeout: Duration) -> TestStep {
    submit_ai_query(query, timeout)
        .add_named_assertion(
            "Assert the agent task is complete",
            assert_latest_task_succeeds_or_blocked(ConversationTarget::Active, None),
        )
        .add_named_assertion(
            "Assert that that input has been returned to the user",
            assert_input_is_focused(),
        )
}

/// Submits an AI query and waits until the task is blocked (waiting for user approval).
/// This is useful for tests where auto-execution is disabled and you want to verify
/// the command that would be executed without actually running it.
pub fn submit_ai_query_and_wait_until_blocked(query: &str, timeout: Duration) -> TestStep {
    submit_ai_query(query, timeout).add_named_assertion(
        "Assert the agent task is blocked",
        assert_task_is_blocked(ConversationTarget::Active),
    )
}

// Runs an AI query without waiting for anything.
// This is useful if you expect a specific sequence of responses (e.g. expect a certain command to be requested immediately),
// since it lets you make assertions on responses as they become ready and fail early instead of waiting for the agent to finish all its turns.
pub fn submit_ai_query(query: &str, timeout: Duration) -> TestStep {
    new_step_with_default_assertions_for_pane(&format!("Enter AI query: {query}"), 0, 0)
        .set_timeout(timeout)
        .set_step_group_name(AGENT_MODE_RUNNING_STEP_GROUP_NAME)
        .with_typed_characters(&[query])
        .with_keystrokes(&["enter"])
        .add_named_assertion(
            "Print conversation ID to stdout",
            print_conversation_id_assertion(),
        )
}

/// Persists the implementation conversation's token usage / cost / exchange
/// count to runtime tags (namespaced `impl.*`) and its `git diff` to a durable
/// file. Must run before the judge starts a new conversation with `/new`,
/// which replaces the active conversation — analytics captured at `on_finish`
/// would then reflect the judge's conversation rather than the implementation.
pub fn capture_impl_artifacts() -> TestStep {
    new_step_with_default_assertions("Capture impl conversation artifacts").add_named_assertion(
        "Persist impl token usage + diff before judge swaps active conversation",
        |app, window_id| {
            let terminal_view = terminal_view(app, window_id, 0, 0);
            let Some(pwd) = terminal_view.read(app, |terminal_view, _| terminal_view.pwd()) else {
                return AssertionOutcome::immediate_failure(
                    "Could not get current directory".to_owned(),
                );
            };

            BlocklistAIHistoryModel::handle(app).update(app, |history_model, _| {
                let Some(conversation) = history_model.active_conversation(terminal_view.id())
                else {
                    return AssertionOutcome::immediate_failure(
                        "No active conversation to capture".to_owned(),
                    );
                };

                super::record_pending_runtime_tag(
                    "impl.total_request_cost",
                    conversation.total_request_cost().to_string(),
                );
                super::record_pending_runtime_tag(
                    "impl.total_exchanges",
                    conversation.all_exchanges().len().to_string(),
                );
                super::record_pending_runtime_tag(
                    "impl.was_summarized",
                    conversation.was_summarized().to_string(),
                );
                if let Some(token) = conversation.server_conversation_token() {
                    super::record_pending_runtime_tag(
                        "impl.conversation_debug_link",
                        token
                            .debug_link()
                            .replace("host.docker.internal:8080", "staging.warp.dev"),
                    );
                }
                for usage in conversation.total_token_usage().iter() {
                    let prefix = format!(
                        "impl.{}{}",
                        super::RUNTIME_TAG_TOKEN_USAGE_PREFIX,
                        usage.model_id
                    );
                    super::record_pending_runtime_tag(
                        format!("{prefix}.total_input"),
                        usage.total_input.to_string(),
                    );
                    super::record_pending_runtime_tag(
                        format!("{prefix}.output"),
                        usage.output.to_string(),
                    );
                    super::record_pending_runtime_tag(
                        format!("{prefix}.input_cache_read"),
                        usage.input_cache_read.to_string(),
                    );
                    super::record_pending_runtime_tag(
                        format!("{prefix}.input_cache_write"),
                        usage.input_cache_write.to_string(),
                    );
                    super::record_pending_runtime_tag(
                        format!("{prefix}.cost_in_cents"),
                        usage.cost_in_cents.to_string(),
                    );
                }

                // Persist the impl `git diff` to a dedicated file so `on_finish`'s
                // CODE_DIFF_OUTPUT_FILE still works for whichever conversation
                // happens to be active at finish time.
                let impl_diff_path = std::env::var(IMPL_CODE_DIFF_OUTPUT_FILE_ENV_VAR)
                    .unwrap_or_else(|_| "/tmp/impl_code_diff.txt".to_owned());
                let edited_files = collect_edited_files(conversation);
                let Ok(mut file) = std::fs::File::create(&impl_diff_path) else {
                    return AssertionOutcome::immediate_failure(format!(
                        "Failed to create impl diff file {impl_diff_path}"
                    ));
                };
                if edited_files.is_empty() {
                    let _ = writeln!(file, "No files were edited by the impl conversation");
                } else {
                    for file_name in edited_files {
                        let output = Command::new("git")
                            .args(["diff", "--", &file_name])
                            .current_dir(&pwd)
                            .output();
                        super::write_git_diff_output_to_file(output, &mut file, &file_name);
                    }
                }

                AssertionOutcome::Success
            })
        },
    )
}

/// Walks the conversation's exchanges and returns the unique set of files the
/// agent requested edits for. Mirrors the corresponding loop inside
/// `output_code_diff_debug_info`.
fn collect_edited_files(conversation: &AIConversation) -> HashSet<String> {
    let mut edited_files = HashSet::new();
    for exchange in conversation.all_exchanges() {
        let AIAgentOutputStatus::Finished { finished_output } = &exchange.output_status else {
            continue;
        };
        let FinishedAIAgentOutput::Success { output } = finished_output else {
            continue;
        };
        for action in output.get().actions() {
            if let AIAgentActionType::RequestFileEdits { file_edits, .. } = &action.action {
                for edit in file_edits {
                    if let Some(file) = edit.file() {
                        edited_files.insert(file.to_owned());
                    }
                }
            }
        }
    }
    edited_files
}

/// Returns an assertion that prints the conversation ID to stdout once available.
/// This assertion will poll until the conversation token is received from the server.
fn print_conversation_id_assertion(
) -> impl FnMut(&mut warpui::App, warpui::WindowId) -> warpui::integration::AssertionOutcome {
    |app, window_id| {
        let terminal_view = terminal_view(app, window_id, 0, 0);
        BlocklistAIHistoryModel::handle(app).read(app, |history_model, _| {
            if let Some(conversation) = history_model.active_conversation(terminal_view.id()) {
                if let Some(token) = conversation.server_conversation_token() {
                    // The debug link within the container will be using host.docker.internal, but we're opening
                    // from outside the container.
                    let debug_link = token
                        .debug_link()
                        .replace("host.docker.internal", "localhost");
                    println!("Conversation ID (debug link): {debug_link}");
                    return AssertionOutcome::Success;
                }
            }
            // If we don't have a conversation token yet, keep polling
            AssertionOutcome::failure("Waiting for conversation token to be available".to_string())
        })
    }
}

/// Sets the preferred agent mode LLM. This is the base model for agent and inline AI conversations.
pub fn set_preferred_agent_mode_llm(llm_id: &str) -> TestStep {
    let llm_id = LLMId::from(llm_id);
    TestStep::new(&format!("Set preferred agent mode LLM to {llm_id}")).add_named_assertion(
        "Update preferred agent mode LLM",
        move |app, window_id| {
            let llm_id = llm_id.clone();
            let terminal_view_id = terminal_view(app, window_id, 0, 0).id();
            LLMPreferences::handle(app).update(app, |llm_preferences, ctx| {
                // Validate that the LLM ID is actually available. We only do this
                // for the base model, since the coding and planning models are
                // currently unused in the product.
                assert!(
                    llm_preferences.is_available_agent_mode_llm(&llm_id),
                    "LLM ID '{llm_id}' is not a valid agent mode LLM",
                );
                llm_preferences.update_preferred_agent_mode_llm(&llm_id, terminal_view_id, ctx);
            });
            async_assert!(true, "Successfully updated preferred agent mode LLM")
        },
    )
}

/// Sets the preferred coding LLM. Note that the server currently ignores this.
pub fn set_preferred_coding_llm(llm_id: &str) -> TestStep {
    let llm_id = LLMId::from(llm_id);
    TestStep::new(&format!("Set preferred coding LLM to {llm_id}")).add_named_assertion(
        "Update preferred coding LLM",
        move |app, window_id| {
            let llm_id = llm_id.clone();
            let terminal_view_id = terminal_view(app, window_id, 0, 0).id();
            LLMPreferences::handle(app).update(app, |llm_preferences, ctx| {
                llm_preferences.update_preferred_coding_llm(&llm_id, Some(terminal_view_id), ctx);
            });
            async_assert!(true, "Successfully updated preferred coding LLM")
        },
    )
}

fn get_input_data(file_name: &str) -> Cursor<Vec<u8>> {
    let input_data_dir = std::env::var("INPUT_DATA_DIR").expect(
        "INPUT_DATA_DIR is not set. This is needed to hydrate conversations from eval tests.",
    );
    let path = Path::new(&input_data_dir).join(file_name);
    Cursor::new(read(&path).expect("Failed to read binary input data"))
}

/// Sets the execution profile to not auto-execute commands.
/// This changes the `execute_commands` permission from `AlwaysAllow` to `AlwaysAsk`,
/// which means commands will be proposed but not automatically executed.
pub fn set_execution_profile_no_auto_execute() -> TestStep {
    TestStep::new("Set execution profile to not auto-execute commands").add_named_assertion(
        "Update execution profile",
        |app, _window_id| {
            AIExecutionProfilesModel::handle(app).update(app, |profiles, ctx| {
                let default_profile_id = *profiles.default_profile(ctx).id();
                profiles.set_execute_commands(
                    default_profile_id,
                    &ActionPermission::AlwaysAsk,
                    ctx,
                );
            });
            async_assert!(true, "Successfully updated execution profile")
        },
    )
}
