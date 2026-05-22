use std::fs;
use std::path::Path;

use warp_multi_agent_api as api;

use super::{base_dir, materialize_tasks_to_yaml};
use crate::test_util::ai_agent_tasks::{
    create_api_subtask, create_api_task, create_message, create_subagent_tool_call_message,
};

/// Lists filenames (not full paths) in a directory, sorted.
fn list_dir_sorted(dir: &Path) -> Vec<String> {
    let mut entries: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    entries.sort();
    entries
}

fn make_user_query_message(id: &str, task_id: &str, query: &str) -> api::Message {
    api::Message {
        id: id.to_string(),
        task_id: task_id.to_string(),
        server_message_data: String::new(),
        citations: vec![],
        message: Some(api::message::Message::UserQuery(api::message::UserQuery {
            query: query.to_string(),
            context: None,
            mode: None,
            referenced_attachments: Default::default(),
            intended_agent: Default::default(),
        })),
        request_id: String::new(),
        timestamp: None,
    }
}

fn make_tool_call_message(
    id: &str,
    task_id: &str,
    tool_call_id: &str,
    tool: api::message::tool_call::Tool,
) -> api::Message {
    api::Message {
        id: id.to_string(),
        task_id: task_id.to_string(),
        server_message_data: String::new(),
        citations: vec![],
        message: Some(api::message::Message::ToolCall(api::message::ToolCall {
            tool_call_id: tool_call_id.to_string(),
            tool: Some(tool),
        })),
        request_id: String::new(),
        timestamp: None,
    }
}

fn make_tool_call_result_message(
    id: &str,
    task_id: &str,
    tool_call_id: &str,
    result: api::message::tool_call_result::Result,
) -> api::Message {
    api::Message {
        id: id.to_string(),
        task_id: task_id.to_string(),
        server_message_data: String::new(),
        citations: vec![],
        message: Some(api::message::Message::ToolCallResult(
            api::message::ToolCallResult {
                tool_call_id: tool_call_id.to_string(),
                result: Some(result),
                context: None,
            },
        )),
        request_id: String::new(),
        timestamp: None,
    }
}

fn cleanup_dir(path: &str) {
    let _ = fs::remove_dir_all(path);
}

#[test]
fn mixed_message_types_produce_sequentially_indexed_files() {
    let task_id = "root";
    let tasks = vec![create_api_task(
        task_id,
        vec![
            make_user_query_message("m1", task_id, "hello"),
            // AgentOutput via create_message helper
            create_message("m2", task_id),
            make_tool_call_message(
                "m3",
                task_id,
                "tc1",
                api::message::tool_call::Tool::Grep(api::message::tool_call::Grep {
                    queries: vec!["foo".into()],
                    path: "/src".into(),
                }),
            ),
        ],
    )];

    let dir = materialize_tasks_to_yaml(&tasks).unwrap();
    assert!(
        Path::new(&dir).starts_with(base_dir()),
        "returned path should be under temp_dir(), got: {dir}",
    );
    // Verify no mixed separators: on Windows the path should use only '\',
    // on Unix only '/'. This catches the original bug where tempdir_in
    // joined a forward-slash parent with a native backslash separator.
    assert!(
        !dir.contains('/') || !dir.contains('\\'),
        "returned path has mixed separators: {dir}",
    );
    let files = list_dir_sorted(Path::new(&dir));

    assert_eq!(files.len(), 3);
    assert!(files[0].starts_with("000.m1.user_query"));
    assert!(files[1].starts_with("001.m2.agent_output"));
    assert!(files[2].starts_with("002.m3.tool_call.tc1.grep"));

    // Verify user_query content is searchable.
    let content = fs::read_to_string(Path::new(&dir).join(&files[0])).unwrap();
    assert!(content.contains("type: user_query"));
    assert!(content.contains("hello"));

    cleanup_dir(&dir);
}

