use std::path::PathBuf;
use std::pin::pin;
use std::sync::Arc;

use chrono::Local;
use futures::Future;
use futures::future::join_all;
use futures_lite::StreamExt;
use itertools::Itertools;
use warp_core::command::ExitCode;
use warpui::{App, ModelHandle};

use super::{HistoryEntry, HistoryEvent, PersistedCommand, ShellHost};
use crate::ai::agent::conversation::AIConversationId;
use crate::terminal::History;
use crate::terminal::model::block::{
    AgentInteractionMetadata, SerializedAIMetadata, SerializedBlock,
};
use crate::terminal::model::bootstrap::BootstrapStage;
use crate::terminal::model::session::command_executor::testing::TestCommandExecutor;
use crate::terminal::model::session::{BootstrapSessionType, Session, SessionId, SessionInfo};
use crate::terminal::model::test_utils::TestBlockBuilder;
use crate::terminal::shell::ShellType;
use crate::test_util::{Stub, VirtualFS};

impl History {
    /// Returns a Future that completes when `History` is initialized for all sessions with IDs in
    /// the given `session_ids` vector.
    pub async fn initialized_sessions(
        history_handle: &mut ModelHandle<History>,
        app: &mut App,
        session_ids: Vec<SessionId>,
    ) {
        let mut history_initialization_receivers = vec![];
        for session_id in session_ids {
            let is_session_initialized = history_handle.read(app, |history, _| {
                history.is_session_initialized(&session_id)
            });
            if !is_session_initialized {
                let (tx, rx) = async_channel::unbounded();
                let history_handle_clone = history_handle.clone();
                app.update(|ctx| {
                    ctx.subscribe_to_model(&history_handle_clone, move |_, event, _| {
                        let HistoryEvent::Initialized(event_id) = event;
                        if session_id == *event_id {
                            let _ = tx.try_send(());
                        }
                    });
                });
                history_initialization_receivers.push(rx);
            }
        }

        join_all(
            history_initialization_receivers
                .into_iter()
                .map(|rx| async move { rx.recv().await }),
        )
        .await;
    }
}

impl HistoryEntry {
    fn with_session_id<S: Into<String>>(session_id: SessionId, command: S) -> Self {
        Self {
            session_id: Some(session_id),
            exit_code: Some(ExitCode::from(0)),
            command: command.into(),
            workflow_id: None,
            workflow_command: None,
            pwd: None,
            start_ts: None,
            completed_ts: None,
            git_head: None,
            shell_host: None,
            is_agent_executed: false,
            is_for_restored_block: false,
        }
    }

    pub fn with_pwd_and_exit_code<S: Into<String>>(
        command: S,
        pwd: S,
        exit_code: impl Into<ExitCode>,
    ) -> Self {
        Self {
            session_id: None,
            exit_code: Some(exit_code.into()),
            command: command.into(),
            workflow_id: None,
            workflow_command: None,
            pwd: Some(pwd.into()),
            start_ts: None,
            completed_ts: None,
            git_head: None,
            shell_host: None,
            is_agent_executed: false,
            is_for_restored_block: false,
        }
    }
}

#[test]
fn history_entry_for_restored_block_preserves_agent_execution() {
    let mut block = TestBlockBuilder::new()
        .with_bootstrap_stage(BootstrapStage::RestoreBlocks)
        .build();
    block.set_agent_interaction_mode_for_requested_command(
        String::from("action-id").into(),
        None,
        AIConversationId::new(),
    );

    let entry = HistoryEntry::for_restored_block("ls".to_string(), &block);

    assert!(entry.is_agent_executed);
}

#[test]
fn history_entry_for_restored_block_does_not_treat_all_agent_interactions_as_agent_execution() {
    let mut block = TestBlockBuilder::new()
        .with_bootstrap_stage(BootstrapStage::RestoreBlocks)
        .build();
    block.set_agent_interaction_mode(AgentInteractionMetadata::new(
        None,
        AIConversationId::new(),
        None,
        None,
        false,
        false,
    ));

    let entry = HistoryEntry::for_restored_block("ls".to_string(), &block);

    assert!(!entry.is_agent_executed);
}

