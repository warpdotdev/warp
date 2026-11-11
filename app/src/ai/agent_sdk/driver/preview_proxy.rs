use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::SystemTime;

use ::preview_proxy::{DEFAULT_LISTENER_PORT, Proxy};
use anyhow::Context as _;
use tokio_util::sync::CancellationToken;

use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::server::server_api::ai::AIClient;

pub(super) struct Manager {
    cancel: CancellationToken,
    thread: Option<JoinHandle<()>>,
}

fn listener_port(value: &str) -> anyhow::Result<u16> {
    if value.is_empty() {
        return Ok(DEFAULT_LISTENER_PORT);
    }
    anyhow::ensure!(
        value.bytes().all(|byte| byte.is_ascii_digit()),
        "WARP_PREVIEW_PROXY_PORT must be a decimal port from 1 to 65535"
    );
    let port = value
        .parse::<u16>()
        .context("Invalid WARP_PREVIEW_PROXY_PORT")?;
    anyhow::ensure!(port != 0, "WARP_PREVIEW_PROXY_PORT must not be zero");
    Ok(port)
}

impl Manager {
    pub(super) fn start(
        task_id: AmbientAgentTaskId,
        client: Arc<dyn AIClient>,
        sandbox_deadline: Option<SystemTime>,
    ) -> anyhow::Result<Self> {
        let port = match std::env::var("WARP_PREVIEW_PROXY_PORT") {
            Ok(value) => listener_port(&value)?,
            Err(std::env::VarError::NotPresent) => DEFAULT_LISTENER_PORT,
            Err(error) => return Err(error).context("Invalid WARP_PREVIEW_PROXY_PORT"),
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("Failed to create preview proxy runtime")?;
        let cancel = CancellationToken::new();
        let task_cancel = cancel.clone();
        let thread = std::thread::Builder::new()
            .name("preview-proxy".into())
            .spawn(move || {
                runtime.block_on(async move {
                    let deadline = sandbox_deadline.map(|deadline| {
                        tokio::time::Instant::now()
                            + deadline
                                .duration_since(SystemTime::now())
                                .unwrap_or_default()
                    });
                    let source = Arc::new(Client { task_id, client });
                    let mut proxy = match Proxy::bind(port, deadline, source).await {
                        Ok(proxy) => proxy,
                        Err(_) => {
                            log::error!("Preview proxy listener failed to bind");
                            return;
                        }
                    };
                    tokio::select! {
                        _ = task_cancel.cancelled() => {},
                        _ = async {
                            match deadline {
                                Some(deadline) => tokio::time::sleep_until(deadline).await,
                                None => std::future::pending().await,
                            }
                        } => {},
                    }
                    proxy.shutdown().await;
                });
            })
            .context("Failed to start preview proxy thread")?;
        Ok(Self {
            cancel,
            thread: Some(thread),
        })
    }
}

impl ::preview_proxy::PortSource for Client {
    async fn ports(&self) -> anyhow::Result<Vec<u16>> {
        let response = self.client.list_run_previews(&self.task_id).await?;
        anyhow::ensure!(
            response.execution_id.is_some_and(|id| id > 0),
            "preview execution unavailable"
        );
        Ok(response
            .previews
            .into_iter()
            .map(|preview| preview.port)
            .collect())
    }
}

impl Drop for Manager {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Client {
    task_id: AmbientAgentTaskId,
    client: Arc<dyn AIClient>,
}

#[cfg(test)]
#[path = "preview_proxy_tests.rs"]
mod tests;
