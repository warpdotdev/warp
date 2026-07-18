use url::Url;
use warpui::{App, EntityId};

use super::*;
use crate::ai::blocklist::handoff::HandoffLaunchAttachments;
use crate::ai::llms::{AvailableLLMs, LLMId, LLMInfo, LLMPreferences, ModelsByFeature};
use crate::test_util::terminal::initialize_app_for_terminal_view;

fn attachment() -> AttachmentInput {
    AttachmentInput {
        file_name: "context.txt".to_owned(),
        mime_type: "text/plain".to_owned(),
        data: "hello".to_owned(),
    }
}

fn pending_launch() -> PendingCloudLaunch {
    PendingCloudLaunch {
        prompt: "fix tests".to_owned(),
        attachments: HandoffLaunchAttachments {
            request_attachments: vec![attachment()],
            display_attachments: vec![],
        },
    }
}

/// Empty-prompt launch fixture for empty-prompt handoff tests. Mirrors what
/// the workspace synthesizes when the chip / `&` / `/handoff` is dispatched
/// with `launch: None`.
fn empty_pending_launch() -> PendingCloudLaunch {
    PendingCloudLaunch {
        prompt: String::new(),
        attachments: HandoffLaunchAttachments::default(),
    }
}

fn pending_handoff() -> PendingHandoff {
    PendingHandoff {
        forked_conversation_id: Some("forked-conversation".to_owned()),
        title: None,
        touched_workspace: None,
        snapshot_upload: SnapshotUploadStatus::Pending,
        submission_state: HandoffSubmissionState::Idle,
        auto_submit: Some(pending_launch()),
        orchestration_handoff: None,
        should_inject_continue: false,
    }
}

fn pending_handoff_fresh_launch() -> PendingHandoff {
    PendingHandoff {
        forked_conversation_id: None,
        title: None,
        touched_workspace: None,
        snapshot_upload: SnapshotUploadStatus::Pending,
        submission_state: HandoffSubmissionState::Idle,
        auto_submit: Some(pending_launch()),
        orchestration_handoff: None,
        should_inject_continue: false,
    }
}

fn pending_handoff_with_orchestration() -> PendingHandoff {
    PendingHandoff {
        forked_conversation_id: Some("forked-conversation".to_owned()),
        title: None,
        touched_workspace: None,
        snapshot_upload: SnapshotUploadStatus::Pending,
        submission_state: HandoffSubmissionState::Idle,
        auto_submit: Some(pending_launch()),
        orchestration_handoff: Some(true),
        should_inject_continue: false,
    }
}

/// Variant of `pending_handoff` for empty-prompt handoff tests. Lets the caller
/// set the source-conversation state and substitute an empty-prompt launch.
fn pending_handoff_empty(inject_continue: bool) -> PendingHandoff {
    PendingHandoff {
        forked_conversation_id: Some("forked-conversation".to_owned()),
        title: None,
        touched_workspace: None,
        snapshot_upload: SnapshotUploadStatus::Pending,
        submission_state: HandoffSubmissionState::Idle,
        auto_submit: Some(empty_pending_launch()),
        orchestration_handoff: None,
        should_inject_continue: inject_continue,
    }
}

/// Non-empty `TouchedWorkspace` for the snapshot-rehydration substitution test.
fn touched_workspace_with_orphan_file() -> TouchedWorkspace {
    TouchedWorkspace {
        repos: vec![],
        orphan_files: vec![std::path::PathBuf::from("/tmp/handoff-fixture.txt")],
    }
}

fn add_model(app: &mut App) -> warpui::ModelHandle<AmbientAgentViewModel> {
    app.add_model(|ctx| AmbientAgentViewModel::new(EntityId::new(), ctx))
}