#[test]
fn history_entry_for_completed_block_preserves_agent_execution() {
    let ai_metadata = serde_json::to_string(&SerializedAIMetadata::from(
        AgentInteractionMetadata::new_hidden(
            String::from("action-id").into(),
            AIConversationId::new(),
        ),
    ))
    .unwrap();
    let block = SerializedBlock {
        ai_metadata: Some(ai_metadata),
        ..SerializedBlock::new_for_test("ls".as_bytes().to_vec(), vec![])
    };

    let entry = HistoryEntry::for_completed_block("ls".to_string(), &block);

    assert!(entry.is_agent_executed);
}

#[test]
fn history_entry_for_completed_block_does_not_treat_all_agent_interactions_as_agent_execution() {
    let ai_metadata = serde_json::to_string(&SerializedAIMetadata::from(
        AgentInteractionMetadata::new(None, AIConversationId::new(), None, None, false, false),
    ))
    .unwrap();
    let block = SerializedBlock {
        ai_metadata: Some(ai_metadata),
        ..SerializedBlock::new_for_test("ls".as_bytes().to_vec(), vec![])
    };

    let entry = HistoryEntry::for_completed_block("ls".to_string(), &block);

    assert!(!entry.is_agent_executed);
}

/// Initializes history for testing
/// initialization is complete.
///
/// `history_commands_fn` is an async function that should emulate the async operation of reading
/// command from a session's history file.
///
/// `session_commands_to_append` are commands that are appended to the in-memory session history
/// after commands have been read from the history file.
async fn initialize_history_for_testing<F>(
    history_handle: &mut ModelHandle<History>,
    session: Arc<Session>,
    history_commands_fn: F,
    session_commands_to_append: Vec<String>,
    app: &mut App,
) where
    F: 'static + Future<Output = Vec<String>> + Send,
{
    let session_id = session.id();
    history_handle.update(app, move |history, ctx| {
        history.init_session_with(session, history_commands_fn, ctx);
    });
    History::initialized_sessions(history_handle, app, vec![session_id]).await;
    history_handle.update(app, move |history, _| {
        history.append_commands(
            session_id,
            session_commands_to_append
                .into_iter()
                .map(|command| HistoryEntry::with_session_id(session_id, command))
                .collect_vec(),
        );
    });
}

#[test]
fn test_append_commands() {
    VirtualFS::test("history_append_command", |dirs, mut sandbox| {
        App::test((), |mut app| async move {
            sandbox.with_files(vec![Stub::FileWithContentToBeTrimmed(
                ".bash_history",
                r#"
                    ls
                    pwd
                    warp --listen --ports=8080,8081
                "#,
            )]);

            let mut history_handle = app.add_model(|_| History::default());
            let file = Some(dirs.tests().join(".bash_history").display().to_string());
            let session = Arc::new(Session::new(
                SessionInfo::new_for_test()
                    .with_histfile(file)
                    .with_shell_type(ShellType::Bash),
                Arc::new(TestCommandExecutor::default()),
            ));

            let session_clone = session.clone();
            initialize_history_for_testing(
                &mut history_handle,
                session.clone(),
                async move { session_clone.read_history(false).await },
                vec![
                    "pwd".to_owned(),
                    "ls".to_owned(),
                    "pwd".to_owned(),
                    "git status".to_owned(),
                ],
                &mut app,
            )
            .await;

            history_handle.read(&app, |history, _ctx| {
                assert_eq!(
                    history.commands(session.id()).unwrap_or_default(),
                    vec![
                        &HistoryEntry::command_only("warp --listen --ports=8080,8081"),
                        &HistoryEntry::with_session_id(session.id(), "ls"),
                        &HistoryEntry::with_session_id(session.id(), "pwd"),
                        &HistoryEntry::with_session_id(session.id(), "git status"),
                    ]
                );
            });
        });
    });
}

