use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock};

use ai::api_keys::{ApiKeyManager, ApiKeyManagerEvent, CustomEndpoint, CustomEndpointModel};
pub use ai::LLMId;
use anyhow::Context as _;
use parking_lot::FairMutex;
use serde::{de, Deserialize, Serialize};
use settings::Setting as _;
use warp_core::features::FeatureFlag;
use warp_core::ui::icons::Icon;
use warp_core::user_preferences::GetUserPreferences;
use warp_errors::report_error;
use warp_multi_agent_api as api;
use warpui::{AppContext, Entity, EntityId, ModelContext, SingletonEntity};

use super::custom_model_routers::{self, CustomModelRouter, ModelConfigError};
use super::execution_profiles::profiles::AIExecutionProfilesModel;
use crate::auth::auth_manager::{AuthManager, AuthManagerEvent};
use crate::auth::AuthStateProvider;
use crate::network::{NetworkStatus, NetworkStatusEvent, NetworkStatusKind};
use crate::server::server_api::ServerApiProvider;
use crate::settings::AISettings;
use crate::user_config::{WarpConfig, WarpConfigUpdateEvent};
use crate::workspaces::user_workspaces::{UserWorkspaces, UserWorkspacesEvent};

/// Checks if a user's' API key is being used for the given provider.
/// Returns `true` if BYO API key is enabled and a key exists for the provider.
/// For xAI, a connected Grok subscription counts: its OAuth access token is
/// sent like a BYO key (see `ApiKeyManager::api_keys_for_request`).
pub fn is_using_api_key_for_provider(provider: &LLMProvider, app: &AppContext) -> bool {
    if !UserWorkspaces::as_ref(app).is_byo_api_key_enabled(app) {
        return false;
    }
    let manager = ApiKeyManager::as_ref(app);

    match provider {
        LLMProvider::OpenAI => manager.keys().openai.is_some(),
        LLMProvider::Anthropic => manager.keys().anthropic.is_some(),
        LLMProvider::Google => manager.keys().google.is_some(),
        LLMProvider::Xai => manager.grok_tokens().is_some(),
        LLMProvider::Unknown => false,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ByoKeySource {
    UserProvided,
    TeamProvided,
}

impl ByoKeySource {
    pub fn inference_label(self) -> &'static str {
        match self {
            ByoKeySource::UserProvided => "Inference via User-provided API key",
            ByoKeySource::TeamProvided => "Inference via Team-provided API key",
        }
    }
}

/// Returns the first-party key source that will be used for this provider.
/// Member-provided keys win when team policy allows them; otherwise a
/// configured team-managed key is used when available.
pub fn first_party_key_source_for_provider(
    provider: &LLMProvider,
    app: &AppContext,
) -> Option<ByoKeySource> {
    let workspaces = UserWorkspaces::as_ref(app);
    if workspaces.are_member_byo_keys_allowed() && is_using_api_key_for_provider(provider, app) {
        return Some(ByoKeySource::UserProvided);
    }
    if is_using_team_first_party_key_for_provider(provider, app) {
        return Some(ByoKeySource::TeamProvided);
    }
    None
}

pub fn is_using_first_party_key_for_provider(provider: &LLMProvider, app: &AppContext) -> bool {
    first_party_key_source_for_provider(provider, app).is_some()
}

fn is_using_team_first_party_key_for_provider(provider: &LLMProvider, app: &AppContext) -> bool {
    UserWorkspaces::as_ref(app)
        .current_workspace()
        .is_some_and(|workspace| {
            workspace.billing_metadata.is_managed_byok_byoe_enabled()
                && workspace
                    .settings
                    .team_byo
                    .as_ref()
                    .is_some_and(|team_byo| {
                        team_byo.first_party_enabled
                            && team_byo
                                .first_party_keys
                                .iter()
                                .any(|key| key.provider == *provider)
                    })
        })
}

pub fn byo_key_source_for_model(llm: &LLMInfo, app: &AppContext) -> Option<ByoKeySource> {
    let is_custom_endpoint = LLMPreferences::as_ref(app)
        .custom_llm_info_for_id(&llm.id)
        .is_some();
    if is_custom_endpoint && UserWorkspaces::as_ref(app).are_member_byo_endpoints_allowed() {
        return Some(ByoKeySource::UserProvided);
    }
    if is_using_team_byo_endpoint_for_model(llm, app) {
        return Some(ByoKeySource::TeamProvided);
    }
    first_party_key_source_for_provider(&llm.provider, app)
}

fn is_using_team_byo_endpoint_for_model(llm: &LLMInfo, app: &AppContext) -> bool {
    UserWorkspaces::as_ref(app)
        .current_workspace()
        .is_some_and(|workspace| {
            workspace.billing_metadata.is_managed_byok_byoe_enabled()
                && workspace
                    .settings
                    .team_byo
                    .as_ref()
                    .is_some_and(|team_byo| {
                        team_byo.endpoints_enabled
                            && team_byo.endpoints.iter().any(|endpoint| {
                                endpoint.enabled
                                    && endpoint.models.iter().any(|model| {
                                        model.enabled && model.config_key == llm.id.as_str()
                                    })
                            })
                    })
        })
}

pub fn should_show_key_icon_for_model(llm: &LLMInfo, app: &AppContext) -> bool {
    byo_key_source_for_model(llm, app).is_some()
}
pub fn should_show_bedrock_icon_for_model(llm: &LLMInfo, app: &AppContext) -> bool {
    UserWorkspaces::as_ref(app).is_aws_bedrock_credentials_enabled(app)
        && llm
            .host_configs
            .get(&LLMModelHost::AwsBedrock)
            .is_some_and(|config| config.enabled)
}

/// Key for cached LLM metadata in user preferences.
///
/// Note: this key used to store a single [`AvailableLLMs`]
/// but was migrated to store a full [`ModelsByFeature`].
pub const MODELS_BY_FEATURE_CACHE_KEY: &str = "AvailableLLMs";
const CUSTOM_ENDPOINT_USAGE_FALLBACK_LABEL: &str = "Custom endpoint";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LLMUsageMetadata {
    pub request_multiplier: usize,
    pub credit_multiplier: Option<f32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DisableReason {
    AdminDisabled,
    OutOfRequests,
    ProviderOutage,
    RequiresUpgrade,
    Unavailable,
}

impl DisableReason {
    /// Returns a user-facing tooltip explaining why the model is disabled.
    pub fn tooltip_text(&self) -> &'static str {
        match self {
            DisableReason::AdminDisabled => "This model has been disabled by your team admin.",
            DisableReason::OutOfRequests => "Please upgrade your plan to make more requests.",
            DisableReason::ProviderOutage => {
                "This model is temporarily unavailable due to a provider outage."
            }
            DisableReason::RequiresUpgrade => "Please upgrade your plan to access this model.",
            DisableReason::Unavailable => "This model is unavailable.",
        }
    }

    /// Returns `true` when this disable reason means the user cannot use the model
    /// and we should clear their stored preference.
    ///
    /// `RequiresUpgrade` is BYOK-aware: if the user has a BYO API key for the
    /// model's provider (`has_byok_key = true`), the server will still accept
    /// the request, so we keep the selection.
    ///
    /// `OutOfRequests` and `ProviderOutage` are transient and expected to
    /// resolve without user action, so we preserve the selection.
    fn should_clear_preference(&self, has_byok_key: bool) -> bool {
        match self {
            DisableReason::AdminDisabled | DisableReason::Unavailable => true,
            DisableReason::RequiresUpgrade => !has_byok_key,
            DisableReason::OutOfRequests | DisableReason::ProviderOutage => false,
        }
    }
}

/// Returns `true` when the model is usable for the current user: not disabled,
/// or disabled for a reason that doesn't block requests (see
/// [`DisableReason::should_clear_preference`]).
fn is_usable_llm(info: &LLMInfo, app: &AppContext) -> bool {
    let has_byok_key = is_using_first_party_key_for_provider(&info.provider, app);
    info.disable_reason
        .as_ref()
        .is_none_or(|reason| !reason.should_clear_preference(has_byok_key))
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LLMSpec {
    pub cost: f32,
    pub quality: f32,
    pub speed: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum LLMProvider {
    OpenAI,
    Anthropic,
    Google,
    Xai,
    Unknown,
}

impl LLMProvider {
    /// Maps an LLMProvider to its corresponding icon.
    pub fn icon(&self) -> Option<Icon> {
        match self {
            LLMProvider::OpenAI => Some(Icon::OpenAILogo),
            LLMProvider::Anthropic => Some(Icon::ClaudeLogo),
            LLMProvider::Google => Some(Icon::GeminiLogo),
            LLMProvider::Xai => None,
            LLMProvider::Unknown => None,
        }
    }

    /// Human-readable provider name for user-facing copy.
    pub fn display_name(&self) -> &'static str {
        match self {
            LLMProvider::OpenAI => "OpenAI",
            LLMProvider::Anthropic => "Anthropic",
            LLMProvider::Google => "Google",
            LLMProvider::Xai => "xAI",
            LLMProvider::Unknown => "this provider",
        }
    }
}

/// The host where an LLM can be routed to.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LLMModelHost {
    DirectApi,
    AwsBedrock,
    CustomEndpoint,
    GeminiEnterprise,
    #[serde(other)]
    Unknown,
}

/// Configuration for routing an LLM to a specific host.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RoutingHostConfig {
    pub enabled: bool,
    pub model_routing_host: LLMModelHost,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LLMContextWindow {
    #[serde(default)]
    pub is_configurable: bool,
    #[serde(default)]
    pub min: u32,
    #[serde(default)]
    pub max: u32,
    #[serde(default)]
    pub default_max: u32,
}

/// Metadata about an LLM.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LLMInfo {
    pub display_name: String,
    pub base_model_name: String,
    pub id: LLMId,
    pub reasoning_level: Option<String>,
    pub usage_metadata: LLMUsageMetadata,
    pub description: Option<String>,
    pub disable_reason: Option<DisableReason>,
    pub vision_supported: bool,
    pub spec: Option<LLMSpec>,
    pub provider: LLMProvider,
    pub host_configs: HashMap<LLMModelHost, RoutingHostConfig>,
    pub discount_percentage: Option<f32>,
    pub context_window: LLMContextWindow,
}

