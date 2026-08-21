use chrono::Utc;
use std::sync::Arc;
use std::time::Duration;

use warpui::r#async::FutureExt as _;
use warpui::r#async::executor::Background;

use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::server::retry_strategies::with_bounded_retry;
use crate::server::server_api::ai::{AIClient, RepositoryRevisionSnapshotRequest};

const REPOSITORY_REVISION_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(3);

async fn post_snapshot_with_retry(
    run_id: AmbientAgentTaskId,
    ai_client: Arc<dyn AIClient>,
    request: RepositoryRevisionSnapshotRequest,
) -> anyhow::Result<()> {
    with_bounded_retry("post repository revision snapshot", || {
        let ai_client = ai_client.clone();
        let request = request.clone();
        async move {
            ai_client
                .post_repository_revision_snapshot(&run_id, request)
                .with_timeout(REPOSITORY_REVISION_ATTEMPT_TIMEOUT)
                .await
                .map_err(|_| anyhow::anyhow!("repository revision snapshot request timed out"))?
        }
    })
    .await
}

#[derive(Clone)]
pub(crate) struct RepositoryRevisionReporter {
    run_id: Option<AmbientAgentTaskId>,
    ai_client: Arc<dyn AIClient>,
    background: Arc<Background>,
}

impl RepositoryRevisionReporter {
    pub(crate) fn new(
        run_id: AmbientAgentTaskId,
        ai_client: Arc<dyn AIClient>,
        background: Arc<Background>,
    ) -> Self {
        Self {
            run_id: Some(run_id),
            ai_client,
            background,
        }
    }

    pub(crate) fn noop(ai_client: Arc<dyn AIClient>, background: Arc<Background>) -> Self {
        Self {
            run_id: None,
            ai_client,
            background,
        }
    }

    /// Detaches serialization, network delivery, timeouts, and retries from environment startup.
    pub(crate) fn report(&self, request: RepositoryRevisionSnapshotRequest) {
        let Some(run_id) = self.run_id else {
            return;
        };
        let ai_client = self.ai_client.clone();
        self.background
            .spawn(async move {
                let result = post_snapshot_with_retry(run_id, ai_client, request).await;
                if let Err(error) = result {
                    log::warn!(
                        "Failed to post repository revision snapshot; continuing run startup: {error:#}"
                    );
                }
            })
            .detach();
    }

    pub(crate) fn report_empty(&self) {
        self.report(RepositoryRevisionSnapshotRequest {
            snapshot_uuid: uuid::Uuid::new_v4().to_string(),
            captured_at: Utc::now(),
            unresolved_repository_count: 0,
            repositories: Vec::new(),
        });
    }
}

#[cfg(test)]
#[path = "repository_revisions_tests.rs"]
mod tests;