#[cfg_attr(windows, ignore = "TODO(CORE-3626)")]
#[test]
fn test_append_multiple_sessions() {
    VirtualFS::test("append_multiple_sessions", |dirs, mut sandbox| {
        App::test((), |mut app| async move {
            sandbox.with_files(vec![Stub::FileWithContentToBeTrimmed(
                ".bash_history",
                r#"
                    cd warp
                    cargo run --bin dev
                "#,
            )]);

            let mut history_handle = app.add_model(|_| History::default());
            let file = Some(dirs.tests().join(".bash_history").display().to_string());
            let session = Arc::new(Session::new(
                SessionInfo::new_for_test()
                    .with_histfile(file)
                    .with_shell_type(ShellType::Bash),
                Arc::new(TestCommandExecutor::default()),
            ));

            let session_clone = session.clone();
            initialize_history_for_testing(
                &mut history_handle,
                session.clone(),
                async move { session_clone.read_history(false).await },
                vec![
                    "ls target/".to_owned(),
                    "cargo clean".to_owned(),
                    "ls target/".to_owned(),
                ],
                &mut app,
            )
            .await;

            history_handle.read(&app, |history, _ctx| {
                assert_eq!(
                    history.commands(session.id()).unwrap_or_default(),
                    vec![
                        &HistoryEntry::command_only("cd warp"),
                        &HistoryEntry::command_only("cargo run --bin dev"),
                        &HistoryEntry::with_session_id(session.id(), "cargo clean"),
                        &HistoryEntry::with_session_id(session.id(), "ls target/"),
                    ]
                );
            });

            let second_session = Arc::new(Session::new(
                SessionInfo::new_for_test().with_id(1),
                Arc::new(TestCommandExecutor::default()),
            ));

            initialize_history_for_testing(
                &mut history_handle,
                second_session.clone(),
                async move { Vec::new() },
                vec!["ls target/".to_owned()],
                &mut app,
            )
            .await;

            history_handle.read(&app, |history, _ctx| {
                assert_eq!(
                    history.commands(second_session.id()).unwrap_or_default(),
                    vec![
                        &HistoryEntry::command_only("cd warp"),
                        &HistoryEntry::command_only("cargo run --bin dev"),
                        &HistoryEntry::with_session_id(session.id(), "cargo clean"),
                        &HistoryEntry::with_session_id(second_session.id(), "ls target/"),
                    ]
                );
            });
        });
    });
}

#[test]
fn test_len() {
    VirtualFS::test("history_len", |dirs, mut sandbox| {
        App::test((), |mut app| async move {
            sandbox.with_files(vec![Stub::FileWithContentToBeTrimmed(
                ".bash_history",
                r#"
                    cd warp
                    cargo run --bin dev
                    touch
                "#,
            )]);

            let mut history_handle = app.add_model(|_| History::default());
            let file = Some(dirs.tests().join(".bash_history").display().to_string());
            let session = Arc::new(Session::new(
                SessionInfo::new_for_test()
                    .with_histfile(file)
                    .with_shell_type(ShellType::Bash),
                Arc::new(TestCommandExecutor::default()),
            ));

            let session_clone = session.clone();
            initialize_history_for_testing(
                &mut history_handle,
                session.clone(),
                async move { session_clone.read_history(false).await },
                vec!["ls".to_owned(), "echo 'hello'".to_owned()],
                &mut app,
            )
            .await;

            history_handle.read(&app, |history, _ctx| {
                assert_eq!(history.len(session.id()), 5);
            });

            history_handle.update(&mut app, |history, _ctx| {
                history.append_commands(
                    session.id(),
                    vec![HistoryEntry::with_session_id(
                        session.id(),
                        "cargo run --bin dev".to_string(),
                    )],
                );
            });

            history_handle.read(&app, |history, _ctx| {
                assert_eq!(history.len(session.id()), 5);

                assert_eq!(
                    history.commands(session.id()).unwrap_or_default(),
                    vec![
                        &HistoryEntry::command_only("cd warp"),
                        &HistoryEntry::command_only("touch"),
                        &HistoryEntry::with_session_id(session.id(), "ls"),
                        &HistoryEntry::with_session_id(session.id(), "echo 'hello'"),
                        &HistoryEntry::with_session_id(session.id(), "cargo run --bin dev")
                    ]
                );
            });
        });
    });
}