#[test]
fn record_ambient_execution_ended_clears_active_session_and_enables_followup() {
    // REMOTE-2017: once the live execution session ends, the ambient pane must
    // drop `active_execution_session_id` so a follow-up routes to a cloud
    // handoff (`is_ready_for_cloud_followup_prompt`) instead of a local agent.
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let model = add_model(&mut app);
        let session_id = SessionId::new();
        let task = "11111111-1111-1111-1111-111111111111"
            .parse::<AmbientAgentTaskId>()
            .expect("hardcoded task id parses");

        model.update(&mut app, |model, _ctx| {
            model.task_id = Some(task);
            model.status = Status::AgentRunning;
            model.active_execution_session_id = Some(session_id);
            // A live execution session is attached, so no cloud follow-up yet.
            assert!(!model.is_ready_for_cloud_followup_prompt());
        });

        model.update(&mut app, |model, ctx| {
            model.record_ambient_execution_ended(session_id, ctx);
        });

        model.read(&app, |model, _| {
            assert_eq!(model.active_execution_session_id, None);
            assert_eq!(model.last_ended_execution_session_id, Some(session_id));
            assert!(
                model.is_ready_for_cloud_followup_prompt(),
                "after the live execution session ends the pane should accept a cloud follow-up"
            );
        });
    });
}

#[test]
fn record_ambient_execution_ended_keeps_active_session_when_id_differs() {
    // A teardown signal for a different (stale) session must not clear the live
    // session for the one currently attached.
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let model = add_model(&mut app);
        let live_session_id = SessionId::new();
        let other_session_id = SessionId::new();

        model.update(&mut app, |model, ctx| {
            model.active_execution_session_id = Some(live_session_id);
            model.record_ambient_execution_ended(other_session_id, ctx);
        });

        model.read(&app, |model, _| {
            assert_eq!(model.active_execution_session_id, Some(live_session_id));
            assert_eq!(
                model.last_ended_execution_session_id,
                Some(other_session_id)
            );
        });
    });
}

#[test]
fn set_live_execution_session_marks_session_live_until_it_ends() {
    // REMOTE-2047: a viewer that joins an already-running ambient session records the live
    // session id so a follow-up is not prematurely routed as a new cloud VM while the run is
    // live. When the session ends, `record_ambient_execution_ended` clears it and the pane
    // accepts a cloud follow-up.
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let model = add_model(&mut app);
        let session_id = SessionId::new();
        let task = "22222222-2222-2222-2222-222222222222"
            .parse::<AmbientAgentTaskId>()
            .expect("hardcoded task id parses");

        model.update(&mut app, |model, _ctx| {
            model.task_id = Some(task);
            model.status = Status::AgentRunning;
            // With no live session recorded yet, an AgentRunning task would already accept a
            // cloud follow-up.
            assert!(model.is_ready_for_cloud_followup_prompt());

            model.set_live_execution_session(session_id);
            assert_eq!(model.active_execution_session_id, Some(session_id));
            assert_eq!(model.last_ended_execution_session_id, None);
            assert!(
                !model.is_ready_for_cloud_followup_prompt(),
                "while the joined session is live, follow-ups go to the live sharer"
            );
        });

        model.update(&mut app, |model, ctx| {
            model.record_ambient_execution_ended(session_id, ctx);
        });

        model.read(&app, |model, _| {
            assert_eq!(model.active_execution_session_id, None);
            assert_eq!(model.last_ended_execution_session_id, Some(session_id));
            assert!(
                model.is_ready_for_cloud_followup_prompt(),
                "after the live session ends the viewer can start a cloud follow-up"
            );
        });
    });
}

fn retry_request(prompt: impl Into<String>) -> SpawnAgentRequest {
    SpawnAgentRequest {
        prompt: Some(prompt.into()),
        mode: crate::server::server_api::ai::UserQueryMode::Normal,
        config: Some(AgentConfigSnapshot {
            environment_id: Some("env-123".to_string()),
            model_id: Some("model-123".to_string()),
            worker_host: Some("worker-123".to_string()),
            computer_use_enabled: Some(false),
            ..Default::default()
        }),
        title: Some("Retry title".to_string()),
        team: Some(true),
        agent_identity_uid: Some("agent-123".to_string()),
        skill: None,
        attachments: vec![attachment()],
        interactive: Some(true),
        parent_run_id: Some("parent-run-123".to_string()),
        runtime_skills: vec!["runtime-skill".to_string()],
        referenced_attachments: vec!["referenced-attachment".to_string()],
        conversation_id: Some("conversation-123".to_string()),
        initial_snapshot_token: Some(
            serde_json::from_str("\"snapshot-token-123\"").expect("snapshot token should parse"),
        ),
        snapshot_disabled: Some(true),
        orchestration_handoff: None,
    }
}

