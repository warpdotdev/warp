mod assertions;
pub mod llm_judge;
mod step;
mod user_defaults;
mod util;
use std::collections::{BTreeMap, HashSet};
use std::fs::File;
use std::io::Write;
use std::sync::Mutex;

pub use assertions::*;
pub use step::*;
pub use user_defaults::*;
pub use util::*;
use warp_errors::report_error;
use warpui::integration::PersistedDataMap;
pub use warpui::integration::RUNTIME_TAG_FAILURE_REASON;
use warpui::{App, SingletonEntity as _, WindowId};

use crate::BlocklistAIHistoryModel;
use crate::ai::agent::{AIAgentActionType, AIAgentOutputStatus, FinishedAIAgentOutput};
pub use crate::integration_testing::view_getters::terminal_view;

pub const TOTAL_REQUEST_COST_PREFIX: &str = "Total request cost: ";
pub const TOTAL_EXCHANGES_PREFIX: &str = "Total number of exchanges: ";
pub const TOTAL_TOKEN_USAGE_PREFIX: &str = "Total token usage: ";

pub const RUNTIME_TAG_TOTAL_REQUEST_COST: &str = "total_request_cost";
pub const RUNTIME_TAG_TOTAL_EXCHANGES: &str = "total_exchanges";
pub const RUNTIME_TAG_TOKEN_USAGE_PREFIX: &str = "token_usage.";

const CODE_DIFF_OUTPUT_FILE_ENV_VAR: &str = "CODE_DIFF_OUTPUT_FILE";

/// Runtime tags recorded from inside test steps. Steps cannot reach the
/// driver's persisted data map (only `on_finish` receives it), so tags
/// recorded mid-run are staged here and merged by `drain_pending_runtime_tags`.
static PENDING_RUNTIME_TAGS: Mutex<BTreeMap<String, String>> = Mutex::new(BTreeMap::new());

/// Buffers a runtime tag recorded from inside a test step (e.g. analytics for
/// a conversation that will no longer be active at `on_finish` time).
pub fn record_pending_runtime_tag(key: impl Into<String>, value: impl Into<String>) {
    let mut tags = PENDING_RUNTIME_TAGS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    tags.insert(key.into(), value.into());
}

/// Merges (and clears) tags staged by `record_pending_runtime_tag` into the
/// driver's persisted data. Call this from the eval's `on_finish`.
pub fn drain_pending_runtime_tags(persisted_data: &mut PersistedDataMap) {
    let mut tags = PENDING_RUNTIME_TAGS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    persisted_data.extend(std::mem::take(&mut *tags));
}

pub fn output_code_diff_with_base_commit(
    base_commit: &str,
    working_dir: &str,
    test_files_str: &str,
) {
    use command::blocking::Command;

    let Some(mut output_file) = open_debug_file_from_env(CODE_DIFF_OUTPUT_FILE_ENV_VAR) else {
        report_error!("Could not open debug file from env");
        return;
    };
    // Clear the test files from the diff, because we are not interested in seeing those.
    log::debug!(
        "[GIT OPERATION] mod.rs output_code_diff_with_base_commit git checkout {base_commit} -- {test_files_str}"
    );
    let _ = Command::new("git")
        .args(["checkout", base_commit, "--", test_files_str])
        .current_dir(working_dir)
        .output();
    log::debug!(
        "[GIT OPERATION] mod.rs output_code_diff_with_base_commit git --no-pager diff {base_commit}"
    );
    let git_diff_output = Command::new("git")
        .args(["--no-pager", "diff", base_commit])
        .current_dir(working_dir)
        .output();
    write_git_diff_output_to_file(
        git_diff_output,
        &mut output_file,
        &std::env::var(CODE_DIFF_OUTPUT_FILE_ENV_VAR)
            .expect("Could not find diff output file env var"),
    )
}