impl<'de> Deserialize<'de> for LLMInfo {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        /// Helper type that can deserialize host_configs from either:
        /// - A Vec (wire format from server)
        /// - A HashMap (cached format after commit a8a82421c3)
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum HostConfigsWire {
            Vec(Vec<RoutingHostConfig>),
            Map(HashMap<LLMModelHost, RoutingHostConfig>),
        }

        impl Default for HostConfigsWire {
            fn default() -> Self {
                HostConfigsWire::Vec(Vec::new())
            }
        }

        #[derive(Deserialize)]
        struct WireLLMInfo {
            display_name: String,
            #[serde(default)]
            base_model_name: Option<String>,
            id: LLMId,
            #[serde(default)]
            reasoning_level: Option<String>,
            usage_metadata: LLMUsageMetadata,
            #[serde(default)]
            description: Option<String>,
            #[serde(default)]
            disable_reason: Option<DisableReason>,
            #[serde(default)]
            vision_supported: bool,
            #[serde(default)]
            spec: Option<LLMSpec>,
            provider: LLMProvider,
            #[serde(default)]
            host_configs: HostConfigsWire,
            #[serde(default)]
            discount_percentage: Option<f32>,
            #[serde(default)]
            context_window: LLMContextWindow,
        }

        let wire = WireLLMInfo::deserialize(deserializer)?;
        let host_configs = match wire.host_configs {
            HostConfigsWire::Map(map) => map,
            HostConfigsWire::Vec(vec) => {
                let mut map = HashMap::new();
                for config in vec {
                    let host = config.model_routing_host.clone();
                    if map.insert(host.clone(), config).is_some() {
                        log::warn!(
                            "Duplicate LLMModelHost entry for {:?}, using latest value",
                            host
                        );
                    }
                }
                map
            }
        };
        Ok(Self {
            base_model_name: wire
                .base_model_name
                .unwrap_or_else(|| wire.display_name.clone()),
            vision_supported: wire.vision_supported,
            provider: wire.provider,
            display_name: wire.display_name,
            id: wire.id,
            reasoning_level: wire.reasoning_level,
            usage_metadata: wire.usage_metadata,
            description: wire.description,
            disable_reason: wire.disable_reason,
            spec: wire.spec,
            host_configs,
            discount_percentage: wire.discount_percentage,
            context_window: wire.context_window,
        })
    }
}

/// Deduplicates a list of LLMInfo choices by base_model_name and returns an alphabetically sorted
/// list of display names.
pub fn dedupe_model_display_names<'a>(
    choices: impl IntoIterator<Item = &'a LLMInfo>,
) -> Vec<String> {
    let names: HashSet<String> = choices
        .into_iter()
        .map(|choice| choice.base_model_name.clone())
        .collect();
    let mut sorted: Vec<String> = names.into_iter().collect();
    sorted.sort();
    sorted
}

impl LLMInfo {
    /// Returns the display name for the LLM, to be used in the LLM selector menu.
    pub fn menu_display_name(&self) -> String {
        // Custom model routers carry a routing/source description that belongs in
        // the sidecar detail panel, not inline in the chip label. Appending it
        // here would produce a redundant "(Routes by … · …)" suffix.
        if custom_model_routers::is_custom_router_id(self.id.as_str()) {
            return self.display_name.clone();
        }
        // Base label includes optional description in parentheses
        match &self.description {
            // This is a temporary implementation that won't scale well for longer
            // descriptions. We should implement a better approach for displaying
            // model descriptions, maybe through subtext.
            Some(desc) => format!("{} ({})", self.display_name, desc),
            None => self.display_name.clone(),
        }
    }

    /// Returns the given model's base name.
    /// For non-reasoning models, this is the same as the display name.
    /// E.g. gpt-5.1 (low reasoning) -> gpt-5.1
    pub fn base_model_name(&self) -> &str {
        &self.base_model_name
    }

    /// Returns true if this model has a reasoning level configured.
    pub fn has_reasoning_level(&self) -> bool {
        self.reasoning_level.is_some()
    }

    /// Returns the reasoning level label formatted for display.
    pub fn reasoning_level(&self) -> Option<String> {
        self.reasoning_level.clone()
    }

    #[cfg(any(test, feature = "integration_tests"))]
    pub(crate) fn new_for_test(llm_name: &str) -> Self {
        Self {
            display_name: llm_name.to_string(),
            base_model_name: llm_name.to_string(),
            id: llm_name.into(),
            reasoning_level: None,
            usage_metadata: LLMUsageMetadata {
                request_multiplier: 1,
                credit_multiplier: None,
            },
            description: None,
            disable_reason: None,
            vision_supported: false, // Default to false for tests
            spec: None,
            provider: LLMProvider::Unknown,
            host_configs: HashMap::new(),
            discount_percentage: None,
            context_window: LLMContextWindow::default(),
        }
    }
}

/// The set of LLMs available for a feature.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AvailableLLMs {
    /// The Warp "default" LLM.
    default_id: LLMId,
    choices: Vec<LLMInfo>,

    #[serde(default)]
    preferred_codex_model_id: Option<LLMId>,
}

impl AvailableLLMs {
    /// Constructs an `AvailableLLMs` instance from the given default ID and choices.
    ///
    /// If choices is empty, returns an error.
    ///
    /// If default_id is not a valid ID present in `choices`, takes the first choice in `choices
    /// and uses it as the default.
    pub fn new<T: Into<LLMInfo>>(
        mut default_id: LLMId,
        choices: impl IntoIterator<Item = T>,
        preferred_codex_model_id: Option<LLMId>,
    ) -> Result<Self, anyhow::Error> {
        let choices: Vec<LLMInfo> = choices.into_iter().map(Into::into).collect();
        if choices.is_empty() {
            return Err(anyhow::anyhow!(
                "Tried to create AvailableLLMs with empty`choices`.",
            ));
        } else if !choices.iter().any(|info| info.id == default_id) {
            let fallback_default = choices
                .first()
                .ok_or_else(|| anyhow::anyhow!("Choices should not be empty"))?;
            report_error!(
                "Default LLM ID not present in choices, falling back to first choice",
                extra: {
                    "default_id" => %default_id,
                    "fallback_choice" => %fallback_default.display_name
                }
            );
            default_id = fallback_default.id.clone();
        }

        Ok(Self {
            default_id,
            choices: choices.into_iter().collect(),
            preferred_codex_model_id,
        })
    }

    fn info_for_id(&self, id: &LLMId) -> Option<&LLMInfo> {
        self.choices.iter().find(|info| info.id == *id)
    }

    /// Returns the info for the given id only if the model is usable (present
    /// and not effectively disabled for the current user).
    fn usable_info_for_id(&self, id: &LLMId, app: &AppContext) -> Option<&LLMInfo> {
        self.info_for_id(id).filter(|info| is_usable_llm(info, app))
    }

    /// Disable-aware default: the server default when usable, otherwise the
    /// first usable choice. `None` when no server-provided choice is usable
    /// (e.g. an admin disabled every hosted model).
    fn usable_default_llm_info(&self, app: &AppContext) -> Option<&LLMInfo> {
        self.usable_info_for_id(&self.default_id, app)
            .or_else(|| self.choices.iter().find(|info| is_usable_llm(info, app)))
    }

    fn default_llm_info(&self) -> &LLMInfo {
        if let Some(info) = self.info_for_id(&self.default_id) {
            return info;
        }

        // `new()` enforces that `default_id` is one of `choices`, but
        // deserialization bypasses `new()`, so a stale persisted cache or a
        // server payload can produce an `AvailableLLMs` whose `default_id` is
        // absent from `choices`. Rather than panic, mirror `new()` and fall
        // back to the first choice.
        let fallback = self
            .choices
            .first()
            .expect("AvailableLLMs must have at least one choice");
        report_error!(
            "Default LLM ID not present in choices, falling back to first choice",
            extra: {
                "default_id" => %self.default_id,
                "fallback_choice" => %fallback.display_name
            }
        );
        fallback
    }