fn test_environment_id() -> ServerId {
    ServerId::from(123)
}

#[test]
fn github_auth_url_for_initial_run_includes_focus_cloud_mode_next() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let model = add_model(&mut app);

        model.update(&mut app, |model, ctx| {
            model.status = Status::WaitingForSession {
                progress: AgentProgress::new(),
                kind: SessionStartupKind::InitialRun,
            };
            model.request = Some(retry_request("fix tests"));
            model.handle_needs_github_auth(
                "https://example.com/oauth/connect/github?scheme=warpdev".to_string(),
                "auth required".to_string(),
                ctx,
            );
        });

        model.read(&app, |model, _| {
            let auth_url = model.github_auth_url().expect("auth url should be present");
            assert_eq!(model.github_auth_error_message(), Some("auth required"));
            let parsed = Url::parse(auth_url).expect("auth url should parse");
            let next = parsed
                .query_pairs()
                .find(|(key, _)| key == "next")
                .map(|(_, value)| value.into_owned());
            assert_eq!(
                next,
                Some("warpdev://action/focus_cloud_mode?source=cloud_setup".to_string())
            );
        });
    });
}

#[test]
fn github_auth_completed_retries_stored_initial_run_request() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let model = add_model(&mut app);

        model.update(&mut app, |model, ctx| {
            model.status = Status::NeedsGithubAuth {
                progress: AgentProgress::new(),
                error_message: "auth required".to_string(),
                auth_url: "https://example.com/oauth/connect/github".to_string(),
            };
            model.request = Some(retry_request("retry this"));

            model.handle_github_auth_completed(ctx);

            assert!(matches!(
                model.status(),
                Status::WaitingForSession {
                    kind: SessionStartupKind::InitialRun,
                    ..
                }
            ));
            let request = model.request().expect("retry should spawn a request");
            assert_eq!(request.prompt.as_deref(), Some("retry this"));
            assert_eq!(request.attachments.len(), 1);
            assert_eq!(request.interactive, Some(true));
            assert_eq!(request.team, Some(true));
            assert_eq!(request.parent_run_id.as_deref(), Some("parent-run-123"));
            assert_eq!(request.title.as_deref(), Some("Retry title"));
            assert_eq!(request.agent_identity_uid.as_deref(), Some("agent-123"));
            assert_eq!(request.runtime_skills, vec!["runtime-skill"]);
            assert_eq!(
                request.referenced_attachments,
                vec!["referenced-attachment"]
            );
            assert_eq!(request.conversation_id.as_deref(), Some("conversation-123"));
            assert_eq!(
                request
                    .initial_snapshot_token
                    .as_ref()
                    .map(|token| token.as_str()),
                Some("snapshot-token-123")
            );
            assert_eq!(request.snapshot_disabled, Some(true));
            let config = request.config.as_ref().expect("config should be preserved");
            assert_eq!(config.environment_id.as_deref(), Some("env-123"));
            assert_eq!(config.model_id.as_deref(), Some("model-123"));
            assert_eq!(config.worker_host.as_deref(), Some("worker-123"));
            assert_eq!(config.computer_use_enabled, Some(false));
        });
    });
}

#[test]
fn viewed_task_config_preserves_environment_before_cloud_model_load() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let model = add_model(&mut app);
        let environment_id = test_environment_id();

        model.update(&mut app, |model, ctx| {
            model.apply_viewed_task_config_snapshot(
                Some(&AgentConfigSnapshot {
                    environment_id: Some(environment_id.to_string()),
                    ..Default::default()
                }),
                ctx,
            );
            model.validate_environment_after_initial_load(ctx);
        });

        model.read(&app, |model, _| {
            assert_eq!(
                model.selected_environment_id(),
                Some(&SyncId::ServerId(environment_id))
            );
        });
    });
}

#[test]
fn viewed_task_config_applies_oz_model_override() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let model = add_model(&mut app);
        let terminal_view_id = model.read(&app, |model, _| model.terminal_view_id);

        model.update(&mut app, |model, ctx| {
            model.apply_viewed_task_config_snapshot(
                Some(&AgentConfigSnapshot {
                    model_id: Some("model-from-run".to_string()),
                    ..Default::default()
                }),
                ctx,
            );
        });

        let override_value = model.read(&app, |_, app| {
            LLMPreferences::as_ref(app)
                .get_base_llm_override(terminal_view_id)
                .expect("viewed run model should be stored as a pane override")
        });
        assert_eq!(override_value, "\"model-from-run\"");
    });
}

