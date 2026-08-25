use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use ai::api_keys::{
    ApiKeyManager, AwsCredentials, AwsCredentialsState, CustomEndpointParams, GeapCredentials,
    GeapCredentialsState,
};
use chrono::Local;
use uuid::Uuid;
use warp_core::features::FeatureFlag;
use warp_multi_agent_api::response_event;
use warpui::{App, SingletonEntity, ViewHandle};

use super::response_stream::{PendingResume, RecoveryBudget};
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::task::TaskId;
use crate::ai::agent::{
    AIAgentAttachment, AIAgentContext, AIAgentInput, CancellationReason, ImageContext,
    PassiveSuggestionTrigger, UserQueryMode,
};
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::blocklist::orchestration_events::{
    OrchestrationEventService, PendingEvent, PendingEventDetail,
};
use crate::ai::blocklist::{
    BlocklistAIHistoryEvent, BlocklistAIHistoryModel, PendingAttachment, PendingFile, RequestInput,
    ResponseStream, ResponseStreamId,
};
use crate::ai::geap_credentials::{GeapPolicy, current_geap_policy};
use crate::ai::llms::{LLMId, LLMModelHost, LLMProvider};
use crate::server::ids::ServerId;
use crate::terminal::TerminalView;
use crate::test_util::terminal::{
    add_window_with_id_and_terminal, add_window_with_terminal, initialize_app_for_terminal_view,
};
use crate::workspaces::team::{Team, TeamVisibility};
use crate::workspaces::user_workspaces::{TeamScope, UserWorkspaces};
use crate::workspaces::workspace::{
    ByoApiKeyPolicy, ByoEndpointPolicy, HostEnablementSetting, LlmHostSettings,
    ManagedByokByoePolicy, TeamByoSettings, Workspace,
};

/// A workload identity provider resource name shaped like a real one, used only to satisfy
/// [`current_geap_policy`]'s non-empty-audience check.
const GEAP_TEST_AUDIENCE: &str = "//iam.googleapis.com/projects/123456/locations/global/workloadIdentityPools/warp-pool/providers/warp-provider";
const GEAP_TEST_SA_EMAIL: &str = "warp-geap@test-project.iam.gserviceaccount.com";

fn new_ambient_agent_task_id() -> AmbientAgentTaskId {
    Uuid::new_v4().to_string().parse().unwrap()
}

fn image_attachment(file_name: &str) -> PendingAttachment {
    PendingAttachment::Image(ImageContext {
        data: String::new(),
        mime_type: "image/png".to_owned(),
        file_name: file_name.to_owned(),
        is_figma: false,
    })
}

fn file_attachment(file_name: &str) -> PendingAttachment {
    PendingAttachment::File(PendingFile {
        file_name: file_name.to_owned(),
        file_path: file_name.into(),
        mime_type: "text/plain".to_owned(),
    })
}

#[test]
fn passive_suggestions_request_params_omit_ambient_agent_task_id() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |terminal, ctx| {
            let task_id = new_ambient_agent_task_id();
            let conversation_id =
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
                    history_model.start_new_conversation(terminal.id(), false, false, false, ctx)
                });

            terminal.ai_controller().update(ctx, |controller, ctx| {
                controller.set_ambient_agent_task_id(Some(task_id), ctx);

                assert_eq!(controller.get_ambient_agent_task_id(), Some(task_id));
                assert_eq!(
                    controller
                        .build_passive_suggestions_request_params(
                            Some(conversation_id),
                            PassiveSuggestionTrigger::FilesChanged,
                            vec![],
                            ctx,
                        )
                        .expect("existing conversation should build passive suggestion params")
                        .1
                        .ambient_agent_task_id,
                    None
                );
                assert_eq!(
                    controller
                        .build_passive_suggestions_request_params(
                            None,
                            PassiveSuggestionTrigger::FilesChanged,
                            vec![],
                            ctx,
                        )
                        .expect("new conversation should build passive suggestion params")
                        .1
                        .ambient_agent_task_id,
                    None
                );
            });
        });
    });
}

