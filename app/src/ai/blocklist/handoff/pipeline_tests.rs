use std::collections::{HashMap, HashSet};
use std::io::Write as _;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use chrono::Local;
use futures::channel::oneshot;
use tempfile::NamedTempFile;
use warp_util::standardized_path::StandardizedPath;
use warpui::App;

use super::*;
use crate::ai::agent::conversation::{AIConversation, ConversationStatus};
use crate::ai::agent::{
    AIAgentExchange, AIAgentExchangeId, AIAgentInput, AIAgentOutputStatus, FinishedAIAgentOutput,
    Shared, UserQueryMode,
};
use crate::ai::blocklist::{
    PendingAttachment, PendingFile, RequestInput, ResponseStream, ResponseStreamId,
};
use crate::ai::llms::{AvailableLLMs, LLMId, LLMInfo, LLMPreferences, ModelsByFeature};
use crate::features::FeatureFlag;
use crate::server::ids::{ServerId, SyncId};
use crate::server::server_api::ServerApiProvider;
use crate::server::server_api::ai::{ForkConversationResponse, MockAIClient, SpawnAgentResponse};
use crate::test_util::add_window_with_terminal;
use crate::test_util::terminal::initialize_app_for_terminal_view;

fn task_id() -> AmbientAgentTaskId {
    "550e8400-e29b-41d4-a716-446655440000"
        .parse()
        .expect("valid task id")
}
fn exchange_with_working_directory(
    working_directory: &str,
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
        working_directory: Some(working_directory.to_owned()),
        model_id: LLMId::from("test-model"),
        request_cost: None,
        coding_model_id: LLMId::from("test-coding-model"),
        cli_agent_model_id: LLMId::from("test-cli-model"),
        computer_use_model_id: LLMId::from("test-computer-use-model"),
        response_initiator: None,
    }
}

#[test]
fn required_environment_revalidates_after_catalog_refresh() {
    let mock = Arc::new(MockAIClient::new());
    let mut pending = pending(mock, None, false, "continue");
    let environment_id = SyncId::ServerId(ServerId::from(1));
    pending.environment_required = true;

    assert_eq!(
        pending.validate(),
        Err(HandoffPrepareError::MissingRequiredEnvironment)
    );
    pending.set_environment_id(Some(environment_id), true);
    assert_eq!(
        pending.validate(),
        Err(HandoffPrepareError::InvalidEnvironment)
    );
    pending.set_valid_environment_ids(HashSet::from([environment_id]));
    assert!(pending.validate().is_ok());
    pending.set_valid_environment_ids(HashSet::new());
    assert_eq!(
        pending.validate(),
        Err(HandoffPrepareError::InvalidEnvironment)
    );
}

#[test]
fn model_selection_refreshes_cloud_compatibility_in_both_directions() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let mock = Arc::new(MockAIClient::new());
        let mut pending = pending(mock, None, false, "continue");
        app.update(|ctx| {
            pending.set_model_id("custom-router:local:byok".to_owned(), false, ctx);
        });
        assert_eq!(pending.selected_model_id, "auto");
        assert_eq!(pending.config.model_id.as_deref(), Some("auto"));
        assert!(pending.validate().is_ok());

        app.update(|ctx| {
            pending.set_model_id("custom-router:local:byok".to_owned(), true, ctx);
        });
        assert_eq!(pending.validate(), Err(HandoffPrepareError::InvalidModel));

        app.update(|ctx| {
            pending.set_model_id("auto".to_owned(), false, ctx);
        });
        assert_eq!(pending.validate(), Err(HandoffPrepareError::InvalidModel));

        app.update(|ctx| {
            pending.set_model_id("auto".to_owned(), true, ctx);
        });
        assert!(pending.validate().is_ok());

        app.update(|ctx| {
            pending.set_model_id("custom-router:local:byok".to_owned(), true, ctx);
        });
        assert_eq!(pending.validate(), Err(HandoffPrepareError::InvalidModel));
    });
}

