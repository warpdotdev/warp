use serde::{de, Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    sync::OnceLock,
    time::Duration,
};
use warp_core::{
    channel::{Channel, ChannelState},
    ui::icons::Icon,
};
use warpui::{AppContext, Entity, EntityId, ModelContext, SingletonEntity};

use crate::workspaces::user_workspaces::UserWorkspaces;

use super::execution_profiles::profiles::AIExecutionProfilesModel;

use ai::api_keys::{ApiKeyManager, ApiKeyManagerEvent};
pub use ai::LLMId;

/// Checks if a user's' API key is being used for the given provider.
/// Returns `true` if BYO API key is enabled and a key exists for the provider.
pub fn is_using_api_key_for_provider(provider: &LLMProvider, app: &AppContext) -> bool {
    use ai::api_keys::ApiKeyManager;

    if ChannelState::channel() == Channel::Oss && matches!(provider, LLMProvider::OpenRouter) {
        return ApiKeyManager::as_ref(app)
            .keys()
            .open_router
            .as_ref()
            .is_some_and(|key| !key.trim().is_empty());
    }

    let api_keys = UserWorkspaces::as_ref(app)
        .is_byo_api_key_enabled()
        .then(|| ApiKeyManager::as_ref(app).keys().clone());

    match provider {
        LLMProvider::OpenAI => api_keys.is_some_and(|keys| keys.openai.is_some()),
        LLMProvider::Anthropic => api_keys.is_some_and(|keys| keys.anthropic.is_some()),
        LLMProvider::Google => api_keys.is_some_and(|keys| keys.google.is_some()),
        LLMProvider::OpenRouter => api_keys.is_some_and(|keys| keys.open_router.is_some()),
        _ => false,
    }
}

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
    #[serde(alias = "Openrouter", alias = "OPENROUTER", alias = "openrouter")]
    OpenRouter,
    Unknown,
}

impl LLMProvider {
    /// Maps an LLMProvider to its corresponding icon.
    pub fn icon(&self) -> Option<Icon> {
        match self {
            LLMProvider::OpenAI => Some(Icon::OpenAILogo),
            LLMProvider::Anthropic => Some(Icon::ClaudeLogo),
            LLMProvider::Google => Some(Icon::GeminiLogo),
            LLMProvider::OpenRouter => Some(Icon::Globe),
            LLMProvider::Xai => None,
            LLMProvider::Unknown => None,
        }
    }
}

/// The host where an LLM can be routed to.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LLMModelHost {
    DirectApi,
    AwsBedrock,
    #[serde(other)]
    Unknown,
}

/// Configuration for routing an LLM to a specific host.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RoutingHostConfig {
    pub enabled: bool,
    pub model_routing_host: LLMModelHost,
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
        })
    }
}

impl LLMInfo {
    /// Returns the display name for the LLM, to be used in the LLM selector menu.
    pub fn menu_display_name(&self) -> String {
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

    #[cfg(feature = "integration_tests")]
    #[allow(dead_code)]
    fn new_for_test(llm_name: &str) -> Self {
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
            log::error!(
                "Default LLM ID {} not present in choices, falling back to first choice {}",
                default_id,
                fallback_default.display_name
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

    fn default_llm_info(&self) -> &LLMInfo {
        self.info_for_id(&self.default_id)
            .expect("Default LLM ID must be present in choices")
    }

    fn upsert_choice(&mut self, llm: LLMInfo) -> bool {
        if let Some(existing) = self.choices.iter_mut().find(|choice| choice.id == llm.id) {
            if *existing != llm {
                *existing = llm;
                return true;
            }
            false
        } else {
            self.choices.push(llm);
            true
        }
    }

    #[cfg(feature = "integration_tests")]
    #[allow(dead_code)]
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

    fn ensure_openrouter_custom_model(&mut self, model_id: Option<&str>) -> bool {
        let Some(llm) = model_id.and_then(openrouter_custom_llm) else {
            return false;
        };

        self.upsert_openrouter_model(llm)
    }

    fn upsert_openrouter_model(&mut self, llm: LLMInfo) -> bool {
        let mut changed = false;
        changed |= self.agent_mode.upsert_choice(llm.clone());
        changed |= self.coding.upsert_choice(llm.clone());
        if let Some(cli_agent) = &mut self.cli_agent {
            changed |= cli_agent.upsert_choice(llm.clone());
        }
        if let Some(computer_use) = &mut self.computer_use {
            changed |= computer_use.upsert_choice(llm);
        }
        changed
    }

    fn upsert_openrouter_models(&mut self, models: Vec<LLMInfo>) -> bool {
        models.into_iter().fold(false, |changed, llm| {
            self.upsert_openrouter_model(llm) || changed
        })
    }

    fn configured_openrouter_llm<'a>(
        &'a self,
        app: &AppContext,
        available: &'a AvailableLLMs,
    ) -> Option<&'a LLMInfo> {
        if ChannelState::channel() != Channel::Oss {
            return None;
        }

        let model_id = ApiKeyManager::as_ref(app)
            .keys()
            .open_router_model
            .as_deref()
            .and_then(normalize_openrouter_model_id)?;
        available.info_for_id(&model_id.into())
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
        }],
        preferred_codex_model_id: None,
    }
}