#[test]
fn subagent_file_and_subdirectory_share_same_index() {
    let root_id = "root";
    let subtask_id = "subtask1";

    let root_task = create_api_task(
        root_id,
        vec![
            make_user_query_message("m1", root_id, "search my conversation"),
            create_subagent_tool_call_message(
                "m2",
                root_id,
                subtask_id,
                Some(
                    api::message::tool_call::subagent::Metadata::ConversationSearch(
                        Default::default(),
                    ),
                ),
            ),
        ],
    );
    let subtask = create_api_subtask(
        subtask_id,
        root_id,
        vec![create_message("sub_m1", subtask_id)],
    );

    let dir = materialize_tasks_to_yaml(&[root_task, subtask]).unwrap();
    let entries = list_dir_sorted(Path::new(&dir));

    // Should have: 000.m1.user_query.yaml, 001.m2.subagent.*.yaml, 001.subtask1/ (directory)
    assert_eq!(entries.len(), 3);

    // The subagent YAML file and its subdirectory must share the same "001" prefix.
    let subagent_file = entries
        .iter()
        .find(|e| e.contains("subagent") && e.ends_with(".yaml"))
        .expect("should have subagent yaml file");
    let subdir = entries
        .iter()
        .find(|e| e.contains(subtask_id) && !e.ends_with(".yaml"))
        .expect("should have subtask directory");

    let file_prefix: String = subagent_file.chars().take(3).collect();
    let dir_prefix: String = subdir.chars().take(3).collect();
    assert_eq!(
        file_prefix, dir_prefix,
        "subagent file ({subagent_file}) and directory ({subdir}) must share the same index prefix"
    );
    assert_eq!(file_prefix, "001");

    // Verify subtask directory contains the subtask's messages.
    let sub_entries = list_dir_sorted(&Path::new(&dir).join(subdir));
    assert_eq!(sub_entries.len(), 1);
    assert!(sub_entries[0].contains("sub_m1"));

    cleanup_dir(&dir);
}

#[test]
fn missing_subtask_in_task_map_produces_file_but_no_directory() {
    let root_id = "root";

    // Subagent references subtask "missing_task" which is not in the task list.
    let root_task = create_api_task(
        root_id,
        vec![create_subagent_tool_call_message(
            "m1",
            root_id,
            "missing_task",
            Some(api::message::tool_call::subagent::Metadata::Cli(
                Default::default(),
            )),
        )],
    );

    let dir = materialize_tasks_to_yaml(&[root_task]).unwrap();
    let entries = list_dir_sorted(Path::new(&dir));

    // Should have just the YAML file, no subdirectory since the subtask is missing.
    assert_eq!(entries.len(), 1);
    assert!(entries[0].ends_with(".yaml"));
    assert!(entries[0].contains("subagent"));

    cleanup_dir(&dir);
}

#[test]
fn empty_task_list_returns_error() {
    let result = materialize_tasks_to_yaml(&[]);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("No root task found"));
}

#[test]
fn tool_call_result_resolves_tool_name_from_matching_call() {
    let task_id = "root";
    let tasks = vec![create_api_task(
        task_id,
        vec![
            make_tool_call_message(
                "m1",
                task_id,
                "tc1",
                api::message::tool_call::Tool::Grep(api::message::tool_call::Grep {
                    queries: vec!["pattern".into()],
                    path: "/src".into(),
                }),
            ),
            make_tool_call_result_message(
                "m2",
                task_id,
                "tc1",
                api::message::tool_call_result::Result::Grep(api::GrepResult {
                    result: Some(api::grep_result::Result::Success(
                        api::grep_result::Success {
                            matched_files: vec![api::grep_result::success::GrepFileMatch {
                                file_path: "foo.rs".into(),
                                matched_lines: vec![
                                    api::grep_result::success::grep_file_match::GrepLineMatch {
                                        line_number: 42,
                                    },
                                ],
                            }],
                        },
                    )),
                }),
            ),
        ],
    )];

    let dir = materialize_tasks_to_yaml(&tasks).unwrap();
    let files = list_dir_sorted(Path::new(&dir));

    assert_eq!(files.len(), 2);
    // The result file should contain "grep" in its name, resolved from the tool call.
    assert!(
        files[1].contains("grep"),
        "result filename should contain tool name 'grep', got: {}",
        files[1]
    );

    // Verify line numbers are serialized.
    let content = fs::read_to_string(Path::new(&dir).join(&files[1])).unwrap();
    assert!(content.contains("foo.rs"), "should contain file path");
    assert!(content.contains("42"), "should contain line number");

    cleanup_dir(&dir);
}

#[test]
fn server_tool_calls_are_skipped() {
    let task_id = "root";
    let tasks = vec![create_api_task(
        task_id,
        vec![
            make_user_query_message("m1", task_id, "hello"),
            make_tool_call_message(
                "m2",
                task_id,
                "tc_server",
                api::message::tool_call::Tool::Server(api::message::tool_call::Server {
                    payload: String::new(),
                }),
            ),
            create_message("m3", task_id),
        ],
    )];

    let dir = materialize_tasks_to_yaml(&tasks).unwrap();
    let files = list_dir_sorted(Path::new(&dir));

    // Server tool call should be skipped; only user_query and agent_output.
    assert_eq!(files.len(), 2);
    assert!(files[0].contains("user_query"));
    assert!(files[1].contains("agent_output"));
    // Index should still be sequential (000, 001) since server call was skipped.
    assert!(files[0].starts_with("000"));
    assert!(files[1].starts_with("001"));

    cleanup_dir(&dir);
}

