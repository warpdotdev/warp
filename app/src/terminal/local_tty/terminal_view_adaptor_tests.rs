use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use base64::Engine as _;
use prost::Message as _;
use session_sharing_protocol::common::{
    AgentPromptRequest, AgentPromptRequestId, ParticipantId, ProfileData,
};
use warpui::{App, SingletonEntity};

use super::{accept_agent_prompt, is_setup_failure_debug_prompt_authorized};
use crate::ai::agent::BaseUserQuery;
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::blocklist::local_agent_task_sync_model::LocalAgentTaskSyncModel;
use crate::ai::blocklist::pending_cli_harness_prompt_queue::PendingCliHarnessPromptQueue;
use crate::auth::UserUid;
use crate::server::server_api::ai::{AIClient, MockAIClient};
use crate::terminal::CLIAgent;
use crate::terminal::cli_agent_sessions::{
    CLIAgentInputState, CLIAgentSession, CLIAgentSessionStatus, CLIAgentSessionsModel,
};
use crate::terminal::view::Event;
use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};

fn fixed_task_id() -> AmbientAgentTaskId {
    "550e8400-e29b-41d4-a716-446655440b00"
        .parse()
        .expect("valid task id")
}

fn owner_uid() -> UserUid {
    UserUid::new("task-owner-uid")
}

fn prompt_request(prompt: &str, user_query_b64: Option<String>) -> AgentPromptRequest {
    AgentPromptRequest {
        id: AgentPromptRequestId::new(),
        server_conversation_token: None,
        prompt: prompt.into(),
        attachments: vec![],
        user_query_b64,
    }
}

fn encoded_author(uid: &str) -> String {
    let base = BaseUserQuery::for_viewer(Some(&ProfileData {
        firebase_uid: uid.into(),
        ..Default::default()
    }));
    base64::engine::general_purpose::STANDARD.encode(base.to_proto().encode_to_vec())
}

#[test]
fn pending_cli_prompts_keep_independent_identity_in_fifo_order() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let alice = ParticipantId::new();
        let bob = ParticipantId::new();

        app.update(|ctx| {
            LocalAgentTaskSyncModel::handle(ctx).update(ctx, |model, _| {
                model.register_cli_session_for_test(terminal.id(), fixed_task_id());
            });
            for (text, encoded, participant) in [
                ("first", Some(encoded_author("alice")), alice.clone()),
                ("second", Some(encoded_author("bob")), bob.clone()),
                ("invalid", Some("invalid!".into()), alice.clone()),
                ("unknown", None, ParticipantId::new()),
                ("/clear", Some(encoded_author("bob")), bob.clone()),
            ] {
                accept_agent_prompt(&terminal, prompt_request(text, encoded), participant, ctx);
            }

            PendingCliHarnessPromptQueue::handle(ctx).update(ctx, |queue, _| {
                let prompts = queue.drain(fixed_task_id());
                assert_eq!(prompts.len(), 5);
                assert!(prompts[0].prompt.ends_with("\n\nfirst"));
                assert!(prompts[0].prompt.contains("coauthor_user_uid: \"alice\""));
                assert_eq!(prompts[0].participant_id, alice);
                assert!(prompts[1].prompt.ends_with("\n\nsecond"));
                assert!(prompts[1].prompt.contains("coauthor_user_uid: \"bob\""));
                assert!(!prompts[1].prompt.contains("alice"));
                assert_eq!(prompts[1].participant_id, bob);
                for prompt in &prompts[2..4] {
                    assert!(prompt.prompt.contains("coauthor_user_uid: \"\""));
                    assert!(!prompt.prompt.contains("alice"));
                    assert!(!prompt.prompt.contains("bob"));
                }
                assert_eq!(prompts[4].prompt, "/clear");
                assert!(queue.drain(fixed_task_id()).is_empty());
            });
        });
    });
}

