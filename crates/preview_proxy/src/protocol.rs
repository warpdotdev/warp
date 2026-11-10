use std::future::Future;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::time::{Instant, sleep, timeout};
use tokio_util::sync::CancellationToken;

use crate::{Proxy, Route};

pub const VERSION: u32 = 1;
const MAX_LEASE: Duration = Duration::from_secs(30);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Serialize)]
pub struct StartRequest {
    pub protocol_version: u32,
    pub boot_nonce: String,
}

#[derive(Deserialize)]
pub struct Session {
    pub session_id: String,
}

#[derive(Deserialize)]
pub struct Snapshot {
    pub session_id: String,
    pub revision: i64,
    pub lease_sequence: i64,
    pub lease_seconds: u64,
    pub routes: Vec<DesiredRoute>,
}

#[derive(Deserialize)]
pub struct DesiredRoute {
    pub id: String,
    pub generation: i64,
    pub app_port: u16,
    pub token: String,
    pub valid_for_seconds: u64,
}

#[derive(Serialize)]
pub struct Acknowledgement {
    pub revision: i64,
    pub lease_sequence: i64,
    pub routes: Vec<RouteAcknowledgement>,
}

#[derive(Serialize)]
pub struct RouteAcknowledgement {
    pub id: String,
    pub generation: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proxy_port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<&'static str>,
}

pub enum Error {
    Fenced,
    Transport,
}

pub trait ControlPlane: Send + Sync {
    fn start(&self, request: &StartRequest) -> impl Future<Output = Result<Session, Error>> + Send;
    fn poll(
        &self,
        session: &str,
        revision: i64,
    ) -> impl Future<Output = Result<Snapshot, Error>> + Send;
    fn acknowledge(
        &self,
        session: &str,
        ack: &Acknowledgement,
    ) -> impl Future<Output = Result<(), Error>> + Send;
}

struct Reconciler {
    proxy: Proxy,
    session: String,
    revision: i64,
    lease_sequence: i64,
    sandbox_deadline: Option<Instant>,
}

impl Reconciler {
    async fn apply(
        &mut self,
        snapshot: Snapshot,
        request_started: Instant,
    ) -> Option<Acknowledgement> {
        let lease = Duration::from_secs(snapshot.lease_seconds);
        let mut deadline = request_started.checked_add(lease)?;
        if let Some(sandbox_deadline) = self.sandbox_deadline {
            deadline = deadline.min(sandbox_deadline);
        }
        if snapshot.session_id != self.session
            || snapshot.revision < self.revision
            || snapshot.lease_sequence <= self.lease_sequence
            || lease.is_zero()
            || lease > MAX_LEASE
            || deadline <= Instant::now()
        {
            return None;
        }
        let mut ids = std::collections::HashSet::new();
        if snapshot
            .routes
            .iter()
            .any(|route| !ids.insert(route.id.clone()))
        {
            return None;
        }
        let deadlines = snapshot
            .routes
            .iter()
            .map(|route| {
                request_started
                    .checked_add(Duration::from_secs(route.valid_for_seconds))
                    .map(|route_deadline| route_deadline.min(deadline))
            })
            .collect::<Option<Vec<_>>>()?;
        self.proxy.retain(&ids.into_iter().collect::<Vec<_>>());
        let mut routes = Vec::with_capacity(snapshot.routes.len());
        for (desired, route_deadline) in snapshot.routes.into_iter().zip(deadlines) {
            let route = Route {
                id: desired.id.clone(),
                generation: desired.generation,
                app_port: desired.app_port,
                token: desired.token,
            };
            let port = self.proxy.register(route, route_deadline).await.ok();
            routes.push(RouteAcknowledgement {
                id: desired.id,
                generation: desired.generation,
                proxy_port: port,
                failure: port.is_none().then_some("listener_unavailable"),
            });
        }
        self.revision = snapshot.revision;
        self.lease_sequence = snapshot.lease_sequence;
        Some(Acknowledgement {
            revision: self.revision,
            lease_sequence: self.lease_sequence,
            routes,
        })
    }
}

pub async fn run(
    control: impl ControlPlane,
    boot_nonce: String,
    sandbox_deadline: Option<Instant>,
    cancel: CancellationToken,
) {
    let start = StartRequest {
        protocol_version: VERSION,
        boot_nonce,
    };
    let mut attempt = 0;
    let session = loop {
        let result = tokio::select! {
            _ = cancel.cancelled() => return,
            result = timeout(REQUEST_TIMEOUT, control.start(&start)) => result,
        };
        match result {
            Ok(Ok(session)) => break session,
            Ok(Err(Error::Fenced)) => return,
            Ok(Err(Error::Transport)) | Err(_) => {}
        }
        if wait_retry(attempt, &cancel).await {
            return;
        }
        attempt += 1;
    };
    let mut reconciler = Reconciler {
        proxy: Proxy::default(),
        session: session.session_id,
        revision: -1,
        lease_sequence: -1,
        sandbox_deadline,
    };
    attempt = 0;
    loop {
        let request_started = Instant::now();
        let result = tokio::select! {
            _ = cancel.cancelled() => break,
            _ = async {
                match sandbox_deadline {
                    Some(deadline) => tokio::time::sleep_until(deadline).await,
                    None => std::future::pending().await,
                }
            } => break,
            result = timeout(REQUEST_TIMEOUT, control.poll(&reconciler.session, reconciler.revision)) => result,
        };
        let snapshot = match result {
            Ok(Ok(snapshot)) => snapshot,
            Ok(Err(Error::Fenced)) => break,
            Ok(Err(Error::Transport)) | Err(_) => {
                if wait_retry(attempt, &cancel).await {
                    break;
                }
                attempt += 1;
                continue;
            }
        };
        let Some(ack) = reconciler.apply(snapshot, request_started).await else {
            if wait_retry(attempt, &cancel).await {
                break;
            }
            attempt += 1;
            continue;
        };
        let result = tokio::select! {
            _ = cancel.cancelled() => break,
            result = timeout(REQUEST_TIMEOUT, control.acknowledge(&reconciler.session, &ack)) => result,
        };
        match result {
            Ok(Ok(())) => attempt = 0,
            Ok(Err(Error::Fenced)) => break,
            Ok(Err(Error::Transport)) | Err(_) => {
                if wait_retry(attempt, &cancel).await {
                    break;
                }
                attempt += 1;
            }
        }
    }
    reconciler.proxy.shutdown().await;
}

async fn wait_retry(attempt: u32, cancel: &CancellationToken) -> bool {
    let base = (250_u64.saturating_mul(1 << attempt.min(5))).min(4000);
    let jitter = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as u64
        % 1000;
    tokio::select! {
        _ = cancel.cancelled() => true,
        _ = sleep(Duration::from_millis(base + jitter)) => false,
    }
}

#[cfg(test)]
#[path = "protocol_tests.rs"]
mod tests;
