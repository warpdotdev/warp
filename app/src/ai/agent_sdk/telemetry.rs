use crate::features::FeatureFlag;
use serde_json::{json, Value};
use strum_macros::{EnumDiscriminants, EnumIter};
use warp_core::telemetry::{EnablementState, TelemetryEvent, TelemetryEventDesc};

#[derive(Debug, EnumDiscriminants)]
#[strum_discriminants(derive(EnumIter))]
pub(super) enum CliTelemetryEvent {
    /// Executing `warp agent run`
    AgentRun {
        gui: bool,
        requested_mcp_servers: usize,
        has_environment: bool,
        /// Optional task ID when running against an ambient agent task.
        task_id: Option<String>,
        /// Which execution harness was selected (e.g. "oz", "claude").
        harness: String,
    },
    /// Executing `warp agent run-ambient`
    AgentRunAmbient,
    /// Executing `warp agent profile list`
    AgentProfileList,
    /// Executing `warp agent list`
    AgentList,
    /// Executing `warp agent get`
    AgentGet,
    /// Executing `warp agent create`
    AgentCreate,
    /// Executing `warp agent update`
    AgentUpdate,
    /// Executing `warp agent delete`
    AgentDelete,
    /// Executing `warp agent skills`
    AgentSkills,
    /// Executing `warp environment list`
    EnvironmentList,
    /// Executing `warp environment create`
    EnvironmentCreate,
    /// Executing `warp environment delete`
    EnvironmentDelete,
    /// Executing `warp environment update`
    EnvironmentUpdate,
    /// Executing `warp environment get`
    EnvironmentGet,
    /// Executing `warp environment image list`
    EnvironmentImageList,
    /// Executing `warp mcp list`
    MCPList,
    /// Executing `warp model list`
    ModelList,
    /// Executing `warp task list`
    TaskList,
    /// Executing `warp task get`
    TaskGet,
    /// Executing `warp run conversation get`
    ConversationGet,
    /// Executing `warp run get <id> --conversation`
    RunConversationGet,
    /// Executing `warp run message watch`
    RunMessageWatch { harness: &'static str },
    /// Executing `warp run message send`
    RunMessageSend { harness: &'static str },
    /// Executing `warp run message list`
    RunMessageList { harness: &'static str },
    /// Executing `warp run message read`
    RunMessageRead { harness: &'static str },
    /// Executing `warp run message mark-delivered`
    RunMessageMarkDelivered { harness: &'static str },
    /// Executing `warp login`
    Login,
    /// Executing `warp logout`
    Logout,
    /// Executing `warp whoami`
    Whoami,
    /// Executing `warp provider setup`
    ProviderSetup,
    /// Executing `warp provider list`
    ProviderList,
    /// Executing `warp integration create`
    IntegrationCreate,
    /// Executing `warp integration update`
    IntegrationUpdate,
    /// Executing `warp integration list`
    IntegrationList,
    /// Executing `warp artifact upload`
    ArtifactUpload,
    /// Executing `warp artifact get`
    ArtifactGet,
    /// Executing `warp artifact download`
    ArtifactDownload,
    /// Executing `warp api-key list`
    ApiKeyList,
    /// Executing `warp api-key create`
    ApiKeyCreate,
    /// Executing `warp api-key expire`
    ApiKeyExpire,
    /// Executing `warp schedule create`
    ScheduleCreate,
    /// Executing `warp schedule list`
    ScheduleList,
    /// Executing `warp schedule get`
    ScheduleGet,
    /// Executing `warp schedule pause`
    SchedulePause,
    /// Executing `warp schedule unpause`
    ScheduleUnpause,
    /// Executing `warp schedule update`
    ScheduleUpdate,
    /// Executing `warp schedule delete`
    ScheduleDelete,
    /// Executing `warp secret create`
    SecretCreate,
    /// Executing `warp secret delete`
    SecretDelete,
    /// Executing `warp secret update`
    SecretUpdate,
    /// Executing `warp secret list`
    SecretList,
    /// Executing `warp federate issue-token`
    FederateIssueToken,
    /// Executing `warp federate issue-gcp-token`
    FederateIssueGcpToken,
    /// Executing `warp harness-support ping`
    HarnessSupportPing,
    /// Executing `warp harness-support report-artifact`
    HarnessSupportReportArtifact { artifact_type: &'static str },
    /// Executing `warp harness-support notify-user`
    HarnessSupportNotifyUser,
    /// Executing `warp harness-support finish-task`
    HarnessSupportFinishTask { success: bool },
    /// Executing `warp harness-support report-shutdown`
    HarnessSupportReportShutdown,
}

impl TelemetryEvent for CliTelemetryEvent {
    fn name(&self) -> &'static str {
        CliTelemetryEventDiscriminants::from(self).name()
    }

    fn payload(&self) -> Option<Value> {
        match self {
            CliTelemetryEvent::AgentRun {
                gui,
                requested_mcp_servers,
                has_environment,
                task_id,
                harness,
            } => Some(json!({
                "gui": gui,
                "requested_mcp_servers": requested_mcp_servers,
                "has_environment": has_environment,
                "task_id": task_id,
                "harness": harness,
            })),
            CliTelemetryEvent::AgentRunAmbient => None,
            CliTelemetryEvent::AgentProfileList => None,
            CliTelemetryEvent::AgentList => None,
            CliTelemetryEvent::AgentGet => None,
            CliTelemetryEvent::AgentCreate => None,
            CliTelemetryEvent::AgentUpdate => None,
            CliTelemetryEvent::AgentDelete => None,
            CliTelemetryEvent::AgentSkills => None,
            CliTelemetryEvent::EnvironmentList => None,
            CliTelemetryEvent::EnvironmentCreate => None,
            CliTelemetryEvent::EnvironmentDelete => None,
            CliTelemetryEvent::EnvironmentUpdate => None,
            CliTelemetryEvent::EnvironmentGet => None,
            CliTelemetryEvent::EnvironmentImageList => None,
            CliTelemetryEvent::MCPList => None,
            CliTelemetryEvent::ModelList => None,
            CliTelemetryEvent::TaskList => None,
            CliTelemetryEvent::TaskGet => None,
            CliTelemetryEvent::ConversationGet => None,
            CliTelemetryEvent::RunConversationGet => None,
            CliTelemetryEvent::RunMessageWatch { harness } => Some(json!({ "harness": harness })),
            CliTelemetryEvent::RunMessageSend { harness } => Some(json!({ "harness": harness })),
            CliTelemetryEvent::RunMessageList { harness } => Some(json!({ "harness": harness })),
            CliTelemetryEvent::RunMessageRead { harness } => Some(json!({ "harness": harness })),
            CliTelemetryEvent::RunMessageMarkDelivered { harness } => {
                Some(json!({ "harness": harness }))
            }
            CliTelemetryEvent::Login => None,
            CliTelemetryEvent::Logout => None,
            CliTelemetryEvent::Whoami => None,
            CliTelemetryEvent::ProviderSetup => None,
            CliTelemetryEvent::ProviderList => None,
            CliTelemetryEvent::IntegrationCreate => None,
            CliTelemetryEvent::IntegrationUpdate => None,
            CliTelemetryEvent::IntegrationList => None,
            CliTelemetryEvent::ArtifactUpload => None,
            CliTelemetryEvent::ArtifactGet => None,
            CliTelemetryEvent::ArtifactDownload => None,
            CliTelemetryEvent::ApiKeyList => None,
            CliTelemetryEvent::ApiKeyCreate => None,
            CliTelemetryEvent::ApiKeyExpire => None,
            CliTelemetryEvent::ScheduleCreate => None,
            CliTelemetryEvent::ScheduleList => None,
            CliTelemetryEvent::ScheduleGet => None,
            CliTelemetryEvent::SchedulePause => None,
            CliTelemetryEvent::ScheduleUnpause => None,
            CliTelemetryEvent::ScheduleUpdate => None,
            CliTelemetryEvent::ScheduleDelete => None,
            CliTelemetryEvent::SecretCreate => None,
            CliTelemetryEvent::SecretDelete => None,
            CliTelemetryEvent::SecretUpdate => None,
            CliTelemetryEvent::SecretList => None,
            CliTelemetryEvent::FederateIssueToken => None,
            CliTelemetryEvent::FederateIssueGcpToken => None,
            CliTelemetryEvent::HarnessSupportPing => None,
            CliTelemetryEvent::HarnessSupportReportArtifact { artifact_type } => {
                Some(json!({ "artifact_type": artifact_type }))
            }
            CliTelemetryEvent::HarnessSupportNotifyUser => None,
            CliTelemetryEvent::HarnessSupportFinishTask { success } => {
                Some(json!({ "success": success }))
            }
            CliTelemetryEvent::HarnessSupportReportShutdown => None,
        }
    }

    fn description(&self) -> &'static str {
        CliTelemetryEventDiscriminants::from(self).description()
    }

    fn enablement_state(&self) -> EnablementState {
        CliTelemetryEventDiscriminants::from(self).enablement_state()
    }

    fn contains_ugc(&self) -> bool {
        false
    }

    fn event_descs() -> impl Iterator<Item = Box<dyn TelemetryEventDesc>> {
        warp_core::telemetry::enum_events::<Self>()
    }
}

impl TelemetryEventDesc for CliTelemetryEventDiscriminants {
    fn name(&self) -> &'static str {
        match self {
            CliTelemetryEventDiscriminants::AgentRun => "CLI.Execute.Agent.Run",
            CliTelemetryEventDiscriminants::AgentRunAmbient => "CLI.Execute.Agent.RunAmbient",
            CliTelemetryEventDiscriminants::AgentProfileList => "CLI.Execute.Agent.Profile.List",
            CliTelemetryEventDiscriminants::AgentList => "CLI.Execute.Agent.List",
            CliTelemetryEventDiscriminants::AgentGet => "CLI.Execute.Agent.Get",
            CliTelemetryEventDiscriminants::AgentCreate => "CLI.Execute.Agent.Create",
            CliTelemetryEventDiscriminants::AgentUpdate => "CLI.Execute.Agent.Update",
            CliTelemetryEventDiscriminants::AgentDelete => "CLI.Execute.Agent.Delete",
            CliTelemetryEventDiscriminants::AgentSkills => "CLI.Execute.Agent.Skills",
            CliTelemetryEventDiscriminants::EnvironmentList => "CLI.Execute.Environment.List",
            CliTelemetryEventDiscriminants::EnvironmentCreate => "CLI.Execute.Environment.Create",
            CliTelemetryEventDiscriminants::EnvironmentDelete => "CLI.Execute.Environment.Delete",
            CliTelemetryEventDiscriminants::EnvironmentUpdate => "CLI.Execute.Environment.Update",
            CliTelemetryEventDiscriminants::EnvironmentGet => "CLI.Execute.Environment.Get",
            CliTelemetryEventDiscriminants::EnvironmentImageList => {
                "CLI.Execute.Environment.Image.List"
            }
            CliTelemetryEventDiscriminants::MCPList => "CLI.Execute.MCP.List",
            CliTelemetryEventDiscriminants::ModelList => "CLI.Execute.Model.List",
            CliTelemetryEventDiscriminants::TaskList => "CLI.Execute.Task.List",
            CliTelemetryEventDiscriminants::TaskGet => "CLI.Execute.Task.Get",
            CliTelemetryEventDiscriminants::ConversationGet => "CLI.Execute.Conversation.Get",
            CliTelemetryEventDiscriminants::RunConversationGet => {
                "CLI.Execute.Run.Conversation.Get"
            }
            CliTelemetryEventDiscriminants::RunMessageWatch => "CLI.Execute.Run.Message.Watch",
            CliTelemetryEventDiscriminants::RunMessageSend => "CLI.Execute.Run.Message.Send",
            CliTelemetryEventDiscriminants::RunMessageList => "CLI.Execute.Run.Message.List",
            CliTelemetryEventDiscriminants::RunMessageRead => "CLI.Execute.Run.Message.Read",
            CliTelemetryEventDiscriminants::RunMessageMarkDelivered => {
                "CLI.Execute.Run.Message.MarkDelivered"
            }
            CliTelemetryEventDiscriminants::Login => "CLI.Execute.Login",
            CliTelemetryEventDiscriminants::Logout => "CLI.Execute.Logout",
            CliTelemetryEventDiscriminants::Whoami => "CLI.Execute.Whoami",
            CliTelemetryEventDiscriminants::ProviderSetup => "CLI.Execute.Provider.Setup",
            CliTelemetryEventDiscriminants::ProviderList => "CLI.Execute.Provider.List",
            CliTelemetryEventDiscriminants::IntegrationCreate => "CLI.Execute.Integration.Create",
            CliTelemetryEventDiscriminants::IntegrationUpdate => "CLI.Execute.Integration.Update",
            CliTelemetryEventDiscriminants::IntegrationList => "CLI.Execute.Integration.List",
            CliTelemetryEventDiscriminants::ArtifactUpload => "CLI.Execute.Artifact.Upload",
            CliTelemetryEventDiscriminants::ArtifactGet => "CLI.Execute.Artifact.Get",
            CliTelemetryEventDiscriminants::ArtifactDownload => "CLI.Execute.Artifact.Download",
            CliTelemetryEventDiscriminants::ApiKeyList => "CLI.Execute.ApiKey.List",
            CliTelemetryEventDiscriminants::ApiKeyCreate => "CLI.Execute.ApiKey.Create",
            CliTelemetryEventDiscriminants::ApiKeyExpire => "CLI.Execute.ApiKey.Expire",
            CliTelemetryEventDiscriminants::ScheduleCreate => "CLI.Execute.Schedule.Create",
            CliTelemetryEventDiscriminants::ScheduleList => "CLI.Execute.Schedule.List",
            CliTelemetryEventDiscriminants::ScheduleGet => "CLI.Execute.Schedule.Get",
            CliTelemetryEventDiscriminants::SchedulePause => "CLI.Execute.Schedule.Pause",
            CliTelemetryEventDiscriminants::ScheduleUnpause => "CLI.Execute.Schedule.Unpause",
            CliTelemetryEventDiscriminants::ScheduleUpdate => "CLI.Execute.Schedule.Update",
            CliTelemetryEventDiscriminants::ScheduleDelete => "CLI.Execute.Schedule.Delete",
            CliTelemetryEventDiscriminants::SecretCreate => "CLI.Execute.Secret.Create",
            CliTelemetryEventDiscriminants::SecretDelete => "CLI.Execute.Secret.Delete",
            CliTelemetryEventDiscriminants::SecretUpdate => "CLI.Execute.Secret.Update",
            CliTelemetryEventDiscriminants::SecretList => "CLI.Execute.Secret.List",
            CliTelemetryEventDiscriminants::FederateIssueToken => "CLI.Execute.Federate.IssueToken",
            CliTelemetryEventDiscriminants::FederateIssueGcpToken => {
                "CLI.Execute.Federate.IssueGcpToken"
            }
            CliTelemetryEventDiscriminants::HarnessSupportPing => "CLI.Execute.HarnessSupport.Ping",
            CliTelemetryEventDiscriminants::HarnessSupportReportArtifact => {
                "CLI.Execute.HarnessSupport.ReportArtifact"
            }
            CliTelemetryEventDiscriminants::HarnessSupportNotifyUser => {
                "CLI.Execute.HarnessSupport.NotifyUser"
            }
            CliTelemetryEventDiscriminants::HarnessSupportFinishTask => {
                "CLI.Execute.HarnessSupport.FinishTask"
            }
            CliTelemetryEventDiscriminants::HarnessSupportReportShutdown => {
                "CLI.Execute.HarnessSupport.ReportShutdown"
            }
        }
    }

    fn description(&self) -> &'static str {
        match self {
            CliTelemetryEventDiscriminants::AgentRun => "Ran an agent from the Warp CLI",
            CliTelemetryEventDiscriminants::AgentRunAmbient => {
                "Ran an ambient agent from the Warp CLI"
            }
            CliTelemetryEventDiscriminants::AgentProfileList => {
                "Listed agent profiles from the Warp CLI"
            }
            CliTelemetryEventDiscriminants::AgentList => "Listed agents from the Warp CLI",
            CliTelemetryEventDiscriminants::AgentGet => "Got agent details from the Warp CLI",
            CliTelemetryEventDiscriminants::AgentCreate => "Created an agent from the Warp CLI",
            CliTelemetryEventDiscriminants::AgentUpdate => "Updated an agent from the Warp CLI",
            CliTelemetryEventDiscriminants::AgentDelete => "Deleted an agent from the Warp CLI",
            CliTelemetryEventDiscriminants::AgentSkills => "Listed agent skills from the Warp CLI",
            CliTelemetryEventDiscriminants::EnvironmentList => {
                "Listed cloud environments from the Warp CLI"
            }
            CliTelemetryEventDiscriminants::EnvironmentCreate => {
                "Created a cloud environment from the Warp CLI"
            }
            CliTelemetryEventDiscriminants::EnvironmentDelete => {
                "Deleted a cloud environment from the Warp CLI"
            }
            CliTelemetryEventDiscriminants::EnvironmentUpdate => {
                "Updated a cloud environment from the Warp CLI"
            }
            CliTelemetryEventDiscriminants::EnvironmentGet => {
                "Got cloud environment details from the Warp CLI"
            }
            CliTelemetryEventDiscriminants::EnvironmentImageList => {
                "Listed available base images from the Warp CLI"
            }
            CliTelemetryEventDiscriminants::MCPList => "Listed MCP servers from the Warp CLI",
            CliTelemetryEventDiscriminants::ModelList => "Listed models from the Warp CLI",
            CliTelemetryEventDiscriminants::TaskList => "Listed tasks from the Warp CLI",
            CliTelemetryEventDiscriminants::TaskGet => "Got status of task from the Warp CLI",
            CliTelemetryEventDiscriminants::ConversationGet => {
                "Got conversation by ID from the Warp CLI"
            }
            CliTelemetryEventDiscriminants::RunConversationGet => {
                "Got run conversation from the Warp CLI"
            }
            CliTelemetryEventDiscriminants::RunMessageWatch => {
                "Watched run messages from the Warp CLI"
            }
            CliTelemetryEventDiscriminants::RunMessageSend => {
                "Sent a run message from the Warp CLI"
            }
            CliTelemetryEventDiscriminants::RunMessageList => {
                "Listed run messages from the Warp CLI"
            }
            CliTelemetryEventDiscriminants::RunMessageRead => {
                "Read a run message from the Warp CLI"
            }
            CliTelemetryEventDiscriminants::RunMessageMarkDelivered => {
                "Marked a run message as delivered from the Warp CLI"
            }
            CliTelemetryEventDiscriminants::Login => "Logged in via the Warp CLI",
            CliTelemetryEventDiscriminants::Logout => "Logged out via the Warp CLI",
            CliTelemetryEventDiscriminants::Whoami => "Printed current user info from the Warp CLI",
            CliTelemetryEventDiscriminants::ProviderSetup => "Set up a provider via the Warp CLI",
            CliTelemetryEventDiscriminants::ProviderList => "Listed providers from the Warp CLI",
            CliTelemetryEventDiscriminants::IntegrationCreate => {
                "Created an integration from the Warp CLI"
            }
            CliTelemetryEventDiscriminants::IntegrationUpdate => {
                "Updated an integration from the Warp CLI"
            }
            CliTelemetryEventDiscriminants::IntegrationList => {
                "Listed integrations from the Warp CLI"
            }
            CliTelemetryEventDiscriminants::ArtifactUpload => {
                "Uploaded an artifact from the Warp CLI"
            }
            CliTelemetryEventDiscriminants::ArtifactGet => {
                "Got artifact metadata from the Warp CLI"
            }
            CliTelemetryEventDiscriminants::ArtifactDownload => {
                "Downloaded an artifact from the Warp CLI"
            }
            CliTelemetryEventDiscriminants::ApiKeyList => "Listed API keys from the Warp CLI",
            CliTelemetryEventDiscriminants::ApiKeyCreate => "Created an API key from the Warp CLI",
            CliTelemetryEventDiscriminants::ApiKeyExpire => "Expired an API key from the Warp CLI",
            CliTelemetryEventDiscriminants::ScheduleCreate => {
                "Created a scheduled agent from the Warp CLI"
            }
            CliTelemetryEventDiscriminants::ScheduleList => {
                "Listed scheduled agents from the Warp CLI"
            }
            CliTelemetryEventDiscriminants::ScheduleGet => {
                "Got scheduled agent configuration from the Warp CLI"
            }
            CliTelemetryEventDiscriminants::SchedulePause => {
                "Paused a scheduled agent from the Warp CLI"
            }
            CliTelemetryEventDiscriminants::ScheduleUnpause => {
                "Unpaused a scheduled agent from the Warp CLI"
            }
            CliTelemetryEventDiscriminants::ScheduleUpdate => {
                "Updated a scheduled agent from the Warp CLI"
            }
            CliTelemetryEventDiscriminants::ScheduleDelete => {
                "Deleted a scheduled agent from the Warp CLI"
            }
            CliTelemetryEventDiscriminants::SecretCreate => "Created a secret from the Warp CLI",
            CliTelemetryEventDiscriminants::SecretDelete => "Deleted a secret from the Warp CLI",
            CliTelemetryEventDiscriminants::SecretUpdate => "Updated a secret from the Warp CLI",
            CliTelemetryEventDiscriminants::SecretList => "Listed secrets from the Warp CLI",
            CliTelemetryEventDiscriminants::FederateIssueToken => {
                "Issued a federated identity token from the Warp CLI"
            }
            CliTelemetryEventDiscriminants::FederateIssueGcpToken => {
                "Issued a GCP federated identity token from the Warp CLI"
            }
            CliTelemetryEventDiscriminants::HarnessSupportPing => {
                "Pinged harness-support from the Warp CLI"
            }
            CliTelemetryEventDiscriminants::HarnessSupportReportArtifact => {
                "Reported an artifact via harness-support from the Warp CLI"
            }
            CliTelemetryEventDiscriminants::HarnessSupportNotifyUser => {
                "Sent a user notification via harness-support from the Warp CLI"
            }
            CliTelemetryEventDiscriminants::HarnessSupportFinishTask => {
                "Reported task completion via harness-support from the Warp CLI"
            }
            CliTelemetryEventDiscriminants::HarnessSupportReportShutdown => {
                "Reported agent shutdown via harness-support from the Warp CLI"
            }
        }
    }

    fn enablement_state(&self) -> EnablementState {
        match self {
            Self::FederateIssueToken | Self::FederateIssueGcpToken => {
                EnablementState::Flag(FeatureFlag::OzIdentityFederation)
            }
            Self::HarnessSupportPing
            | Self::HarnessSupportReportArtifact
            | Self::HarnessSupportNotifyUser
            | Self::HarnessSupportFinishTask => EnablementState::Flag(FeatureFlag::AgentHarness),
            Self::ArtifactUpload | Self::ArtifactGet | Self::ArtifactDownload => {
                EnablementState::Flag(FeatureFlag::ArtifactCommand)
            }
            Self::ApiKeyList | Self::ApiKeyCreate | Self::ApiKeyExpire => {
                EnablementState::Flag(FeatureFlag::APIKeyManagement)
            }
            Self::RunMessageWatch
            | Self::RunMessageSend
            | Self::RunMessageList
            | Self::RunMessageRead
            | Self::RunMessageMarkDelivered => EnablementState::Flag(FeatureFlag::OrchestrationV2),
            _ => EnablementState::Always,
        }
    }
}

warp_core::register_telemetry_event!(CliTelemetryEvent);
