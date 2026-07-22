use std::collections::{HashMap, HashSet};
use std::io::Write as _;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use chrono::Local;
use tempfile::NamedTempFile;
use warp_util::standardized_path::StandardizedPath;
use warpui::App;

use super::*;
use crate::ai::agent::UserQueryMode;
use crate::ai::blocklist::{
    PendingAttachment, PendingFile, RequestInput, ResponseStream, ResponseStreamId,
};
use crate::ai::llms::LLMId;
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
    pending.refresh_valid_environment_ids(HashSet::from([environment_id]));
    assert!(pending.validate().is_ok());
    pending.refresh_valid_environment_ids(HashSet::new());
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
fn commit_revalidates_current_model_before_returning_future() {
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

        let future = app.update(|ctx| commit_handoff(pending, client, None, ctx));
        let HandoffCommitOutcome::Rejected { error, .. } = future.await else {
            panic!("current invalid model must reject before external work");
        };
        assert_eq!(error, HandoffPrepareError::InvalidModel);
    });
}

#[test]
fn commit_revalidates_current_environment_catalog_before_returning_future() {
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

        let future = app.update(|ctx| commit_handoff(pending, client, None, ctx));
        let HandoffCommitOutcome::Rejected { error, .. } = future.await else {
            panic!("deleted environment must reject before external work");
        };
        assert_eq!(error, HandoffPrepareError::InvalidEnvironment);
    });
}

#[test]
fn commit_revalidates_current_handoff_enablement_before_returning_future() {
    let _oz_handoff = FeatureFlag::OzHandoff.override_enabled(true);
    let _local_cloud = FeatureFlag::HandoffLocalCloud.override_enabled(false);
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let mut mock = MockAIClient::new();
        mock.expect_spawn_agent().times(0);
        let client: Arc<dyn AIClient> = Arc::new(mock);
        let pending = pending(client.clone(), None, false, "continue");

        let future = app.update(|ctx| commit_handoff(pending, client, None, ctx));
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
fn preparation_guardrails_reject_before_cancellation_eligible_state() {
    assert_eq!(
        validate_prepare_guard(false, true, false, false, false, false, false),
        Err(HandoffPrepareError::EmptySourceAndPrompt)
    );
    assert_eq!(
        validate_prepare_guard(true, false, true, false, false, false, false),
        Err(HandoffPrepareError::SourceNotInProgress)
    );
    assert_eq!(
        validate_prepare_guard(true, true, false, true, true, true, false),
        Err(HandoffPrepareError::LongRunningCommand)
    );
    assert_eq!(
        validate_prepare_guard(true, true, false, true, true, false, true),
        Err(HandoffPrepareError::ActiveOrBlockedChild)
    );
    assert_eq!(
        validate_prepare_guard(true, true, false, true, true, false, false),
        Ok(())
    );
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
        let launch = PendingCloudLaunch::new(
            "continue".to_owned(),
            HandoffLaunchAttachments::new(
                vec![AttachmentInput {
                    file_name: "context.txt".to_owned(),
                    mime_type: "text/plain".to_owned(),
                    data: "contents".to_owned(),
                }],
                vec![pending_attachment.clone()],
            ),
        );

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
                HandoffPrepareInput {
                    terminal_surface_id: view.id(),
                    expected_conversation_id: Some(conversation_id),
                    history: BlocklistAIHistoryModel::handle(ctx),
                    controller: view.ai_controller().clone(),
                    context: view.ai_context_model().clone(),
                    current_working_directory: None,
                    snapshot_target: SnapshotUploadTarget::Local {
                        ai_client: provider.get_ai_client(),
                        http: provider.get_http_client(),
                    },
                    has_long_running_command: true,
                    launch: Some(launch.clone()),
                    environment_id: None,
                    environment_required: false,
                    entry_point: HandoffEntryPoint::Ampersand,
                    surface: HandoffSurface::Gui,
                    cancellation_reason: CancellationReason::ManuallyCancelled,
                    require_in_progress_source: true,
                },
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
                HandoffPrepareInput {
                    terminal_surface_id: view.id(),
                    expected_conversation_id: Some(conversation_id),
                    history: BlocklistAIHistoryModel::handle(ctx),
                    controller: view.ai_controller().clone(),
                    context: view.ai_context_model().clone(),
                    current_working_directory: None,
                    snapshot_target: SnapshotUploadTarget::Local {
                        ai_client: provider.get_ai_client(),
                        http: provider.get_http_client(),
                    },
                    has_long_running_command: false,
                    launch: Some(launch.clone()),
                    environment_id: None,
                    environment_required: false,
                    entry_point: HandoffEntryPoint::Ampersand,
                    surface: HandoffSurface::Gui,
                    cancellation_reason: CancellationReason::ManuallyCancelled,
                    require_in_progress_source: true,
                },
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
                HandoffPrepareInput {
                    terminal_surface_id: view.id(),
                    expected_conversation_id: Some(conversation_id),
                    history: BlocklistAIHistoryModel::handle(ctx),
                    controller: view.ai_controller().clone(),
                    context: view.ai_context_model().clone(),
                    current_working_directory: None,
                    snapshot_target: SnapshotUploadTarget::Local {
                        ai_client: provider.get_ai_client(),
                        http: provider.get_http_client(),
                    },
                    has_long_running_command: false,
                    launch: Some(launch),
                    environment_id: None,
                    environment_required: false,
                    entry_point: HandoffEntryPoint::Ampersand,
                    surface: HandoffSurface::Gui,
                    cancellation_reason: CancellationReason::ManuallyCancelled,
                    require_in_progress_source: false,
                },
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
                materialized.store(true, Ordering::SeqCst);
                Ok(())
            })
        }
    });

    let outcome = execute_committed_handoff(
        pending(
            client.clone(),
            Some("source-conversation".to_owned()),
            true,
            "",
        ),
        client,
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
                materialized.store(true, Ordering::SeqCst);
                Ok(())
            })
        }
    });

    let outcome = execute_committed_handoff(
        pending(client.clone(), None, false, "new task"),
        client,
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

    let outcome = execute_committed_handoff(pending, client, None).await;
    let HandoffCommitOutcome::Created(created) = outcome else {
        panic!("snapshot failure should not fail the handoff");
    };
    assert!(created.snapshot_failed);
    assert!(created.derived_workspace_had_content);
    assert!(created.request.initial_snapshot_token.is_none());
}
