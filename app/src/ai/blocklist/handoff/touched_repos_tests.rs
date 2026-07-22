//! Tests for `touched_repos.rs`.
//!
//! Only covers `find_git_root`, which actually walks the filesystem against a
//! temporary directory layout. The pure helpers (`parse_github_repo`,
//! `pick_handoff_overlap_env`) are exercised end-to-end by the handoff submit
//! path and don't get standalone tests — their correctness is enforced by
//! their call sites.

use std::collections::HashSet;
use std::fs;

use chrono::Local;
use tempfile::tempdir;
use tokio::runtime::Runtime;
use warpui::{App, EntityId};

use super::*;
use crate::ai::agent::conversation::{AIConversation, ConversationStatus};
use crate::ai::agent::{
    AIAgentExchange, AIAgentExchangeId, AIAgentInput, AIAgentOutputStatus, FinishedAIAgentOutput,
    Shared, UserQueryMode,
};
use crate::ai::blocklist::ResponseStreamId;
use crate::ai::llms::LLMId;

#[test]
fn find_git_root_walks_up_to_dot_git() {
    let tmp = tempdir().unwrap();
    let repo = tmp.path().join("repo");
    let nested = repo.join("src").join("nested");
    fs::create_dir_all(&nested).unwrap();
    fs::create_dir_all(repo.join(".git")).unwrap();

    let file_in_repo = nested.join("foo.rs");
    fs::write(&file_in_repo, "").unwrap();

    let outside = tmp.path().join("not_a_repo").join("file.txt");
    fs::create_dir_all(outside.parent().unwrap()).unwrap();
    fs::write(&outside, "").unwrap();

    let rt = Runtime::new().unwrap();
    let (root_for_file, root_for_dir, root_for_outside) = rt.block_on(async {
        (
            find_git_root(&file_in_repo).await,
            find_git_root(&nested).await,
            find_git_root(&outside).await,
        )
    });

    assert_eq!(root_for_file.expect("root for file inside repo"), repo);
    assert_eq!(root_for_dir.expect("root for directory inside repo"), repo);
    assert!(root_for_outside.is_none());
}

fn exchange_with_working_directory(
    working_directory: String,
    output_status: AIAgentOutputStatus,
) -> AIAgentExchange {
    AIAgentExchange {
        id: AIAgentExchangeId::new(),
        input: vec![AIAgentInput::UserQuery {
            query: "test".to_owned(),
            context: Default::default(),
            static_query_type: None,
            referenced_attachments: Default::default(),
            user_query_mode: UserQueryMode::Normal,
            running_command: None,
            intended_agent: None,
        }],
        output_status,
        added_message_ids: HashSet::new(),
        start_time: Local::now(),
        finish_time: None,
        time_to_first_token_ms: None,
        working_directory: Some(working_directory),
        model_id: LLMId::from("test-model"),
        request_cost: None,
        coding_model_id: LLMId::from("test-coding-model"),
        cli_agent_model_id: LLMId::from("test-cli-model"),
        computer_use_model_id: LLMId::from("test-computer-use-model"),
        response_initiator: None,
    }
}

#[test]
fn descendant_safe_paths_include_only_terminal_loaded_children() {
    App::test((), |mut app| async move {
        let terminal_surface_id = EntityId::new();
        let history = app.add_model(|_| BlocklistAIHistoryModel::new_for_test());
        let parent = AIConversation::new(false, false);
        let parent_id = parent.id();
        let mut terminal_child = AIConversation::new(false, false);
        terminal_child.set_parent_conversation_id(parent_id);
        let terminal_child_id = terminal_child.id();
        let mut active_child = AIConversation::new(false, false);
        active_child.set_parent_conversation_id(parent_id);
        let active_child_id = active_child.id();

        history.update(&mut app, |history, ctx| {
            history.restore_conversations(
                terminal_surface_id,
                vec![parent, terminal_child, active_child],
                ctx,
            );
            history
                .conversation_mut(&terminal_child_id)
                .expect("terminal child")
                .append_reassigned_exchange(
                    &ResponseStreamId::new_for_test(),
                    exchange_with_working_directory(
                        "/terminal-child".to_owned(),
                        AIAgentOutputStatus::Finished {
                            finished_output: FinishedAIAgentOutput::Success {
                                output: Shared::new(Default::default()),
                            },
                        },
                    ),
                    terminal_surface_id,
                    ctx,
                )
                .expect("append terminal child exchange");
            history
                .conversation_mut(&active_child_id)
                .expect("active child")
                .append_reassigned_exchange(
                    &ResponseStreamId::new_for_test(),
                    exchange_with_working_directory(
                        "/active-child".to_owned(),
                        AIAgentOutputStatus::Streaming { output: None },
                    ),
                    terminal_surface_id,
                    ctx,
                )
                .expect("append active child exchange");
            history.update_conversation_status(
                terminal_surface_id,
                terminal_child_id,
                ConversationStatus::Success,
                ctx,
            );
            history.update_conversation_status(
                terminal_surface_id,
                active_child_id,
                ConversationStatus::InProgress,
                ctx,
            );
        });

        history.read(&app, |history, _| {
            let paths = descendant_safe_paths(history, parent_id);
            assert!(
                paths.contains(
                    &StandardizedPath::try_new("/terminal-child").expect("absolute path")
                )
            );
            assert!(
                !paths
                    .contains(&StandardizedPath::try_new("/active-child").expect("absolute path"))
            );
        });
    });
}
