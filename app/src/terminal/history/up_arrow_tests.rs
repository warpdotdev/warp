use chrono::{Duration, Local};
use settings::Setting;
use warp_core::SessionId;
use warpui::{App, AppContext, EntityId, SingletonEntity};

use super::{
    TuiHistoryItem, TuiHistoryItemKind, UpArrowHistoryConfig, prompt_history_for_terminal_view,
    up_arrow_history_for_terminal_view,
};
use crate::ai::agent::AIAgentExchangeId;
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::blocklist::history_model::AIQueryHistoryOutputStatus;
use crate::ai::blocklist::{BlocklistAIHistoryModel, PersistedAIInput, PersistedAIInputType};
use crate::ai::llms::LLMId;
use crate::settings::AISettings;
use crate::suggestions::ignored_suggestions_model::{IgnoredSuggestionsModel, SuggestionType};
use crate::terminal::{History, HistoryEntry, LinkedWorkflowData};
use crate::test_util::settings::initialize_settings_for_tests;

fn build_history_model(prompts: Vec<String>) -> BlocklistAIHistoryModel {
    let base = Local::now();
    let persisted_queries = prompts
        .into_iter()
        .enumerate()
        .map(|(index, text)| PersistedAIInput {
            exchange_id: AIAgentExchangeId::new(),
            conversation_id: AIConversationId::new(),
            start_ts: base + Duration::milliseconds(index as i64),
            inputs: vec![PersistedAIInputType::Query {
                text,
                context: Default::default(),
                referenced_attachments: Default::default(),
            }],
            output_status: AIQueryHistoryOutputStatus::Completed,
            working_directory: None,
            model_id: LLMId::from("test-model"),
            coding_model_id: LLMId::from("test-model"),
        })
        .collect();
    BlocklistAIHistoryModel::new(persisted_queries, vec![], &[])
}

fn command_entry(
    session_id: SessionId,
    command: &str,
    age: i64,
    is_agent_executed: bool,
    workflow_command: Option<&str>,
) -> HistoryEntry {
    HistoryEntry {
        session_id: Some(session_id),
        command: command.to_owned(),
        pwd: None,
        start_ts: Some(Local::now() + Duration::milliseconds(age)),
        completed_ts: None,
        exit_code: None,
        git_head: None,
        shell_host: None,
        workflow_id: None,
        workflow_command: workflow_command.map(str::to_owned),
        is_for_restored_block: false,
        is_agent_executed,
    }
}

fn combined_history(
    terminal_surface_id: EntityId,
    session_id: SessionId,
    include_prompts: bool,
    app: &AppContext,
) -> Vec<TuiHistoryItem> {
    up_arrow_history_for_terminal_view(
        terminal_surface_id,
        Some(session_id),
        UpArrowHistoryConfig {
            include_commands: true,
            include_prompts,
        },
        app,
    )
}

fn assert_prompt_history(prompts: &[&str], expected: &[&str]) {
    let prompts: Vec<String> = prompts.iter().map(|prompt| (*prompt).to_owned()).collect();
    let expected: Vec<String> = expected.iter().map(|entry| (*entry).to_owned()).collect();
    App::test((), |app| async move {
        let terminal_surface_id = EntityId::new();
        app.add_singleton_model(move |_| build_history_model(prompts));
        app.read(|ctx| {
            let texts: Vec<String> = prompt_history_for_terminal_view(terminal_surface_id, ctx)
                .into_iter()
                .map(|entry| entry.query_text)
                .collect();
            assert_eq!(texts, expected);
        });
    });
}

#[test]
fn prompt_history_dedupes_orders_and_excludes_whitespace() {
    assert_prompt_history(
        &[
            "deploy the app",
            "delete the cache",
            "deploy the app",
            "   ",
            "build the project",
        ],
        &["delete the cache", "deploy the app", "build the project"],
    );
}

#[test]
fn prompt_history_excludes_ignored_prompts() {
    let prompts: Vec<String> = ["deploy the app", "delete the cache", "build the project"]
        .iter()
        .map(|prompt| (*prompt).to_owned())
        .collect();
    App::test((), |app| async move {
        let terminal_surface_id = EntityId::new();
        app.add_singleton_model(move |_| build_history_model(prompts));
        app.add_singleton_model(|_| {
            IgnoredSuggestionsModel::new(vec![(
                "delete the cache".to_owned(),
                SuggestionType::AIQuery,
            )])
        });
        app.read(|ctx| {
            let texts: Vec<String> = prompt_history_for_terminal_view(terminal_surface_id, ctx)
                .into_iter()
                .map(|entry| entry.query_text)
                .collect();
            assert_eq!(
                texts,
                vec!["deploy the app".to_owned(), "build the project".to_owned()]
            );
        });
    });
}