#[test]
fn live_cli_prompts_format_identity_without_changing_raw_control_submission() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let writes = Rc::new(RefCell::new(Vec::new()));
        let captured = writes.clone();
        app.update(|ctx| {
            ctx.subscribe_to_view(&terminal, move |_, event, _| {
                if let Event::WriteBytesToPty { bytes } = event {
                    captured.borrow_mut().push(bytes.to_vec());
                }
            });
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.set_session(
                    terminal.id(),
                    CLIAgentSession {
                        agent: CLIAgent::Pi,
                        status: CLIAgentSessionStatus::InProgress,
                        session_context: Default::default(),
                        input_state: CLIAgentInputState::Closed,
                        should_auto_toggle_input: false,
                        listener: None,
                        remote_host: None,
                        plugin_version: None,
                        draft_text: None,
                        custom_command_prefix: None,
                        received_rich_notification: false,
                    },
                    ctx,
                );
            });
            for (prompt, encoded) in [
                ("first", Some(encoded_author("alice"))),
                ("second", Some(encoded_author("bob"))),
                ("unknown", None),
                ("/clear", Some(encoded_author("bob"))),
            ] {
                accept_agent_prompt(
                    &terminal,
                    prompt_request(prompt, encoded),
                    ParticipantId::new(),
                    ctx,
                );
            }
        });

        let writes = writes.borrow();
        assert_eq!(writes.len(), 4);
        let first = String::from_utf8(writes[0].clone()).unwrap();
        let second = String::from_utf8(writes[1].clone()).unwrap();
        let unknown = String::from_utf8(writes[2].clone()).unwrap();
        assert!(first.contains("coauthor_user_uid: \"alice\""));
        assert!(first.ends_with("\n\nfirst\r"));
        assert!(second.contains("coauthor_user_uid: \"bob\""));
        assert!(second.ends_with("\n\nsecond\r"));
        assert!(!second.contains("alice"));
        assert!(unknown.contains("coauthor_user_uid: \"\""));
        assert_eq!(writes[3], b"/clear\r");
    });
}

/// The task owner (or any principal the server recognizes) is accepted when the server
/// explicitly authorizes the request.
#[tokio::test]
async fn authorized_principal_is_accepted() {
    let mut mock = MockAIClient::new();
    mock.expect_setup_failure_debug_authorization()
        .times(1)
        .returning(|_, _, _| Ok(true));
    let ai_client: Arc<dyn AIClient> = Arc::new(mock);

    let authorized = is_setup_failure_debug_prompt_authorized(
        &ai_client,
        fixed_task_id(),
        Some(owner_uid()),
        Some("workload-token".to_string()),
    )
    .await;

    assert!(authorized, "an explicit server Ok(true) must be honored");
}

/// An unauthorized participant (e.g. a link/view-only viewer the server does not recognize as
/// eligible) must be rejected.
#[tokio::test]
async fn unauthorized_participant_is_rejected() {
    let mut mock = MockAIClient::new();
    mock.expect_setup_failure_debug_authorization()
        .times(1)
        .returning(|_, _, _| Ok(false));
    let ai_client: Arc<dyn AIClient> = Arc::new(mock);

    let authorized = is_setup_failure_debug_prompt_authorized(
        &ai_client,
        fixed_task_id(),
        Some(UserUid::new("view-only-uid")),
        Some("workload-token".to_string()),
    )
    .await;

    assert!(
        !authorized,
        "an explicit server denial must reject the prompt"
    );
}

/// The `setupFailureDebugAuthorization` query erroring or being unreachable (network failure,
/// `UserFacingError`, or talking to an old server without the query) must never fail open.
#[tokio::test]
async fn server_error_is_rejected() {
    let mut mock = MockAIClient::new();
    mock.expect_setup_failure_debug_authorization()
        .times(1)
        .returning(|_, _, _| {
            Err(anyhow::anyhow!(
                "setupFailureDebugAuthorization unreachable"
            ))
        });
    let ai_client: Arc<dyn AIClient> = Arc::new(mock);

    let authorized = is_setup_failure_debug_prompt_authorized(
        &ai_client,
        fixed_task_id(),
        Some(owner_uid()),
        Some("workload-token".to_string()),
    )
    .await;

    assert!(!authorized, "a query error must never fail open");
}

/// A participant with no resolvable `firebase_uid` (e.g. truly anonymous link access) must be
/// rejected without even attempting the server call -- there is nothing to authorize.
#[tokio::test]
async fn unresolvable_participant_is_rejected_without_calling_the_server() {
    let mut mock = MockAIClient::new();
    mock.expect_setup_failure_debug_authorization().times(0);
    let ai_client: Arc<dyn AIClient> = Arc::new(mock);

    let authorized = is_setup_failure_debug_prompt_authorized(
        &ai_client,
        fixed_task_id(),
        None,
        Some("workload-token".to_string()),
    )
    .await;

    assert!(
        !authorized,
        "a participant with no resolvable firebase_uid must be rejected before any server call"
    );
}

/// A workload-token issuance failure (surfaced here as `workload_token: None`) must also reject,
/// even when the participant's UID resolved fine.
#[tokio::test]
async fn missing_workload_token_is_rejected_without_calling_the_server() {
    let mut mock = MockAIClient::new();
    mock.expect_setup_failure_debug_authorization().times(0);
    let ai_client: Arc<dyn AIClient> = Arc::new(mock);

    let authorized = is_setup_failure_debug_prompt_authorized(
        &ai_client,
        fixed_task_id(),
        Some(owner_uid()),
        None,
    )
    .await;

    assert!(
        !authorized,
        "a failed workload-token issuance must be rejected before any server call"
    );
}