pub const DEFAULT_OPENROUTER_MODEL_ID: &str = "openrouter/auto";
const OPENROUTER_MODELS_URL: &str = "https://openrouter.ai/api/v1/models";
const OPENROUTER_MODELS_TIMEOUT: Duration = Duration::from_secs(15);

fn normalize_openrouter_model_id(model_id: &str) -> Option<&str> {
    let model_id = model_id.trim();
    (!model_id.is_empty()).then_some(model_id)
}

pub fn is_openrouter_custom_model_query(query: &str) -> bool {
    normalize_openrouter_model_id(query).is_some_and(|model_id| {
        model_id != DEFAULT_OPENROUTER_MODEL_ID
            && model_id.contains('/')
            && !model_id.chars().any(char::is_whitespace)
    })
}

fn openrouter_custom_llm(model_id: &str) -> Option<LLMInfo> {
    let model_id = normalize_openrouter_model_id(model_id)?;
    (model_id != DEFAULT_OPENROUTER_MODEL_ID).then(|| openrouter_llm(model_id, model_id, true))
}

#[derive(Debug, Deserialize)]
struct OpenRouterModelsResponse {
    #[serde(default)]
    data: Vec<OpenRouterModel>,
}

#[derive(Debug, Deserialize)]
struct OpenRouterModel {
    id: String,
    name: Option<String>,
    description: Option<String>,
    architecture: Option<OpenRouterModelArchitecture>,
}

#[derive(Debug, Default, Deserialize)]
struct OpenRouterModelArchitecture {
    #[serde(default)]
    input_modalities: Vec<String>,
}

impl OpenRouterModel {
    fn into_llm(self) -> Option<LLMInfo> {
        let id = normalize_openrouter_model_id(&self.id)?.to_owned();
        let display_name = self
            .name
            .as_deref()
            .and_then(normalize_openrouter_model_id)
            .unwrap_or(&id)
            .to_owned();
        let vision_supported = self.architecture.is_some_and(|architecture| {
            architecture
                .input_modalities
                .iter()
                .any(|modality| modality.eq_ignore_ascii_case("image"))
        });

        Some(LLMInfo {
            display_name: display_name.clone(),
            base_model_name: display_name,
            id: id.into(),
            reasoning_level: None,
            usage_metadata: LLMUsageMetadata {
                request_multiplier: 1,
                credit_multiplier: None,
            },
            description: self.description,
            disable_reason: None,
            vision_supported,
            spec: None,
            provider: LLMProvider::OpenRouter,
            host_configs: HashMap::from([(
                LLMModelHost::DirectApi,
                RoutingHostConfig {
                    enabled: true,
                    model_routing_host: LLMModelHost::DirectApi,
                },
            )]),
            discount_percentage: None,
        })
    }
}

async fn fetch_openrouter_models() -> Result<Vec<LLMInfo>, anyhow::Error> {
    let response = http_client::Client::new()
        .get(OPENROUTER_MODELS_URL)
        .timeout(OPENROUTER_MODELS_TIMEOUT)
        .header("HTTP-Referer", "https://warper.dev")
        .header("X-Title", "Warper")
        .send()
        .await?
        .error_for_status()?
        .json::<OpenRouterModelsResponse>()
        .await?;

    let mut seen = HashSet::new();
    let mut models = response
        .data
        .into_iter()
        .filter_map(OpenRouterModel::into_llm)
        .filter(|llm| seen.insert(llm.id.clone()))
        .collect::<Vec<_>>();
    models.sort_by_cached_key(|llm| llm.display_name.to_lowercase());
    Ok(models)
}

fn openrouter_llm(display_name: &str, id: &str, vision_supported: bool) -> LLMInfo {
    LLMInfo {
        display_name: display_name.to_owned(),
        base_model_name: display_name.to_owned(),
        id: id.into(),
        reasoning_level: None,
        usage_metadata: LLMUsageMetadata {
            request_multiplier: 1,
            credit_multiplier: None,
        },
        description: None,
        disable_reason: None,
        vision_supported,
        spec: None,
        provider: LLMProvider::OpenRouter,
        host_configs: HashMap::from([(
            LLMModelHost::DirectApi,
            RoutingHostConfig {
                enabled: true,
                model_routing_host: LLMModelHost::DirectApi,
            },
        )]),
        discount_percentage: None,
    }
}

