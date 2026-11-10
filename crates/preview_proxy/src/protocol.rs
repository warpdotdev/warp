use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::time::{Instant, sleep, timeout};
use tokio_util::sync::CancellationToken;

use crate::{PortSource, Proxy};

pub const VERSION: u32 = 1;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Serialize)]
pub struct StartRequest {
    pub protocol_version: u32,
    pub boot_nonce: String,
    pub proxy_port: u16,
    pub token: String,
}

#[derive(Deserialize)]
pub struct Session {
    pub session_id: String,
}

pub enum Error {
    Fenced,
    Transport,
}

pub trait ControlPlane: PortSource + 'static {
    fn start(&self, request: &StartRequest) -> impl Future<Output = Result<Session, Error>> + Send;
}

pub async fn run(
    control: impl ControlPlane,
    boot_nonce: String,
    token: String,
    sandbox_deadline: Option<Instant>,
    cancel: CancellationToken,
) {
    let control = Arc::new(control);
    let Ok(mut proxy) = Proxy::bind(token.clone(), sandbox_deadline, control.clone()).await else {
        return;
    };
    let start = StartRequest {
        protocol_version: VERSION,
        boot_nonce,
        proxy_port: proxy.port(),
        token,
    };
    let lifecycle = async {
        if !register(control.as_ref(), &start).await {
            return;
        }
        std::future::pending::<()>().await;
    };
    tokio::select! {
        _ = cancel.cancelled() => {},
        _ = lifecycle => {},
        _ = async {
            match sandbox_deadline {
                Some(deadline) => tokio::time::sleep_until(deadline).await,
                None => std::future::pending().await,
            }
        } => {},
    }
    proxy.shutdown().await;
}

async fn register(control: &impl ControlPlane, request: &StartRequest) -> bool {
    let mut attempt = 0;
    loop {
        match timeout(REQUEST_TIMEOUT, control.start(request)).await {
            Ok(Ok(_)) => return true,
            Ok(Err(Error::Fenced)) => return false,
            Ok(Err(Error::Transport)) | Err(_) => {}
        }
        let base = (250_u64.saturating_mul(1 << attempt.min(5))).min(4000);
        let jitter = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos() as u64
            % 1000;
        sleep(Duration::from_millis(base + jitter)).await;
        attempt += 1;
    }
}

#[cfg(test)]
#[path = "protocol_tests.rs"]
mod tests;