pub fn output_code_diff_debug_info(app: &mut App, window_id: WindowId) {
    let terminal_view = terminal_view(app, window_id, 0, 0);
    let Some(current_dir) = terminal_view.read(app, |terminal_view, _| terminal_view.pwd()) else {
        report_error!("Could not get current directory");
        return;
    };
    BlocklistAIHistoryModel::handle(app).update(app, |history_model, _| {
        let Some(conversation) = history_model.active_conversation(terminal_view.id()) else {
            return;
        };
        let mut edited_files = HashSet::new();
        for exchange in conversation.all_exchanges().into_iter() {
            let AIAgentOutputStatus::Finished { finished_output } = &exchange.output_status else {
                continue;
            };
            if let FinishedAIAgentOutput::Success { output } = finished_output {
                let agent_output = output.get();
                for action in agent_output.actions() {
                    if let AIAgentActionType::RequestFileEdits { file_edits, .. } = &action.action {
                        for edit in file_edits {
                            if let Some(file) = edit.file() {
                                edited_files.insert(file.to_owned());
                            }
                        }
                    }
                }
            }
        }

        let mut output_file = open_debug_file_from_env(CODE_DIFF_OUTPUT_FILE_ENV_VAR);
        if let Some(output_file) = &mut output_file {
            use std::io::Write;

            use command::blocking::Command;
            if edited_files.is_empty() {
                writeln!(output_file, "No files were edited for this test")
                    .expect("Failed to write to code diff file");
            } else {
                for file_name in edited_files {
                    log::debug!(
                        "[GIT OPERATION] mod.rs output_code_diff_debug_info git diff -- {file_name}"
                    );
                    let output = Command::new("git")
                        .args(["diff", "--", &file_name])
                        .current_dir(&current_dir)
                        .output();
                    write_git_diff_output_to_file(output, output_file, &file_name);
                }
            }
        }
    });
}

fn write_git_diff_output_to_file(
    diff_output: std::io::Result<std::process::Output>,
    file: &mut File,
    file_name: &str,
) {
    match diff_output {
        Ok(output) if output.status.success() => {
            let diff = String::from_utf8_lossy(&output.stdout);
            writeln!(file, "Diff for file: {file_name}\n{diff}\n")
                .expect("Failed to write diff to code diff file");
        }
        Ok(output) => {
            let err = String::from_utf8_lossy(&output.stderr);
            writeln!(
                file,
                "Failed to get diff for file: {file_name}\nGit error:\n{err}\n"
            )
            .expect("Failed to write error to code diff file");
        }
        Err(e) => {
            writeln!(
                file,
                "Failed to run git diff for file: {file_name}\nError: {e}\n"
            )
            .expect("Failed to write command error to code diff file");
        }
    }
}