    #[cfg(feature = "integration_tests")]
    pub fn new_for_test(llm_name: &str) -> Self {
        Self {
            default_id: llm_name.into(),
            choices: vec![LLMInfo::new_for_test(llm_name)],
            preferred_codex_model_id: None,
        }
    }
}

/// The set of models available to the client, grouped by the feature they support.
/// This is fetched from the server and cached.
///
/// Currently, if a model is available for multiple features,
/// it will appear denormalized in each of the feature's
/// [`AvailableLLMs`]. While this denormalization doesn't add much value today,
/// it eventually lets us add feature-specific properties to an [`LLMInfo`].
///
/// NOTE: This used to include a `planning` field; this was removed after planning via subagent was
/// deprecated.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelsByFeature {
    pub agent_mode: AvailableLLMs,
    pub coding: AvailableLLMs,
    /// The set of LLMs available for CLI agent.
    /// This field is optional during deserialization, as older clients might not have this field.
    #[serde(default)]
    pub cli_agent: Option<AvailableLLMs>,
    /// The set of LLMs available for computer use agent.
    /// This field is optional during deserialization, as older clients might not have this field.
    #[serde(default)]
    pub computer_use: Option<AvailableLLMs>,
}

impl ModelsByFeature {
    /// Returns the info about the LLM identified by `id`, if we have it.
    ///
    /// For models that are available across multiple features,
    /// any one of the metadata will be returned.
    fn info_for_id(&self, id: &LLMId) -> Option<&LLMInfo> {
        self.agent_mode.info_for_id(id)
    }
}

/// Returns the default AvailableLLMs for computer use.
/// Used both in `ModelsByFeature::default()` and as a fallback in `get_computer_use_available()`.
fn default_computer_use_llms() -> AvailableLLMs {
    AvailableLLMs {
        default_id: "computer-use-agent-auto".to_owned().into(),
        choices: vec![LLMInfo {
            display_name: "auto".to_owned(),
            base_model_name: "auto".to_owned(),
            id: "computer-use-agent-auto".to_owned().into(),
            reasoning_level: None,
            usage_metadata: LLMUsageMetadata {
                request_multiplier: 1,
                credit_multiplier: None,
            },
            description: None,
            disable_reason: None,
            vision_supported: true,
            spec: None,
            provider: LLMProvider::Unknown,
            host_configs: HashMap::new(),
            discount_percentage: None,
            context_window: LLMContextWindow::default(),
        }],
        preferred_codex_model_id: None,
    }
}

impl Default for ModelsByFeature {
    fn default() -> Self {
        Self {
            agent_mode: AvailableLLMs {
                default_id: "auto".to_owned().into(),
                choices: vec![LLMInfo {
                    display_name: "auto (cost-efficient)".to_owned(),
                    base_model_name: "auto (cost-efficient)".to_owned(),
                    id: "auto".to_owned().into(),
                    reasoning_level: None,
                    usage_metadata: LLMUsageMetadata {
                        request_multiplier: 1,
                        credit_multiplier: None,
                    },
                    description: None,
                    disable_reason: None,
                    vision_supported: true,
                    spec: None,
                    provider: LLMProvider::Unknown,
                    host_configs: HashMap::new(),
                    discount_percentage: None,
                    context_window: LLMContextWindow::default(),
                }],
                preferred_codex_model_id: None,
            },
            coding: AvailableLLMs {
                default_id: "auto".to_owned().into(),
                choices: vec![LLMInfo {
                    display_name: "auto (responsive)".to_owned(),
                    base_model_name: "auto (responsive)".to_owned(),
                    id: "auto".to_owned().into(),
                    reasoning_level: None,
                    usage_metadata: LLMUsageMetadata {
                        request_multiplier: 1,
                        credit_multiplier: None,
                    },
                    description: None,
                    disable_reason: None,
                    vision_supported: true,
                    spec: None,
                    provider: LLMProvider::Unknown,
                    host_configs: HashMap::new(),
                    discount_percentage: None,
                    context_window: LLMContextWindow::default(),
                }],
                preferred_codex_model_id: None,
            },
            cli_agent: Some(AvailableLLMs {
                default_id: "cli-agent-auto".to_owned().into(),
                choices: vec![LLMInfo {
                    display_name: "auto".to_owned(),
                    base_model_name: "auto".to_owned(),
                    id: "cli-agent-auto".to_owned().into(),
                    reasoning_level: None,
                    usage_metadata: LLMUsageMetadata {
                        request_multiplier: 1,
                        credit_multiplier: None,
                    },
                    description: None,
                    disable_reason: None,
                    vision_supported: false,
                    spec: None,
                    provider: LLMProvider::Unknown,
                    host_configs: HashMap::new(),
                    discount_percentage: None,
                    context_window: LLMContextWindow::default(),
                }],
                preferred_codex_model_id: None,
            }),
            computer_use: Some(default_computer_use_llms()),
        }
    }
}

enum UpdatePopupVisibilityState {
    WaitingToBeShown,
    Visible(EntityId),
    Hidden,
}

struct AvailableLLMsUpdate {
    new_choices: Vec<LLMInfo>,
    popup_visibility_state: Arc<FairMutex<UpdatePopupVisibilityState>>,
}

/// Singleton model holding user/workspace LLM preferences, including the set of LLMs available for
/// use as well as the user's preferred LLM for Agent Mode.
pub struct LLMPreferences {
    models_by_feature: ModelsByFeature,
    last_update: Option<AvailableLLMsUpdate>,
    // Stores temporary model overrides for a given terminal view.
    // NOTE: We only store an override if the model selected by the user is different
    // from the base LLM for the active profile. This means that if the user selects the
    // profile's default model and changes their profile, the model will update to that profile's default.
    base_llm_for_terminal_view: HashMap<EntityId, LLMId>,
    /// Synthetic `LLMInfo` entries built from the user's `ApiKeyManager.custom_endpoints` so
    /// custom models surface in the model picker and resolve through `info_for_id` lookups.
    /// Each entry's `id` is the model's `config_key` (UUID), which is also what flows out to
    /// `Request.Settings.custom_model_providers.providers[*].models[*].config_key`.
    ///
    /// Rebuilt from scratch on every `ApiKeyManagerEvent::KeysUpdated`, so adds, edits, and
    /// removals all immediately propagate to the picker.
    custom_llms: Vec<LLMInfo>,
    /// All custom model routers, including both local and cloud-backed.
    custom_model_routers: Vec<CustomModelRouter>,
}

impl LLMPreferences {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let models_by_feature = get_cached_models(ctx).unwrap_or_default();

        ctx.subscribe_to_model(&NetworkStatus::handle(ctx), |me, _, event, ctx| {
            if let NetworkStatusEvent::NetworkStatusChanged {
                new_status: NetworkStatusKind::Online,
            } = event
            {
                me.refresh_authed_models(ctx);
            }
        });

        // TODO: Instead of querying this ad-hoc upon a successful log in, we should add the
        // available LLMs query to the general workspace metadata query which is polled
        // and hooked up to workspace changes. For that to work, each user would need to
        // have a personal workspace. This is a stop-gap.
        ctx.subscribe_to_model(&AuthManager::handle(ctx), |me, _, event, ctx| {
            if let AuthManagerEvent::AuthComplete = event {
                me.refresh_authed_models(ctx);
            }
        });

        ctx.subscribe_to_model(&UserWorkspaces::handle(ctx), |me, _, event, ctx| {
            if let UserWorkspacesEvent::TeamsChanged = event {
                me.sanitize_disabled_custom_model_preferences(ctx);
                me.refresh_authed_models(ctx);
            }
        });

        // Re-reconcile disabled model preferences when BYOK keys change, since
        // RequiresUpgrade models may become usable or unusable.
        // Also rebuild `custom_llms` so adds/edits/removals to the user's custom endpoints
        // immediately flow through to the model picker.
        ctx.subscribe_to_model(
            &ApiKeyManager::handle(ctx),
            |me, _, _event: &ApiKeyManagerEvent, ctx| {
                me.rebuild_custom_llms(ctx);
                me.reconcile_disabled_model_preferences(ctx);
                ctx.emit(LLMPreferencesEvent::UpdatedAvailableLLMs);
            },
        );

        // Rebuild custom model routers whenever the local `model_configs/` directory
        // changes, and reconcile any now-stale local selection.
        if FeatureFlag::CustomModelRouters.is_enabled() {
            ctx.subscribe_to_model(&WarpConfig::handle(ctx), |me, _, event, ctx| {
                if matches!(event, WarpConfigUpdateEvent::ModelConfigs) {
                    me.rebuild_custom_model_routers(ctx);
                    me.reconcile_stale_custom_router_selection(ctx);
                }
            });
        }