#[test]
fn test_multiple_shells() {
    VirtualFS::test("multiple_shells", |dirs, mut sandbox| {
        App::test((), |mut app| async move {
            sandbox.with_files(vec![
                Stub::FileWithContentToBeTrimmed(
                    ".bash_history",
                    r#"
                    bash-cmd1
                    bash-cmd2
                "#,
                ),
                Stub::FileWithContentToBeTrimmed(
                    ".zsh_history",
                    r#"
                    zsh-cmd1
                    zsh-cmd2
                "#,
                ),
            ]);

            let mut history_handle = app.add_model(|_| History::default());

            let root = dirs.tests();
            let (bash_file, zsh_file) = (
                Some(root.join(".bash_history").display().to_string()),
                Some(root.join(".zsh_history").display().to_string()),
            );

            let (bash_session, zsh_session) = (
                Arc::new(Session::new(
                    SessionInfo::new_for_test()
                        .with_id(0)
                        .with_histfile(bash_file)
                        .with_shell_type(ShellType::Bash),
                    Arc::new(TestCommandExecutor::default()),
                )),
                Arc::new(Session::new(
                    SessionInfo::new_for_test()
                        .with_id(1)
                        .with_histfile(zsh_file)
                        .with_shell_type(ShellType::Zsh),
                    Arc::new(TestCommandExecutor::default()),
                )),
            );

            let bash_session_clone = bash_session.clone();
            initialize_history_for_testing(
                &mut history_handle,
                bash_session.clone(),
                async move { bash_session_clone.read_history(false).await },
                vec!["bash-cmd3".to_owned()],
                &mut app,
            )
            .await;

            let zsh_session_clone = zsh_session.clone();
            initialize_history_for_testing(
                &mut history_handle,
                zsh_session.clone(),
                async move { zsh_session_clone.read_history(false).await },
                vec!["zsh-cmd3".to_owned()],
                &mut app,
            )
            .await;

            history_handle.read(&app, |history, _ctx| {
                assert_eq!(history.len(bash_session.id()), 3);
                assert_eq!(history.len(zsh_session.id()), 3);
            });

            history_handle.update(&mut app, |history, _ctx| {
                history.append_commands(
                    bash_session.id(),
                    vec![HistoryEntry::with_session_id(
                        bash_session.id(),
                        "bash-cmd4".to_string(),
                    )],
                );
            });
            history_handle.read(&app, |history, _ctx| {
                assert_eq!(history.len(bash_session.id()), 4);
                assert_eq!(history.len(zsh_session.id()), 3);
            });

            history_handle.update(&mut app, |history, _ctx| {
                history.append_commands(
                    zsh_session.id(),
                    vec![HistoryEntry::with_session_id(
                        zsh_session.id(),
                        "zsh-cmd4".to_string(),
                    )],
                );
            });
            history_handle.read(&app, |history, _ctx| {
                assert_eq!(history.len(bash_session.id()), 4);
                assert_eq!(history.len(zsh_session.id()), 4);
            });
        });
    });
}