#[test]
fn input_for_query_converts_prompt_attachments_and_ignores_live_staging() {
    // `input_for_query` builds its image/file context purely from the explicitly-provided
    // attachment set (resolved by `send_query` from either the queued row or live staging),
    // never from the context model's pending attachments.
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |terminal, ctx| {
            let conversation_id =
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
                    history_model.start_new_conversation(terminal.id(), false, false, false, ctx)
                });

            let controller = terminal.ai_controller();
            let context_model = controller.as_ref(ctx).context_model.clone();
            let active_session = controller.as_ref(ctx).active_session.clone();

            // Stage *live* attachments that must NOT leak into a query built from a different,
            // explicitly-provided attachment set.
            context_model.update(ctx, |m, ctx| {
                m.append_pending_attachments(
                    vec![image_attachment("live.png"), file_attachment("live.txt")],
                    ctx,
                );
            });

            let task_id = TaskId::new("test-task".to_owned());
            // Two files sharing a basename to exercise duplicate-basename suffixing.
            let prompt_attachments = vec![
                image_attachment("queued.png"),
                file_attachment("notes.txt"),
                file_attachment("notes.txt"),
            ];

            let input = super::input_for_query(
                "build a query".to_owned(),
                &task_id,
                conversation_id,
                None,
                UserQueryMode::Normal,
                None,
                HashMap::new(),
                prompt_attachments,
                context_model.as_ref(ctx),
                active_session.as_ref(ctx),
                ctx,
            );

            let AIAgentInput::UserQuery {
                context,
                referenced_attachments,
                ..
            } = input
            else {
                panic!("expected UserQuery");
            };

            // The provided image is attached as image context; the live-staged image is not.
            let image_names: Vec<&str> = context
                .iter()
                .filter_map(|c| match c {
                    AIAgentContext::Image(img) => Some(img.file_name.as_str()),
                    _ => None,
                })
                .collect();
            assert_eq!(image_names, vec!["queued.png"]);

            // The provided files are attached as FilePathReference with duplicate-basename
            // suffixing; the live-staged file is not.
            let mut file_names: Vec<String> = referenced_attachments
                .values()
                .filter_map(|a| match a {
                    AIAgentAttachment::FilePathReference { file_name, .. } => {
                        Some(file_name.clone())
                    }
                    _ => None,
                })
                .collect();
            file_names.sort();
            assert_eq!(
                file_names,
                vec!["notes.txt".to_owned(), "notes.txt".to_owned()]
            );
            assert!(referenced_attachments.contains_key("notes.txt"));
            assert!(referenced_attachments.contains_key("notes.txt (1)"));
            assert!(!referenced_attachments.contains_key("live.txt"));
        });
    });
}

#[test]
fn cancelling_conversation_aborts_pending_auto_resume() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        // An ID with no backing conversation: if the scheduled wait ever
        // completes, the resume is a harmless no-op.
        let conversation_id = AIConversationId::new();

        terminal.update(&mut app, |terminal, ctx| {
            terminal.ai_controller().update(ctx, |controller, ctx| {
                let resume = PendingResume::new_for_test(
                    RecoveryBudget::fresh().next_attempt(),
                    std::time::Duration::from_millis(1),
                );
                controller.schedule_auto_resume_after_error(conversation_id, resume, ctx);
                assert!(
                    controller
                        .pending_auto_resume_handles
                        .contains_key(&conversation_id)
                );

                controller.cancel_conversation_progress(
                    conversation_id,
                    CancellationReason::ManuallyCancelled,
                    ctx,
                );
                assert!(
                    !controller
                        .pending_auto_resume_handles
                        .contains_key(&conversation_id)
                );
            });
        });
    });
}