#[test]
fn start_agent_v2_tool_call_serializes_name_and_prompt() {
    let task_id = "root";
    let tasks = vec![create_api_task(
        task_id,
        vec![make_tool_call_message(
            "m1",
            task_id,
            "tc_start_agent_v2",
            api::message::tool_call::Tool::StartAgentV2(api::StartAgentV2 {
                name: "Remote child".to_string(),
                prompt: "Investigate the build failure".to_string(),
                execution_mode: None,
                lifecycle_subscription: None,
            }),
        )],
    )];

    let dir = materialize_tasks_to_yaml(&tasks).unwrap();
    let files = list_dir_sorted(Path::new(&dir));
    let content = fs::read_to_string(Path::new(&dir).join(&files[0])).unwrap();

    assert!(content.contains("tool_name: start_agent"));
    assert!(content.contains("name: \"Remote child\""));
    assert!(content.contains("prompt: |"));
    assert!(content.contains("Investigate the build failure"));

    cleanup_dir(&dir);
}

#[test]
fn start_agent_v2_tool_call_result_serializes_agent_id_and_error() {
    let task_id = "root";
    let tasks = vec![create_api_task(
        task_id,
        vec![
            make_tool_call_message(
                "m1",
                task_id,
                "tc_start_agent_v2",
                api::message::tool_call::Tool::StartAgentV2(api::StartAgentV2 {
                    name: "Remote child".to_string(),
                    prompt: "Investigate the build failure".to_string(),
                    execution_mode: None,
                    lifecycle_subscription: None,
                }),
            ),
            make_tool_call_result_message(
                "m2",
                task_id,
                "tc_start_agent_v2",
                api::message::tool_call_result::Result::StartAgentV2(api::StartAgentV2Result {
                    result: Some(api::start_agent_v2_result::Result::Success(
                        api::start_agent_v2_result::Success {
                            agent_id: "agent-123".to_string(),
                        },
                    )),
                }),
            ),
            make_tool_call_result_message(
                "m3",
                task_id,
                "tc_start_agent_v2",
                api::message::tool_call_result::Result::StartAgentV2(api::StartAgentV2Result {
                    result: Some(api::start_agent_v2_result::Result::Error(
                        api::start_agent_v2_result::Error {
                            error: "child failed".to_string(),
                        },
                    )),
                }),
            ),
        ],
    )];

    let dir = materialize_tasks_to_yaml(&tasks).unwrap();
    let files = list_dir_sorted(Path::new(&dir));
    let success_content = fs::read_to_string(Path::new(&dir).join(&files[1])).unwrap();
    let error_content = fs::read_to_string(Path::new(&dir).join(&files[2])).unwrap();

    assert!(success_content.contains("agent_id: agent-123"));
    assert!(error_content.contains("error: child failed"));

    cleanup_dir(&dir);
}

#[test]
fn upload_file_artifact_tool_call_result_serializes_only_supported_success_fields() {
    let task_id = "root";
    let tasks = vec![create_api_task(
        task_id,
        vec![
            make_tool_call_message(
                "m1",
                task_id,
                "tc_upload_file_artifact",
                api::message::tool_call::Tool::UploadFileArtifact(api::UploadFileArtifact {
                    file: Some(api::FilePathReference {
                        file_path: "outputs/report.txt".to_string(),
                    }),
                    description: "Daily summary".to_string(),
                }),
            ),
            make_tool_call_result_message(
                "m2",
                task_id,
                "tc_upload_file_artifact",
                api::message::tool_call_result::Result::UploadFileArtifact(
                    api::UploadFileArtifactResult {
                        result: Some(api::upload_file_artifact_result::Result::Success(
                            api::upload_file_artifact_result::Success {
                                artifact_uid: "artifact-123".to_string(),
                                mime_type: "text/plain".to_string(),
                                size_bytes: 42,
                            },
                        )),
                    },
                ),
            ),
        ],
    )];

    let dir = materialize_tasks_to_yaml(&tasks).unwrap();
    let files = list_dir_sorted(Path::new(&dir));
    let success_content = fs::read_to_string(Path::new(&dir).join(&files[1])).unwrap();

    assert!(success_content.contains("artifact_uid: artifact-123"));
    assert!(success_content.contains("mime_type: text/plain"));
    assert!(success_content.contains("size_bytes: 42"));
    assert!(!success_content.contains("filepath:"));
    assert!(!success_content.contains("description:"));

    cleanup_dir(&dir);
}
