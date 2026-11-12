use std::sync::Arc;
use std::thread::JoinHandle;

use ::preview_proxy::Proxy;
use anyhow::Context as _;
use tokio_util::sync::CancellationToken;

use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::server::server_api::ai::AIClient;

pub(super) struct Manager {
    cancel: CancellationToken,
    thread: Option<JoinHandle<()>>,
}

fn listener_port(value: &str) -> anyhow::Result<u16> {
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
        execution_id: Option<&str>,
        client: Arc<dyn AIClient>,
    ) -> anyhow::Result<Self> {
        let execution_id = execution_id
            .context("Preview proxy requires an execution ID")?
            .parse::<i64>()
            .context("Invalid preview proxy execution ID")?;
        anyhow::ensure!(
            execution_id > 0,
            "Preview proxy execution ID must be positive"
        );
        let port = listener_port(
            &std::env::var("WARP_PREVIEW_PROXY_PORT")
                .context("WARP_PREVIEW_PROXY_PORT must be supplied when the proxy is enabled")?,
        )?;
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
                    let source = Arc::new(Client {
                        task_id,
                        execution_id,
                        client,
                    });
                    let mut proxy = match Proxy::bind(port, source).await {
                        Ok(proxy) => proxy,
                        Err(error) => {
                            log::error!(
                                "Preview proxy listener failed to bind on port {port}: {error:#}"
                            );
                            return;
                        }
                    };
                    task_cancel.cancelled().await;
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
            response.execution_id == Some(self.execution_id),
            "Preview execution mismatch: expected {}, received {:?}",
            self.execution_id,
            response.execution_id
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
    execution_id: i64,
    client: Arc<dyn AIClient>,
}

#[cfg(test)]
#[path = "preview_proxy_tests.rs"]
mod tests;