#[test]
fn test_multiple_machines() {
    App::test((), |mut app| async move {
        let session_a = Arc::new(Session::new(
            SessionInfo::new_for_test()
                .with_id(0)
                .with_shell_type(ShellType::Zsh)
                .with_session_type(BootstrapSessionType::WarpifiedRemote)
                .with_hostname("prod".to_string())
                .with_user("user".to_string())
                .with_ssh_socket_path(PathBuf::from("~/.ssh/12345"))
                .with_home_dir("/users/warpuser".to_owned()),
            Arc::new(TestCommandExecutor::default()),
        ));

        let session_b = Arc::new(Session::new(
            SessionInfo::new_for_test()
                .with_id(1)
                .with_shell_type(ShellType::Zsh)
                .with_session_type(BootstrapSessionType::WarpifiedRemote)
                .with_hostname("dev".to_string())
                .with_user("user2".to_string())
                .with_ssh_socket_path(PathBuf::from("~/.ssh/12345"))
                .with_home_dir("/users/warpuser".to_owned()),
            Arc::new(TestCommandExecutor::default()),
        ));

        let mut history_handle = app.add_model(|_| History::default());
        initialize_history_for_testing(
            &mut history_handle,
            session_a.clone(),
            async { vec!["p1".to_string(), "p2".to_string()] },
            vec!["p3".to_string()],
            &mut app,
        )
        .await;
        initialize_history_for_testing(
            &mut history_handle,
            session_b.clone(),
            async { vec!["d1".to_string(), "d2".to_string()] },
            vec!["d3".to_string()],
            &mut app,
        )
        .await;

        history_handle.read(&app, |history, _ctx| {
            assert_eq!(history.len(session_a.id()), 3);
            assert_eq!(history.len(session_b.id()), 3);
        });

        history_handle.update(&mut app, |history, _ctx| {
            history.append_commands(
                session_a.id(),
                vec![HistoryEntry::with_session_id(
                    session_a.id(),
                    "p4".to_string(),
                )],
            );
        });
        history_handle.read(&app, |history, _ctx| {
            assert_eq!(history.len(session_a.id()), 4);
            assert_eq!(history.len(session_b.id()), 3);
        });

        history_handle.update(&mut app, |history, _ctx| {
            history.append_commands(
                session_b.id(),
                vec![HistoryEntry::with_session_id(
                    session_b.id(),
                    "d4".to_string(),
                )],
            );
        });
        history_handle.read(&app, |history, _ctx| {
            assert_eq!(history.len(session_a.id()), 4);
            assert_eq!(history.len(session_b.id()), 4);
        });
    });
}

#[test]
fn test_sessions_same_shell_same_machine() {
    App::test((), |mut app| async move {
        let session_a = Arc::new(Session::new(
            SessionInfo::new_for_test()
                .with_id(0)
                .with_shell_type(ShellType::Zsh),
            Arc::new(TestCommandExecutor::default()),
        ));
        let session_b = Arc::new(Session::new(
            SessionInfo::new_for_test()
                .with_id(1)
                .with_shell_type(ShellType::Zsh),
            Arc::new(TestCommandExecutor::default()),
        ));

        let mut history_handle = app.add_model(|_| History::default());
        initialize_history_for_testing(
            &mut history_handle,
            session_a.clone(),
            async { vec!["z1".to_string(), "z2".to_string()] },
            vec!["z3".to_string()],
            &mut app,
        )
        .await;
        initialize_history_for_testing(
            &mut history_handle,
            session_b.clone(),
            async { vec![] },
            vec!["z4".to_string()],
            &mut app,
        )
        .await;

        history_handle.read(&app, |history, _ctx| {
            assert_eq!(history.len(session_a.id()), 3);
            assert_eq!(history.len(session_b.id()), 4);
        });

        history_handle.update(&mut app, |history, _ctx| {
            history.append_commands(
                session_a.id(),
                vec![HistoryEntry::with_session_id(session_a.id(), "z4")],
            );
        });

        history_handle.read(&app, |history, _ctx| {
            assert_eq!(history.len(session_a.id()), 4);
            assert_eq!(history.len(session_b.id()), 4);
        });

        history_handle.update(&mut app, |history, _ctx| {
            history.append_commands(
                session_b.id(),
                vec![HistoryEntry::with_session_id(session_b.id(), "z5")],
            );
        });
        history_handle.read(&app, |history, _ctx| {
            assert_eq!(history.len(session_a.id()), 4);
            assert_eq!(history.len(session_b.id()), 5);
        });

        history_handle.update(&mut app, |history, _ctx| {
            history.append_commands(
                session_b.id(),
                vec![HistoryEntry::with_session_id(session_b.id(), "z5")],
            );
        });
        history_handle.read(&app, |history, _ctx| {
            assert_eq!(history.len(session_a.id()), 4);
            assert_eq!(history.len(session_b.id()), 5);
        });
    });
}