#[test]
fn followup_github_auth_does_not_reuse_stored_initial_request() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let model = add_model(&mut app);

        model.update(&mut app, |model, ctx| {
            model.status = Status::WaitingForSession {
                progress: AgentProgress::new(),
                kind: SessionStartupKind::Followup,
            };
            model.request = Some(retry_request("do not retry"));
            model.handle_needs_github_auth(
                "https://example.com/oauth/connect/github".to_string(),
                "auth required".to_string(),
                ctx,
            );

            assert!(matches!(model.status(), Status::NeedsGithubAuth { .. }));
            assert!(model.request().is_none());

            model.handle_github_auth_completed(ctx);

            assert!(matches!(model.status(), Status::NeedsGithubAuth { .. }));
        });
    });
}

#[test]
fn queue_handoff_auto_submit_enters_waiting_state_without_consuming_launch() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let model = add_model(&mut app);

        model.update(&mut app, |model, ctx| {
            model.set_pending_handoff(Some(pending_handoff()), ctx);
        });

        let queued = model.update(&mut app, |model, ctx| model.queue_handoff_auto_submit(ctx));

        assert!(queued);
        model.read(&app, |model, _| {
            assert!(matches!(
                model.status(),
                Status::WaitingForSession {
                    kind: SessionStartupKind::InitialRun,
                    ..
                }
            ));
            let request = model.request().expect("request should be populated");
            assert_eq!(request.prompt.as_deref(), Some("fix tests"));
            assert_eq!(
                request.conversation_id.as_deref(),
                Some("forked-conversation")
            );
            assert_eq!(request.attachments.len(), 1);
            assert!(request.initial_snapshot_token.is_none());

            let handoff = model
                .pending_handoff
                .as_ref()
                .expect("handoff should remain");
            assert_eq!(handoff.submission_state, HandoffSubmissionState::Queued);
            assert!(handoff.auto_submit.is_some());
        });

        let queued_again =
            model.update(&mut app, |model, ctx| model.queue_handoff_auto_submit(ctx));
        assert!(!queued_again);
    });
}

#[test]
fn maybe_auto_submit_handoff_waits_for_workspace_and_snapshot_then_consumes_launch() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let model = add_model(&mut app);

        model.update(&mut app, |model, ctx| {
            model.set_pending_handoff(Some(pending_handoff()), ctx);
            assert!(model.maybe_auto_submit_handoff(ctx).is_none());

            model.set_pending_handoff_workspace(TouchedWorkspace::default(), ctx);
            assert!(model.maybe_auto_submit_handoff(ctx).is_none());

            model.set_pending_handoff_snapshot_upload(
                SnapshotUploadStatus::SkippedEmptyWorkspace,
                ctx,
            );
            let launch = model
                .maybe_auto_submit_handoff(ctx)
                .expect("ready handoff should auto-submit");
            assert_eq!(launch.prompt, "fix tests");
            assert!(model.maybe_auto_submit_handoff(ctx).is_none());
        });
    });
}

#[test]
fn fresh_launch_queues_handoff_with_no_conversation_id() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let model = add_model(&mut app);

        model.update(&mut app, |model, ctx| {
            model.set_pending_handoff(Some(pending_handoff_fresh_launch()), ctx);
        });

        let queued = model.update(&mut app, |model, ctx| model.queue_handoff_auto_submit(ctx));

        assert!(queued);
        model.read(&app, |model, _| {
            let request = model.request().expect("request should be populated");
            assert_eq!(request.prompt.as_deref(), Some("fix tests"));
            assert!(request.conversation_id.is_none());
            assert_eq!(request.attachments.len(), 1);
        });
    });
}

