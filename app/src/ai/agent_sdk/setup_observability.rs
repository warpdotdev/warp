use std::future::Future;
use std::sync::Arc;

use chrono::{DateTime, Utc};

use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::server::server_api::ai::{AIClient, AgentRunClientEventRequest};

#[derive(Clone)]
pub(crate) struct SetupClientEventReporter {
    run_id: Option<AmbientAgentTaskId>,
    ai_client: Arc<dyn AIClient>,
}

impl SetupClientEventReporter {
    pub(crate) fn new(run_id: Option<AmbientAgentTaskId>, ai_client: Arc<dyn AIClient>) -> Self {
        Self { run_id, ai_client }
    }

    pub(crate) async fn record_result<T, E>(
        &self,
        step: SetupStep,
        future: impl Future<Output = Result<T, E>>,
    ) -> Result<T, E> {
        let start_timestamp = Utc::now();
        let result = future.await;
        let finish_timestamp = Utc::now();
        self.post(step, start_timestamp, finish_timestamp, result.is_err())
            .await;
        result
    }

    pub(crate) async fn record_value<T>(
        &self,
        step: SetupStep,
        future: impl Future<Output = T>,
    ) -> T {
        let start_timestamp = Utc::now();
        let value = future.await;
        let finish_timestamp = Utc::now();
        self.post(step, start_timestamp, finish_timestamp, false)
            .await;
        value
    }

    pub(crate) async fn post_instant(&self, step: SetupStep) {
        let timestamp = Utc::now();
        self.post(step, timestamp, timestamp, false).await;
    }

    async fn post(
        &self,
        step: SetupStep,
        start_timestamp: DateTime<Utc>,
        finish_timestamp: DateTime<Utc>,
        is_error: bool,
    ) {
        let Some(run_id) = self.run_id else {
            return;
        };

        let request = AgentRunClientEventRequest::new(
            step.as_event_type(),
            start_timestamp,
            finish_timestamp,
            is_error,
        );
        if let Err(err) = self
            .ai_client
            .post_agent_run_client_event(&run_id, request)
            .await
        {
            log::warn!(
                "Failed to post best-effort setup client event {} for run {run_id}: {err:#}",
                step.as_event_type()
            );
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum SetupStep {
    WorkerContainerReady,
    TeamMetadataRefresh,
    WarpDriveSync,
    TaskMetadataSecretsAttachmentsGitCredentialsFetch,
    EnvironmentResolution,
    TerminalBootstrap,
    McpServerStartup,
    AgentProfileConfiguration,
    ProfileMcpServerStartup,
    SharedSessionEstablishment,
    GlobalSkillResolution,
    GlobalSkillRepoClone,
    EnvironmentPreparation,
    FileBasedMcpDiscovery,
    FileBasedMcpReadiness,
    EnvironmentSkillLoading,
    GlobalSkillLoading,
    ThirdPartyHarnessExternalConversation,
}

impl SetupStep {
    fn as_event_type(self) -> &'static str {
        match self {
            Self::WorkerContainerReady => "worker_container_ready",
            Self::TeamMetadataRefresh => "setup_team_metadata_refresh",
            Self::WarpDriveSync => "setup_warp_drive_sync",
            Self::TaskMetadataSecretsAttachmentsGitCredentialsFetch => {
                "setup_task_metadata_secrets_attachments_git_credentials_fetch"
            }
            Self::EnvironmentResolution => "setup_environment_resolution",
            Self::TerminalBootstrap => "setup_terminal_bootstrap",
            Self::McpServerStartup => "setup_mcp_server_startup",
            Self::AgentProfileConfiguration => "setup_agent_profile_configuration",
            Self::ProfileMcpServerStartup => "setup_profile_mcp_server_startup",
            Self::SharedSessionEstablishment => "setup_shared_session_establishment",
            Self::GlobalSkillResolution => "setup_global_skill_resolution",
            Self::GlobalSkillRepoClone => "setup_global_skill_repo_clone",
            Self::EnvironmentPreparation => "setup_environment_preparation",
            Self::FileBasedMcpDiscovery => "setup_file_based_mcp_discovery",
            Self::FileBasedMcpReadiness => "setup_file_based_mcp_readiness",
            Self::EnvironmentSkillLoading => "setup_environment_skill_loading",
            Self::GlobalSkillLoading => "setup_global_skill_loading",
            Self::ThirdPartyHarnessExternalConversation => {
                "setup_third_party_harness_external_conversation"
            }
        }
    }
}