#[test]
fn test_sessions_no_dupes_new_session() {
    App::test((), |mut app| async move {
        let session_a = Arc::new(Session::new(
            SessionInfo::new_for_test()
                .with_id(0)
                .with_shell_type(ShellType::Zsh),
            Arc::new(TestCommandExecutor::default()),
        ));
        let session_b = Arc::new(Session::new(
            SessionInfo::new_for_test()
                .with_id(1)
                .with_shell_type(ShellType::Zsh),
            Arc::new(TestCommandExecutor::default()),
        ));
        let session_c = Arc::new(Session::new(
            SessionInfo::new_for_test()
                .with_id(2)
                .with_shell_type(ShellType::Zsh),
            Arc::new(TestCommandExecutor::default()),
        ));

        let mut history_handle = app.add_model(|_| History::default());

        initialize_history_for_testing(
            &mut history_handle,
            session_a.clone(),
            async { vec!["z1".to_string(), "z2".to_string()] },
            vec!["z3".to_string()],
            &mut app,
        )
        .await;
        initialize_history_for_testing(
            &mut history_handle,
            session_b.clone(),
            async { vec![] },
            vec!["z2".to_string()],
            &mut app,
        )
        .await;

        history_handle.read(&app, |history, _ctx| {
            assert_eq!(
                history.commands(session_a.id()).unwrap_or_default(),
                vec![
                    &HistoryEntry::command_only("z1"),
                    &HistoryEntry::command_only("z2"),
                    &HistoryEntry::with_session_id(session_a.id(), "z3"),
                ]
            );
            assert_eq!(
                history.commands(session_b.id()).unwrap_or_default(),
                vec![
                    &HistoryEntry::command_only("z1"),
                    &HistoryEntry::with_session_id(session_a.id(), "z3"),
                    &HistoryEntry::with_session_id(session_b.id(), "z2"),
                ]
            );
        });

        initialize_history_for_testing(
            &mut history_handle,
            session_c.clone(),
            async { vec![] },
            vec!["z4".to_string()],
            &mut app,
        )
        .await;

        history_handle.read(&app, |history, _ctx| {
            assert_eq!(
                history.commands(session_c.id()).unwrap_or_default(),
                vec![
                    &HistoryEntry::command_only("z1"),
                    &HistoryEntry::with_session_id(session_a.id(), "z3"),
                    &HistoryEntry::with_session_id(session_b.id(), "z2"),
                    &HistoryEntry::with_session_id(session_c.id(), "z4")
                ]
            );
        });
    });
}