#[test]
fn mock_response_stream_updates_history_through_controller() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let captured_events = Arc::new(Mutex::new(Vec::new()));
        let events_for_subscription = Arc::clone(&captured_events);
        app.update(|ctx| {
            ctx.subscribe_to_model(&BlocklistAIHistoryModel::handle(ctx), move |_, event, _| {
                events_for_subscription.lock().unwrap().push(event.clone())
            });
        });

        let (conversation_id, stream) = terminal.update(&mut app, |view, ctx| {
            let terminal_surface_id = view.id();
            let stream_id = ResponseStreamId::new_for_test();
            let conversation_id =
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                    let conversation_id = history.start_new_conversation(
                        terminal_surface_id,
                        false,
                        false,
                        false,
                        ctx,
                    );
                    let task_id = history
                        .conversation(&conversation_id)
                        .unwrap()
                        .get_root_task_id()
                        .clone();
                    history
                        .update_conversation_for_new_request_input(
                            RequestInput {
                                conversation_id,
                                input_messages: HashMap::from([(task_id, vec![])]),
                                working_directory: None,
                                model_id: LLMId::from("test-model"),
                                coding_model_id: LLMId::from("test-coding-model"),
                                cli_agent_model_id: LLMId::from("test-cli-agent-model"),
                                computer_use_model_id: LLMId::from("test-computer-use-model"),
                                shared_session_response_initiator: None,
                                request_start_ts: Local::now(),
                                supported_tools_override: None,
                            },
                            stream_id.clone(),
                            terminal_surface_id,
                            ctx,
                        )
                        .unwrap();
                    conversation_id
                });
            let stream = ctx.add_model(|_| ResponseStream::new_for_test(stream_id.clone()));
            view.ai_controller().update(ctx, |controller, ctx| {
                controller.register_mock_stream_for_test(
                    stream_id,
                    conversation_id,
                    stream.clone(),
                    ctx,
                );
            });
            (conversation_id, stream)
        });

        stream.update(&mut app, |stream, ctx| {
            stream.emit_response_event_for_test(
                warp_multi_agent_api::ResponseEvent {
                    r#type: Some(response_event::Type::Init(response_event::StreamInit {
                        request_id: "test-request".to_string(),
                        conversation_id: "test-server-conversation".to_string(),
                        run_id: String::new(),
                    })),
                },
                ctx,
            );
            stream.emit_response_event_for_test(
                warp_multi_agent_api::ResponseEvent {
                    r#type: Some(response_event::Type::Finished(
                        response_event::StreamFinished {
                            reason: Some(response_event::stream_finished::Reason::Done(
                                response_event::stream_finished::Done {},
                            )),
                            conversation_usage_metadata: None,
                            token_usage: vec![],
                            should_refresh_model_config: false,
                            #[allow(deprecated)]
                            request_cost: None,
                            request_charges: None,
                        },
                    )),
                },
                ctx,
            );
        });

        BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
            assert_eq!(
                history.conversation(&conversation_id).map(|c| c.status()),
                Some(&crate::ai::agent::conversation::ConversationStatus::Success)
            );
        });
        let events = captured_events.lock().unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            BlocklistAIHistoryEvent::ConversationServerTokenAssigned {
                conversation_id: id,
                ..
            } if *id == conversation_id
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            BlocklistAIHistoryEvent::UpdatedStreamingExchange {
                conversation_id: id,
                ..
            } if *id == conversation_id
        )));
    });
}