#[test]
fn fresh_launch_auto_submits_after_snapshot_settles() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let model = add_model(&mut app);

        model.update(&mut app, |model, ctx| {
            model.set_pending_handoff(Some(pending_handoff_fresh_launch()), ctx);
            assert!(model.maybe_auto_submit_handoff(ctx).is_none());

            model.set_pending_handoff_workspace(TouchedWorkspace::default(), ctx);
            assert!(model.maybe_auto_submit_handoff(ctx).is_none());

            model.set_pending_handoff_snapshot_upload(
                SnapshotUploadStatus::SkippedEmptyWorkspace,
                ctx,
            );
            let launch = model
                .maybe_auto_submit_handoff(ctx)
                .expect("ready fresh-launch handoff should auto-submit");
            assert_eq!(launch.prompt, "fix tests");
            assert!(model.maybe_auto_submit_handoff(ctx).is_none());
        });
    });
}

#[test]
fn handoff_request_omits_orchestration_handoff_when_pending_handoff_has_none() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let model = add_model(&mut app);

        model.update(&mut app, |model, ctx| {
            model.set_pending_handoff(Some(pending_handoff()), ctx);
            model.queue_handoff_auto_submit(ctx);
        });

        model.read(&app, |model, _| {
            let request = model.request().expect("request should be populated");
            assert!(request.orchestration_handoff.is_none());
            let json = serde_json::to_value(request).expect("request should serialize to JSON");
            assert!(json.get("orchestration_handoff").is_none());
        });
    });
}

#[test]
fn handoff_request_carries_universal_orchestration_handoff_marker() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let model = add_model(&mut app);

        model.update(&mut app, |model, ctx| {
            model.set_pending_handoff(Some(pending_handoff_with_orchestration()), ctx);
            model.queue_handoff_auto_submit(ctx);
        });

        model.read(&app, |model, _| {
            let request = model.request().expect("request should be populated");
            assert_eq!(request.orchestration_handoff, Some(true));

            let json = serde_json::to_value(request).expect("request should serialize to JSON");
            assert_eq!(
                json.get("orchestration_handoff")
                    .and_then(serde_json::Value::as_bool),
                Some(true)
            );
        });
    });
}

/// Installs a single-model Agent Mode catalog (also used as the default) so
/// `get_active_base_model` resolves to it via the profile-default fallback.
fn install_default_agent_mode_model(
    model: &warpui::ModelHandle<AmbientAgentViewModel>,
    app: &mut App,
    info: LLMInfo,
) {
    let default_id = info.id.clone();
    model.update(app, |_model, ctx| {
        let models = ModelsByFeature {
            agent_mode: AvailableLLMs::new(default_id, vec![info], None)
                .expect("valid available llms"),
            ..Default::default()
        };
        LLMPreferences::handle(ctx).update(ctx, |prefs, ctx| {
            prefs.update_feature_model_choices(Ok(models), ctx);
        });
    });
}

/// The local→cloud handoff "invalid model_id" regression: when the pane's
/// active Agent Mode model is not cloud-runnable (custom-endpoint/BYOK model or
/// local custom router), `build_default_spawn_config` must fall back to the
/// `auto` slug instead of forwarding an id the cloud `start_agent` endpoint
/// rejects, while a server-accepted Oz slug is forwarded unchanged. A local
/// custom-router id stands in for the custom-endpoint UUID case, which the
/// `is_cloud_runnable_oz_model_id` unit tests in `llms_tests.rs` cover.
#[test]
fn spawn_config_falls_back_to_auto_only_for_non_cloud_runnable_model() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let model = add_model(&mut app);

        // Handoff mode makes `selected_harness()` == Oz so the Oz model-id
        // branch of `build_default_spawn_config` runs.
        model.update(&mut app, |model, ctx| {
            model.set_pending_handoff(Some(pending_handoff()), ctx);
        });

        install_default_agent_mode_model(
            &model,
            &mut app,
            LLMInfo::new_for_test("custom-router:local:byok"),
        );
        model.read(&app, |model, app| {
            assert_eq!(
                model.build_default_spawn_config(app).model_id.as_deref(),
                Some("auto"),
                "a non-cloud-runnable model must fall back to the `auto` slug"
            );
        });

        install_default_agent_mode_model(&model, &mut app, LLMInfo::new_for_test("auto-genius"));
        model.read(&app, |model, app| {
            assert_eq!(
                model.build_default_spawn_config(app).model_id.as_deref(),
                Some("auto-genius"),
                "a server-accepted Oz slug should be forwarded unchanged"
            );
        });
    });
}

