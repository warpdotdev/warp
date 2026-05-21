use super::workspace::{
    AiAutonomySettings, HostEnablementSetting, LocalPolicySetting, SandboxedAgentSettings,
    Workspace, WorkspaceUid,
};
use crate::{
    ai::llms::LLMModelHost,
    channel::ChannelState,
    settings::{AISettings, CodeSettings},
};
use regex::Regex;
use warp_core::{features::FeatureFlag, settings::Setting};
use warpui::{AppContext, Entity, ModelContext, SingletonEntity, Tracked};

/// Local capability settings used by retained AI/provider features.
///
/// The historical workspace shape is kept as a local-only cache wrapper while
/// hosted workspace/team membership is amputated from Warper startup.
pub struct UserWorkspaces {
    current_workspace_uid: Tracked<Option<WorkspaceUid>>,
    workspaces: Tracked<Vec<Workspace>>,
}

impl UserWorkspaces {
    pub fn local_only(_ctx: &mut ModelContext<Self>) -> Self {
        Self {
            current_workspace_uid: None.into(),
            workspaces: Vec::new().into(),
        }
    }

    #[cfg(test)]
    pub fn mock(cached_workspaces: Vec<Workspace>, _ctx: &mut ModelContext<Self>) -> Self {
        Self {
            current_workspace_uid: cached_workspaces.first().map(|w| w.uid.clone()).into(),
            workspaces: cached_workspaces.into(),
        }
    }

    #[cfg(test)]
    pub fn default_mock(ctx: &mut ModelContext<Self>) -> Self {
        Self::mock(vec![], ctx)
    }

    fn workspace_from_uid(&self, workspace_uid: WorkspaceUid) -> Option<&Workspace> {
        self.workspaces.iter().find(|w| w.uid == workspace_uid)
    }

    #[cfg(test)]
    fn workspace_from_uid_mut(&mut self, workspace_uid: WorkspaceUid) -> Option<&mut Workspace> {
        self.workspaces.iter_mut().find(|w| w.uid == workspace_uid)
    }

    fn current_workspace(&self) -> Option<&Workspace> {
        self.current_workspace_uid
            .clone()
            .and_then(|workspace_uid| self.workspace_from_uid(workspace_uid))
    }

    #[cfg(test)]
    fn current_workspace_mut(&mut self) -> Option<&mut Workspace> {
        self.current_workspace_uid
            .clone()
            .and_then(|workspace_uid| self.workspace_from_uid_mut(workspace_uid))
    }

    pub fn ai_allowed_by_local_policy(&self) -> bool {
        true
    }

    pub fn is_prompt_suggestions_toggleable(&self) -> bool {
        true
    }

    pub fn is_code_suggestions_toggleable(&self) -> bool {
        true
    }

    pub fn is_next_command_enabled(&self) -> bool {
        true
    }

    /// If voice input support is not compiled into this build, always returns `false`.
    pub fn is_voice_enabled(&self) -> bool {
        cfg!(feature = "voice_input")
    }

    /// Whether BYO API key is enabled for the current local user.
    /// For non-OSS builds, this is controlled by the `SoloUserByok` feature flag.
    pub fn is_byo_api_key_enabled(&self) -> bool {
        if ChannelState::channel() == warp_core::channel::Channel::Oss {
            return true;
        }

        FeatureFlag::SoloUserByok.is_enabled()
    }

    pub fn aws_bedrock_host_settings(&self) -> Option<&super::workspace::LlmHostSettings> {
        self.current_workspace().and_then(|workspace| {
            workspace
                .settings
                .llm_settings
                .host_configs
                .get(&LLMModelHost::AwsBedrock)
        })
    }

    /// Is AWS Bedrock enabled by the current local workspace policy?
    pub fn is_aws_bedrock_available_from_workspace(&self) -> bool {
        self.current_workspace().is_some_and(|workspace| {
            workspace.settings.llm_settings.enabled
                && self
                    .aws_bedrock_host_settings()
                    .is_some_and(|settings| settings.enabled)
        })
    }
    pub fn aws_bedrock_host_enablement_setting(&self) -> HostEnablementSetting {
        self.aws_bedrock_host_settings()
            .map(|settings| settings.enablement_setting.clone())
            .unwrap_or_default()
    }

    pub fn is_aws_bedrock_credentials_toggleable(&self) -> bool {
        matches!(
            self.aws_bedrock_host_enablement_setting(),
            HostEnablementSetting::RespectUserSetting
        )
    }

    pub fn is_aws_bedrock_credentials_enabled(&self, app: &AppContext) -> bool {
        if !self.is_aws_bedrock_available_from_workspace() {
            return false;
        }

        match self.aws_bedrock_host_enablement_setting() {
            HostEnablementSetting::Enforce => true,
            HostEnablementSetting::RespectUserSetting => *AISettings::as_ref(app)
                .aws_bedrock_credentials_enabled
                .value(),
        }
    }