#[cfg_attr(windows, ignore = "TODO(CORE-3626)")]
#[test]
fn append_command_with_rich_history_data() {
    App::test((), |mut app| async move {
        let session = Arc::new(Session::new(
            SessionInfo::new_for_test().with_id(0),
            Arc::new(TestCommandExecutor::default()),
        ));

        let start_ts_1 = Local::now();
        let end_ts_1 = Local::now();
        let start_ts_2 = Local::now();
        let end_ts_2 = Local::now();
        let start_ts_3 = Local::now();
        let end_ts_3 = Local::now();
        let start_ts_4 = Local::now();
        let end_ts_4 = Local::now();

        let shell_host = ShellHost {
            shell_type: ShellType::Bash,
            user: String::from("local:user"),
            hostname: String::from("local:host"),
        };
        let persisted_commands = vec![
            PersistedCommand {
                id: 0,
                command: String::from("ls"),
                exit_code: Some(ExitCode::from(0)),
                start_ts: Some(start_ts_1),
                completed_ts: Some(end_ts_1),
                pwd: Some(String::from("/")),
                shell_host: Some(shell_host.clone()),
                session_id: None,
                git_branch: None,
                workflow_id: None,
                workflow_command: None,
                is_agent_executed: false,
            },
            PersistedCommand {
                id: 0,
                command: String::from("date"),
                exit_code: Some(ExitCode::from(0)),
                start_ts: Some(start_ts_2),
                completed_ts: Some(end_ts_2),
                pwd: Some(String::from("/usr/bin")),
                shell_host: Some(shell_host.clone()),
                session_id: None,
                git_branch: Some(String::from("foobar")),
                workflow_id: None,
                workflow_command: None,
                is_agent_executed: false,
            },
        ];

        let mut history_handle = app.add_model(|_| History::new(persisted_commands));
        initialize_history_for_testing(
            &mut history_handle,
            session.clone(),
            async {
                vec![
                    "cd ~/Desktop".to_string(),
                    "ls".to_string(),
                    "date".to_string(),
                ]
            },
            Vec::new(),
            &mut app,
        )
        .await;

        history_handle.update(&mut app, |history, _ctx| {
            history.append_commands(
                session.id(),
                vec![
                    HistoryEntry {
                        session_id: Some(session.id()),
                        command: String::from("touch foobar"),
                        pwd: Some(String::from("/Users/andy/")),
                        start_ts: Some(start_ts_3),
                        completed_ts: Some(end_ts_3),
                        workflow_id: None,
                        workflow_command: None,
                        exit_code: Some(ExitCode::from(0)),
                        git_head: None,
                        shell_host: None,
                        is_agent_executed: false,
                        is_for_restored_block: false,
                    },
                    HistoryEntry {
                        session_id: Some(session.id()),
                        command: String::from("ls"),
                        pwd: Some(String::from("/Users/andy/")),
                        start_ts: Some(start_ts_4),
                        completed_ts: Some(end_ts_4),
                        workflow_id: None,
                        workflow_command: None,
                        exit_code: Some(ExitCode::from(0)),
                        git_head: None,
                        shell_host: None,
                        is_agent_executed: false,
                        is_for_restored_block: false,
                    },
                ],
            );
        });

        history_handle.read(&app, |history, _ctx| {
            assert_eq!(
                history.commands(session.id()).unwrap_or_default(),
                vec![
                    &HistoryEntry::command_only("cd ~/Desktop"),
                    &HistoryEntry {
                        session_id: None,
                        command: String::from("date"),
                        pwd: Some(String::from("/usr/bin")),
                        start_ts: Some(start_ts_2),
                        completed_ts: Some(end_ts_2),
                        workflow_id: None,
                        workflow_command: None,
                        exit_code: Some(ExitCode::from(0)),
                        git_head: Some(String::from("foobar")),
                        shell_host: Some(shell_host.clone()),
                        is_agent_executed: false,
                        is_for_restored_block: false,
                    },
                    &HistoryEntry {
                        session_id: Some(session.id()),
                        command: String::from("touch foobar"),
                        pwd: Some(String::from("/Users/andy/")),
                        start_ts: Some(start_ts_3),
                        completed_ts: Some(end_ts_3),
                        workflow_id: None,
                        workflow_command: None,
                        exit_code: Some(ExitCode::from(0)),
                        git_head: None,
                        shell_host: None,
                        is_agent_executed: false,
                        is_for_restored_block: false,
                    },
                    &HistoryEntry {
                        session_id: Some(session.id()),
                        command: String::from("ls"),
                        pwd: Some(String::from("/Users/andy/")),
                        start_ts: Some(start_ts_4),
                        completed_ts: Some(end_ts_4),
                        workflow_id: None,
                        workflow_command: None,
                        exit_code: Some(ExitCode::from(0)),
                        git_head: None,
                        shell_host: None,
                        is_agent_executed: false,
                        is_for_restored_block: false,
                    },
                ]
            );
        });
    });
}