/// The handoff carries the source conversation's selected model onto the new
/// pane by seeding a per-pane override on the new pane's `terminal_view_id`.
/// This verifies that such an override is honored by
/// `build_default_spawn_config` (instead of reverting to the profile default).
#[test]
fn handoff_honors_pane_model_override() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let model = add_model(&mut app);
        let terminal_view_id = model.read(&app, |model, _| model.terminal_view_id);

        // Catalog with two slug models; `auto` is the profile default.
        model.update(&mut app, |_model, ctx| {
            let models = ModelsByFeature {
                agent_mode: AvailableLLMs::new(
                    "auto".into(),
                    vec![
                        LLMInfo::new_for_test("auto"),
                        LLMInfo::new_for_test("auto-genius"),
                    ],
                    None,
                )
                .expect("valid available llms"),
                ..Default::default()
            };
            LLMPreferences::handle(ctx).update(ctx, |prefs, ctx| {
                prefs.update_feature_model_choices(Ok(models), ctx);
            });
        });

        // Seed a per-pane selection, as the handoff open path's carry-over does.
        model.update(&mut app, |_model, ctx| {
            LLMPreferences::handle(ctx).update(ctx, |prefs, ctx| {
                prefs.update_preferred_agent_mode_llm(
                    &LLMId::from("auto-genius"),
                    terminal_view_id,
                    ctx,
                );
            });
        });

        model.update(&mut app, |model, ctx| {
            model.set_pending_handoff(Some(pending_handoff()), ctx);
        });

        model.read(&app, |model, app| {
            assert_eq!(
                model.build_default_spawn_config(app).model_id.as_deref(),
                Some("auto-genius"),
                "the carried per-pane model override must be honored over the profile default"
            );
        });
    });
}

#[test]
fn spawn_agent_omits_orchestration_handoff_for_fresh_launches() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let model = add_model(&mut app);

        model.update(&mut app, |model, ctx| {
            model.spawn_agent("new run".to_owned(), vec![], ctx);
        });

        model.read(&app, |model, _| {
            let request = model.request().expect("request should be populated");
            assert!(request.orchestration_handoff.is_none());
            let json = serde_json::to_value(request).expect("request should serialize to JSON");
            assert!(json.get("orchestration_handoff").is_none());
        });
    });
}

#[test]
fn snapshot_failure_is_treated_as_settled_for_auto_submit() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let model = add_model(&mut app);

        model.update(&mut app, |model, ctx| {
            model.set_pending_handoff(Some(pending_handoff()), ctx);
            model.set_pending_handoff_workspace(TouchedWorkspace::default(), ctx);
            model.set_pending_handoff_snapshot_upload(
                SnapshotUploadStatus::Failed("upload failed".to_owned()),
                ctx,
            );

            let launch = model
                .maybe_auto_submit_handoff(ctx)
                .expect("Failed snapshot should be treated as settled");
            assert_eq!(launch.prompt, "fix tests");
            assert!(model.maybe_auto_submit_handoff(ctx).is_none());
        });
    });
}

#[test]
fn empty_prompt_auto_submit_with_active_source_substitutes_continue_on_wire() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let model = add_model(&mut app);

        model.update(&mut app, |model, ctx| {
            model.set_pending_handoff(Some(pending_handoff_empty(/*inject_continue*/ true)), ctx);
        });

        let queued = model.update(&mut app, |model, ctx| model.queue_handoff_auto_submit(ctx));
        assert!(
            queued,
            "empty-prompt auto-submit should be accepted, not gated"
        );

        model.read(&app, |model, _| {
            let request = model.request().expect("request should be populated");
            assert_eq!(
                request.prompt.as_deref(),
                Some("Continue"),
                "active source + empty prompt must substitute the wire prompt with \"Continue\"",
            );
        });
    });
}