/// When an agent command exits the shell, the conversation must be finalized as
/// `Error` (not `Cancelled`), and a subsequent `ManuallyCancelled` (as fired by
/// the pane-close path) must not overwrite that failure.
#[test]
fn fail_conversation_due_to_shell_exit_reports_error_and_survives_manual_cancel() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        let conversation_id = terminal.update(&mut app, |view, ctx| {
            let terminal_surface_id = view.id();
            let stream_id = ResponseStreamId::new_for_test();
            let conversation_id =
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                    let conversation_id = history.start_new_conversation(
                        terminal_surface_id,
                        false,
                        false,
                        false,
                        ctx,
                    );
                    let task_id = history
                        .conversation(&conversation_id)
                        .unwrap()
                        .get_root_task_id()
                        .clone();
                    history
                        .update_conversation_for_new_request_input(
                            RequestInput {
                                conversation_id,
                                input_messages: HashMap::from([(task_id, vec![])]),
                                working_directory: None,
                                model_id: LLMId::from("test-model"),
                                coding_model_id: LLMId::from("test-coding-model"),
                                cli_agent_model_id: LLMId::from("test-cli-agent-model"),
                                computer_use_model_id: LLMId::from("test-computer-use-model"),
                                shared_session_response_initiator: None,
                                request_start_ts: Local::now(),
                                supported_tools_override: None,
                            },
                            stream_id.clone(),
                            terminal_surface_id,
                            ctx,
                        )
                        .unwrap();
                    conversation_id
                });
            let stream = ctx.add_model(|_| ResponseStream::new_for_test(stream_id.clone()));
            view.ai_controller().update(ctx, |controller, ctx| {
                controller.register_mock_stream_for_test(stream_id, conversation_id, stream, ctx);
                controller.fail_conversation_due_to_shell_exit(
                    conversation_id,
                    "exit 1".to_string(),
                    ctx,
                );
            });
            conversation_id
        });

        // The in-flight request is finalized as Error (with the shell-exit error
        // on its exchange), not Cancelled.
        BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
            assert_eq!(
                history.conversation(&conversation_id).map(|c| c.status()),
                Some(&crate::ai::agent::conversation::ConversationStatus::Error)
            );
        });

        // The pane-close cancellation path must be a no-op now that the
        // conversation is terminal.
        terminal.update(&mut app, |view, ctx| {
            view.ai_controller().update(ctx, |controller, ctx| {
                controller.cancel_conversation_progress(
                    conversation_id,
                    CancellationReason::ManuallyCancelled,
                    ctx,
                );
            });
        });
        BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
            assert_eq!(
                history.conversation(&conversation_id).map(|c| c.status()),
                Some(&crate::ai::agent::conversation::ConversationStatus::Error)
            );
        });
    });
}

/// An optimistic long-running-command completion that cancels an in-flight
/// stream must finalize the conversation as `Success`, not `Cancelled`. This is
/// a regression test for the reason -> status mapping living in a single place
/// (`CancellationReason::conversation_outcome`).
#[test]
fn optimistic_cli_subagent_completion_with_in_flight_stream_reports_success() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        let conversation_id = terminal.update(&mut app, |view, ctx| {
            let terminal_surface_id = view.id();
            let stream_id = ResponseStreamId::new_for_test();
            let conversation_id =
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                    let conversation_id = history.start_new_conversation(
                        terminal_surface_id,
                        false,
                        false,
                        false,
                        ctx,
                    );
                    let task_id = history
                        .conversation(&conversation_id)
                        .unwrap()
                        .get_root_task_id()
                        .clone();
                    history
                        .update_conversation_for_new_request_input(
                            RequestInput {
                                conversation_id,
                                input_messages: HashMap::from([(task_id, vec![])]),
                                working_directory: None,
                                model_id: LLMId::from("test-model"),
                                coding_model_id: LLMId::from("test-coding-model"),
                                cli_agent_model_id: LLMId::from("test-cli-agent-model"),
                                computer_use_model_id: LLMId::from("test-computer-use-model"),
                                shared_session_response_initiator: None,
                                request_start_ts: Local::now(),
                                supported_tools_override: None,
                            },
                            stream_id.clone(),
                            terminal_surface_id,
                            ctx,
                        )
                        .unwrap();
                    conversation_id
                });
            let stream = ctx.add_model(|_| ResponseStream::new_for_test(stream_id.clone()));
            view.ai_controller().update(ctx, |controller, ctx| {
                controller.register_mock_stream_for_test(stream_id, conversation_id, stream, ctx);
                // The long-running command finished while the agent was still
                // streaming, cancelling the in-flight stream optimistically.
                controller.cancel_conversation_progress(
                    conversation_id,
                    CancellationReason::CommandFinishedDuringInlineAgentView,
                    ctx,
                );
            });
            conversation_id
        });

        BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
            assert_eq!(
                history.conversation(&conversation_id).map(|c| c.status()),
                Some(&crate::ai::agent::conversation::ConversationStatus::Success)
            );
        });
    });
}