        let base_llm_for_terminal_view = HashMap::new();
        let custom_llms = build_custom_llm_infos(ApiKeyManager::as_ref(ctx).keys());

        let mut me = Self {
            models_by_feature,
            last_update: None,
            base_llm_for_terminal_view,
            custom_llms,
            custom_model_routers: Vec::new(),
        };

        // Seed from any already-loaded local config (the async load emits
        // `ModelConfigs` shortly after startup to populate fully).
        if FeatureFlag::CustomModelRouters.is_enabled() {
            me.rebuild_custom_model_routers(ctx);
        }

        // In agent mode eval builds, eagerly kick off a fetch of the model list from the server
        // so that it's available by the time test steps like `set_preferred_agent_mode_llm` run.
        // In production, this is handled reactively (on auth complete, network online, etc.)
        // to avoid duplicate requests at startup.
        #[cfg(feature = "agent_mode_evals")]
        me.refresh_available_models(ctx);

        me
    }

    /// Returns the `LLMInfo` for the base LLM to be used for an Agent Mode request.
    pub fn get_active_base_model<'a>(
        &'a self,
        app: &'a AppContext,
        terminal_view_id: Option<EntityId>,
    ) -> &'a LLMInfo {
        self.get_preferred_base_model(app, terminal_view_id)
    }
    /// Returns the effective execution-profile default for orchestrated Oz children.
    pub fn get_active_orchestration_model<'a>(
        &'a self,
        app: &'a AppContext,
        terminal_view_id: Option<EntityId>,
    ) -> Option<&'a LLMInfo> {
        let profile = AIExecutionProfilesModel::as_ref(app).active_profile(terminal_view_id, app);

        profile
            .data()
            .orchestration_model
            .as_ref()
            .and_then(|id| {
                self.usable_model_info_for_id(&self.models_by_feature.agent_mode, id, app)
            })
            .or_else(|| self.get_default_orchestration_model(app))
    }

    /// Returns `LLMInfo` for the currently selected LLM to be used for Agent Mode.
    fn get_preferred_base_model(
        &self,
        app: &AppContext,
        terminal_view_id: Option<EntityId>,
    ) -> &LLMInfo {
        // In the TUI, the file-backed `agents.model` setting is the source of
        // truth for the base model: it overrides both per-surface overrides
        // and the cloud-synced execution profile, keeping the TUI's TOML file
        // the single place the model is configured.
        if settings::settings_mode() == settings::SettingsMode::Tui {
            return self.tui_agent_model_info(AISettings::as_ref(app).agent_model.value(), app);
        }

        if let Some(terminal_view_id) = terminal_view_id {
            let raw_override = self.base_llm_for_terminal_view.get(&terminal_view_id);
            if let Some(llm_id) = raw_override {
                if let Some(llm_info) =
                    self.model_info_for_id(&self.models_by_feature.agent_mode, llm_id, app)
                {
                    return llm_info;
                }
            }
        }

        let profile = AIExecutionProfilesModel::as_ref(app).active_profile(terminal_view_id, app);

        profile
            .data()
            .base_model
            .clone()
            .and_then(|id| self.model_info_for_id(&self.models_by_feature.agent_mode, &id, app))
            .unwrap_or_else(|| self.fallback_llm_info(&self.models_by_feature.agent_mode, app))
    }

    /// Disable-aware fallback for when the user has no explicit (usable)
    /// selection: the feature default when usable, else the first usable
    /// server choice, else the user's first custom-endpoint model, else the
    /// (possibly disabled) server default as a last resort.
    fn fallback_llm_info<'a>(
        &'a self,
        available: &'a AvailableLLMs,
        app: &AppContext,
    ) -> &'a LLMInfo {
        available
            .usable_default_llm_info(app)
            .or_else(|| self.custom_llm_choices(app).next())
            .unwrap_or_else(|| available.default_llm_info())
    }

    /// Resolves `id` against `available` (a feature's server-provided model
    /// list, custom-router gated), then the user's custom-endpoint models and
    /// local custom routers (both gated on their respective entitlement /
    /// feature flag).
    ///
    /// Shared by the per-surface override, execution-profile, and TUI
    /// `agents.model` resolution paths so their lookup semantics can't drift.
    fn model_info_for_id<'a>(
        &'a self,
        available: &'a AvailableLLMs,
        id: &LLMId,
        app: &AppContext,
    ) -> Option<&'a LLMInfo> {
        Self::server_info_for_id_router_gated(available, id)
            .or_else(|| self.custom_llm_info_for_id_if_enabled(id, app))
            .or_else(|| self.custom_router_llm_info_for_id_if_enabled(id))
    }

    fn usable_model_info_for_id<'a>(
        &'a self,
        available: &'a AvailableLLMs,
        id: &LLMId,
        app: &AppContext,
    ) -> Option<&'a LLMInfo> {
        available
            .usable_info_for_id(id, app)
            .filter(|info| {
                FeatureFlag::CustomModelRouters.is_enabled()
                    || !custom_model_routers::is_cloud_custom_router_id(info.id.as_str())
            })
            .or_else(|| self.custom_llm_info_for_id_if_enabled(id, app))
            .or_else(|| self.custom_router_llm_info_for_id_if_enabled(id))
    }

    /// Resolves the TUI's file-backed `agents.model` setting (the
    /// `TuiAgentModel` setting) to an `LLMInfo`.
    ///
    /// `"auto"` — the default — resolves to the server-provided default model
    /// (i.e. defers to Warp's automatic model selection). Unknown ids also
    /// fall back to the default, so an invalid TOML value never sends an
    /// unresolvable model id to the server.
    ///
    /// TODO: once the TUI grows general invalid-settings UI support, surface
    /// unknown `agents.model` values to the user instead of silently falling
    /// back to the default model.
    fn tui_agent_model_info(&self, setting: &str, app: &AppContext) -> &LLMInfo {
        if setting != TUI_AUTO_MODEL_SETTING {
            let id = LLMId::from(setting);
            if let Some(info) = self.model_info_for_id(&self.models_by_feature.agent_mode, &id, app)
            {
                return info;
            }
        }
        self.models_by_feature.agent_mode.default_llm_info()
    }

    pub fn get_active_coding_model<'a>(
        &'a self,
        app: &'a AppContext,
        terminal_view_id: Option<EntityId>,
    ) -> &'a LLMInfo {
        self.get_preferred_coding_model(app, terminal_view_id)
    }

    /// Returns `LLMInfo` for user's preferred coding model.
    fn get_preferred_coding_model(
        &self,
        app: &AppContext,
        terminal_view_id: Option<EntityId>,
    ) -> &LLMInfo {
        let profile = AIExecutionProfilesModel::as_ref(app).active_profile(terminal_view_id, app);

        profile
            .data()
            .coding_model
            .clone()
            .and_then(|id| self.model_info_for_id(&self.models_by_feature.coding, &id, app))
            .unwrap_or_else(|| self.fallback_llm_info(&self.models_by_feature.coding, app))
    }

    /// Resolves `id` against a server-provided model list, but hides cloud/team
    /// custom routers when the custom-router feature flag is off. Mirrors the
    /// gating applied to local routers (see
    /// [`Self::custom_router_llm_info_for_id_if_enabled`]) so the whole
    /// custom-router feature is controlled by a single client flag.
    fn server_info_for_id_router_gated<'a>(
        available: &'a AvailableLLMs,
        id: &LLMId,
    ) -> Option<&'a LLMInfo> {
        let info = available.info_for_id(id)?;
        if !FeatureFlag::CustomModelRouters.is_enabled()
            && custom_model_routers::is_cloud_custom_router_id(info.id.as_str())
        {
            return None;
        }
        Some(info)
    }

    /// Returns the set of LLMs available for Agent Mode use.
    pub fn get_base_llm_choices_for_agent_mode(
        &self,
        app: &AppContext,
    ) -> impl Iterator<Item = &LLMInfo> {
        // Don't show admin-disabled models in the dropdown
        let routers_enabled = FeatureFlag::CustomModelRouters.is_enabled();
        self.models_by_feature
            .agent_mode
            .choices
            .iter()
            .filter(|llm| !matches!(llm.disable_reason, Some(DisableReason::AdminDisabled)))
            // Gate cloud/team routers behind the same flag as local routers so
            // the entire custom-router feature is controlled by one flag.
            .filter(move |llm| {
                routers_enabled || !custom_model_routers::is_cloud_custom_router_id(llm.id.as_str())
            })
            .chain(self.custom_llm_choices(app))
            .chain(self.custom_router_choices())
    }

    /// Returns Agent Mode's model and router catalog for orchestration defaults.
    pub fn get_orchestration_llm_choices(
        &self,
        app: &AppContext,
    ) -> impl Iterator<Item = &LLMInfo> {
        self.get_base_llm_choices_for_agent_mode(app)
    }

    /// Returns the set of LLMs available for coding.
    pub fn get_coding_llm_choices(&self, app: &AppContext) -> impl Iterator<Item = &LLMInfo> {
        // Don't show admin-disabled models in the dropdown
        let routers_enabled = FeatureFlag::CustomModelRouters.is_enabled();
        self.models_by_feature
            .coding
            .choices
            .iter()
            .filter(|llm| !matches!(llm.disable_reason, Some(DisableReason::AdminDisabled)))
            // Gate cloud/team routers behind the same flag as local routers.
            .filter(move |llm| {
                routers_enabled || !custom_model_routers::is_cloud_custom_router_id(llm.id.as_str())
            })
            .chain(self.custom_llm_choices(app))
            .chain(self.custom_router_choices())
    }

    /// Returns the set of LLMs available for CLI agent.
    pub fn get_cli_agent_llm_choices(&self, app: &AppContext) -> impl Iterator<Item = &LLMInfo> {
        // Don't show admin-disabled models in the dropdown
        self.get_cli_agent_available()
            .choices
            .iter()
            .filter(|llm| !matches!(llm.disable_reason, Some(DisableReason::AdminDisabled)))
            .chain(self.custom_llm_choices(app))
    }

    /// Returns the `LLMInfo` for the CLI agent model.
    pub fn get_active_cli_agent_model<'a>(
        &'a self,
        app: &'a AppContext,
        terminal_view_id: Option<EntityId>,
    ) -> &'a LLMInfo {
        let profile = AIExecutionProfilesModel::as_ref(app).active_profile(terminal_view_id, app);

        let available = self.get_cli_agent_available();
        profile
            .data()
            .cli_agent_model
            .clone()
            .and_then(|id| {
                available
                    .info_for_id(&id)
                    .or_else(|| self.custom_llm_info_for_id_if_enabled(&id, app))
            })
            .unwrap_or_else(|| self.fallback_llm_info(available, app))
    }

    /// Returns the effective default CLI agent model as a fallback
    /// (disable-aware, see [`Self::fallback_llm_info`]).
    pub fn get_default_cli_agent_model(&self, app: &AppContext) -> &LLMInfo {
        self.fallback_llm_info(self.get_cli_agent_available(), app)
    }

    /// Helper to get the AvailableLLMs for cli_agent, falling back to agent_mode.
    fn get_cli_agent_available(&self) -> &AvailableLLMs {
        self.models_by_feature
            .cli_agent
            .as_ref()
            .unwrap_or(&self.models_by_feature.agent_mode)
    }

    /// Returns the set of LLMs available for computer use agent.
    pub fn get_computer_use_llm_choices(&self) -> impl Iterator<Item = &LLMInfo> {
        self.get_computer_use_available().choices.iter()
    }

    /// Returns the `LLMInfo` for the computer use agent model.
    pub fn get_active_computer_use_model<'a>(
        &'a self,
        app: &'a AppContext,
        terminal_view_id: Option<EntityId>,
    ) -> &'a LLMInfo {
        let profile = AIExecutionProfilesModel::as_ref(app).active_profile(terminal_view_id, app);

        let available = self.get_computer_use_available();
        profile
            .data()
            .computer_use_model
            .clone()
            .and_then(|id| available.info_for_id(&id))
            .unwrap_or_else(|| self.get_default_computer_use_model(app))
    }

    /// Returns the effective default computer use model as a fallback: the
    /// server default when usable, else the first usable choice, else the
    /// (possibly disabled) server default. No custom-endpoint fallback here:
    /// custom models aren't offered for computer use.
    pub fn get_default_computer_use_model(&self, app: &AppContext) -> &LLMInfo {
        let available = self.get_computer_use_available();
        available
            .usable_default_llm_info(app)
            .unwrap_or_else(|| available.default_llm_info())
    }

    /// Helper to get the AvailableLLMs for computer_use.
    /// Falls back to a computer-use-specific default if None.
    fn get_computer_use_available(&self) -> &AvailableLLMs {
        static DEFAULT: OnceLock<AvailableLLMs> = OnceLock::new();
        self.models_by_feature
            .computer_use
            .as_ref()
            .unwrap_or_else(|| DEFAULT.get_or_init(default_computer_use_llms))
    }

    /// Returns metadata about an LLM, if the client knows about it.
    /// Falls back to the user's custom-endpoint LLMs when the id isn't a server-known model
    /// id (e.g. when it's a `config_key` UUID).
    pub fn get_llm_info(&self, id: &LLMId) -> Option<&LLMInfo> {
        self.models_by_feature
            .info_for_id(id)
            .or_else(|| self.custom_llm_info_for_id(id))
            .or_else(|| self.custom_router_llm_info_for_id(id))
    }

    /// Resolves an `LLMId` against the user's custom-endpoint LLMs.
    /// Returns `None` if the id isn't a known custom model `config_key`.
    pub fn custom_llm_info_for_id(&self, id: &LLMId) -> Option<&LLMInfo> {
        self.custom_llms.iter().find(|info| info.id == *id)
    }

    /// Returns `true` when `id` identifies a model that can run in a Warp cloud
    /// (Oz) agent, and is therefore safe to forward as a cloud
    /// `config.model_id`.
    ///
    /// Custom-endpoint (BYOK) models — whose `LLMId` is a bare `config_key`
    /// UUID — and local (YAML-authored) custom routers depend on the user's
    /// local credentials / local config and cannot run in the cloud. Their ids
    /// are not in the server's accepted Oz model-slug namespace, so forwarding
    /// one makes the cloud `start_agent` reject the spawn.
    ///
    /// Cloud/team custom routers (`custom-router:cloud:*`) ARE cloud-runnable:
    /// the server's spawn-time model_id validation explicitly allows the
    /// `custom-router:cloud:` prefix, and each cloud AI request re-resolves the
    /// router entirely server-side (no local config or credentials needed), so
    /// they are treated as runnable here.
    pub fn is_cloud_runnable_oz_model_id(&self, id: &LLMId) -> bool {
        !(self.custom_llm_info_for_id(id).is_some()
            || custom_model_routers::is_local_custom_router_id(id.as_str()))
    }

    /// True when the pane's active Agent Mode model can run in a Warp cloud
    /// (Oz) agent (see [`Self::is_cloud_runnable_oz_model_id`]).
    pub(crate) fn is_active_base_model_cloud_runnable(
        &self,
        terminal_view_id: EntityId,
        app: &AppContext,
    ) -> bool {
        self.is_cloud_runnable_oz_model_id(
            &self.get_active_base_model(app, Some(terminal_view_id)).id,
        )
    }

    /// Footer label for custom endpoint usage keyed by the request config_key.
    /// The synthetic custom LLMInfo already owns alias-or-name display semantics.
    pub fn custom_endpoint_usage_display_label(&self, config_key: &str) -> String {
        let config_key = LLMId::from(config_key);
        self.custom_llm_info_for_id(&config_key)
            .map(|info| info.display_name.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| CUSTOM_ENDPOINT_USAGE_FALLBACK_LABEL.to_string())
    }

    fn custom_llm_info_for_id_if_enabled(&self, id: &LLMId, app: &AppContext) -> Option<&LLMInfo> {
        Self::custom_inference_enabled(app)
            .then(|| self.custom_llm_info_for_id(id))
            .flatten()
    }

    /// Iterator over the user's custom-endpoint LLMs, gated on the feature flag and entitlement.
    pub fn custom_llm_choices(&self, app: &AppContext) -> std::slice::Iter<'_, LLMInfo> {
        if Self::custom_inference_enabled(app) {
            self.custom_llms.iter()
        } else {
            // Empty slice with a matching element type so the return type stays consistent
            // across both branches.
            (&[] as &[LLMInfo]).iter()
        }
    }

    fn custom_inference_enabled(app: &AppContext) -> bool {
        let workspaces = UserWorkspaces::as_ref(app);
        workspaces.is_custom_inference_enabled(app) && workspaces.are_member_byo_endpoints_allowed()
    }

    /// Resolves a custom model router by its `config_key`/`LLMId`.
    pub fn custom_model_router_for_id(&self, id: &LLMId) -> Option<&CustomModelRouter> {
        self.custom_model_routers.iter().find(|m| m.llm_id() == *id)
    }

    fn custom_router_llm_info_for_id(&self, id: &LLMId) -> Option<&LLMInfo> {
        self.custom_model_routers
            .iter()
            .find(|m| m.info.id == *id)
            .map(|m| &m.info)
    }

    fn custom_router_llm_info_for_id_if_enabled(&self, id: &LLMId) -> Option<&LLMInfo> {
        FeatureFlag::CustomModelRouters
            .is_enabled()
            .then(|| self.custom_router_llm_info_for_id(id))
            .flatten()
    }

    /// Iterator over the custom router picker entries, gated on the feature flag.
    /// Mirrors [`Self::custom_llm_choices`].
    pub fn custom_router_choices(&self) -> impl Iterator<Item = &LLMInfo> {
        let enabled = FeatureFlag::CustomModelRouters.is_enabled();
        self.custom_model_routers
            .iter()
            .filter(move |_| enabled)
            .map(|m| &m.info)
    }

    /// Builds the custom_model_routers registry for an outbound request.
    pub fn custom_model_routers_for_request(
        &self,
        base_id: &LLMId,
        coding_id: &LLMId,
    ) -> api::request::settings::CustomModelRouters {
        let mut models = Vec::new();
        let mut seen = HashSet::new();
        for id in [base_id, coding_id] {
            if let Some(entry) = self.custom_router_proto_entry(id) {
                if seen.insert(entry.config_key.clone()) {
                    models.push(entry);
                }
            }
        }
        api::request::settings::CustomModelRouters { routers: models }
    }

    /// Returns the proto registry entry for a local custom-router id, or `None`
    /// if `id` is not a known local router.
    fn custom_router_proto_entry(
        &self,
        id: &LLMId,
    ) -> Option<api::request::settings::custom_model_routers::CustomModelRouter> {
        self.custom_model_router_for_id(id).map(|m| m.to_proto())
    }

    /// Rebuilds `custom_model_routers` from the `model_configs/` directory,
    /// then notifies subscribers.
    ///
    /// Routers whose targets include an unknown model are excluded and a
    /// warning is logged. The check uses the currently loaded model list
    /// (server-fetched + cached), so it is best-effort at startup before
    /// the server responds.
    fn rebuild_custom_model_routers(&mut self, ctx: &mut ModelContext<Self>) {
        let local = WarpConfig::as_ref(ctx).custom_model_routers().clone();

        let mut deduped = Vec::with_capacity(local.len());
        let mut seen = HashSet::new();
        for model in local {
            if seen.insert(model.config_key()) {
                deduped.push(model);
            }
        }
        let mut validation_errors: Vec<ModelConfigError> = Vec::new();
        deduped.retain(|router| {
            let unknown: Vec<&str> = router
                .all_targets()
                .into_iter()
                .filter(|id| self.get_llm_info(&LLMId::from(*id)).is_none())
                .collect();
            if unknown.is_empty() {
                return true;
            }
            let error_message = format!("unknown target model(s): {}", unknown.join(", "));
            log::warn!(
                "Custom model router '{}': {} — excluding from picker",
                router.info.display_name,
                error_message,
            );
            validation_errors.push(ModelConfigError {
                file_name: router
                    .source_path
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .and_then(|n| n.to_str())
                    .unwrap_or(router.info.display_name.as_str())
                    .to_owned(),
                file_path: router.source_path.clone().unwrap_or_default(),
                error_message,
            });
            false
        });
        if !validation_errors.is_empty() {
            WarpConfig::handle(ctx).update(ctx, |_, ctx| {
                ctx.emit(WarpConfigUpdateEvent::ModelConfigErrors(validation_errors));
            });
        }

        // vision is supported only when every concrete target model supports it.
        for router in &mut deduped {
            router.info.vision_supported = router.all_targets().iter().all(|id| {
                self.get_llm_info(&LLMId::from(*id))
                    .is_some_and(|info| info.vision_supported)
            });
        }

        self.custom_model_routers = deduped;
        ctx.emit(LLMPreferencesEvent::UpdatedAvailableLLMs);
    }

    /// Resets any persisted *local* custom-router selection that no longer resolves
    /// to a loaded definition, so a deleted/invalid local config falls back to the
    /// default model and the visible selection updates. Scoped to local
    /// ids so a cloud selection isn't reset by a local reload.
    fn reconcile_stale_custom_router_selection(&mut self, ctx: &mut ModelContext<Self>) {
        let valid_local: HashSet<LLMId> = self
            .custom_model_routers
            .iter()
            .map(|m| m.llm_id())
            .collect();

        let mut updated_agent_mode = false;
        let mut updated_orchestration = false;
        let mut updated_coding = false;

        self.base_llm_for_terminal_view.retain(|_, id| {
            let stale = custom_model_routers::is_local_custom_router_id(id.as_str())
                && !valid_local.contains(&*id);
            updated_agent_mode |= stale;
            !stale
        });

        AIExecutionProfilesModel::handle(ctx).update(ctx, |profiles, ctx| {
            for profile_id in profiles.get_all_profile_ids() {
                let Some(profile) = profiles.get_profile_by_id(profile_id, ctx) else {
                    continue;
                };
                let profile_data = profile.data();
                let base_stale = profile_data.base_model.as_ref().is_some_and(|id| {
                    custom_model_routers::is_local_custom_router_id(id.as_str())
                        && !valid_local.contains(id)
                });
                if base_stale {
                    profiles.set_base_model(profile_id, None, ctx);
                    profiles.set_context_window_limit(profile_id, None, ctx);
                    updated_agent_mode = true;
                }
                let orchestration_stale =
                    profile_data.orchestration_model.as_ref().is_some_and(|id| {
                        custom_model_routers::is_local_custom_router_id(id.as_str())
                            && !valid_local.contains(id)
                    });
                if orchestration_stale {
                    profiles.set_orchestration_model(profile_id, None, ctx);
                    updated_orchestration = true;
                }
                let coding_stale = profile_data.coding_model.as_ref().is_some_and(|id| {
                    custom_model_routers::is_local_custom_router_id(id.as_str())
                        && !valid_local.contains(id)
                });
                if coding_stale {
                    profiles.set_coding_model(profile_id, None, ctx);
                    updated_coding = true;
                }
            }
        });

        if updated_agent_mode {
            self.trigger_snapshot_save(ctx);
            ctx.emit(LLMPreferencesEvent::UpdatedActiveAgentModeLLM);
        }
        if updated_orchestration {
            ctx.emit(LLMPreferencesEvent::UpdatedAvailableLLMs);
        }
        if updated_coding {
            ctx.emit(LLMPreferencesEvent::UpdatedActiveCodingLLM);
        }
    }

    /// Reads the user's current `ApiKeyManager.custom_endpoints` and replaces `custom_llms`
    /// with synthetic `LLMInfo`s. Called on every `ApiKeyManagerEvent::KeysUpdated`, so adds,
    /// edits, and removals all propagate immediately.
    fn rebuild_custom_llms(&mut self, app: &AppContext) {
        self.custom_llms = build_custom_llm_infos(ApiKeyManager::as_ref(app).keys());
    }

    fn sanitize_disabled_custom_model_preferences(&mut self, ctx: &mut ModelContext<Self>) {
        if Self::custom_inference_enabled(ctx) || self.custom_llms.is_empty() {
            return;
        }

        let custom_ids: HashSet<_> = self
            .custom_llms
            .iter()
            .map(|info| info.id.clone())
            .collect();
        let mut updated_agent_mode = false;
        let mut updated_orchestration = false;
        let mut updated_coding = false;
        let mut updated_other = false;

        self.base_llm_for_terminal_view.retain(|_, id| {
            let keep = !custom_ids.contains(id);
            updated_agent_mode |= !keep;
            keep
        });

        AIExecutionProfilesModel::handle(ctx).update(ctx, |profiles, ctx| {
            for profile_id in profiles.get_all_profile_ids() {
                let Some(profile) = profiles.get_profile_by_id(profile_id, ctx) else {
                    continue;
                };
                let profile_data = profile.data();

                if profile_data
                    .base_model
                    .as_ref()
                    .is_some_and(|id| custom_ids.contains(id))
                {
                    profiles.set_base_model(profile_id, None, ctx);
                    profiles.set_context_window_limit(profile_id, None, ctx);
                    updated_agent_mode = true;
                }
                if profile_data
                    .orchestration_model
                    .as_ref()
                    .is_some_and(|id| custom_ids.contains(id))
                {
                    profiles.set_orchestration_model(profile_id, None, ctx);
                    updated_orchestration = true;
                }
                if profile_data
                    .coding_model
                    .as_ref()
                    .is_some_and(|id| custom_ids.contains(id))
                {
                    profiles.set_coding_model(profile_id, None, ctx);
                    updated_coding = true;
                }
                if profile_data
                    .cli_agent_model
                    .as_ref()
                    .is_some_and(|id| custom_ids.contains(id))
                {
                    profiles.set_cli_agent_model(profile_id, None, ctx);
                    updated_other = true;
                }
                if profile_data
                    .computer_use_model
                    .as_ref()
                    .is_some_and(|id| custom_ids.contains(id))
                {
                    profiles.set_computer_use_model(profile_id, None, ctx);
                    updated_other = true;
                }
            }
        });

        if updated_agent_mode {
            self.trigger_snapshot_save(ctx);
            ctx.emit(LLMPreferencesEvent::UpdatedActiveAgentModeLLM);
        }
        if updated_coding {
            ctx.emit(LLMPreferencesEvent::UpdatedActiveCodingLLM);
        }
        if updated_orchestration || updated_other {
            ctx.emit(LLMPreferencesEvent::UpdatedAvailableLLMs);
        }
    }

    /// Returns the effective default base model as a fallback
    /// (disable-aware, see [`Self::fallback_llm_info`]).
    pub fn get_default_base_model(&self, app: &AppContext) -> &LLMInfo {
        self.fallback_llm_info(&self.models_by_feature.agent_mode, app)
    }

    /// Returns built-in `auto` when usable, then Agent Mode's disable-aware fallback.
    pub fn get_default_orchestration_model(&self, app: &AppContext) -> Option<&LLMInfo> {
        let auto_id = LLMId::from("auto");
        self.models_by_feature
            .agent_mode
            .usable_info_for_id(&auto_id, app)
            .or_else(|| {
                self.models_by_feature
                    .agent_mode
                    .usable_default_llm_info(app)
            })
    }

    /// Returns the effective default coding model as a fallback
    /// (disable-aware, see [`Self::fallback_llm_info`]).
    pub fn get_default_coding_model(&self, app: &AppContext) -> &LLMInfo {
        self.fallback_llm_info(&self.models_by_feature.coding, app)
    }

    /// Returns the preferred Codex model, if set by the server.
    pub fn get_preferred_codex_model(&self) -> Option<&LLMInfo> {
        self.models_by_feature
            .agent_mode
            .preferred_codex_model_id
            .as_ref()
            .and_then(|id| self.models_by_feature.agent_mode.info_for_id(id))
    }

    #[cfg(feature = "integration_tests")]
    pub fn is_available_agent_mode_llm(&self, id: &LLMId) -> bool {
        self.models_by_feature.agent_mode.info_for_id(id).is_some()
    }

    /// Creates a pane-level override for the Agent Mode LLM.
    pub fn update_preferred_agent_mode_llm(
        &mut self,
        preferred_llm_id: &LLMId,
        terminal_view_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) {
        let profile =
            AIExecutionProfilesModel::as_ref(ctx).active_profile(Some(terminal_view_id), ctx);

        let profile_default_model_id = profile
            .data()
            .base_model
            .as_ref()
            .and_then(|id| self.models_by_feature.agent_mode.info_for_id(id))
            .unwrap_or_else(|| self.models_by_feature.agent_mode.default_llm_info())
            .id
            .clone();

        // Only remove override if we're setting to the profile's default.
        // Otherwise, always set the override explicitly.
        let changed = if preferred_llm_id == &profile_default_model_id {
            self.base_llm_for_terminal_view
                .remove(&terminal_view_id)
                .is_some()
        } else {
            self.base_llm_for_terminal_view
                .insert(terminal_view_id, preferred_llm_id.clone());
            true
        };

        if changed {
            self.trigger_snapshot_save(ctx);
            ctx.emit(LLMPreferencesEvent::UpdatedActiveAgentModeLLM);
        }
    }

    /// Copies the raw per-pane Agent Mode override from `source_terminal_view_id`
    /// onto `new_terminal_view_id`, removing any existing override when the
    /// source has none. Combined with copying the source's execution profile,
    /// this reproduces the source pane's model resolution exactly. Unlike
    /// [`Self::update_preferred_agent_mode_llm`], the copied override is not
    /// normalized against the destination's current profile default, so it is
    /// order-independent with respect to the profile copy.
    pub(crate) fn copy_agent_mode_selection(
        &mut self,
        source_terminal_view_id: EntityId,
        new_terminal_view_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) {
        let changed = match self
            .base_llm_for_terminal_view
            .get(&source_terminal_view_id)
            .cloned()
        {
            Some(id) => {
                self.base_llm_for_terminal_view
                    .insert(new_terminal_view_id, id.clone())
                    != Some(id)
            }
            None => self
                .base_llm_for_terminal_view
                .remove(&new_terminal_view_id)
                .is_some(),
        };

        if changed {
            self.trigger_snapshot_save(ctx);
            ctx.emit(LLMPreferencesEvent::UpdatedActiveAgentModeLLM);
        }
    }

    /// Triggers a snapshot save to persist LLM override changes.
    fn trigger_snapshot_save(&self, ctx: &mut ModelContext<Self>) {
        ctx.dispatch_global_action("workspace:save_app", ());
    }

    pub fn update_preferred_coding_llm(
        &self,
        preferred_llm_id: &LLMId,
        terminal_view_id: Option<EntityId>,
        ctx: &mut ModelContext<Self>,
    ) {
        let new_value = if preferred_llm_id == &self.models_by_feature.coding.default_id {
            None
        } else {
            Some(preferred_llm_id.clone())
        };

        let mut changed = false;
        AIExecutionProfilesModel::handle(ctx).update(ctx, |profiles, ctx| {
            let profile = profiles.active_profile(terminal_view_id, ctx);

            if profile.data().coding_model != new_value {
                profiles.set_coding_model(*profile.id(), new_value, ctx);
                changed = true;
            }
        });

        if changed {
            ctx.emit(LLMPreferencesEvent::UpdatedActiveCodingLLM);
        }
    }

    pub fn new_choices_since_last_update(&self) -> Option<Vec<LLMInfo>> {
        self.last_update.as_ref().map(|update| {
            // We don't want to display new choices if they are warp branded.
            let filter_choices: Vec<LLMInfo> = update
                .new_choices
                .clone()
                .into_iter()
                .filter(|choice| !choice.display_name.starts_with("lite"))
                .collect();

            filter_choices
        })
    }

    pub fn should_show_new_choices_popup(&self, view_id: EntityId) -> bool {
        self.last_update.as_ref().is_some_and(|update| {
            let popup_state = &*update.popup_visibility_state.lock();
            matches!(popup_state, UpdatePopupVisibilityState::WaitingToBeShown)
                || matches!(
                popup_state,
                UpdatePopupVisibilityState::Visible(id) if *id == view_id)
        })
    }

    pub fn mark_new_choices_popup_as_shown(&self, view_id: EntityId) {
        if let Some(update) = self.last_update.as_ref() {
            if matches!(
                &*update.popup_visibility_state.lock(),
                UpdatePopupVisibilityState::WaitingToBeShown
            ) {
                *update.popup_visibility_state.lock() =
                    UpdatePopupVisibilityState::Visible(view_id);
            }
        }
    }

    pub fn hide_llm_popup(&self, view_id: EntityId) {
        if !self.should_show_new_choices_popup(view_id) {
            return;
        }
        let Some(last_update) = self.last_update.as_ref() else {
            return;
        };
        *last_update.popup_visibility_state.lock() = UpdatePopupVisibilityState::Hidden;
    }

    /// Fetches the latest set of models from the server for the currently logged in user, and updates the model.
    pub fn refresh_authed_models(&self, ctx: &mut ModelContext<Self>) {
        // Don't try to fetch auth'd models if the user is not logged in yet.
        if !AuthStateProvider::as_ref(ctx).get().is_logged_in() {
            return;
        }

        let ai_api_client = ServerApiProvider::as_ref(ctx).get_ai_client();
        ctx.spawn(
            async move { ai_api_client.get_feature_model_choices().await },
            |me, result, ctx| match result {
                Ok(update) => {
                    if update != me.models_by_feature {
                        me.on_server_update(update, ctx);
                    }
                }
                Err(e) => {
                    report_error!(e.context("Failed to fetch LLMs from server"));
                }
            },
        );
    }

    /// No auth required (i.e. to populate the pre-login onboarding picker).
    fn refresh_public_models(&self, ctx: &mut ModelContext<Self>) {
        let ai_api_client = ServerApiProvider::as_ref(ctx).get_ai_client();
        ctx.spawn(
            async move { ai_api_client.get_free_available_models(None).await },
            |me, result, ctx| match result {
                Ok(update) => {
                    if update != me.models_by_feature {
                        me.on_server_update(update, ctx);
                    }
                }
                Err(e) => {
                    report_error!(e.context("Failed to fetch free-tier LLMs from server"));
                }
            },
        );
    }

    pub fn refresh_available_models(&self, ctx: &mut ModelContext<Self>) {
        if AuthStateProvider::as_ref(ctx).get().is_logged_in() {
            self.refresh_authed_models(ctx);
        } else {
            self.refresh_public_models(ctx);
        }
    }

    pub fn update_feature_model_choices(
        &mut self,
        choices_result: Result<ModelsByFeature, anyhow::Error>,
        ctx: &mut ModelContext<Self>,
    ) {
        if let Ok(choices) = choices_result {
            self.on_server_update(choices, ctx);
        }
    }

    fn on_server_update(&mut self, update: ModelsByFeature, ctx: &mut ModelContext<Self>) {
        let has_existing_persisted_config = get_cached_models(ctx).is_some();

        let old = std::mem::replace(&mut self.models_by_feature, update);

        match serde_json::to_string(&self.models_by_feature)
            .context("Failed to serialize LLMs for cache")
        {
            Ok(serialized_update) => {
                if let Err(e) = ctx
                    .private_user_preferences()
                    .write_value(MODELS_BY_FEATURE_CACHE_KEY, serialized_update)
                    .context("Failed to cache LLMs")
                {
                    report_error!(e);
                }
            }
            Err(e) => {
                report_error!(e);
            }
        }

        self.reconcile_disabled_model_preferences(ctx);

        // Re-evaluate custom model routers now that the server catalog is fresh.
        // A router that was excluded at startup (because its target wasn't in the
        // cached catalog) is reconsidered here with the authoritative model list.
        if FeatureFlag::CustomModelRouters.is_enabled() {
            self.rebuild_custom_model_routers(ctx);
            self.reconcile_stale_custom_router_selection(ctx);
        }

        let new_choices =
            get_new_agent_mode_choices(&old.agent_mode, &self.models_by_feature.agent_mode);
        if !new_choices.is_empty() {
            self.last_update = Some(AvailableLLMsUpdate {
                new_choices,
                // We shouldn't show the update for the initial LLM config creation.
                popup_visibility_state: Arc::new(FairMutex::new(
                    if has_existing_persisted_config {
                        UpdatePopupVisibilityState::WaitingToBeShown
                    } else {
                        UpdatePopupVisibilityState::Hidden
                    },
                )),
            });
        }

        ctx.emit(LLMPreferencesEvent::UpdatedAvailableLLMs);
    }

    /// Clear any model selections where the model is no longer supported
    /// or effectively disabled, and clear orphaned context window limits
    /// for non-configurable or unusable models.
    ///
    /// Called both when the model list is refreshed from the server and when
    /// BYOK API keys change (since `RequiresUpgrade` usability is BYOK-aware).
    fn reconcile_disabled_model_preferences(&self, ctx: &mut ModelContext<Self>) {
        let profiles_model = AIExecutionProfilesModel::handle(ctx);
        profiles_model.update(ctx, |profiles, ctx| {
            for profile_id in profiles.get_all_profile_ids() {
                if let Some(profile) = profiles.get_profile_by_id(profile_id, ctx) {
                    let profile_data = profile.data();
                    let preferred_base_model = profile_data.base_model.clone();
                    let effective_base_model_id = preferred_base_model
                        .as_ref()
                        .unwrap_or(&self.models_by_feature.agent_mode.default_id);
                    let effective_base_model_usable = self
                        .models_by_feature
                        .agent_mode
                        .usable_info_for_id(effective_base_model_id, ctx)
                        .or_else(|| {
                            self.custom_llm_info_for_id_if_enabled(effective_base_model_id, ctx)
                        });
                    let effective_base_model_unusable = effective_base_model_usable.is_none();
                    let effective_base_model_is_configurable = effective_base_model_usable
                        .is_some_and(|info| info.context_window.is_configurable);
                    let has_context_window_limit = profile_data.context_window_limit.is_some();

                    if preferred_base_model.is_some() && effective_base_model_unusable {
                        profiles.set_base_model(profile_id, None, ctx);
                    }
                    if has_context_window_limit
                        && (effective_base_model_unusable || !effective_base_model_is_configurable)
                    {
                        profiles.set_context_window_limit(profile_id, None, ctx);
                    }
                    if let Some(preferred_llm_id) = &profile.data().orchestration_model {
                        if self
                            .usable_model_info_for_id(
                                &self.models_by_feature.agent_mode,
                                preferred_llm_id,
                                ctx,
                            )
                            .is_none()
                        {
                            profiles.set_orchestration_model(profile_id, None, ctx);
                        }
                    }
                    if let Some(preferred_llm_id) = &profile.data().coding_model {
                        if self
                            .models_by_feature
                            .coding
                            .usable_info_for_id(preferred_llm_id, ctx)
                            .or_else(|| {
                                self.custom_llm_info_for_id_if_enabled(preferred_llm_id, ctx)
                            })
                            .is_none()
                        {
                            profiles.set_coding_model(profile_id, None, ctx);
                        }
                    }
                    if let Some(preferred_llm_id) = &profile.data().cli_agent_model {
                        if self
                            .get_cli_agent_available()
                            .usable_info_for_id(preferred_llm_id, ctx)
                            .or_else(|| {
                                self.custom_llm_info_for_id_if_enabled(preferred_llm_id, ctx)
                            })
                            .is_none()
                        {
                            profiles.set_cli_agent_model(profile_id, None, ctx);
                        }
                    }
                    if let Some(preferred_llm_id) = &profile.data().computer_use_model {
                        if self
                            .get_computer_use_available()
                            .usable_info_for_id(preferred_llm_id, ctx)
                            .is_none()
                        {
                            profiles.set_computer_use_model(profile_id, None, ctx);
                        }
                    }
                }
            }
        });
    }

    pub fn vision_supported(&self, app: &AppContext, terminal_view_id: Option<EntityId>) -> bool {
        self.get_active_base_model(app, terminal_view_id)
            .vision_supported
    }

    pub fn get_base_llm_override(&self, terminal_view_id: EntityId) -> Option<String> {
        if let Some(override_str) = self
            .base_llm_for_terminal_view
            .get(&terminal_view_id)
            .and_then(|llm_id| serde_json::to_string(llm_id).ok())
        {
            return Some(override_str);
        }

        log::debug!("LLM override not found in memory for terminal view: {terminal_view_id:?}");
        None
    }

    /// Removes the LLM override for a terminal view.
    /// This ensures that the new profile's default model is used.
    pub fn remove_llm_override(
        &mut self,
        terminal_view_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) {
        let old = self.base_llm_for_terminal_view.remove(&terminal_view_id);
        if old.is_some() {
            self.trigger_snapshot_save(ctx);
            ctx.emit(LLMPreferencesEvent::UpdatedActiveAgentModeLLM);
        }
    }
}

