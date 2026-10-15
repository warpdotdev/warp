use std::sync::Arc;

use parking_lot::Mutex;

use super::{
    NO_TURN_TO_INTERRUPT, ack_interrupt_outcome, is_setup_failure_debug_prompt_authorized,
};
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::delivery_acks::{DeliveryAckState, DeliveryAckTarget, DeliveryAcker};
use crate::auth::UserUid;
use crate::server::server_api::ai::{AIClient, MockAIClient};

#[derive(Default)]
struct RecordingAcker {
    acks: Mutex<Vec<(DeliveryAckTarget, DeliveryAckState, Option<String>)>>,
}

impl DeliveryAcker for RecordingAcker {
    fn ack(&self, target: DeliveryAckTarget, state: DeliveryAckState, reason: Option<String>) {
        self.acks.lock().push((target, state, reason));
    }
}

/// An interrupt that arrives while the conversation is finished or unknown has nothing to
/// apply to, so it must settle as `failed` instead of staying at `received` forever.
#[test]
fn an_interrupt_with_nothing_to_cancel_is_acked_failed() {
    let acker = RecordingAcker::default();
    ack_interrupt_outcome(
        &acker,
        DeliveryAckTarget::ControlAction("77".to_owned()),
        false,
    );
    assert_eq!(
        acker.acks.lock().as_slice(),
        &[(
            DeliveryAckTarget::ControlAction("77".to_owned()),
            DeliveryAckState::Failed,
            Some(NO_TURN_TO_INTERRUPT.to_owned()),
        )]
    );
}

/// An interrupt a turn took is acked `applied` by that turn once the agent answers the cancel,
/// not here.
#[test]
fn an_interrupt_that_was_taken_is_left_for_the_turn_to_ack() {
    let acker = RecordingAcker::default();
    ack_interrupt_outcome(
        &acker,
        DeliveryAckTarget::ControlAction("77".to_owned()),
        true,
    );
    assert!(acker.acks.lock().is_empty());
}

fn fixed_task_id() -> AmbientAgentTaskId {
    "550e8400-e29b-41d4-a716-446655440b00"
        .parse()
        .expect("valid task id")
}

fn owner_uid() -> UserUid {
    UserUid::new("task-owner-uid")
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
