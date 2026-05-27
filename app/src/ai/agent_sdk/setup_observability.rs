use std::future::Future;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use warpui::r#async::executor::Background;

use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::server::server_api::ai::{AIClient, AgentRunClientEventRequest};

#[derive(Clone)]
pub(crate) struct SetupClientEventReporter {
    run_id: Option<AmbientAgentTaskId>,
    ai_client: Arc<dyn AIClient>,
    background: Arc<Background>,
}

impl SetupClientEventReporter {
    /// Constructs a reporter for setup events associated with an existing Oz run.
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

    /// Constructs a reporter for setup paths that are intentionally not backed by an Oz run.
    pub(crate) fn noop(ai_client: Arc<dyn AIClient>, background: Arc<Background>) -> Self {
        Self {
            run_id: None,
            ai_client,
            background,
        }
    }

    pub(crate) async fn record_result<T, E>(
        &self,
        step: SetupStep,
        future: impl Future<Output = Result<T, E>>,
    ) -> Result<T, E> {
        let start_timestamp = Utc::now();
        let result = future.await;
        let finish_timestamp = Utc::now();
        self.post_setup_metric_event_best_effort(
            step,
            start_timestamp,
            finish_timestamp,
            result.is_err(),
        );
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
        self.post_setup_metric_event_best_effort(step, start_timestamp, finish_timestamp, false);
        value
    }
    pub(crate) fn record_value_detached<T>(
        &self,
        step: SetupStep,
        future: impl Future<Output = T> + Send + 'static,
    ) where
        T: Send + 'static,
    {
        let reporter = self.clone();
        self.background
            .spawn(async move {
                let start_timestamp = Utc::now();
                future.await;
                let finish_timestamp = Utc::now();
                reporter.post_setup_metric_event_best_effort(
                    step,
                    start_timestamp,
                    finish_timestamp,
                    false,
                );
            })
            .detach();
    }

    pub(crate) async fn post_timeline_event(&self, event: SetupTimelineEvent) {
        let Some(run_id) = self.run_id else {
            return;
        };
        let timestamp = Utc::now();
        let event_name = event.as_event_name();
        let request = AgentRunClientEventRequest::timeline_event(event_name, timestamp);
        Self::post_client_event(run_id, self.ai_client.clone(), event_name, request).await;
    }

    fn post_setup_metric_event_best_effort(
        &self,
        step: SetupStep,
        start_timestamp: DateTime<Utc>,
        finish_timestamp: DateTime<Utc>,
        is_error: bool,
    ) {
        let Some(run_id) = self.run_id else {
            return;
        };

        let ai_client = self.ai_client.clone();
        self.background
            .spawn(async move {
                let event_name = step.as_event_name();
                let request = AgentRunClientEventRequest::setup_metric_event(
                    event_name,
                    start_timestamp,
                    finish_timestamp,
                    is_error,
                );
                Self::post_client_event(run_id, ai_client, event_name, request).await;
            })
            .detach();
    }

    async fn post_client_event(
        run_id: AmbientAgentTaskId,
        ai_client: Arc<dyn AIClient>,
        event_name: &'static str,
        request: AgentRunClientEventRequest,
    ) {
        if let Err(err) = ai_client
            .post_agent_run_client_event(&run_id, request)
            .await
        {
            log::warn!("Failed to post setup client event {event_name} for run {run_id}: {err:#}");
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum SetupTimelineEvent {
    WorkerContainerReady,
}

impl SetupTimelineEvent {
    fn as_event_name(self) -> &'static str {
        match self {
            Self::WorkerContainerReady => "worker_container_ready",
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum SetupStep {
    TeamMetadataRefresh,
    WarpDriveSync,
    TaskDataFetch,
    EnvironmentResolution,
    SkillRepoClone,
    TerminalBootstrap,
    CloudProviderSetup,
    McpServerStartup,
    AgentProfileConfiguration,
    ProfileMcpServerStartup,
    SharedSessionEstablishment,
    GlobalSkillResolution,
    GlobalSkillRepoClone,
    EnvironmentRepoClone,
    EnvironmentSetupCommands,
    EnvironmentCodebaseIndexing,
    FileBasedMcpDiscovery,
    FileBasedMcpReadiness,
    EnvironmentSkillLoading,
    GlobalSkillLoading,
    ConversationResumeLoading,
    ThirdPartyHarnessPreparation,
    ThirdPartyHarnessExternalConversation,
}

impl SetupStep {
    fn as_event_name(self) -> &'static str {
        match self {
            Self::TeamMetadataRefresh => "setup_team_metadata_refresh",
            Self::WarpDriveSync => "setup_warp_drive_sync",
            Self::TaskDataFetch => "setup_task_metadata_secrets_attachments_git_credentials_fetch",
            Self::EnvironmentResolution => "setup_environment_resolution",
            Self::SkillRepoClone => "setup_skill_repo_clone",
            Self::TerminalBootstrap => "setup_terminal_bootstrap",
            Self::CloudProviderSetup => "setup_cloud_provider_setup",
            Self::McpServerStartup => "setup_mcp_server_startup",
            Self::AgentProfileConfiguration => "setup_agent_profile_configuration",
            Self::ProfileMcpServerStartup => "setup_profile_mcp_server_startup",
            Self::SharedSessionEstablishment => "setup_shared_session_establishment",
            Self::GlobalSkillResolution => "setup_global_skill_resolution",
            Self::GlobalSkillRepoClone => "setup_global_skill_repo_clone",
            Self::EnvironmentRepoClone => "setup_environment_repo_clone",
            Self::EnvironmentSetupCommands => "setup_environment_setup_commands",
            Self::EnvironmentCodebaseIndexing => "setup_environment_codebase_indexing",
            Self::FileBasedMcpDiscovery => "setup_file_based_mcp_discovery",
            Self::FileBasedMcpReadiness => "setup_file_based_mcp_readiness",
            Self::EnvironmentSkillLoading => "setup_environment_skill_loading",
            Self::GlobalSkillLoading => "setup_global_skill_loading",
            Self::ConversationResumeLoading => "setup_conversation_resume_loading",
            Self::ThirdPartyHarnessPreparation => "setup_third_party_harness_preparation",
            Self::ThirdPartyHarnessExternalConversation => {
                "setup_third_party_harness_external_conversation"
            }
        }
    }
}