#[test]
fn prepare_falls_back_to_auto_for_an_implicit_local_model() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |_view, ctx| {
            let models = ModelsByFeature {
                agent_mode: AvailableLLMs::new(
                    "custom-router:local:byok".into(),
                    vec![LLMInfo::new_for_test("custom-router:local:byok")],
                    None,
                )
                .expect("valid available llms"),
                ..Default::default()
            };
            LLMPreferences::handle(ctx).update(ctx, |preferences, ctx| {
                preferences.update_feature_model_choices(Ok(models), ctx);
            });
        });

        let pending = terminal
            .update(&mut app, |view, ctx| {
                let provider = ServerApiProvider::as_ref(ctx);
                prepare_handoff(
                    HandoffPrepareInput::new(
                        view.id(),
                        BlocklistAIHistoryModel::handle(ctx),
                        view.ai_controller().clone(),
                        view.ai_context_model().clone(),
                        SnapshotUploadTarget::Local {
                            ai_client: provider.get_ai_client(),
                            http: provider.get_http_client(),
                        },
                        HandoffEntryPoint::Ampersand,
                        HandoffSurface::Gui,
                    )
                    .with_launch(Some(PendingCloudLaunch {
                        prompt: "new task".to_owned(),
                        attachments: HandoffLaunchAttachments::default(),
                    })),
                    ctx,
                )
            })
            .expect("implicit local model should fall back");

        assert_eq!(pending.selected_model_id, "auto");
        assert_eq!(pending.config.model_id.as_deref(), Some("auto"));
        assert!(pending.validate().is_ok());
    });
}

#[test]
fn execute_revalidates_current_model_before_returning_future() {
    let _oz_handoff = FeatureFlag::OzHandoff.override_enabled(true);
    let _local_cloud = FeatureFlag::HandoffLocalCloud.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let mut mock = MockAIClient::new();
        mock.expect_spawn_agent().times(0);
        let client: Arc<dyn AIClient> = Arc::new(mock);
        let mut pending = pending(client.clone(), None, false, "continue");
        pending.selected_model_id = "custom-router:local:byok".to_owned();
        pending.model_is_cloud_runnable = true;

        let future = app.update(|ctx| execute_handoff(pending, client, None, None, ctx));
        let HandoffCommitOutcome::Rejected { error, .. } = future.await else {
            panic!("current invalid model must reject before external work");
        };
        assert_eq!(error, HandoffPrepareError::InvalidModel);
    });
}

#[test]
fn execute_revalidates_current_environment_catalog_before_returning_future() {
    let _oz_handoff = FeatureFlag::OzHandoff.override_enabled(true);
    let _local_cloud = FeatureFlag::HandoffLocalCloud.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let mut mock = MockAIClient::new();
        mock.expect_spawn_agent().times(0);
        let client: Arc<dyn AIClient> = Arc::new(mock);
        let mut pending = pending(client.clone(), None, false, "continue");
        let environment_id = SyncId::ServerId(ServerId::from(1));
        pending.selected_environment_id = Some(environment_id);
        pending.valid_environment_ids.insert(environment_id);
        pending.config.environment_id = Some(environment_id.to_string());

        let future = app.update(|ctx| execute_handoff(pending, client, None, None, ctx));
        let HandoffCommitOutcome::Rejected { error, .. } = future.await else {
            panic!("deleted environment must reject before external work");
        };
        assert_eq!(error, HandoffPrepareError::InvalidEnvironment);
    });
}

#[test]
fn execute_revalidates_current_handoff_enablement_before_returning_future() {
    let _oz_handoff = FeatureFlag::OzHandoff.override_enabled(true);
    let _local_cloud = FeatureFlag::HandoffLocalCloud.override_enabled(false);
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let mut mock = MockAIClient::new();
        mock.expect_spawn_agent().times(0);
        let client: Arc<dyn AIClient> = Arc::new(mock);
        let pending = pending(client.clone(), None, false, "continue");

        let future = app.update(|ctx| execute_handoff(pending, client, None, None, ctx));
        let HandoffCommitOutcome::Rejected { error, .. } = future.await else {
            panic!("disabled handoff must reject before external work");
        };
        assert_eq!(error, HandoffPrepareError::HandoffDisabled);
    });
}
fn snapshot_token() -> InitialSnapshotToken {
    serde_json::from_str("\"snapshot-token\"").expect("valid snapshot token")
}