/// `drop_pending_events_for_exiting_conversation` drops any orchestration events still
/// queued for the conversation at the moment it's called, since they arrived too late to
/// ever be delivered once the run is exiting. Complements the controller-level guard above.
/// The exiting flag itself is a separate mechanism
/// ([`OrchestrationEventService::exit_commit_handle`]) this method doesn't touch.
#[test]
fn drop_pending_events_for_exiting_conversation_drops_pending_events() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        let conversation_id = terminal.update(&mut app, |view, ctx| {
            let conversation_id = BlocklistAIHistoryModel::handle(ctx)
                .update(ctx, |history, ctx| {
                    history.start_new_conversation(view.id(), false, false, false, ctx)
                });
            OrchestrationEventService::handle(ctx).update(ctx, |service, ctx| {
                service.enqueue_event_batch(
                    conversation_id,
                    vec![PendingEvent {
                        event_id: "event-1".to_string(),
                        source_agent_id: "child".to_string(),
                        attempt_count: 0,
                        detail: PendingEventDetail::Message {
                            message_id: "message-1".to_string(),
                            addresses: vec!["target".to_string()],
                            subject: "subject".to_string(),
                            message_body: "body".to_string(),
                        },
                    }],
                    ctx,
                );
            });
            conversation_id
        });

        terminal.update(&mut app, |_, ctx| {
            OrchestrationEventService::handle(ctx).update(ctx, |service, _| {
                assert!(service.has_pending_events(conversation_id));
                service.drop_pending_events_for_exiting_conversation(conversation_id);
                assert!(!service.has_pending_events(conversation_id));
            });
        });
    });
}

fn team_for_test(uid: i64, name: &str) -> Team {
    Team {
        uid: uid.into(),
        name: name.to_owned(),
        color: None,
        invite_link: None,
        members: vec![],
        pending_email_invites: vec![],
        invite_link_domain_restrictions: vec![],
        billing_metadata: Default::default(),
        stripe_customer_id: None,
        settings: Default::default(),
        is_eligible_for_discovery: false,
        has_billing_history: false,
        visibility: TeamVisibility::Open,
    }
}

fn workspace_for_test(teams: Vec<Team>) -> Workspace {
    Workspace {
        uid: "workspace_uid123456789".to_string().into(),
        name: "test".to_owned(),
        stripe_customer_id: None,
        teams,
        billing_metadata: Default::default(),
        bonus_grants_purchased_this_month: Default::default(),
        billing_cycle_usage: None,
        has_billing_history: false,
        settings: Default::default(),
        invite_link_domain_restrictions: vec![],
        pending_email_invites: vec![],
        is_eligible_for_discovery: false,
        members: vec![],
        total_requests_used_since_last_refresh: 0,
    }
}

fn set_current_workspace(app: &mut App, workspace: Workspace) {
    let workspace_uid = workspace.uid;
    let user_workspaces = UserWorkspaces::handle(app);
    user_workspaces.update(app, |user_workspaces, ctx| {
        user_workspaces.update_workspaces(vec![workspace], ctx);
        user_workspaces.set_current_workspace_uid(workspace_uid, ctx);
    });
}

fn controller_team_uid(terminal: &ViewHandle<TerminalView>, app: &mut App) -> Option<ServerId> {
    terminal.update(app, |terminal, ctx| {
        let controller = terminal.ai_controller().clone();
        controller.as_ref(ctx).team_context(ctx).team_uid()
    })
}

#[test]
fn team_context_follows_each_terminals_window() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let team_a = team_for_test(123, "team-a");
        let team_b = team_for_test(456, "team-b");
        set_current_workspace(
            &mut app,
            workspace_for_test(vec![team_a.clone(), team_b.clone()]),
        );

        let (window_a, terminal_a) = add_window_with_id_and_terminal(&mut app, None);
        let (window_b, terminal_b) = add_window_with_id_and_terminal(&mut app, None);
        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.set_team_for_window(window_a, team_a.uid, ctx);
            user_workspaces.set_team_for_window(window_b, team_b.uid, ctx);
        });

        assert_eq!(
            controller_team_uid(&terminal_a, &mut app),
            Some(team_a.uid),
            "the blocklist in window A is scoped to team A"
        );
        assert_eq!(
            controller_team_uid(&terminal_b, &mut app),
            Some(team_b.uid),
            "the blocklist in window B is scoped to team B, concurrently with A"
        );

        let terminal_a_id = terminal_a.id();
        app.update(|ctx| {
            ctx.transfer_view_tree_to_window(terminal_a_id, window_a, window_b);
        });

        assert_eq!(
            controller_team_uid(&terminal_a, &mut app),
            Some(team_b.uid),
            "after the transfer the blocklist is scoped to the destination window's team"
        );
    });
}

