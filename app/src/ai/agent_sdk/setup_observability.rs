use std::future::Future;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use futures::FutureExt as _;
use tracing::Instrument as _;
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

    pub(crate) async fn record_result<T, E: std::error::Error>(
        &self,
        step: SetupStep,
        future: impl Future<Output = Result<T, E>>,
    ) -> Result<T, E> {
        let (step_name, span) = step.to_event_name_and_span();

        let start_timestamp = Utc::now();
        let result = future
            .map(|result| {
                result.inspect_err(|err| {
                    tracing::error!(error = %err);
                })
            })
            .instrument(span)
            .await;
        let finish_timestamp = Utc::now();

        self.post_setup_metric_event_best_effort(
            step_name,
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
        let (step_name, span) = step.to_event_name_and_span();

        let start_timestamp = Utc::now();
        let value = future.instrument(span).await;
        let finish_timestamp = Utc::now();

        self.post_setup_metric_event_best_effort(
            step_name,
            start_timestamp,
            finish_timestamp,
            false,
        );
        value
    }
    pub(crate) fn record_value_detached<T>(
        &self,
        step: SetupStep,
        future: impl Future<Output = T> + Send + 'static,
    ) where
        T: Send + 'static,
    {
        let (step_name, span) = step.to_event_name_and_span();

        let reporter = self.clone();
        self.background
            .spawn(async move {
                let start_timestamp = Utc::now();
                future.instrument(span).await;
                let finish_timestamp = Utc::now();
                reporter.post_setup_metric_event_best_effort(
                    step_name,
                    start_timestamp,
                    finish_timestamp,
                    false,
                );
            })
            .detach();
    }

    pub(crate) async fn post_timeline_event(&self, event: OzRunTimelineEvent) {
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
        event_name: &'static str,
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
        tracing::info!(event_name, tags.cloud_agent = true);

        if let Err(err) = ai_client
            .post_agent_run_client_event(&run_id, request)
            .await
        {
            log::warn!("Failed to post setup client event {event_name} for run {run_id}: {err:#}");
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum OzRunTimelineEvent {
    AgentStarted,
    WorkerContainerReady,
}

impl OzRunTimelineEvent {
    fn as_event_name(self) -> &'static str {
        match self {
            Self::AgentStarted => "agent_started",
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

macro_rules! span_and_name {
    ($name:literal) => {
        ($name, tracing::info_span!($name, tags.cloud_agent = true))
    };
}

impl SetupStep {
    fn to_event_name_and_span(self) -> (&'static str, tracing::Span) {
        match self {
            Self::TeamMetadataRefresh => {
                span_and_name!("setup_team_metadata_refresh")
            }
            Self::WarpDriveSync => {
                span_and_name!("setup_warp_drive_sync")
            }
            Self::TaskDataFetch => {
                span_and_name!("setup_task_metadata_secrets_attachments_git_credentials_fetch")
            }
            Self::EnvironmentResolution => {
                span_and_name!("setup_environment_resolution")
            }
            Self::SkillRepoClone => {
                span_and_name!("setup_skill_repo_clone")
            }
            Self::TerminalBootstrap => {
                span_and_name!("setup_terminal_bootstrap")
            }
            Self::CloudProviderSetup => {
                span_and_name!("setup_cloud_provider_setup")
            }
            Self::McpServerStartup => {
                span_and_name!("setup_mcp_server_startup")
            }
            Self::AgentProfileConfiguration => {
                span_and_name!("setup_agent_profile_configuration")
            }
            Self::ProfileMcpServerStartup => {
                span_and_name!("setup_profile_mcp_server_startup")
            }
            Self::SharedSessionEstablishment => {
                span_and_name!("setup_shared_session_establishment")
            }
            Self::GlobalSkillResolution => {
                span_and_name!("setup_global_skill_resolution")
            }
            Self::GlobalSkillRepoClone => {
                span_and_name!("setup_global_skill_repo_clone")
            }
            Self::EnvironmentRepoClone => {
                span_and_name!("setup_environment_repo_clone")
            }
            Self::EnvironmentSetupCommands => {
                span_and_name!("setup_environment_setup_commands")
            }
            Self::EnvironmentCodebaseIndexing => {
                span_and_name!("setup_environment_codebase_indexing")
            }
            Self::FileBasedMcpDiscovery => {
                span_and_name!("setup_file_based_mcp_discovery")
            }
            Self::FileBasedMcpReadiness => {
                span_and_name!("setup_file_based_mcp_readiness")
            }
            Self::EnvironmentSkillLoading => {
                span_and_name!("setup_environment_skill_loading")
            }
            Self::GlobalSkillLoading => {
                span_and_name!("setup_global_skill_loading")
            }
            Self::ConversationResumeLoading => {
                span_and_name!("setup_conversation_resume_loading")
            }
            Self::ThirdPartyHarnessPreparation => {
                span_and_name!("setup_third_party_harness_preparation")
            }
            Self::ThirdPartyHarnessExternalConversation => {
                span_and_name!("setup_third_party_harness_external_conversation")
            }
        }
    }
}
