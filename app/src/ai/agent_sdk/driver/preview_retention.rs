use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use warp_core::features::FeatureFlag;
use warp_isolation_platform::IsolationPlatformType;
use warpui::r#async::executor::Background;
use warpui::r#async::{BoxFuture, FutureExt as _, Timer};

use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::server::server_api::ai::{AIClient, ListRunPreviewsResponse};

const PREVIEW_CHECK_TIMEOUT: Duration = Duration::from_secs(5);
const PREVIEW_RECHECK_INTERVAL: Duration = Duration::from_secs(60);
pub(super) fn is_namespace_sandbox(platform: Option<IsolationPlatformType>) -> bool {
    match platform {
        #[cfg(not(target_family = "wasm"))]
        Some(IsolationPlatformType::Namespace) => true,
        #[cfg(not(target_family = "wasm"))]
        Some(
            IsolationPlatformType::Docker
            | IsolationPlatformType::DockerSandbox
            | IsolationPlatformType::Kubernetes,
        ) => false,
        None => false,
    }
}

#[derive(Clone)]
pub(super) struct PreviewRetention {
    pub background: Arc<Background>,
    pub check: Arc<dyn Fn() -> BoxFuture<'static, Option<Duration>> + Send + Sync>,
    pub wait: Arc<dyn Fn(Duration) -> BoxFuture<'static, ()> + Send + Sync>,
}
async fn check_previews(
    request: impl Future<Output = anyhow::Result<ListRunPreviewsResponse>>,
    expected_execution_id: Option<i64>,
    timeout: Duration,
) -> Option<Duration> {
    match request.with_timeout(timeout).await {
        Ok(Ok(response)) => retention_delay(&response, expected_execution_id),
        Ok(Err(error)) => {
            log::warn!("Preview idle retention check failed; allowing idle exit: {error:#}");
            None
        }
        Err(_) => {
            log::warn!("Preview idle retention check timed out; allowing idle exit");
            None
        }
    }
}

impl PreviewRetention {
    pub fn new(
        task_id: AmbientAgentTaskId,
        execution_id: Option<&str>,
        client: Arc<dyn AIClient>,
        background: Arc<Background>,
    ) -> Option<Self> {
        if !FeatureFlag::PreviewUrlsMcp.is_enabled() {
            return None;
        }
        let expected_execution_id = match execution_id {
            Some(raw) => Some(raw.parse::<i64>().ok().filter(|id| *id > 0)?),
            None => None,
        };
        Some(Self {
            background,
            check: Arc::new(move || {
                let client = client.clone();
                Box::pin(async move {
                    check_previews(
                        client.list_run_previews(&task_id),
                        expected_execution_id,
                        PREVIEW_CHECK_TIMEOUT,
                    )
                    .await
                })
            }),
            wait: Arc::new(|delay| {
                Box::pin(async move {
                    Timer::after(delay).await;
                })
            }),
        })
    }
}

fn retention_delay(
    response: &ListRunPreviewsResponse,
    expected_execution_id: Option<i64>,
) -> Option<Duration> {
    let execution_id = response.execution_id.filter(|id| *id > 0)?;
    if expected_execution_id.is_some_and(|expected| expected != execution_id) {
        return None;
    }
    (!response.previews.is_empty()).then_some(PREVIEW_RECHECK_INTERVAL)
}

#[cfg(test)]
#[path = "preview_retention_tests.rs"]
mod tests;