#[test]
fn team_context_has_no_team_when_the_window_has_none() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let team = team_for_test(123, "team-a");
        set_current_workspace(&mut app, workspace_for_test(vec![team]));

        let (window_id, terminal) = add_window_with_id_and_terminal(&mut app, None);
        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.register_window(window_id, None, ctx);
        });

        assert_eq!(
            controller_team_uid(&terminal, &mut app),
            None,
            "a window with no team must not borrow the workspace's only team's policy"
        );
    });
}

/// Two teams with opposing `team_byo` policy, so only the team's own policy can explain a
/// difference in behaviour between them. The plan-level BYO entitlement
/// (`byo_api_key_policy`/`byo_endpoint_policy`/`managed_byok_byoe_policy`) is workspace-owned,
/// not team-owned (see [`UserWorkspaces::is_managed_byok_byoe_enabled`]), so it is set on the
/// workspace by the caller instead of here.
fn two_teams_of_opposing_byo_policy() -> (Team, Team) {
    let mut team_a = team_for_test(123, "team-a");
    team_a.settings.team_byo = Some(TeamByoSettings {
        first_party_enabled: true,
        endpoints_enabled: true,
        allow_user_keys: true,
        allow_user_endpoints: true,
        first_party_keys: vec![],
        endpoints: vec![],
    });
    let mut team_b = team_for_test(456, "team-b");
    team_b.settings.team_byo = Some(TeamByoSettings {
        first_party_enabled: true,
        endpoints_enabled: true,
        allow_user_keys: false,
        allow_user_endpoints: false,
        first_party_keys: vec![],
        endpoints: vec![],
    });
    (team_a, team_b)
}