fn openrouter_models_by_feature() -> ModelsByFeature {
    let choices = vec![
        openrouter_llm("OpenRouter Auto", DEFAULT_OPENROUTER_MODEL_ID, true),
        openrouter_llm("GPT-4o mini", "openai/gpt-4o-mini", true),
        openrouter_llm("GPT-4o", "openai/gpt-4o", true),
        openrouter_llm("Claude 3.5 Sonnet", "anthropic/claude-3.5-sonnet", true),
        openrouter_llm("Gemini Flash 1.5", "google/gemini-flash-1.5", true),
        openrouter_llm(
            "Llama 3.1 70B Instruct",
            "meta-llama/llama-3.1-70b-instruct",
            false,
        ),
    ];

    let agent_mode = AvailableLLMs::new(
        DEFAULT_OPENROUTER_MODEL_ID.into(),
        choices.clone(),
        Some(DEFAULT_OPENROUTER_MODEL_ID.into()),
    )
    .expect("OpenRouter default model list should not be empty");

    let coding = AvailableLLMs::new(
        DEFAULT_OPENROUTER_MODEL_ID.into(),
        choices.clone(),
        Some(DEFAULT_OPENROUTER_MODEL_ID.into()),
    )
    .expect("OpenRouter coding model list should not be empty");

    let cli_agent = Some(
        AvailableLLMs::new(
            DEFAULT_OPENROUTER_MODEL_ID.into(),
            choices.clone(),
            Some(DEFAULT_OPENROUTER_MODEL_ID.into()),
        )
        .expect("OpenRouter CLI agent model list should not be empty"),
    );

    let computer_use = Some(
        AvailableLLMs::new(
            DEFAULT_OPENROUTER_MODEL_ID.into(),
            choices,
            Some(DEFAULT_OPENROUTER_MODEL_ID.into()),
        )
        .expect("OpenRouter computer use model list should not be empty"),
    );

    ModelsByFeature {
        agent_mode,
        coding,
        cli_agent,
        computer_use,
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
                }],
                preferred_codex_model_id: None,
            }),
            computer_use: Some(default_computer_use_llms()),
        }
    }
}

/// Singleton model holding user/workspace LLM preferences, including the set of LLMs available for
/// use as well as the user's preferred LLM for Agent Mode.
pub struct LLMPreferences {
    models_by_feature: ModelsByFeature,
    // Stores temporary model overrides for a given terminal view.
    // NOTE: We only store an override if the model selected by the user is different
    // from the base LLM for the active profile. This means that if the user selects the
    // profile's default model and changes their profile, the model will update to that profile's default.
    base_llm_for_terminal_view: HashMap<EntityId, LLMId>,
}

impl LLMPreferences {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let mut models_by_feature = if ChannelState::channel() == Channel::Oss {
            openrouter_models_by_feature()
        } else {
            ModelsByFeature::default()
        };

        if ChannelState::channel() == Channel::Oss {
            let configured_model = ApiKeyManager::as_ref(ctx).keys().open_router_model.clone();
            models_by_feature.ensure_openrouter_custom_model(configured_model.as_deref());
        }

        ctx.subscribe_to_model(&ApiKeyManager::handle(ctx), |me, event, ctx| {
            if ChannelState::channel() != Channel::Oss
                || !matches!(event, ApiKeyManagerEvent::KeysUpdated)
            {
                return;
            }

            let configured_model = ApiKeyManager::as_ref(ctx).keys().open_router_model.clone();
            if me
                .models_by_feature
                .ensure_openrouter_custom_model(configured_model.as_deref())
            {
                ctx.emit(LLMPreferencesEvent::UpdatedAvailableLLMs);
            }
            me.refresh_openrouter_models(ctx);
            ctx.emit(LLMPreferencesEvent::UpdatedActiveAgentModeLLM);
            ctx.emit(LLMPreferencesEvent::UpdatedActiveCodingLLM);
        });

        let base_llm_for_terminal_view = HashMap::new();

        let me = Self {
            models_by_feature,
            base_llm_for_terminal_view,
        };

        if ChannelState::channel() == Channel::Oss {
            me.refresh_openrouter_models(ctx);
        }

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

