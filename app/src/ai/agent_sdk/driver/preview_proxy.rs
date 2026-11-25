use std::collections::HashSet;
use std::sync::Arc;
use std::thread::JoinHandle;

#[cfg(not(target_family = "wasm"))]
use ::preview_proxy::IngressTokenValidation;
use ::preview_proxy::{PortSource, PreviewRecord, Proxy};
use anyhow::Context as _;
use tokio_util::sync::CancellationToken;
use url::Url;
use uuid::Uuid;
use warp_isolation_platform::IsolationPlatformType;

use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::server::server_api::ai::{AIClient, ListRunPreviewsResponse};

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

fn validate_platform(platform: Option<IsolationPlatformType>) -> anyhow::Result<()> {
    match platform {
        #[cfg(not(target_family = "wasm"))]
        Some(IsolationPlatformType::Namespace | IsolationPlatformType::DockerSandbox) => Ok(()),
        #[cfg(not(target_family = "wasm"))]
        Some(IsolationPlatformType::Docker | IsolationPlatformType::Kubernetes) => {
            anyhow::bail!("Preview proxy requires a supported hosted sandbox")
        }
        None => anyhow::bail!("Preview proxy requires a supported hosted sandbox"),
    }
}

impl Manager {
    pub(super) fn start(
        task_id: AmbientAgentTaskId,
        execution_id: Option<&str>,
        client: Arc<dyn AIClient>,
    ) -> anyhow::Result<Self> {
        validate_platform(warp_isolation_platform::detect())?;
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
                    let mut proxy = match Proxy::bind(port, Arc::clone(&source)).await {
                        Ok(proxy) => proxy,
                        Err(error) => {
                            log::error!(
                                "Preview proxy listener failed to bind on port {port}: {error:#}"
                            );
                            return;
                        }
                    };
                    loop {
                        tokio::select! {
                            biased;
                            _ = task_cancel.cancelled() => break,
                            result = source.previews() => {
                                if result.is_ok() {
                                    task_cancel.cancelled().await;
                                    break;
                                }
                            }
                        }
                        tokio::select! {
                            _ = task_cancel.cancelled() => break,
                            _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => {}
                        }
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

impl PortSource for Client {
    #[cfg(not(target_family = "wasm"))]
    async fn validate_ingress_token(
        &self,
        request: IngressTokenValidation<'_>,
    ) -> anyhow::Result<()> {
        self.client
            .validate_preview_ingress_token(
                &self.task_id,
                self.execution_id,
                request.preview_id,
                request.port,
                request.ingress_token,
            )
            .await
    }

    async fn previews(&self) -> anyhow::Result<Vec<PreviewRecord>> {
        let response = self.client.list_run_previews(&self.task_id, true).await?;
        preview_records(response, self.execution_id)
    }
}

fn preview_records(
    response: ListRunPreviewsResponse,
    expected_execution_id: i64,
) -> anyhow::Result<Vec<PreviewRecord>> {
    anyhow::ensure!(
        response.execution_id == Some(expected_execution_id),
        "Preview execution mismatch: expected {}, received {:?}",
        expected_execution_id,
        response.execution_id
    );

    let mut ids = HashSet::new();
    let mut ports = HashSet::new();
    let mut origins = HashSet::new();
    response
        .previews
        .into_iter()
        .map(|preview| {
            anyhow::ensure!(
                preview.port >= 1024,
                "Invalid preview app port {}",
                preview.port
            );
            anyhow::ensure!(ports.insert(preview.port), "Duplicate preview app port");
            let id_string = preview.id.context("Preview record missing UUID")?;
            let public_url = preview.url.context("Preview record missing public URL")?;
            let id = Uuid::parse_str(&id_string).context("Invalid preview UUID")?;
            anyhow::ensure!(
                !id.is_nil() && id.to_string() == id_string,
                "Preview UUID must be non-nil and use canonical lowercase formatting"
            );
            anyhow::ensure!(ids.insert(id), "Duplicate preview UUID");
            let origin = preview_origin(&public_url)?;
            anyhow::ensure!(origins.insert(origin.clone()), "Duplicate preview origin");

            Ok(PreviewRecord {
                id,
                port: preview.port,
                origin,
            })
        })
        .collect()
}

fn preview_origin(public_url: &str) -> anyhow::Result<String> {
    let url = Url::parse(public_url).context("Invalid preview public URL")?;
    anyhow::ensure!(
        matches!(url.scheme(), "http" | "https"),
        "Preview URL must use HTTP or HTTPS"
    );
    anyhow::ensure!(
        url.username().is_empty() && url.password().is_none(),
        "Preview URL must not contain credentials"
    );
    Ok(url.origin().ascii_serialization())
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