/// This is `RequestParams::new`'s construction-time regression fence: member keys and
/// endpoints are included or stripped by the requesting window's own team, while org-level
/// (AWS Bedrock, GEAP) credentials survive either team's policy.
#[test]
fn passive_suggestions_request_params_scope_member_byo_credentials_by_the_windows_team() {
    App::test((), |mut app| async move {
        let _geap_flag = FeatureFlag::GeminiEnterprise.override_enabled(true);
        initialize_app_for_terminal_view(&mut app);

        let (team_a, team_b) = two_teams_of_opposing_byo_policy();
        let mut workspace = workspace_for_test(vec![team_a.clone(), team_b.clone()]);
        // Plan-level BYO entitlement is workspace-owned (see
        // `UserWorkspaces::is_managed_byok_byoe_enabled`), so it's shared by both teams; only
        // `team_byo` differs between them.
        workspace.billing_metadata.tier.byo_api_key_policy =
            Some(ByoApiKeyPolicy { enabled: true });
        workspace.billing_metadata.tier.byo_endpoint_policy =
            Some(ByoEndpointPolicy { enabled: true });
        workspace.billing_metadata.tier.managed_byok_byoe_policy =
            Some(ManagedByokByoePolicy { enabled: true });
        workspace.settings.llm_settings.enabled = true;
        workspace.settings.llm_settings.host_configs.insert(
            LLMModelHost::AwsBedrock,
            LlmHostSettings {
                enabled: true,
                enablement_setting: HostEnablementSetting::Enforce,
                gcp_audience: None,
                gcp_sa_email: None,
            },
        );
        workspace.settings.llm_settings.host_configs.insert(
            LLMModelHost::GeminiEnterprise,
            LlmHostSettings {
                enabled: true,
                enablement_setting: HostEnablementSetting::Enforce,
                gcp_audience: Some(GEAP_TEST_AUDIENCE.to_string()),
                gcp_sa_email: Some(GEAP_TEST_SA_EMAIL.to_string()),
            },
        );
        set_current_workspace(&mut app, workspace);

        ApiKeyManager::handle(&app).update(&mut app, |manager, ctx| {
            manager
                .persist_provider_key(LLMProvider::Anthropic, Some("sk-ant-test".to_owned()), ctx)
                .expect("no-op secure storage should accept the provider key");
            manager.add_custom_endpoint(
                CustomEndpointParams {
                    name: "member-endpoint".to_string(),
                    url: "https://example.com/v1".to_string(),
                    api_key: "endpoint-key".to_string(),
                    models: vec![("member-model".to_string(), None, None)],
                    schema: Default::default(),
                },
                ctx,
            );
            manager.set_aws_credentials_state(
                AwsCredentialsState::Loaded {
                    credentials: AwsCredentials::new(
                        "access-key".to_string(),
                        "secret-key".to_string(),
                        None,
                        None,
                    ),
                    loaded_at: SystemTime::now(),
                },
                ctx,
            );
            let binding = match current_geap_policy(ctx) {
                GeapPolicy::Mintable(binding) => binding,
                other => panic!("expected a mintable GEAP policy, got {other:?}"),
            };
            manager.set_geap_credentials_state(
                GeapCredentialsState::Loaded {
                    credentials: GeapCredentials::new("geap-token".to_string(), None),
                    loaded_at: SystemTime::now(),
                    minted_for: binding,
                },
                ctx,
            );
        });

        let (window_a, terminal_a) = add_window_with_id_and_terminal(&mut app, None);
        let (window_b, terminal_b) = add_window_with_id_and_terminal(&mut app, None);
        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.set_team_for_window(window_a, team_a.uid, ctx);
            user_workspaces.set_team_for_window(window_b, team_b.uid, ctx);
        });

        let build_params = |terminal: &ViewHandle<TerminalView>, app: &mut App| {
            terminal.update(app, |terminal, ctx| {
                terminal.ai_controller().update(ctx, |controller, ctx| {
                    controller
                        .build_passive_suggestions_request_params(
                            None,
                            PassiveSuggestionTrigger::FilesChanged,
                            vec![],
                            ctx,
                        )
                        .expect("should build passive suggestion params")
                        .1
                })
            })
        };

        let params_a = build_params(&terminal_a, &mut app);
        let keys_a = params_a
            .api_keys
            .expect("team A's policy allows members to use their own keys");
        assert!(
            !keys_a.anthropic.is_empty(),
            "team A's policy allows members to use their own keys"
        );
        assert!(
            keys_a.aws_credentials.is_some(),
            "Bedrock credentials are org-level and must survive either team's policy"
        );
        assert!(
            keys_a.google_cloud_credentials.is_some(),
            "GEAP credentials are org-level and must survive either team's policy"
        );
        assert!(
            params_a.custom_model_providers.is_some(),
            "team A's policy allows members to use their own custom endpoints"
        );
        assert!(
            params_a.member_byo_credentials_allowed,
            "the permissive team's decision must be recorded on the params"
        );

        let params_b = build_params(&terminal_b, &mut app);
        let keys_b = params_b.api_keys.expect(
            "Bedrock/GEAP credentials must keep api_keys populated even once member keys are stripped",
        );
        assert!(
            keys_b.anthropic.is_empty(),
            "team B's policy disallows members from using their own keys"
        );
        assert!(
            keys_b.aws_credentials.is_some(),
            "a restrictive team_byo policy must not strip org-level Bedrock credentials"
        );
        assert!(
            keys_b.google_cloud_credentials.is_some(),
            "a restrictive team_byo policy must not strip org-level GEAP credentials"
        );
        assert!(
            params_b.custom_model_providers.is_none(),
            "team B's policy disallows members from using their own custom endpoints"
        );
        assert!(
            !params_b.member_byo_credentials_allowed,
            "the restrictive team's decision must be recorded on the params"
        );
    });
}