    /// Returns `LLMInfo` for the currently selected LLM to be used for Agent Mode.
    fn get_preferred_base_model(
        &self,
        app: &AppContext,
        terminal_view_id: Option<EntityId>,
    ) -> &LLMInfo {
        if let Some(llm_info) = self
            .models_by_feature
            .configured_openrouter_llm(app, &self.models_by_feature.agent_mode)
        {
            return llm_info;
        }

        if let Some(terminal_view_id) = terminal_view_id {
            let raw_override = self.base_llm_for_terminal_view.get(&terminal_view_id);
            if let Some(llm_id) = raw_override {
                if let Some(llm_info) = self.models_by_feature.agent_mode.info_for_id(llm_id) {
                    return llm_info;
                }
            }
        }

        let profile = AIExecutionProfilesModel::as_ref(app).active_profile(terminal_view_id, app);

        profile
            .data()
            .base_model
            .clone()
            .and_then(|id| self.models_by_feature.agent_mode.info_for_id(&id))
            .unwrap_or_else(|| self.models_by_feature.agent_mode.default_llm_info())
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
        if let Some(llm_info) = self
            .models_by_feature
            .configured_openrouter_llm(app, &self.models_by_feature.coding)
        {
            return llm_info;
        }

        let profile = AIExecutionProfilesModel::as_ref(app).active_profile(terminal_view_id, app);

        profile
            .data()
            .coding_model
            .clone()
            .and_then(|id| self.models_by_feature.coding.info_for_id(&id))
            .unwrap_or_else(|| self.models_by_feature.coding.default_llm_info())
    }

    /// Returns the set of LLMs available for Agent Mode use.
    pub fn get_base_llm_choices_for_agent_mode(&self) -> impl Iterator<Item = &LLMInfo> {
        // Don't show admin-disabled models in the dropdown
        self.models_by_feature
            .agent_mode
            .choices
            .iter()
            .filter(|llm| !matches!(llm.disable_reason, Some(DisableReason::AdminDisabled)))
    }

    /// Returns the set of LLMs available for coding.
    pub fn get_coding_llm_choices(&self) -> impl Iterator<Item = &LLMInfo> {
        // Don't show admin-disabled models in the dropdown
        self.models_by_feature
            .coding
            .choices
            .iter()
            .filter(|llm| !matches!(llm.disable_reason, Some(DisableReason::AdminDisabled)))
    }

    /// Returns the set of LLMs available for CLI agent.
    pub fn get_cli_agent_llm_choices(&self) -> impl Iterator<Item = &LLMInfo> {
        self.get_cli_agent_available().choices.iter()
    }

    /// Returns the `LLMInfo` for the CLI agent model.
    pub fn get_active_cli_agent_model<'a>(
        &'a self,
        app: &'a AppContext,
        terminal_view_id: Option<EntityId>,
    ) -> &'a LLMInfo {
        let available = self.get_cli_agent_available();
        if let Some(llm_info) = self
            .models_by_feature
            .configured_openrouter_llm(app, available)
        {
            return llm_info;
        }

        let profile = AIExecutionProfilesModel::as_ref(app).active_profile(terminal_view_id, app);
        profile
            .data()
            .cli_agent_model
            .clone()
            .and_then(|id| available.info_for_id(&id))
            .unwrap_or_else(|| available.default_llm_info())
    }

    /// Returns the default CLI agent model as a fallback.
    pub fn get_default_cli_agent_model(&self) -> &LLMInfo {
        self.get_cli_agent_available().default_llm_info()
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
        let available = self.get_computer_use_available();
        if let Some(llm_info) = self
            .models_by_feature
            .configured_openrouter_llm(app, available)
        {
            return llm_info;
        }

        let profile = AIExecutionProfilesModel::as_ref(app).active_profile(terminal_view_id, app);
        profile
            .data()
            .computer_use_model
            .clone()
            .and_then(|id| available.info_for_id(&id))
            .unwrap_or_else(|| available.default_llm_info())
    }

    /// Returns the default computer use model as a fallback.
    pub fn get_default_computer_use_model(&self) -> &LLMInfo {
        self.get_computer_use_available().default_llm_info()
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
    pub fn get_llm_info(&self, id: &LLMId) -> Option<&LLMInfo> {
        self.models_by_feature.info_for_id(id)
    }

    /// Returns the default base model as a fallback.
    pub fn get_default_base_model(&self) -> &LLMInfo {
        self.models_by_feature.agent_mode.default_llm_info()
    }

    /// Returns the default coding model as a fallback.
    pub fn get_default_coding_model(&self) -> &LLMInfo {
        self.models_by_feature.coding.default_llm_info()
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

    fn refresh_openrouter_models(&self, ctx: &mut ModelContext<Self>) {
        if ChannelState::channel() != Channel::Oss {
            return;
        }

        ctx.spawn(
            async { fetch_openrouter_models().await },
            |me, result, ctx| match result {
                Ok(models) => {
                    if me.models_by_feature.upsert_openrouter_models(models) {
                        ctx.emit(LLMPreferencesEvent::UpdatedAvailableLLMs);
                    }
                }
                Err(error) => {
                    log::warn!("Failed to fetch OpenRouter models: {error}");
                }
            },
        );
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

#[cfg(test)]
#[path = "llms_tests.rs"]
mod tests;
