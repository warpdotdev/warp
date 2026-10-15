//! Acknowledging what became of a follow-up or control action warp-server injected into a run.
//!
//! The id a prompt or control action arrives with over session sharing is warp-server's own row
//! id, so the client reports each state transition straight back to the run's API. Acks are
//! best-effort: they never hold up the turn, retry briefly on transient failures, and an unknown
//! id (an older server, or a session-sharing-minted id) is tolerated.
use std::sync::Arc;

use serde::Serialize;
use warpui::r#async::executor::Background;

use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::server::retry_strategies::with_bounded_retry;
use crate::server::server_api::harness_support::HarnessSupportClient;

/// Which injected item an ack is about, by the id it arrived with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DeliveryAckTarget {
    Followup(String),
    ControlAction(String),
}

impl DeliveryAckTarget {
    pub(crate) fn followup(request_id: Option<&str>) -> Option<Self> {
        request_id.map(|id| Self::Followup(id.to_owned()))
    }

    pub(crate) fn control_action(request_id: Option<&str>) -> Option<Self> {
        request_id.map(|id| Self::ControlAction(id.to_owned()))
    }
}

/// Where an injected item is on its way from received to applied. Transitions are monotonic on
/// the server, so a repeated or out-of-order ack is harmless.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DeliveryAckState {
    Received,
    Steering,
    Applied,
    Failed,
}

/// Body of an ack request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct DeliveryAck {
    pub state: DeliveryAckState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Reports delivery states without blocking the caller.
pub(crate) trait DeliveryAcker: Send + Sync {
    fn ack(&self, target: DeliveryAckTarget, state: DeliveryAckState, reason: Option<String>);

    /// Acks `target` when the item carried an id; otherwise only logs the transition.
    fn ack_optional(
        &self,
        target: Option<DeliveryAckTarget>,
        state: DeliveryAckState,
        reason: Option<String>,
    ) {
        match target {
            Some(target) => self.ack(target, state, reason),
            None => log::info!(
                "event=delivery_ack_skipped state={state:?} reason={reason:?} cause=no_request_id"
            ),
        }
    }
}

/// Acks over the run's authenticated API, on the background executor.
pub(crate) struct ServerDeliveryAcker {
    client: Arc<dyn HarnessSupportClient>,
    run_id: Option<AmbientAgentTaskId>,
    background: Arc<Background>,
}

impl ServerDeliveryAcker {
    pub(crate) fn new(
        client: Arc<dyn HarnessSupportClient>,
        run_id: Option<AmbientAgentTaskId>,
        background: Arc<Background>,
    ) -> Self {
        Self {
            client,
            run_id,
            background,
        }
    }
}

impl DeliveryAcker for ServerDeliveryAcker {
    fn ack(&self, target: DeliveryAckTarget, state: DeliveryAckState, reason: Option<String>) {
        let Some(run_id) = self.run_id else {
            log::info!(
                "event=delivery_ack_skipped target={target:?} state={state:?} reason={reason:?} \
                 cause=no_run_id"
            );
            return;
        };
        log::info!("event=delivery_ack target={target:?} state={state:?} reason={reason:?}");
        let client = self.client.clone();
        let ack = DeliveryAck { state, reason };
        self.background
            .spawn(async move {
                let result = with_bounded_retry("delivery ack", || async {
                    match &target {
                        DeliveryAckTarget::Followup(id) => {
                            client.ack_followup(&run_id, id, &ack).await
                        }
                        DeliveryAckTarget::ControlAction(id) => {
                            client.ack_control_action(&run_id, id, &ack).await
                        }
                    }
                })
                .await;
                if let Err(error) = result {
                    log::warn!(
                        "event=delivery_ack_failed target={target:?} state={state:?}: {error:#}"
                    );
                }
            })
            .detach();
    }
}
