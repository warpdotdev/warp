pub use cloud_object_models::{
    AIExecutionProfile, ActionPermission, AskUserQuestionPermission, CloudAIExecutionProfile,
    CloudAIExecutionProfileModel, ComputerUsePermission, RunAgentsPermission, WriteToPtyPermission,
    PROFILE_NAME_MAX_LENGTH,
};
use markdown_parser::{FormattedTextFragment, FormattedTextInline};
use warp_core::features::FeatureFlag;
use warpui::{AppContext, SingletonEntity};

use super::llms::{LLMContextWindow, LLMInfo, LLMPreferences, LLMProvider};
use crate::cloud_object::model::generic_string_model::StringModel;
use crate::cloud_object::model::json_model::JsonModel;
use crate::cloud_object::{
    GenericStringObjectFormat, GenericStringObjectUniqueKey, JsonObjectType, Revision, UniquePer,
};
use crate::server::sync_queue::QueueItem;
use crate::settings::AISettings;
use crate::workspaces::user_workspaces::UserWorkspaces;
/// This threshold currently only applies to GPT 5.4 and GPT 5.5 models
pub const LONG_CONTEXT_WARNING_THRESHOLD: u32 = 272_000;
pub(crate) const LONG_CONTEXT_PRICING_WARNING_URL: &str =
    "https://developers.openai.com/api/docs/pricing";
pub(crate) fn long_context_pricing_warning_title() -> FormattedTextInline {
    vec![
        FormattedTextFragment::plain_text(
            "OpenAI automatically applies long-context pricing when context exceeds 272,000 tokens. ",
        ),
        FormattedTextFragment::hyperlink("Learn more", LONG_CONTEXT_PRICING_WARNING_URL),
    ]
}

pub mod editor;
pub mod model_menu_items;
pub mod profiles;

/// Result of resolving the cloud agent computer use setting.
/// Contains both the effective value and whether it's forced by organization policy.
pub struct CloudAgentComputerUseState {
    /// Whether computer use is enabled for cloud agents.
    pub enabled: bool,
    /// Whether this value is forced by organization settings (true = user cannot change it).
    pub is_forced_by_org: bool,
}
fn effective_base_model<'a>(profile: &AIExecutionProfile, app: &'a AppContext) -> &'a LLMInfo {
    let prefs = LLMPreferences::as_ref(app);
    profile
        .base_model
        .as_ref()
        .and_then(|id| prefs.get_llm_info(id))
        .unwrap_or_else(|| prefs.get_default_base_model())
}

/// Resolves the effective cloud agent computer use state by reading the workspace
/// autonomy setting and user's local preference from their respective singletons.
pub fn resolve_cloud_agent_computer_use_state(ctx: &AppContext) -> CloudAgentComputerUseState {
    if !FeatureFlag::AgentModeComputerUse.is_enabled() {
        return CloudAgentComputerUseState {
            enabled: false,
            is_forced_by_org: false,
        };
    }

    let autonomy_setting = UserWorkspaces::as_ref(ctx)
        .ai_autonomy_settings()
        .computer_use_setting;
    let user_preference = *AISettings::as_ref(ctx).cloud_agent_computer_use_enabled;

    match autonomy_setting {
        Some(ComputerUsePermission::Never) => CloudAgentComputerUseState {
            enabled: false,
            is_forced_by_org: true,
        },
        Some(ComputerUsePermission::AlwaysAllow) => CloudAgentComputerUseState {
            enabled: true,
            is_forced_by_org: true,
        },
        // TODO(QUALITY-297): Currently this case should never be hit because the
        // AlwaysAsk variant isn't accessible in the admin console. We need to figure
        // out how to handle it when it eventually becomes available. For now, I'm
        // treating this conservatively and marking computer use as disabled.
        Some(ComputerUsePermission::AlwaysAsk) => CloudAgentComputerUseState {
            enabled: false,
            is_forced_by_org: true,
        },
        Some(ComputerUsePermission::Unknown) | None => CloudAgentComputerUseState {
            enabled: user_preference,
            is_forced_by_org: false,
        },
    }
}

#[cfg(not(feature = "agent_mode_evals"))]
pub fn create_default_from_legacy_settings(app: &AppContext) -> AIExecutionProfile {
    // Note that the legacy "Autonomy" and "Code Access" settings are not imported here.
    // The "Code Access" setting defaulted to "Always Ask", which is the most restrictive, so
    // it's impossible for us to infer some hesitancy about autonomy from the setting and we should
    // ignore it. The same applies to "Autonomy".
    let ai_settings = AISettings::as_ref(app);
    AIExecutionProfile {
        name: "Default".to_string(),
        is_default_profile: true,
        command_denylist: ai_settings.agent_mode_command_execution_denylist.clone(),
        // We initialize the command allowlist to be anything the user added, excluding all
        // the pre-populated defaults.
        command_allowlist: ai_settings
            .agent_mode_command_execution_allowlist
            .iter()
            .filter(|cmd| !crate::settings::DEFAULT_COMMAND_EXECUTION_ALLOWLIST.contains(cmd))
            .cloned()
            .collect(),
        directory_allowlist: ai_settings.agent_mode_coding_file_read_allowlist.clone(),
        ..Default::default()
    }
}