fn pending(
    ai_client: Arc<dyn AIClient>,
    source_token: Option<String>,
    source_conversation_active: bool,
    prompt: &str,
) -> PendingHandoff {
    PendingHandoff {
        source_conversation: None,
        source_conversation_active,
        source_paths: Vec::new(),
        source_token,
        title: Some("Handoff title".to_owned()),
        prompt: prompt.to_owned(),
        request_attachments: vec![AttachmentInput {
            file_name: "context.txt".to_owned(),
            mime_type: "text/plain".to_owned(),
            data: "contents".to_owned(),
        }],
        restoration: Some(HandoffRestoration {
            prompt: prompt.to_owned(),
            attachments: Vec::new(),
            environment_id: None,
        }),
        selected_environment_id: None,
        environment_required: false,
        environment_selection_is_explicit: false,
        valid_environment_ids: HashSet::new(),
        selected_model_id: "auto".to_owned(),
        model_selection_is_explicit: false,
        model_is_cloud_runnable: true,
        config: AgentConfigSnapshot {
            model_id: Some("auto".to_owned()),
            computer_use_enabled: Some(true),
            ..Default::default()
        },
        snapshot_target: SnapshotUploadTarget::Local {
            ai_client,
            http: Arc::new(http_client::Client::new_for_test()),
        },
        snapshot_disabled: true,
        orchestration_handoff: Some(true),
    }
}

fn request_for_prompt(
    prompt: &str,
    source_active: bool,
    snapshot: Option<InitialSnapshotToken>,
) -> SpawnAgentRequest {
    build_spawn_request(
        SpawnReadyHandoff {
            prompt: prompt.to_owned(),
            source_conversation_active: source_active,
            config: AgentConfigSnapshot::default(),
            title: None,
            attachments: Vec::new(),
            snapshot_disabled: false,
            orchestration_handoff: None,
        },
        None,
        snapshot,
    )
}

#[test]
fn empty_prompt_substitution_matrix_matches_gui_behavior() {
    assert_eq!(
        request_for_prompt("", true, Some(snapshot_token()))
            .prompt
            .as_deref(),
        Some("Continue. Apply the workspace changes from my previous session.")
    );
    assert_eq!(
        request_for_prompt("", true, None).prompt.as_deref(),
        Some("Continue")
    );
    assert_eq!(
        request_for_prompt("", false, Some(snapshot_token()))
            .prompt
            .as_deref(),
        Some("Apply the workspace changes from my previous session.")
    );
    assert!(request_for_prompt("", false, None).prompt.is_none());
    let plan = request_for_prompt("/plan investigate", false, None);
    assert_eq!(plan.prompt.as_deref(), Some("investigate"));
    assert_eq!(plan.mode, UserQueryMode::Plan);
}

#[test]
fn explicit_selection_precedence_and_restoration_are_exactly_once() {
    let mock = Arc::new(MockAIClient::new());
    let mut pending = pending(mock, None, false, "continue");
    let first = SyncId::ServerId(ServerId::from(1));
    let second = SyncId::ServerId(ServerId::from(2));
    pending.valid_environment_ids.extend([first, second]);

    pending.set_environment_id(Some(first), true);
    pending.set_environment_id(Some(second), false);

    let snapshot = pending.presentation_snapshot();
    assert_eq!(snapshot.environment_id, Some(first));
    assert_eq!(snapshot.model_id, "auto");
    assert!(pending.validate().is_ok());
    assert_eq!(
        pending
            .take_restoration()
            .expect("first restoration is available")
            .prompt,
        "continue"
    );
    assert!(pending.take_restoration().is_none());
}

#[test]
fn prepare_rejects_an_empty_source_without_a_prompt() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let result = terminal.update(&mut app, |view, ctx| {
            let provider = ServerApiProvider::as_ref(ctx);
            prepare_handoff(
                HandoffPrepareInput::new(
                    view.id(),
                    BlocklistAIHistoryModel::handle(ctx),
                    view.ai_controller().clone(),
                    view.ai_context_model().clone(),
                    SnapshotUploadTarget::Local {
                        ai_client: provider.get_ai_client(),
                        http: provider.get_http_client(),
                    },
                    HandoffEntryPoint::Ampersand,
                    HandoffSurface::Gui,
                ),
                ctx,
            )
        });
        assert!(matches!(
            result,
            Err(HandoffPrepareError::EmptySourceAndPrompt)
        ));
    });
}

