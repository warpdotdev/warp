use std::future::Future;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use enum_iterator::Sequence;
use futures::FutureExt as _;
use tracing::Instrument as _;
use warpui::r#async::executor::Background;

use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::server::server_api::ai::{AIClient, AgentRunClientEventRequest};

/// Records setup spans and optional run events, with an agent-only scope for standby.
#[derive(Clone)]
pub(crate) struct SetupClientEventReporter {
    run_id: Option<AmbientAgentTaskId>,
    /// Keeps execution scope separate from the run ID that controls event reporting.
    scope: SetupScope,
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
            scope: SetupScope::Run,
            ai_client,
            background,
        }
    }

    /// Disables run events without disabling setup tracing spans.
    pub(crate) fn noop(ai_client: Arc<dyn AIClient>, background: Arc<Background>) -> Self {
        Self {
            run_id: None,
            scope: SetupScope::Run,
            ai_client,
            background,
        }
    }

    /// Restricts setup to agent-scoped steps and disables run event reporting.
    #[cfg_attr(all(target_family = "wasm", not(test)), expect(dead_code))]
    pub(crate) fn standby(ai_client: Arc<dyn AIClient>, background: Arc<Background>) -> Self {
        Self {
            scope: SetupScope::Agent,
            ..Self::noop(ai_client, background)
        }
    }

    fn start_step(&self, step: SetupStep) -> (&'static str, tracing::Span) {
        assert!(
            self.scope == SetupScope::Run || step.scope() == SetupScope::Agent,
            "Run-scoped setup step {step:?} cannot run during standby preparation",
        );
        step.to_event_name_and_span()
    }

    /// Records a setup step's duration and error state without changing its result.
    pub(crate) async fn record_result<T, E: std::error::Error>(
        &self,
        step: SetupStep,
        future: impl Future<Output = Result<T, E>>,
    ) -> Result<T, E> {
        let (step_name, span) = self.start_step(step);

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

    /// Records a setup step's duration without interpreting its return value as an error.
    pub(crate) async fn record_value<T>(
        &self,
        step: SetupStep,
        future: impl Future<Output = T>,
    ) -> T {
        let (step_name, span) = self.start_step(step);

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
    /// Records setup work in the background without making startup wait for it.
    pub(crate) fn record_value_detached<T>(
        &self,
        step: SetupStep,
        future: impl Future<Output = T> + Send + 'static,
    ) where
        T: Send + 'static,
    {
        let (step_name, span) = self.start_step(step);

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

    /// Reports a run milestone without making delivery failures fatal.
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

/// Names run milestones that have a timestamp rather than a setup duration.
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

/// Identifies which setup steps standby may execute before a run exists.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SetupScope {
    /// Inputs are limited to team, agent service account, agent config snapshot,
    /// environment, and runner.
    Agent,
    /// Inputs include task ID, execution ID, conversation ID, requester identity, or prompt.
    Run,
}

/// Names setup operations so normal runs and standby preparation use the same tracing labels.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Sequence)]
pub(crate) enum SetupStep {
    TeamMetadataRefresh,
    WarpDriveSync,
    TaskDataFetch,
    /// Applies standby credentials without the task-keyed APIs of `TaskDataFetch`.
    AgentCredentialsFetch,
    EnvironmentResolution,
    SkillRepoClone,
    TerminalBootstrap,
    CloudProviderSetup,
    McpServerStartup,
    AgentProfileConfiguration,
    SharedSessionEstablishment,
    GlobalSkillResolution,
    GlobalSkillRepoClone,
    EnvironmentRepoClone,
    CacheSetup,
    EnvironmentSetupCommands,
    EnvironmentCodebaseIndexing,
    FileBasedMcpDiscovery,
    FileBasedMcpReadiness,
    InitialGlobalMcpScan,
    InitialGlobalMcpReadiness,
    EnvironmentSkillLoading,
    GlobalSkillLoading,
    SkillsDirsLoading,
    ConversationResumeLoading,
    ThirdPartyHarnessPreparation,
    /// Launching the harness through command startup and ACP runner initialization, including
    /// conversation binding and first-turn setup for ACP harnesses.
    ThirdPartyHarnessStart,
    ThirdPartyHarnessExternalConversation,
    /// Sub-steps of [`SetupStep::ThirdPartyHarnessPreparation`] that track plugin
    /// install/update latency and reliability individually.
    ThirdPartyHarnessPreparationNotificationPluginInstall,
    ThirdPartyHarnessPreparationNotificationPluginUpdate,
    ThirdPartyHarnessPreparationPlatformPluginInstall,
    ThirdPartyHarnessPreparationPlatformPluginUpdate,
}

macro_rules! span_and_name {
    ($name:literal) => {
        ($name, tracing::info_span!($name, tags.cloud_agent = true))
    };
}

impl SetupStep {
    /// Declares each step's setup scope with no default for new steps.
    pub(crate) fn scope(self) -> SetupScope {
        match self {
            Self::TeamMetadataRefresh
            | Self::WarpDriveSync
            | Self::AgentCredentialsFetch
            | Self::EnvironmentResolution
            | Self::SkillRepoClone
            | Self::TerminalBootstrap
            | Self::CloudProviderSetup
            | Self::McpServerStartup
            | Self::AgentProfileConfiguration
            | Self::GlobalSkillResolution
            | Self::GlobalSkillRepoClone
            | Self::EnvironmentRepoClone
            | Self::CacheSetup
            | Self::EnvironmentSetupCommands
            | Self::EnvironmentCodebaseIndexing
            | Self::FileBasedMcpDiscovery
            | Self::FileBasedMcpReadiness
            | Self::InitialGlobalMcpScan
            | Self::InitialGlobalMcpReadiness
            | Self::EnvironmentSkillLoading
            | Self::GlobalSkillLoading
            | Self::SkillsDirsLoading => SetupScope::Agent,
            // Task metadata and attachments are run-scoped, and this step reaches secrets and
            // git credentials through task-keyed APIs even though the server resolves both
            // from agent-scoped inputs. `AgentCredentialsFetch` is the pre-warmed equivalent.
            Self::TaskDataFetch
            // Session ACLs name the requester, who is unknown before a claim.
            | Self::SharedSessionEstablishment
            | Self::ConversationResumeLoading
            // Pre-warming is Oz-only, so these are unreachable from a standby.
            | Self::ThirdPartyHarnessPreparation
            | Self::ThirdPartyHarnessStart
            | Self::ThirdPartyHarnessExternalConversation
            | Self::ThirdPartyHarnessPreparationNotificationPluginInstall
            | Self::ThirdPartyHarnessPreparationNotificationPluginUpdate
            | Self::ThirdPartyHarnessPreparationPlatformPluginInstall
            | Self::ThirdPartyHarnessPreparationPlatformPluginUpdate => SetupScope::Run,
        }
    }

    fn to_event_name_and_span(self) -> (&'static str, tracing::Span) {
        log::debug!("Starting {:?} setup step: {self:?}", self.scope());
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
            Self::AgentCredentialsFetch => {
                span_and_name!("setup_agent_secrets_git_credentials_fetch")
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
            Self::CacheSetup => {
                span_and_name!("setup_caches")
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
            Self::InitialGlobalMcpScan => {
                span_and_name!("setup_initial_global_mcp_scan")
            }
            Self::InitialGlobalMcpReadiness => {
                span_and_name!("setup_initial_global_mcp_readiness")
            }
            Self::EnvironmentSkillLoading => {
                span_and_name!("setup_environment_skill_loading")
            }
            Self::GlobalSkillLoading => {
                span_and_name!("setup_global_skill_loading")
            }
            Self::SkillsDirsLoading => {
                span_and_name!("setup_skills_dirs_loading")
            }
            Self::ConversationResumeLoading => {
                span_and_name!("setup_conversation_resume_loading")
            }
            Self::ThirdPartyHarnessPreparation => {
                span_and_name!("setup_third_party_harness_preparation")
            }
            Self::ThirdPartyHarnessStart => {
                span_and_name!("setup_third_party_harness_start")
            }
            Self::ThirdPartyHarnessExternalConversation => {
                span_and_name!("setup_third_party_harness_external_conversation")
            }
            Self::ThirdPartyHarnessPreparationNotificationPluginInstall => {
                span_and_name!("setup_third_party_harness_preparation_notification_plugin_install")
            }
            Self::ThirdPartyHarnessPreparationNotificationPluginUpdate => {
                span_and_name!("setup_third_party_harness_preparation_notification_plugin_update")
            }
            Self::ThirdPartyHarnessPreparationPlatformPluginInstall => {
                span_and_name!("setup_third_party_harness_preparation_platform_plugin_install")
            }
            Self::ThirdPartyHarnessPreparationPlatformPluginUpdate => {
                span_and_name!("setup_third_party_harness_preparation_platform_plugin_update")
            }
        }
    }
}

#[cfg(test)]
#[path = "setup_observability_tests.rs"]
mod tests;
