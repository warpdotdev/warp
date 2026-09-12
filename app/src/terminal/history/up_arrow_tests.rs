use std::sync::Arc;

use chrono::{Duration, Local};
use settings::Setting;
use warp_core::SessionId;
use warpui::{App, AppContext, EntityId, SingletonEntity};

use super::{UpArrowHistoryConfig, prompt_history_for_terminal_surface};
use crate::ai::agent::AIAgentExchangeId;
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::blocklist::history_model::AIQueryHistoryOutputStatus;
use crate::ai::blocklist::{BlocklistAIHistoryModel, PersistedAIInput, PersistedAIInputType};
use crate::ai::llms::LLMId;
use crate::input_suggestions::HistoryInputSuggestion;
use crate::settings::AISettings;
use crate::suggestions::ignored_suggestions_model::{IgnoredSuggestionsModel, SuggestionType};
use crate::terminal::model::session::command_executor::NoOpCommandExecutor;
use crate::terminal::model::session::{Session, SessionInfo};
use crate::terminal::{History, HistoryEntry, LinkedWorkflowData};
use crate::test_util::settings::initialize_settings_for_tests;

#[derive(Debug, PartialEq, Eq)]
enum TestHistoryItem {
    Prompt(String),
    Command {
        text: String,
        linked_workflow_data: Option<LinkedWorkflowData>,
    },
}

impl TestHistoryItem {
    fn text(&self) -> &str {
        match self {
            Self::Prompt(text) | Self::Command { text, .. } => text,
        }
    }
}

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
    let mut entry = HistoryEntry::command_only(command);
    entry.session_id = Some(session_id);
    entry.start_ts = Some(Local::now() + Duration::milliseconds(age));
    entry.is_agent_executed = is_agent_executed;
    entry.set_linked_workflow(None, workflow_command.map(str::to_owned));
    entry
}

fn combined_history(
    terminal_surface_id: EntityId,
    session_id: SessionId,
    include_prompts: bool,
    app: &AppContext,
) -> Vec<TestHistoryItem> {
    History::handle(app)
        .as_ref(app)
        .up_arrow_suggestions_for_terminal_surface(
            terminal_surface_id,
            Some(session_id),
            UpArrowHistoryConfig {
                include_commands: true,
                include_prompts,
            },
            app,
        )
        .into_iter()
        .map(|suggestion| {
            let text = suggestion.normalized_text().to_owned();
            match suggestion {
                HistoryInputSuggestion::Command { entry } => TestHistoryItem::Command {
                    text,
                    linked_workflow_data: entry.linked_workflow_data(),
                },
                HistoryInputSuggestion::AIQuery { .. } => TestHistoryItem::Prompt(text),
            }
        })
        .collect()
}

async fn add_command_history(app: &mut App, session_id: SessionId, entries: Vec<HistoryEntry>) {
    let mut session_info = SessionInfo::new_for_test();
    session_info.session_id = session_id;
    let session = Arc::new(Session::new(
        session_info,
        Arc::new(NoOpCommandExecutor::default()),
    ));
    let (initialized_tx, initialized_rx) = async_channel::bounded(1);
    let history = app.add_singleton_model(|_| History::default());
    app.update(|ctx| {
        ctx.subscribe_to_model(&history, move |_, event, _| match event {
            crate::terminal::HistoryEvent::Initialized(id) if *id == session_id => {
                let _ = initialized_tx.try_send(());
            }
            crate::terminal::HistoryEvent::Initialized(_) => {}
        });
        history.update(ctx, |history, ctx| {
            history.init_session_with(session, async { Vec::new() }, ctx);
        });
    });
    initialized_rx
        .recv()
        .await
        .expect("history initialization should complete");
    history.update(app, |history, _| {
        for entry in entries {
            history.append_commands(session_id, vec![entry]);
        }
    });
}
fn assert_prompt_history(prompts: &[&str], expected: &[&str]) {
    let prompts: Vec<String> = prompts.iter().map(|prompt| (*prompt).to_owned()).collect();
    let expected: Vec<String> = expected.iter().map(|entry| (*entry).to_owned()).collect();
    App::test((), |app| async move {
        let terminal_surface_id = EntityId::new();
        app.add_singleton_model(move |_| build_history_model(prompts));
        app.read(|ctx| {
            let texts: Vec<String> = prompt_history_for_terminal_surface(terminal_surface_id, ctx)
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
            let texts: Vec<String> = prompt_history_for_terminal_surface(terminal_surface_id, ctx)
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
fn combined_history_dedupes_each_kind() {
    App::test((), |mut app| async move {
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
        add_command_history(
            &mut app,
            session_id,
            vec![
                command_entry(session_id, " same ", 0, false, None),
                command_entry(session_id, "older command", 1, false, None),
                command_entry(session_id, "same", 2, false, None),
                command_entry(session_id, "   ", 3, false, None),
            ],
        )
        .await;

        app.read(|ctx| {
            assert_eq!(
                combined_history(terminal_surface_id, session_id, true, ctx),
                vec![
                    TestHistoryItem::Prompt("older prompt".to_owned()),
                    TestHistoryItem::Prompt("same".to_owned()),
                    TestHistoryItem::Command {
                        text: "older command".to_owned(),
                        linked_workflow_data: None,
                    },
                    TestHistoryItem::Command {
                        text: "same".to_owned(),
                        linked_workflow_data: None,
                    },
                ]
            );
        });
    });
}

#[test]
fn combined_history_preserves_command_workflow_data() {
    App::test((), |mut app| async move {
        let terminal_surface_id = EntityId::new();
        let session_id = SessionId::from(1);
        app.add_singleton_model(|_| build_history_model(vec!["prompt".to_owned()]));
        add_command_history(
            &mut app,
            session_id,
            vec![command_entry(
                session_id,
                "deploy",
                0,
                false,
                Some("deploy {{environment}}"),
            )],
        )
        .await;

        app.read(|ctx| {
            assert_eq!(
                combined_history(terminal_surface_id, session_id, true, ctx),
                vec![
                    TestHistoryItem::Prompt("prompt".to_owned()),
                    TestHistoryItem::Command {
                        text: "deploy".to_owned(),
                        linked_workflow_data: Some(LinkedWorkflowData::Command(
                            "deploy {{environment}}".to_owned(),
                        )),
                    },
                ]
            );
        });
    });
}

#[test]
fn combined_history_excludes_ignored_prompts_and_commands() {
    App::test((), |mut app| async move {
        let terminal_surface_id = EntityId::new();
        let session_id = SessionId::from(1);
        app.add_singleton_model(|_| {
            build_history_model(vec!["keep prompt".to_owned(), "ignore prompt".to_owned()])
        });
        add_command_history(
            &mut app,
            session_id,
            vec![
                command_entry(session_id, "keep command", 0, false, None),
                command_entry(session_id, "ignore command", 1, false, None),
            ],
        )
        .await;
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
                    .map(TestHistoryItem::text)
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
        add_command_history(
            &mut app,
            session_id,
            vec![
                command_entry(session_id, "user command", 0, false, None),
                command_entry(session_id, "agent command", 1, true, None),
            ],
        )
        .await;

        app.read(|ctx| {
            assert_eq!(
                combined_history(terminal_surface_id, session_id, false, ctx)
                    .into_iter()
                    .map(|item| item.text().to_owned())
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
                    .map(|item| item.text().to_owned())
                    .collect::<Vec<_>>(),
                vec!["user command", "agent command"]
            );
        });
    });
}