#[test]
fn prepare_accepts_a_cwd_snapshot_without_a_source_or_prompt() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let pending = terminal
            .update(&mut app, |view, ctx| {
                let provider = ServerApiProvider::as_ref(ctx);
                prepare_handoff(
                    HandoffPrepareInput::new(
                        view.id(),
                        BlocklistAIHistoryModel::handle(ctx),
                        view.ai_controller().clone(),
                        view.ai_context_model().clone(),
                        SnapshotUploadTarget::Local {
                            ai_client: provider.get_ai_client(),
                            http: provider.get_http_client(),
                        },
                        HandoffEntryPoint::Ampersand,
                        HandoffSurface::Gui,
                    )
                    .with_current_working_directory(Some("/repo".to_owned())),
                    ctx,
                )
            })
            .expect("cwd snapshot should be sufficient for a fresh launch");

        assert_eq!(
            pending.source_paths,
            vec![StandardizedPath::try_new("/repo").expect("absolute repo path")]
        );
    });
}

#[test]
fn prepare_preserves_untransferred_source_attachments() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let attachment_file = NamedTempFile::new().expect("temporary attachment file");
        terminal.update(&mut app, |view, ctx| {
            view.ai_context_model().update(ctx, |context, ctx| {
                context.append_pending_attachments(
                    vec![PendingAttachment::File(PendingFile {
                        file_name: "draft.txt".to_owned(),
                        file_path: attachment_file.path().to_path_buf(),
                        mime_type: "text/plain".to_owned(),
                    })],
                    ctx,
                );
            });
        });

        terminal
            .update(&mut app, |view, ctx| {
                let provider = ServerApiProvider::as_ref(ctx);
                prepare_handoff(
                    HandoffPrepareInput::new(
                        view.id(),
                        BlocklistAIHistoryModel::handle(ctx),
                        view.ai_controller().clone(),
                        view.ai_context_model().clone(),
                        SnapshotUploadTarget::Local {
                            ai_client: provider.get_ai_client(),
                            http: provider.get_http_client(),
                        },
                        HandoffEntryPoint::Ampersand,
                        HandoffSurface::Gui,
                    )
                    .with_source_conversation_id(None)
                    .with_launch(Some(PendingCloudLaunch {
                        prompt: "fresh task".to_owned(),
                        attachments: HandoffLaunchAttachments::default(),
                    }))
                    .with_transfer_pending_attachments(false),
                    ctx,
                )
            })
            .expect("handoff preparation");

        terminal.read(&app, |view, ctx| {
            assert_eq!(
                view.ai_context_model()
                    .as_ref(ctx)
                    .pending_attachments()
                    .len(),
                1
            );
        });
    });
}