/// The TUI `agents.model` value that defers model choice to Warp's automatic
/// model selection (the server-provided default).
const TUI_AUTO_MODEL_SETTING: &str = "auto";

#[derive(Clone, Debug)]
pub enum LLMPreferencesEvent {
    UpdatedAvailableLLMs,
    UpdatedActiveAgentModeLLM,
    UpdatedActiveCodingLLM,
}

impl Entity for LLMPreferences {
    type Event = LLMPreferencesEvent;
}

impl SingletonEntity for LLMPreferences {}

fn get_new_agent_mode_choices(
    old_config: &AvailableLLMs,
    new_config: &AvailableLLMs,
) -> Vec<LLMInfo> {
    let old_ids: HashSet<_> = old_config.choices.iter().map(|info| &info.id).collect();
    new_config
        .choices
        .iter()
        .filter(|info| !old_ids.contains(&info.id))
        .cloned()
        .collect()
}

/// Builds synthetic [`LLMInfo`]s from the user's persisted custom endpoints.
///
/// One entry per `CustomEndpointModel`. The display label is the **alias** when present,
/// falling back to the raw model name. The `id` is the model's `config_key`, which is
/// also what flows out to `Request.Settings.custom_model_providers` so the server can map
/// a `ModelConfig.{base,coding,cli_agent,computer_use_agent}` selection back to the
/// user-provided endpoint.
///
/// Endpoints with empty URL or API key, and models with empty name or config_key, are
/// skipped — they shouldn't surface in the picker until the user finishes configuring them.
fn build_custom_llm_infos(keys: &ai::api_keys::ApiKeys) -> Vec<LLMInfo> {
    keys.custom_endpoints
        .iter()
        .filter(|ep| !ep.url.trim().is_empty() && !ep.api_key.is_empty())
        .flat_map(|endpoint| {
            endpoint
                .models
                .iter()
                .filter(|m| !m.name.trim().is_empty() && !m.config_key.is_empty())
                .map(move |model| custom_llm_info_from(endpoint, model))
        })
        .collect()
}

