use std::future::Future;
use std::time::Duration;

use futures::FutureExt;
use futures::future::{Either, select};
use instant::Instant;
use rmcp::model::{
    CallToolRequest, CallToolRequestParams, CallToolResult, CancelledNotificationParam,
    ClientRequest, ErrorData, RequestId, ServerResult,
};
use rmcp::service::PeerRequestOptions;
use rmcp::{Peer, RoleClient, ServiceError};
use uuid::Uuid;
use warpui::r#async::Timer;

/// Total connection, dispatch, and response budget for one tool call.
pub const TOOL_CALL_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const CANCELLATION_TIMEOUT: Duration = Duration::from_secs(5);

/// Calls a tool once, preserving transport resumption but bounding the logical request.
pub async fn call_tool_with_deadline(
    operation_id: Uuid,
    installation_id: Uuid,
    params: CallToolRequestParams,
    connect: impl Future<Output = Result<Peer<RoleClient>, ServiceError>>,
) -> Result<CallToolResult, ServiceError> {
    call_tool_with_deadline_inner(
        operation_id,
        installation_id,
        params,
        connect,
        Timer::after(TOOL_CALL_TIMEOUT).map(|_| ()),
        || Timer::after(CANCELLATION_TIMEOUT).map(|_| ()),
    )
    .await
}

async fn call_tool_with_deadline_inner<Connect, Deadline, CancellationDeadline, CancellationTimer>(
    operation_id: Uuid,
    installation_id: Uuid,
    params: CallToolRequestParams,
    connect: Connect,
    deadline: Deadline,
    cancellation_deadline: CancellationDeadline,
) -> Result<CallToolResult, ServiceError>
where
    Connect: Future<Output = Result<Peer<RoleClient>, ServiceError>>,
    Deadline: Future<Output = ()>,
    CancellationDeadline: FnOnce() -> CancellationTimer,
    CancellationTimer: Future<Output = ()>,
{
    let mut lifecycle = ToolCallLifecycle {
        operation_id,
        installation_id,
        started_at: Instant::now(),
        request_id: None,
        phase: "connecting",
        outcome: "pending",
    };
    lifecycle.log("started");
    let mut cancellation = None;

    let completed = {
        let operation = async {
            let peer = connect.await?;
            lifecycle.phase = "dispatching";
            lifecycle.log("peer_ready");
            let request = ClientRequest::CallToolRequest(CallToolRequest::new(params));
            // rmcp's own timeout awaits cancellation delivery before returning. Keep the
            // logical deadline outside that path so a stalled send cannot defeat it.
            let handle = peer
                .send_cancellable_request(request, PeerRequestOptions::no_options())
                .await?;
            lifecycle.request_id = Some(handle.id.clone());
            cancellation = Some((peer, handle.id.clone()));
            lifecycle.phase = "awaiting_response";
            lifecycle.log("request_enqueued");
            match handle.await_response().await? {
                ServerResult::CallToolResult(result) => Ok(result),
                _ => Err(ServiceError::UnexpectedResponse),
            }
        };
        match select(Box::pin(operation), Box::pin(deadline)).await {
            Either::Left((result, _)) => Some(result),
            Either::Right(((), _)) => None,
        }
    };

    if let Some(result) = completed {
        lifecycle.outcome = match &result {
            Ok(result) if result.is_error == Some(true) => "tool_error",
            Ok(_) => "success",
            Err(_) => "service_error",
        };
        return result;
    }

    lifecycle.outcome = "deadline_exceeded";
    lifecycle.log("deadline");
    if let Some((peer, request_id)) = cancellation {
        let cancel = peer.notify_cancelled(CancelledNotificationParam::new(
            Some(request_id),
            Some("Warp MCP tool call deadline exceeded".to_owned()),
        ));
        let cancellation_outcome =
            match select(Box::pin(cancel), Box::pin(cancellation_deadline())).await {
                Either::Left((Ok(()), _)) => "cancellation_sent",
                Either::Left((Err(_), _)) => "cancellation_failed",
                Either::Right(((), _)) => "cancellation_deadline_exceeded",
            };
        lifecycle.log(cancellation_outcome);
    }

    let message = if lifecycle.phase == "connecting" {
        format!(
            "MCP tool call timed out after {} seconds while connecting. The tool was not dispatched.",
            TOOL_CALL_TIMEOUT.as_secs()
        )
    } else {
        format!(
            "MCP tool call deadline exceeded after {} seconds without a result. The tool may have executed; its outcome is unknown. Do not retry automatically.",
            TOOL_CALL_TIMEOUT.as_secs()
        )
    };
    Err(ServiceError::McpError(ErrorData::internal_error(
        message, None,
    )))
}

struct ToolCallLifecycle {
    operation_id: Uuid,
    installation_id: Uuid,
    started_at: Instant,
    request_id: Option<RequestId>,
    phase: &'static str,
    outcome: &'static str,
}

impl ToolCallLifecycle {
    fn log(&self, event: &str) {
        log::info!(
            "MCP tool lifecycle: event={event} operation_id={} installation_id={} request_id={:?} phase={} elapsed_ms={} outcome={}",
            self.operation_id,
            self.installation_id,
            self.request_id,
            self.phase,
            self.started_at.elapsed().as_millis(),
            self.outcome,
        );
    }
}

impl Drop for ToolCallLifecycle {
    fn drop(&mut self) {
        if self.outcome == "pending" {
            self.outcome = "dropped";
        }
        self.log("finished");
    }
}

#[cfg(test)]
#[path = "tool_call_tests.rs"]
mod tests;