#[test]
fn prepare_collects_completed_descendant_paths() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let parent_id = terminal.update(&mut app, |view, ctx| {
            let terminal_surface_id = view.id();
            let parent = AIConversation::new(false, false);
            let parent_id = parent.id();
            let mut child = AIConversation::new(false, false);
            child.set_parent_conversation_id(parent_id);
            let child_id = child.id();
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                history.restore_conversations(terminal_surface_id, vec![parent, child], ctx);
                let finished_output = || AIAgentOutputStatus::Finished {
                    finished_output: FinishedAIAgentOutput::Success {
                        output: Shared::new(Default::default()),
                    },
                };
                history
                    .conversation_mut(&parent_id)
                    .expect("parent")
                    .append_reassigned_exchange(
                        &ResponseStreamId::new_for_test(),
                        exchange_with_working_directory("/parent", finished_output()),
                        terminal_surface_id,
                        ctx,
                    )
                    .expect("append parent exchange");
                history
                    .conversation_mut(&child_id)
                    .expect("child")
                    .append_reassigned_exchange(
                        &ResponseStreamId::new_for_test(),
                        exchange_with_working_directory("/completed-child", finished_output()),
                        terminal_surface_id,
                        ctx,
                    )
                    .expect("append child exchange");
                history.update_conversation_status(
                    terminal_surface_id,
                    parent_id,
                    ConversationStatus::Success,
                    ctx,
                );
                history.update_conversation_status(
                    terminal_surface_id,
                    child_id,
                    ConversationStatus::Success,
                    ctx,
                );
                history.set_server_conversation_token_for_conversation(
                    parent_id,
                    "server-conversation".to_owned(),
                );
            });
            parent_id
        });

        let pending = terminal
            .update(&mut app, |view, ctx| {
                let provider = ServerApiProvider::as_ref(ctx);
                prepare_handoff(
                    HandoffPrepareInput::new(
                        view.id(),
                        BlocklistAIHistoryModel::handle(ctx),
                        view.ai_controller().clone(),
                        view.ai_context_model().clone(),
                        SnapshotUploadTarget::Local {
                            ai_client: provider.get_ai_client(),
                            http: provider.get_http_client(),
                        },
                        HandoffEntryPoint::Ampersand,
                        HandoffSurface::Gui,
                    )
                    .with_expected_conversation_id(Some(parent_id))
                    .with_launch(Some(PendingCloudLaunch {
                        prompt: "continue".to_owned(),
                        attachments: HandoffLaunchAttachments::default(),
                    })),
                    ctx,
                )
            })
            .expect("completed source and child should prepare");
        assert!(pending.source_paths.contains(
            &StandardizedPath::try_new("/completed-child").expect("absolute child path")
        ));
    });
}

