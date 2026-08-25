use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use cloud_object_models::CodeForge;
use warpui::r#async::executor::Background;
use warpui::r#async::{FutureExt as _, Timer};

use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::server::retry_strategies::{backoff_after_attempts, is_transient_http_error};
use crate::server::server_api::ai::{
    AIClient, AgentRunEnvironmentSnapshotRequest, AgentRunRepositoryRevision,
};

const REPORT_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(3);
const REPORT_MAX_ATTEMPTS: usize = 3;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RepositoryRevision {
    pub code_forge: CodeForge,
    pub repo_owner: String,
    pub repo_name: String,
    pub checkout_path: String,
    pub requested_checkout_ref: Option<String>,
    pub resolved_head_sha: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EnvironmentSnapshot {
    pub captured_at: DateTime<Utc>,
    pub repositories: Vec<RepositoryRevision>,
}

impl EnvironmentSnapshot {
    pub(crate) fn empty() -> Self {
        Self {
            captured_at: Utc::now(),
            repositories: Vec::new(),
        }
    }
}

impl From<EnvironmentSnapshot> for AgentRunEnvironmentSnapshotRequest {
    fn from(snapshot: EnvironmentSnapshot) -> Self {
        Self {
            captured_at: snapshot.captured_at,
            repositories: snapshot
                .repositories
                .into_iter()
                .map(|revision| AgentRunRepositoryRevision {
                    code_forge: revision.code_forge,
                    repo_owner: revision.repo_owner,
                    repo_name: revision.repo_name,
                    checkout_path: revision.checkout_path,
                    requested_checkout_ref: revision.requested_checkout_ref,
                    resolved_head_sha: revision.resolved_head_sha,
                })
                .collect(),
        }
    }
}

#[derive(Clone)]
pub(crate) struct EnvironmentSnapshotReporter {
    run_id: Option<AmbientAgentTaskId>,
    ai_client: Arc<dyn AIClient>,
    background: Arc<Background>,
}

impl EnvironmentSnapshotReporter {
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

    pub(crate) fn report(&self, snapshot: EnvironmentSnapshot) {
        let Some(run_id) = self.run_id else {
            return;
        };
        let request = AgentRunEnvironmentSnapshotRequest::from(snapshot);
        let ai_client = self.ai_client.clone();
        self.background
            .spawn(async move {
                if let Err(error) = publish_with_retry(run_id, ai_client, request).await {
                    log::warn!("Failed to report environment snapshot for run {run_id}: {error:#}");
                }
            })
            .detach();
    }
}

async fn publish_with_retry(
    run_id: AmbientAgentTaskId,
    ai_client: Arc<dyn AIClient>,
    request: AgentRunEnvironmentSnapshotRequest,
) -> anyhow::Result<()> {
    for attempt in 1..=REPORT_MAX_ATTEMPTS {
        let result = ai_client
            .post_agent_run_environment_snapshot(&run_id, request.clone())
            .with_timeout(REPORT_ATTEMPT_TIMEOUT)
            .await;
        let error = match result {
            Ok(Ok(())) => return Ok(()),
            Ok(Err(error)) => error,
            Err(_) => anyhow::anyhow!(
                "environment snapshot report attempt timed out after {:?}",
                REPORT_ATTEMPT_TIMEOUT
            ),
        };

        if attempt == REPORT_MAX_ATTEMPTS || !is_transient_http_error(&error) {
            return Err(error);
        }

        log::warn!(
            "Environment snapshot report attempt {attempt}/{REPORT_MAX_ATTEMPTS} failed for run {run_id}, retrying: {error:#}"
        );
        Timer::after(backoff_after_attempts(attempt)).await;
    }

    unreachable!("environment snapshot reporter always attempts at least once")
}

#[cfg(test)]
#[path = "environment_snapshot_tests.rs"]
mod tests;