#[test]
fn combined_history_returns_owned_kinds_and_dedupes_each_kind() {
    App::test((), |app| async move {
        let terminal_surface_id = EntityId::new();
        let session_id = SessionId::from(1);
        app.add_singleton_model(|_| {
            build_history_model(vec![
                "same".to_owned(),
                "older prompt".to_owned(),
                "same".to_owned(),
                "   ".to_owned(),
            ])
        });
        app.add_singleton_model(|_| {
            History::new_for_up_arrow_test(
                session_id,
                vec![
                    command_entry(session_id, " same ", 0, false, None),
                    command_entry(session_id, "older command", 1, false, None),
                    command_entry(session_id, " same ", 2, false, None),
                    command_entry(session_id, "   ", 3, false, None),
                ],
            )
        });

        app.read(|ctx| {
            assert_eq!(
                combined_history(terminal_surface_id, session_id, true, ctx),
                vec![
                    TuiHistoryItem {
                        text: "older prompt".to_owned(),
                        kind: TuiHistoryItemKind::Prompt,
                    },
                    TuiHistoryItem {
                        text: "same".to_owned(),
                        kind: TuiHistoryItemKind::Prompt,
                    },
                    TuiHistoryItem {
                        text: "older command".to_owned(),
                        kind: TuiHistoryItemKind::Command {
                            linked_workflow_data: None,
                        },
                    },
                    TuiHistoryItem {
                        text: "same".to_owned(),
                        kind: TuiHistoryItemKind::Command {
                            linked_workflow_data: None,
                        },
                    },
                ]
            );
        });
    });
}

#[test]
fn combined_history_preserves_command_workflow_data() {
    App::test((), |app| async move {
        let terminal_surface_id = EntityId::new();
        let session_id = SessionId::from(1);
        app.add_singleton_model(|_| build_history_model(vec!["prompt".to_owned()]));
        app.add_singleton_model(|_| {
            History::new_for_up_arrow_test(
                session_id,
                vec![command_entry(
                    session_id,
                    "deploy",
                    0,
                    false,
                    Some("deploy {{environment}}"),
                )],
            )
        });

        app.read(|ctx| {
            assert_eq!(
                combined_history(terminal_surface_id, session_id, true, ctx),
                vec![
                    TuiHistoryItem {
                        text: "prompt".to_owned(),
                        kind: TuiHistoryItemKind::Prompt,
                    },
                    TuiHistoryItem {
                        text: "deploy".to_owned(),
                        kind: TuiHistoryItemKind::Command {
                            linked_workflow_data: Some(LinkedWorkflowData::Command(
                                "deploy {{environment}}".to_owned(),
                            )),
                        },
                    },
                ]
            );
        });
    });
}

#[test]
fn combined_history_excludes_ignored_prompts_and_commands() {
    App::test((), |app| async move {
        let terminal_surface_id = EntityId::new();
        let session_id = SessionId::from(1);
        app.add_singleton_model(|_| {
            build_history_model(vec!["keep prompt".to_owned(), "ignore prompt".to_owned()])
        });
        app.add_singleton_model(|_| {
            History::new_for_up_arrow_test(
                session_id,
                vec![
                    command_entry(session_id, "keep command", 0, false, None),
                    command_entry(session_id, "ignore command", 1, false, None),
                ],
            )
        });
        app.add_singleton_model(|_| {
            IgnoredSuggestionsModel::new(vec![
                ("ignore prompt".to_owned(), SuggestionType::AIQuery),
                ("ignore command".to_owned(), SuggestionType::ShellCommand),
            ])
        });

        app.read(|ctx| {
            let history = combined_history(terminal_surface_id, session_id, true, ctx);
            assert_eq!(
                history
                    .iter()
                    .map(|item| item.text.as_str())
                    .collect::<Vec<_>>(),
                vec!["keep prompt", "keep command"]
            );
        });
    });
}

#[test]
fn combined_history_respects_agent_command_setting() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        let terminal_surface_id = EntityId::new();
        let session_id = SessionId::from(1);
        app.add_singleton_model(|_| build_history_model(Vec::new()));
        app.add_singleton_model(|_| {
            History::new_for_up_arrow_test(
                session_id,
                vec![
                    command_entry(session_id, "user command", 0, false, None),
                    command_entry(session_id, "agent command", 1, true, None),
                ],
            )
        });

        app.read(|ctx| {
            assert_eq!(
                combined_history(terminal_surface_id, session_id, false, ctx)
                    .into_iter()
                    .map(|item| item.text)
                    .collect::<Vec<_>>(),
                vec!["user command"]
            );
        });

        AISettings::handle(&app).update(&mut app, |settings, ctx| {
            settings
                .include_agent_commands_in_history
                .set_value(true, ctx)
                .unwrap();
        });
        app.read(|ctx| {
            assert_eq!(
                combined_history(terminal_surface_id, session_id, false, ctx)
                    .into_iter()
                    .map(|item| item.text)
                    .collect::<Vec<_>>(),
                vec!["user command", "agent command"]
            );
        });
    });
}