#[test]
fn prepare_orders_guards_cancellation_token_check_and_attachment_transfer() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let attachment_file = NamedTempFile::new().expect("temporary attachment file");
        let pending_attachment = PendingAttachment::File(PendingFile {
            file_name: "context.txt".to_owned(),
            file_path: attachment_file.path().to_path_buf(),
            mime_type: "text/plain".to_owned(),
        });
        let launch = PendingCloudLaunch {
            prompt: "continue".to_owned(),
            attachments: HandoffLaunchAttachments {
                request_attachments: vec![AttachmentInput {
                    file_name: "context.txt".to_owned(),
                    mime_type: "text/plain".to_owned(),
                    data: "contents".to_owned(),
                }],
                display_attachments: vec![pending_attachment.clone()],
            },
        };

        let conversation_id = terminal.update(&mut app, |view, ctx| {
            view.ai_context_model().update(ctx, |context, ctx| {
                context.append_pending_attachments(vec![pending_attachment], ctx);
            });
            let terminal_surface_id = view.id();
            let stream_id = ResponseStreamId::new_for_test();
            let history = BlocklistAIHistoryModel::handle(ctx);
            let conversation_id = history.update(ctx, |history, ctx| {
                let conversation_id =
                    history.start_new_conversation(terminal_surface_id, false, false, false, ctx);
                let task_id = history
                    .conversation(&conversation_id)
                    .expect("conversation")
                    .get_root_task_id()
                    .clone();
                history
                    .update_conversation_for_new_request_input(
                        RequestInput {
                            conversation_id,
                            input_messages: HashMap::from([(task_id, Vec::new())]),
                            working_directory: None,
                            model_id: LLMId::from("test-model"),
                            coding_model_id: LLMId::from("test-coding-model"),
                            cli_agent_model_id: LLMId::from("test-cli-model"),
                            computer_use_model_id: LLMId::from("test-computer-use-model"),
                            shared_session_response_initiator: None,
                            request_start_ts: Local::now(),
                            supported_tools_override: None,
                        },
                        stream_id.clone(),
                        terminal_surface_id,
                        ctx,
                    )
                    .expect("append streaming exchange");
                conversation_id
            });
            let stream = ctx.add_model(|_| ResponseStream::new_for_test(stream_id.clone()));
            view.ai_controller().update(ctx, |controller, ctx| {
                controller.register_mock_stream_for_test(stream_id, conversation_id, stream, ctx);
            });
            conversation_id
        });

        let guarded = terminal.update(&mut app, |view, ctx| {
            let provider = ServerApiProvider::as_ref(ctx);
            prepare_handoff(
                HandoffPrepareInput::new(
                    view.id(),
                    BlocklistAIHistoryModel::handle(ctx),
                    view.ai_controller().clone(),
                    view.ai_context_model().clone(),
                    SnapshotUploadTarget::Local {
                        ai_client: provider.get_ai_client(),
                        http: provider.get_http_client(),
                    },
                    HandoffEntryPoint::Ampersand,
                    HandoffSurface::Gui,
                )
                .with_expected_conversation_id(Some(conversation_id))
                .with_long_running_command(true)
                .with_launch(Some(launch.clone()))
                .with_require_in_progress_source(true),
                ctx,
            )
        });
        assert!(matches!(
            guarded,
            Err(HandoffPrepareError::LongRunningCommand)
        ));
        terminal.read(&app, |view, ctx| {
            assert_eq!(
                BlocklistAIHistoryModel::as_ref(ctx)
                    .conversation(&conversation_id)
                    .map(|conversation| conversation.status()),
                Some(&crate::ai::agent::conversation::ConversationStatus::InProgress)
            );
            assert_eq!(
                view.ai_context_model()
                    .as_ref(ctx)
                    .pending_attachments()
                    .len(),
                1
            );
        });

        let missing_token = terminal.update(&mut app, |view, ctx| {
            let provider = ServerApiProvider::as_ref(ctx);
            prepare_handoff(
                HandoffPrepareInput::new(
                    view.id(),
                    BlocklistAIHistoryModel::handle(ctx),
                    view.ai_controller().clone(),
                    view.ai_context_model().clone(),
                    SnapshotUploadTarget::Local {
                        ai_client: provider.get_ai_client(),
                        http: provider.get_http_client(),
                    },
                    HandoffEntryPoint::Ampersand,
                    HandoffSurface::Gui,
                )
                .with_expected_conversation_id(Some(conversation_id))
                .with_launch(Some(launch.clone()))
                .with_require_in_progress_source(true),
                ctx,
            )
        });
        assert!(matches!(
            missing_token,
            Err(HandoffPrepareError::MissingServerConversationToken)
        ));
        terminal.read(&app, |view, ctx| {
            assert_eq!(
                BlocklistAIHistoryModel::as_ref(ctx)
                    .conversation(&conversation_id)
                    .map(|conversation| conversation.status()),
                Some(&crate::ai::agent::conversation::ConversationStatus::Cancelled)
            );
            assert_eq!(
                view.ai_context_model()
                    .as_ref(ctx)
                    .pending_attachments()
                    .len(),
                1
            );
        });

        BlocklistAIHistoryModel::handle(&app).update(&mut app, |history, _| {
            history.set_server_conversation_token_for_conversation(
                conversation_id,
                "server-conversation".to_owned(),
            );
        });
        let mut pending = terminal.update(&mut app, |view, ctx| {
            let provider = ServerApiProvider::as_ref(ctx);
            prepare_handoff(
                HandoffPrepareInput::new(
                    view.id(),
                    BlocklistAIHistoryModel::handle(ctx),
                    view.ai_controller().clone(),
                    view.ai_context_model().clone(),
                    SnapshotUploadTarget::Local {
                        ai_client: provider.get_ai_client(),
                        http: provider.get_http_client(),
                    },
                    HandoffEntryPoint::Ampersand,
                    HandoffSurface::Gui,
                )
                .with_expected_conversation_id(Some(conversation_id))
                .with_launch(Some(launch)),
                ctx,
            )
            .expect("terminal source with a token should prepare")
        });
        terminal.read(&app, |view, ctx| {
            assert!(
                view.ai_context_model()
                    .as_ref(ctx)
                    .pending_attachments()
                    .is_empty()
            );
        });
        assert_eq!(
            pending
                .take_restoration()
                .expect("restoration")
                .attachments
                .len(),
            1
        );
        assert!(pending.take_restoration().is_none());
    });
}