#[test]
fn empty_prompt_auto_submit_with_idle_source_sends_none_on_the_wire() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let model = add_model(&mut app);

        model.update(&mut app, |model, ctx| {
            model.set_pending_handoff(Some(pending_handoff_empty(/*inject_continue*/ false)), ctx);
            // The queue path passes no snapshot token to
            // `build_handoff_spawn_request`, so the substitution resolves to
            // `None` regardless of the derived workspace. The
            // snapshot-rehydration substitution can only fire on the
            // `submit_handoff` path (covered separately).
            model.set_pending_handoff_workspace(touched_workspace_with_orphan_file(), ctx);
        });

        let queued = model.update(&mut app, |model, ctx| model.queue_handoff_auto_submit(ctx));
        assert!(queued);

        model.read(&app, |model, _| {
            let request = model.request().expect("request should be populated");
            assert!(
                request.prompt.is_none(),
                "idle source + empty prompt must send prompt: None (got {:?})",
                request.prompt
            );
        });
    });
}

#[test]
fn empty_prompt_submit_handoff_with_idle_source_and_snapshot_substitutes_apply_workspace_changes() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let model = add_model(&mut app);

        let token: InitialSnapshotToken =
            serde_json::from_str("\"snapshot-token-abc\"").expect("snapshot token should parse");

        model.update(&mut app, |model, ctx| {
            model.set_pending_handoff(Some(pending_handoff_empty(/*inject_continue*/ false)), ctx);
            model.set_pending_handoff_workspace(touched_workspace_with_orphan_file(), ctx);
            model.set_pending_handoff_snapshot_upload(SnapshotUploadStatus::Uploaded(token), ctx);
        });

        // submit_handoff is the only entry point that can resolve the
        // snapshot-rehydration substitution; it passes the snapshot token
        // through to `build_handoff_spawn_request`.
        model.update(&mut app, |model, ctx| {
            model.submit_handoff(String::new(), vec![], ctx);
        });

        model.read(&app, |model, _| {
            let request = model.request().expect("request should be populated");
            assert_eq!(
                request.prompt.as_deref(),
                Some("Apply the workspace changes from my previous session."),
                "idle source + non-empty snapshot token must substitute the rehydration prompt",
            );
            assert!(
                request.initial_snapshot_token.is_some(),
                "the snapshot token must still ride alongside the substituted prompt",
            );
        });
    });
}

#[test]
fn empty_prompt_submit_handoff_with_active_source_and_snapshot_concatenates_continue_and_apply() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let model = add_model(&mut app);

        let token: InitialSnapshotToken =
            serde_json::from_str("\"snapshot-token-xyz\"").expect("snapshot token should parse");

        model.update(&mut app, |model, ctx| {
            model.set_pending_handoff(Some(pending_handoff_empty(/*inject_continue*/ true)), ctx);
            model.set_pending_handoff_workspace(touched_workspace_with_orphan_file(), ctx);
            model.set_pending_handoff_snapshot_upload(SnapshotUploadStatus::Uploaded(token), ctx);
        });

        model.update(&mut app, |model, ctx| {
            model.submit_handoff(String::new(), vec![], ctx);
        });

        model.read(&app, |model, _| {
            let request = model.request().expect("request should be populated");
            assert_eq!(
                request.prompt.as_deref(),
                Some("Continue. Apply the workspace changes from my previous session."),
                "active source + non-empty snapshot token must concatenate both substitutions",
            );
            assert!(
                request.initial_snapshot_token.is_some(),
                "the snapshot token must still ride alongside the substituted prompt",
            );
        });
    });
}

#[test]
fn empty_prompt_submit_handoff_with_idle_source_and_no_snapshot_sends_none_on_the_wire() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let model = add_model(&mut app);

        model.update(&mut app, |model, ctx| {
            model.set_pending_handoff(Some(pending_handoff_empty(/*inject_continue*/ false)), ctx);
            model.set_pending_handoff_workspace(TouchedWorkspace::default(), ctx);
            model.set_pending_handoff_snapshot_upload(
                SnapshotUploadStatus::SkippedEmptyWorkspace,
                ctx,
            );
        });

        model.update(&mut app, |model, ctx| {
            model.submit_handoff(String::new(), vec![], ctx);
        });

        model.read(&app, |model, _| {
            let request = model.request().expect("request should be populated");
            assert!(
                request.prompt.is_none(),
                "idle source + empty snapshot must send prompt: None (got {:?})",
                request.prompt,
            );
            assert!(request.initial_snapshot_token.is_none());
        });
    });
}