    /// Returns local AI autonomy policy overrides.
    /// If a setting is `None`, local policy doesn't override that setting.
    pub fn ai_autonomy_settings(&self) -> AiAutonomySettings {
        self.current_workspace()
            .map(|workspace| workspace.settings.ai_autonomy_settings.clone())
            .unwrap_or_default()
    }

    /// Returns sandboxed agent policy overrides, if any.
    pub fn sandboxed_agent_settings(&self) -> Option<SandboxedAgentSettings> {
        self.current_workspace()
            .and_then(|workspace| workspace.settings.sandboxed_agent_settings.clone())
    }

    pub fn is_ai_autonomy_allowed(&self) -> bool {
        true
    }

    #[cfg(test)]
    pub fn update_workspaces(&mut self, workspaces: Vec<Workspace>, ctx: &mut ModelContext<Self>) {
        let current_workspace_uid = workspaces.first().map(|workspace| workspace.uid.clone());
        *self.current_workspace_uid = current_workspace_uid;
        *self.workspaces = workspaces;
        ctx.notify();
    }

    pub fn is_enterprise_secret_redaction_enabled(&self) -> bool {
        self.current_workspace()
            .map(|workspace| workspace.settings.secret_redaction_settings.enabled)
            .unwrap_or(false)
    }

    pub fn is_ai_allowed_in_remote_sessions(&self) -> bool {
        self.current_workspace()
            .map(|workspace| {
                workspace
                    .settings
                    .ai_permissions_settings
                    .allow_ai_in_remote_sessions
            })
            .unwrap_or(true)
    }

    pub fn get_remote_session_regex_list(&self) -> Vec<Regex> {
        self.current_workspace()
            .map(|workspace| {
                workspace
                    .settings
                    .ai_permissions_settings
                    .remote_session_regex_list
                    .clone()
            })
            .unwrap_or_default()
    }

    /// Returns the codebase context settings, taking into account local policy,
    /// global AI settings, and codebase-specific settings.
    /// Prefer this function to determine whether to show indexing-related functionality.
    pub fn is_codebase_context_enabled(&self, app: &AppContext) -> bool {
        // If local policy has an explicit setting, respect it and make the user toggle irrelevant.
        // - Enable: forced ON by local policy, regardless of user preference.
        // - Disable: forced OFF by local policy.
        // - RespectUserSetting: respect the user setting.
        let local_policy = self.local_codebase_context_policy();
        let ai_globally_enabled = AISettings::as_ref(app).is_any_ai_enabled(app);

        match local_policy {
            LocalPolicySetting::Enable => ai_globally_enabled,
            LocalPolicySetting::Disable => false,
            LocalPolicySetting::RespectUserSetting => {
                ai_globally_enabled && *CodeSettings::as_ref(app).codebase_context_enabled.value()
            }
        }
    }

    /// Returns only the local codebase context policy.
    /// Do not use this function to determine whether codebase context is generally enabled --
    /// use `is_codebase_context_enabled` instead.
    pub fn local_codebase_context_policy(&self) -> LocalPolicySetting {
        self.current_workspace()
            .map(|workspace| workspace.settings.codebase_context_settings.setting.clone())
            .unwrap_or_default()
    }
}

#[cfg(test)]
impl UserWorkspaces {
    pub fn setup_test_workspace(&mut self, ctx: &mut ModelContext<Self>) {
        self.update_workspaces(
            vec![Workspace::from_local_cache(
                "local_test_workspace".to_string().into(),
                "Local Test Workspace".to_string(),
            )],
            ctx,
        );
    }

    pub fn update_current_workspace<F>(&mut self, f: F, ctx: &mut ModelContext<Self>)
    where
        F: FnOnce(&mut Workspace),
    {
        if let Some(workspace) = self.current_workspace_mut() {
            f(workspace);
            ctx.notify();
        }
    }

    pub fn update_sandboxed_agent_settings<F>(&mut self, f: F, ctx: &mut ModelContext<Self>)
    where
        F: FnOnce(&mut Option<SandboxedAgentSettings>),
    {
        self.update_current_workspace(
            |workspace| f(&mut workspace.settings.sandboxed_agent_settings),
            ctx,
        );
    }

    pub fn update_ai_autonomy_settings<F>(&mut self, f: F, ctx: &mut ModelContext<Self>)
    where
        F: FnOnce(&mut AiAutonomySettings),
    {
        self.update_current_workspace(
            |workspace| f(&mut workspace.settings.ai_autonomy_settings),
            ctx,
        );
    }
}

impl Entity for UserWorkspaces {
    type Event = ();
}

/// Mark UserWorkspaces as global application state.
impl SingletonEntity for UserWorkspaces {}