#[tokio::test]
async fn fork_materialization_precedes_exactly_one_spawn() {
    let materialized = Arc::new(AtomicBool::new(false));
    let spawn_count = Arc::new(AtomicUsize::new(0));
    let observed_request = Arc::new(Mutex::new(None));
    let mut mock = MockAIClient::new();
    mock.expect_fork_conversation()
        .times(1)
        .withf(|conversation_id, title| {
            conversation_id == "source-conversation" && title.as_deref() == Some("Handoff title")
        })
        .returning(|_, _| {
            Ok(ForkConversationResponse {
                forked_conversation_id: "forked-conversation".to_owned(),
            })
        });
    mock.expect_spawn_agent().times(1).returning({
        let materialized = materialized.clone();
        let spawn_count = spawn_count.clone();
        let observed_request = observed_request.clone();
        move |request| {
            assert!(materialized.load(Ordering::SeqCst));
            spawn_count.fetch_add(1, Ordering::SeqCst);
            *observed_request.lock().expect("request lock") = Some(request);
            Ok(SpawnAgentResponse {
                task_id: task_id(),
                run_id: "run-id".to_owned(),
                at_capacity: false,
            })
        }
    });
    let client: Arc<dyn AIClient> = Arc::new(mock);
    let materialize: MaterializeHandoffTarget = Box::new({
        let materialized = materialized.clone();
        move |input| {
            Box::pin(async move {
                assert_eq!(
                    input.forked_conversation_id.as_deref(),
                    Some("forked-conversation")
                );
                assert_eq!(input.request.prompt.as_deref(), Some("Continue"));
                assert!(input.request.initial_snapshot_token.is_none());
                materialized.store(true, Ordering::SeqCst);
                Ok(())
            })
        }
    });

    let outcome = execute_validated_handoff(
        pending(
            client.clone(),
            Some("source-conversation".to_owned()),
            true,
            "",
        ),
        client,
        None,
        Some(materialize),
    )
    .await;
    let HandoffCommitOutcome::Created(created) = outcome else {
        panic!("expected created handoff");
    };
    assert_eq!(created.task_id, task_id());
    assert_eq!(created.run_id, "run-id");
    assert!(created.url.ends_with("/runs/run-id"));
    assert_eq!(spawn_count.load(Ordering::SeqCst), 1);

    let request = observed_request
        .lock()
        .expect("request lock")
        .take()
        .expect("spawn request");
    assert_eq!(request.prompt.as_deref(), Some("Continue"));
    assert_eq!(
        request.conversation_id.as_deref(),
        Some("forked-conversation")
    );
    assert_eq!(request.attachments.len(), 1);
    assert_eq!(request.snapshot_disabled, Some(true));
    assert_eq!(request.orchestration_handoff, Some(true));
    assert_eq!(
        request
            .config
            .as_ref()
            .and_then(|config| config.model_id.as_deref()),
        Some("auto")
    );
}

#[tokio::test]
async fn fresh_launch_skips_fork_and_materializes_before_spawn() {
    let materialized = Arc::new(AtomicBool::new(false));
    let mut mock = MockAIClient::new();
    mock.expect_fork_conversation().times(0);
    mock.expect_spawn_agent().times(1).returning({
        let materialized = materialized.clone();
        move |request| {
            assert!(materialized.load(Ordering::SeqCst));
            assert!(request.conversation_id.is_none());
            Ok(SpawnAgentResponse {
                task_id: task_id(),
                run_id: "fresh-run".to_owned(),
                at_capacity: true,
            })
        }
    });
    let client: Arc<dyn AIClient> = Arc::new(mock);
    let materialize: MaterializeHandoffTarget = Box::new({
        let materialized = materialized.clone();
        move |input| {
            Box::pin(async move {
                assert!(input.source_conversation.is_none());
                assert!(input.forked_conversation_id.is_none());
                assert_eq!(input.request.prompt.as_deref(), Some("new task"));
                materialized.store(true, Ordering::SeqCst);
                Ok(())
            })
        }
    });

    let outcome = execute_validated_handoff(
        pending(client.clone(), None, false, "new task"),
        client,
        None,
        Some(materialize),
    )
    .await;
    let HandoffCommitOutcome::Created(created) = outcome else {
        panic!("expected created handoff");
    };
    assert!(created.at_capacity);
    assert!(!created.derived_workspace_had_content);
}