#[test]
fn append_restored_command_doesnt_overwrite_rich_history() {
    App::test((), |mut app| async move {
        let session = Arc::new(Session::new(
            SessionInfo::new_for_test().with_id(0),
            Arc::new(TestCommandExecutor::default()),
        ));
        let start_ts = Local::now();
        let end_ts = Local::now();

        let shell_host = ShellHost {
            shell_type: ShellType::Bash,
            user: String::from("local:user"),
            hostname: String::from("local:host"),
        };
        let persisted_commands = vec![PersistedCommand {
            id: 0,
            command: String::from("ls"),
            exit_code: Some(ExitCode::from(0)),
            start_ts: Some(start_ts),
            completed_ts: Some(end_ts),
            pwd: Some(String::from("/tmp")),
            shell_host: Some(shell_host),
            session_id: None,
            git_branch: None,
            workflow_id: None,
            workflow_command: None,
            is_agent_executed: false,
        }];
        let mut history_handle = app.add_model(|_| History::new(persisted_commands));
        initialize_history_for_testing(
            &mut history_handle,
            session.clone(),
            async { vec!["cd ~/Desktop".to_string(), "ls".to_string()] },
            Vec::new(),
            &mut app,
        )
        .await;

        history_handle.update(&mut app, |history, _ctx| {
            history.append_restored_commands(
                session.id(),
                vec![HistoryEntry {
                    session_id: Some(session.id()),
                    command: "ls".to_string(),
                    pwd: Some("/tmp".to_string()),
                    start_ts: Some(start_ts),
                    completed_ts: Some(end_ts),
                    workflow_id: None,
                    workflow_command: None,
                    exit_code: Some(ExitCode::from(0)),
                    git_head: None,
                    shell_host: None,
                    is_agent_executed: false,
                    is_for_restored_block: true,
                }],
            );
        });

        history_handle.read(&app, |history, _ctx| {
            assert_eq!(
                history.commands(session.id()).unwrap_or_default(),
                vec![
                    &HistoryEntry::command_only("cd ~/Desktop"),
                    &HistoryEntry {
                        session_id: Some(session.id()),
                        command: String::from("ls"),
                        pwd: Some(String::from("/tmp")),
                        start_ts: Some(start_ts),
                        completed_ts: Some(end_ts),
                        workflow_id: None,
                        workflow_command: None,
                        exit_code: Some(ExitCode::from(0)),
                        git_head: None,
                        shell_host: None,
                        is_agent_executed: false,
                        is_for_restored_block: true,
                    },
                ]
            )
        });
    });
}

#[test]
fn is_appendable_vs_is_queryable() {
    App::test((), |mut app| async move {
        let mut history_handle = app.add_model(|_| History::new(vec![]));
        let (tx, rx) = async_channel::bounded(1);

        let session = Arc::new(Session::new(
            SessionInfo::new_for_test().with_id(0),
            Arc::new(TestCommandExecutor::default()),
        ));
        let session_id = session.id();
        history_handle.update(&mut app, move |history, ctx| {
            history.init_session_with(
                session,
                async move {
                    pin!(rx).next().await;
                    vec![]
                },
                ctx,
            );
        });

        // When the session has just been registered with the history model,
        // its history will be appendable but not queryable.
        history_handle.read(&app, |history, _| {
            assert!(history.is_appendable(&session_id));
            assert!(!history.is_queryable(&session_id));
        });

        // Simulate the asynchronous reading of the histfile.
        tx.send(()).await.unwrap();
        History::initialized_sessions(&mut history_handle, &mut app, vec![session_id]).await;

        // Once we've read the histfile, the model should be appendable and queryable.
        history_handle.read(&app, |history, _| {
            assert!(history.is_appendable(&session_id));
            assert!(history.is_queryable(&session_id));
        });
    });
}