fn custom_llm_info_from(endpoint: &CustomEndpoint, model: &CustomEndpointModel) -> LLMInfo {
    let label = model.display_label().to_owned();
    LLMInfo {
        display_name: label.clone(),
        base_model_name: label,
        id: model.config_key.clone().into(),
        reasoning_level: None,
        usage_metadata: LLMUsageMetadata {
            request_multiplier: 1,
            credit_multiplier: None,
        },
        description: Some(format!("Custom · {}", endpoint.name)),
        disable_reason: None,
        vision_supported: true,
        spec: None,
        provider: LLMProvider::Unknown,
        host_configs: HashMap::new(),
        discount_percentage: None,
        context_window: LLMContextWindow::default(),
    }
}

/// Gets the last cached LLM metadata.
fn get_cached_models(app: &mut AppContext) -> Option<ModelsByFeature> {
    let value = app
        .private_user_preferences()
        .read_value(MODELS_BY_FEATURE_CACHE_KEY)
        .ok()
        .flatten()?;

    // Try to deserialize to the [`ModelsByFeature`] type.
    match serde_json::from_str::<ModelsByFeature>(value.as_str()) {
        Ok(config) => Some(config),
        Err(e1) => {
            // If that fails, try to deserialize directly to [`AvailableLLMs`].
            // Before we had model choice by feature, all available LLMs were solely
            // for Agent Mode.
            match serde_json::from_str::<AvailableLLMs>(value.as_str()) {
                Ok(config) => Some(ModelsByFeature {
                    agent_mode: config,
                    ..Default::default()
                }),
                Err(e2) => {
                    log::warn!("Failed to deserialize cached LLMs: {e1}\n{e2}");
                    None
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "llms_tests.rs"]
mod tests;