#[tokio::test]
async fn cancellation_after_materialization_stops_before_spawn() {
    let mut mock = MockAIClient::new();
    mock.expect_fork_conversation().times(0);
    mock.expect_spawn_agent().times(0);
    let client: Arc<dyn AIClient> = Arc::new(mock);
    let materialize: MaterializeHandoffTarget = Box::new(move |input| {
        Box::pin(async move {
            input
                .cancel
                .send(())
                .expect("handoff cancellation receiver");
            Ok(())
        })
    });

    let outcome = execute_validated_handoff(
        pending(client.clone(), None, false, "new task"),
        client,
        None,
        Some(materialize),
    )
    .await;

    assert!(matches!(outcome, HandoffCommitOutcome::Cancelled));
}

#[tokio::test]
async fn cancellation_during_spawn_cancels_the_created_task() {
    let cancel = Arc::new(Mutex::new(None::<oneshot::Sender<()>>));
    let mut mock = MockAIClient::new();
    mock.expect_fork_conversation().times(0);
    mock.expect_spawn_agent().times(1).returning({
        let cancel = cancel.clone();
        move |_| {
            cancel
                .lock()
                .expect("cancel sender lock")
                .take()
                .expect("cancel sender")
                .send(())
                .expect("handoff cancellation receiver");
            Ok(SpawnAgentResponse {
                task_id: task_id(),
                run_id: "cancelled-run".to_owned(),
                at_capacity: false,
            })
        }
    });
    mock.expect_cancel_ambient_agent_task()
        .times(1)
        .withf(|cancelled_task_id| *cancelled_task_id == task_id())
        .returning(|_| Ok(()));
    let client: Arc<dyn AIClient> = Arc::new(mock);
    let materialize: MaterializeHandoffTarget = Box::new({
        let cancel = cancel.clone();
        move |input| {
            *cancel.lock().expect("cancel sender lock") = Some(input.cancel);
            Box::pin(async { Ok(()) })
        }
    });

    let outcome = execute_validated_handoff(
        pending(client.clone(), None, false, "new task"),
        client,
        None,
        Some(materialize),
    )
    .await;

    assert!(matches!(outcome, HandoffCommitOutcome::Cancelled));
}

#[tokio::test]
async fn snapshot_failure_degrades_to_spawn_without_token() {
    let mut file = NamedTempFile::new().expect("temporary snapshot file");
    file.write_all(b"snapshot contents")
        .expect("write temporary snapshot file");
    let path = StandardizedPath::try_new(
        file.path()
            .to_str()
            .expect("temporary path should be utf-8"),
    )
    .expect("temporary path should be absolute");

    let mut mock = MockAIClient::new();
    mock.expect_fork_conversation().times(0);
    mock.expect_upload_local_handoff_snapshot()
        .times(1)
        .returning(|_| Err(anyhow::anyhow!("snapshot unavailable")));
    mock.expect_spawn_agent().times(1).returning(|request| {
        assert!(request.initial_snapshot_token.is_none());
        Ok(SpawnAgentResponse {
            task_id: task_id(),
            run_id: "degraded-run".to_owned(),
            at_capacity: false,
        })
    });
    let client: Arc<dyn AIClient> = Arc::new(mock);
    let mut pending = pending(client.clone(), None, false, "continue");
    pending.source_paths = vec![path];

    let outcome = execute_validated_handoff(pending, client, None, None).await;
    let HandoffCommitOutcome::Created(created) = outcome else {
        panic!("snapshot failure should not fail the handoff");
    };
    assert!(created.snapshot_failed);
    assert!(created.derived_workspace_had_content);
    assert!(created.request.initial_snapshot_token.is_none());
}

#[tokio::test]
async fn caller_cancellation_stops_before_spawn() {
    let mut mock = MockAIClient::new();
    mock.expect_fork_conversation().times(0);
    mock.expect_spawn_agent().times(0);
    let client: Arc<dyn AIClient> = Arc::new(mock);
    let (cancel, cancellation) = oneshot::channel();
    cancel.send(()).expect("handoff cancellation receiver");

    let outcome = execute_validated_handoff(
        pending(client.clone(), None, false, "new task"),
        client,
        Some(cancellation),
        None,
    )
    .await;

    assert!(matches!(outcome, HandoffCommitOutcome::Cancelled));
}