pub fn output_conversation_debug_info(
    app: &mut App,
    window_id: WindowId,
    persisted_data: &mut PersistedDataMap,
) {
    let terminal_view = terminal_view(app, window_id, 0, 0);
    BlocklistAIHistoryModel::handle(app).update(app, |history_model, _| {
        let Some(conversation) = history_model.active_conversation(terminal_view.id()) else {
            return;
        };
        let mut output_file = open_debug_file_from_env("DEBUG_OUTPUT_FILE");

        // Create a function to handle output
        let mut write_to_debug_file = |text: &str| {
            if let Some(file) = &mut output_file {
                use std::io::Write;
                writeln!(file, "{text}").expect("Failed to write to debug output file");
            } else {
                println!("{text}");
            }
        };

        // Add timestamp to the header
        let current_time = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ");
        write_to_debug_file(&format!(
            "========Conversation Debug Info (Generated: {current_time})========"
        ));
        let debug_link = conversation
            .server_conversation_token()
            .map(|token| {
                // The debug link within the container will be using host.docker.internal, but we're opening
                // from outside the container.
                // The server is configured to always write debug data to GCS instead of locally when run for evals, so we replace
                // with staging.warp.dev instead of localhost:8080.
                token
                    .debug_link()
                    .replace("host.docker.internal:8080", "staging.warp.dev")
            })
            .unwrap_or("unavailable".to_owned());
        write_to_debug_file(&format!("Conversation Debug Link: {debug_link}"));

        let total_request_cost = conversation.total_request_cost();
        let total_exchanges = conversation.all_exchanges().len();
        let token_usage = conversation.total_token_usage();

        // Populate runtime tags with conversation data
        persisted_data.insert(
            RUNTIME_TAG_TOTAL_REQUEST_COST.to_string(),
            total_request_cost.to_string(),
        );
        persisted_data.insert(
            RUNTIME_TAG_TOTAL_EXCHANGES.to_string(),
            total_exchanges.to_string(),
        );

        // Add token usage as separate runtime tags
        for usage in token_usage.iter() {
            persisted_data.insert(
                format!(
                    "{}{}.total_input",
                    RUNTIME_TAG_TOKEN_USAGE_PREFIX, usage.model_id
                ),
                usage.total_input.to_string(),
            );
            persisted_data.insert(
                format!(
                    "{}{}.output",
                    RUNTIME_TAG_TOKEN_USAGE_PREFIX, usage.model_id
                ),
                usage.output.to_string(),
            );
            persisted_data.insert(
                format!(
                    "{}{}.input_cache_read",
                    RUNTIME_TAG_TOKEN_USAGE_PREFIX, usage.model_id
                ),
                usage.input_cache_read.to_string(),
            );
            persisted_data.insert(
                format!(
                    "{}{}.input_cache_write",
                    RUNTIME_TAG_TOKEN_USAGE_PREFIX, usage.model_id
                ),
                usage.input_cache_write.to_string(),
            );
            persisted_data.insert(
                format!(
                    "{}{}.cost_in_cents",
                    RUNTIME_TAG_TOKEN_USAGE_PREFIX, usage.model_id
                ),
                usage.cost_in_cents.to_string(),
            );
        }

        // Write to debug file for backward compatibility
        write_to_debug_file(&format!(
            "{TOTAL_REQUEST_COST_PREFIX}{total_request_cost}"
        ));

        write_to_debug_file(&format!("{TOTAL_EXCHANGES_PREFIX}{total_exchanges}"));

        write_to_debug_file(&format!(
            "{TOTAL_TOKEN_USAGE_PREFIX}{}",
            token_usage
                .iter()
                .map(|usage| format!(
                    "model_id={},total_input={},output={},input_cache_read={},input_cache_write={},cost_in_cents={}",
                    usage.model_id,
                    usage.total_input,
                    usage.output,
                    usage.input_cache_read,
                    usage.input_cache_write,
                    usage.cost_in_cents
                ))
                .collect::<Vec<_>>()
                .join("|")
        ));

        write_to_debug_file("\nConversation Exchanges:\n");
        for (i, exchange) in conversation.all_exchanges().into_iter().enumerate() {
            // Add timestamp for each exchange
            let exchange_time = chrono::DateTime::<chrono::Utc>::from(exchange.start_time)
                .format("%Y-%m-%dT%H:%M:%S%.3fZ");
            write_to_debug_file(&format!(
                "\n--- Exchange {} (Started: {}) ---",
                i + 1,
                exchange_time
            ));

            for input in &exchange.input {
                write_to_debug_file(&format!("\nInput:\n\n{input}\n"));
            }
            write_to_debug_file(&format!("Output:\n{}", &exchange.output_status))
        }

        // Add completion timestamp
        let completion_time = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ");
        write_to_debug_file(&format!(
            "\n========Debug Info Complete ({completion_time})=========="
        ));
    })
}

// Get debug output file path from environment
pub fn open_debug_file_from_env(env_var: &str) -> Option<File> {
    let file_path = std::env::var(env_var).ok();
    if let Some(file_path) = &file_path {
        // Clear the file if it exists
        let _ = std::fs::remove_file(file_path);
        Some(
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(file_path)
                .expect("Failed to open debug output file"),
        )
    } else {
        None
    }
}