pub trait AIExecutionProfileAppExt {
    fn configurable_context_window(&self, app: &AppContext) -> Option<LLMContextWindow>;

    fn context_window_display_value(&self, app: &AppContext) -> Option<u32>;
    fn context_window_limit_for_request(&self, app: &AppContext) -> Option<u32>;
    fn should_show_long_context_pricing_warning(
        &self,
        context_window_limit: Option<u32>,
        app: &AppContext,
    ) -> bool;
}

impl AIExecutionProfileAppExt for AIExecutionProfile {
    fn configurable_context_window(&self, app: &AppContext) -> Option<LLMContextWindow> {
        let llm = effective_base_model(self, app);
        if has_configurable_context_window(
            llm,
            FeatureFlag::GPTConfigurableContextWindow.is_enabled(),
        ) {
            Some(llm.context_window.clone())
        } else {
            None
        }
    }

    fn context_window_display_value(&self, app: &AppContext) -> Option<u32> {
        let cw = self.configurable_context_window(app)?;
        Some(self.context_window_limit.unwrap_or(cw.default_max))
    }
    fn context_window_limit_for_request(&self, app: &AppContext) -> Option<u32> {
        let llm = effective_base_model(self, app);
        if !has_configurable_context_window(
            llm,
            FeatureFlag::GPTConfigurableContextWindow.is_enabled(),
        ) {
            return None;
        }

        self.context_window_limit
            .map(|limit| limit.clamp(llm.context_window.min, llm.context_window.max))
    }

    fn should_show_long_context_pricing_warning(
        &self,
        context_window_limit: Option<u32>,
        app: &AppContext,
    ) -> bool {
        let llm = effective_base_model(self, app);
        should_show_long_context_pricing_warning(
            llm,
            Some(
                context_window_limit
                    .or(self.context_window_limit)
                    .unwrap_or(llm.context_window.default_max),
            ),
            FeatureFlag::GPTConfigurableContextWindow.is_enabled(),
        )
    }
}

pub(crate) fn has_configurable_context_window(
    llm: &LLMInfo,
    gpt_configurable_context_window_enabled: bool,
) -> bool {
    llm.context_window.is_configurable
        && llm.context_window.max > 0
        && (llm.provider != LLMProvider::OpenAI || gpt_configurable_context_window_enabled)
}

pub(crate) fn should_show_long_context_pricing_warning(
    llm: &LLMInfo,
    selected_limit: Option<u32>,
    gpt_configurable_context_window_enabled: bool,
) -> bool {
    llm.provider == LLMProvider::OpenAI
        && has_configurable_context_window(llm, gpt_configurable_context_window_enabled)
        && selected_limit
            .map(|limit| limit.clamp(llm.context_window.min, llm.context_window.max))
            .is_some_and(|limit| limit > LONG_CONTEXT_WARNING_THRESHOLD)
}

impl StringModel for AIExecutionProfile {
    type CloudObjectType = CloudAIExecutionProfile;

    fn model_type_name(&self) -> &'static str {
        "AIExecutionProfile"
    }

    fn should_enforce_revisions() -> bool {
        true
    }

    fn model_format() -> GenericStringObjectFormat {
        GenericStringObjectFormat::Json(JsonObjectType::AIExecutionProfile)
    }

    fn should_show_activity_toasts() -> bool {
        false
    }

    fn warn_if_unsaved_at_quit() -> bool {
        true
    }

    fn display_name(&self) -> String {
        // Handles case where default profile was previously created and named "Untitled"
        if self.is_default_profile {
            "Default".to_string()
        } else if self.name.trim().is_empty() {
            "Untitled".to_string()
        } else {
            self.name.clone()
        }
    }

    fn update_object_queue_item(
        &self,
        revision_ts: Option<Revision>,
        object: &Self::CloudObjectType,
    ) -> QueueItem {
        QueueItem::UpdateAIExecutionProfile {
            model: object.model().clone().into(),
            id: object.id,
            revision: revision_ts.or_else(|| object.metadata.revision.clone()),
        }
    }

    fn should_clear_on_unique_key_conflict(&self) -> bool {
        true
    }

    fn uniqueness_key(&self) -> Option<GenericStringObjectUniqueKey> {
        // We want to prevent the creation of several default profiles per user. If it's not the default
        // profile, then there can be many.
        self.is_default_profile
            .then_some(GenericStringObjectUniqueKey {
                key: "default".to_string(),
                unique_per: UniquePer::User,
            })
    }

    fn renders_in_warp_drive(&self) -> bool {
        false
    }
}

impl JsonModel for AIExecutionProfile {
    fn json_object_type() -> JsonObjectType {
        JsonObjectType::AIExecutionProfile
    }
}
