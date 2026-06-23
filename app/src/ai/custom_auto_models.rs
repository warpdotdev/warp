//! Custom auto models: user-defined "auto" models (named routers) that resolve to
//! a concrete model per task.
//!
//! This module holds the portable definition for **local** (YAML-authored) custom
//! auto models: produced from `~/.warp/custom_auto_models.yaml` (see
//! [`parse_model_configs_yaml`]), surfaced in the model picker as synthetic
//! [`LLMInfo`] entries, and serialized inline into outbound agent requests
//! (`Request.Settings.custom_auto_models`) mirroring the
//! `custom_model_providers` inline-registry pattern.
//!
//! Cloud/team custom autos are delivered separately, as `LLMInfo` entries in the
//! available-LLMs fetch (id = `custom-auto:cloud:<uid>`); at request time the
//! client reverses that id into a `cloud_uid` (see
//! `llms::LLMPreferences::custom_auto_models_for_request`).
//!
//! See `specs/custom-auto-models/PRODUCT.md` and `TECH.md`. Invariant numbers in
//! comments (e.g. "inv. 27") refer to the product spec.

use std::path::PathBuf;

/// Nested proto types live under the snake_cased parent message module, exactly
/// like `custom_model_providers::{CustomModelProvider, CustomModel}`.
use api::request::settings::custom_auto_models as proto;
use serde::{Deserialize, Serialize};
use warp_multi_agent_api as api;

use super::llms::{LLMContextWindow, LLMId, LLMInfo, LLMProvider, LLMUsageMetadata};

/// The `config_key` prefix shared by all custom auto models. Lets us recognize a
/// selection as a custom auto and distinguish it from concrete and built-in autos.
pub const CUSTOM_AUTO_PREFIX: &str = "custom-auto:";
/// The `config_key` prefix for *local* (YAML-authored) custom auto models.
pub const LOCAL_CUSTOM_AUTO_PREFIX: &str = "custom-auto:local:";
/// The `config_key` prefix for *cloud* custom auto models. Cloud autos arrive as
/// `LLMInfo` entries in the available-LLMs fetch with id `custom-auto:cloud:<uid>`.
pub const CLOUD_CUSTOM_AUTO_PREFIX: &str = "custom-auto:cloud:";

/// The routing strategy for a custom auto model. Exactly one is set (inv. 18).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CustomAutoRouting {
    /// Route by Warp-determined task complexity.
    Complexity(ComplexityRouting),
    /// Route by classifying the prompt against user-authored categories.
    Prompt(PromptRouting),
}

/// Complexity routing: each bucket maps to a concrete model id. The required
/// `default` is the catch-all used when a bucket is omitted or task complexity
/// cannot be determined. Omitted buckets fall back to `default` (inv. 3, 28).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComplexityRouting {
    /// The required catch-all model used when no bucket matches.
    pub default: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub easy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub medium: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hard: Option<String>,
}

/// Prompt routing: an ordered list of rules plus a required catch-all default
/// (inv. 29).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptRouting {
    /// The required catch-all model used when no rule matches.
    pub default_model: String,
    /// The ordered list of rules. May be empty (always routes to `default_model`).
    #[serde(default)]
    pub rules: Vec<PromptRule>,
}

/// A single prompt-routing rule: a free-text description paired with a concrete model.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptRule {
    pub description: String,
    pub model: String,
}

/// A local (YAML-authored) custom auto model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomAutoModel {
    /// User-facing name shown in the picker.
    pub name: String,
    /// The routing strategy + targets.
    pub routing: CustomAutoRouting,
}

impl CustomAutoModel {
    /// Builds a local (YAML-sourced) custom auto model.
    pub fn new_local(name: String, routing: CustomAutoRouting) -> Self {
        Self { name, routing }
    }

    /// The `config_key` that identifies this model in the picker (`LLMId`) and in
    /// the request registry (`ModelConfig.base`). Local autos are keyed by name
    /// (the unit of local-wins precedence, inv. 19).
    pub fn config_key(&self) -> String {
        format!("{LOCAL_CUSTOM_AUTO_PREFIX}{}", self.name)
    }

    /// The picker [`LLMId`] for this model (equal to its `config_key`).
    pub fn llm_id(&self) -> LLMId {
        LLMId::from(self.config_key())
    }

    /// Builds the synthetic [`LLMInfo`] picker entry for this custom auto model.
    ///
    /// Mirrors `llms::custom_llm_info_from`: provider `Unknown` and empty
    /// `host_configs` mark it as an auto/router-style entry rather than a concrete
    /// model. The `description` carries the source and routing strategy label shown
    /// in the model picker details panel.
    pub fn to_llm_info(&self) -> LLMInfo {
        let description = match &self.routing {
            CustomAutoRouting::Complexity(_) => "Locally defined · Routes by task complexity",
            CustomAutoRouting::Prompt(_) => "Locally defined · Routes by prompt content",
        };
        LLMInfo {
            display_name: self.name.clone(),
            base_model_name: self.name.clone(),
            id: self.llm_id(),
            reasoning_level: None,
            usage_metadata: LLMUsageMetadata {
                request_multiplier: 1,
                credit_multiplier: None,
            },
            description: Some(description.to_owned()),
            disable_reason: None,
            vision_supported: true,
            spec: None,
            provider: LLMProvider::Unknown,
            host_configs: Default::default(),
            discount_percentage: None,
            context_window: LLMContextWindow::default(),
        }
    }

    /// Builds the proto registry entry sent in `Request.Settings.custom_auto_models`.
    /// Local autos send the full definition inline every request. (Cloud autos are
    /// handled separately in `llms::LLMPreferences::custom_auto_models_for_request`,
    /// which sends just the `cloud_uid`.)
    pub fn to_proto(&self) -> proto::CustomAutoModel {
        proto::CustomAutoModel {
            config_key: self.config_key(),
            name: self.name.clone(),
            router: Some(self.to_proto_router()),
        }
    }

    fn to_proto_router(&self) -> proto::custom_auto_model::Router {
        match &self.routing {
            CustomAutoRouting::Complexity(c) => {
                proto::custom_auto_model::Router::Complexity(proto::ComplexityBasedRouter {
                    default: c.default.clone(),
                    easy: c.easy.clone().unwrap_or_default(),
                    medium: c.medium.clone().unwrap_or_default(),
                    hard: c.hard.clone().unwrap_or_default(),
                })
            }
            CustomAutoRouting::Prompt(p) => {
                proto::custom_auto_model::Router::Prompt(proto::PromptBasedRouter {
                    default: p.default_model.clone(),
                    rules: p
                        .rules
                        .iter()
                        .map(|r| proto::prompt_based_router::PromptRule {
                            rule: r.description.clone(),
                            model: r.model.clone(),
                        })
                        .collect(),
                })
            }
        }
    }

    /// Validates routing targets (inv. 27, 29).
    ///
    /// Availability/entitlement and unknown-model-id handling are NOT hard errors
    /// here — they are resolved by server-side fallback (inv. 26, 28). This only
    /// rejects structurally invalid definitions: routing to an auto model, empty
    /// required targets, or a prompt type missing its catch-all default.
    pub fn validate(&self) -> Result<(), String> {
        match &self.routing {
            CustomAutoRouting::Complexity(c) => {
                if c.default.trim().is_empty() {
                    return Err(
                        "complexity routing requires a non-empty `default` model".to_owned(),
                    );
                }
                validate_target(&c.default).map_err(|e| format!("`default`: {e}"))?;
                for (bucket, target) in
                    [("easy", &c.easy), ("medium", &c.medium), ("hard", &c.hard)]
                {
                    if let Some(model) = target {
                        validate_target(model)
                            .map_err(|e| format!("complexity bucket `{bucket}`: {e}"))?;
                    }
                }
            }
            CustomAutoRouting::Prompt(p) => {
                if p.default_model.trim().is_empty() {
                    // inv. 29: the catch-all default is required.
                    return Err("prompt routing requires a non-empty `default` model".to_owned());
                }
                validate_target(&p.default_model).map_err(|e| format!("`default`: {e}"))?;
                for (index, rule) in p.rules.iter().enumerate() {
                    if rule.description.trim().is_empty() {
                        return Err(format!("prompt rule {index}: `description` is empty"));
                    }
                    validate_target(&rule.model)
                        .map_err(|e| format!("prompt rule {index}: {e}"))?;
                }
            }
        }
        Ok(())
    }
}

/// Validates a single routing target id: non-empty and concrete (inv. 27).
fn validate_target(model_id: &str) -> Result<(), String> {
    let trimmed = model_id.trim();
    if trimmed.is_empty() {
        return Err("target model id is empty".to_owned());
    }
    if is_auto_target(trimmed) {
        return Err(format!(
            "target `{trimmed}` is an auto model; custom auto models must route to concrete models"
        ));
    }
    Ok(())
}

/// Returns whether a model id refers to an auto/router model (built-in or custom).
/// Custom auto models may not route to these (inv. 27).
pub fn is_auto_target(model_id: &str) -> bool {
    let id = model_id.trim();
    id == "auto"
        || id.starts_with("auto-")
        || id == "cli-agent-auto"
        || id == "computer-use-agent-auto"
        || is_custom_auto_id(id)
}

/// Returns whether an id is the `config_key`/`LLMId` of a custom auto model.
pub fn is_custom_auto_id(id: &str) -> bool {
    id.starts_with(CUSTOM_AUTO_PREFIX)
}

/// Returns whether an id is the `config_key` of a *local* custom auto model.
/// Used to reconcile stale local selections without touching cloud selections.
pub fn is_local_custom_auto_id(id: &str) -> bool {
    id.starts_with(LOCAL_CUSTOM_AUTO_PREFIX)
}

/// Extracts the GSO uid from a *cloud* custom-auto `config_key`
/// (`custom-auto:cloud:<uid>`), returning `None` if `id` is not a cloud auto.
///
/// Cloud autos arrive via the available-LLMs fetch; at request time we reverse
/// the id into a `cloud_uid` so the server can fetch + authorize the GSO.
pub fn cloud_uid_from_id(id: &str) -> Option<&str> {
    id.strip_prefix(CLOUD_CUSTOM_AUTO_PREFIX)
}

/// Describes a `model_configs/` YAML file that failed to parse or validate.
///
/// Mirrors `tab_configs::TabConfigError` so the parse error can surface as a
/// non-blocking toast naming the offending file (inv. 10).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelConfigError {
    /// The file name shown in the toast (e.g. `"my_models.yaml"`).
    pub file_name: String,
    /// Full path used by the "Open file" action.
    pub file_path: PathBuf,
    /// The full error from parsing/validation.
    pub error_message: String,
}

// ── Local YAML authoring shape (PRODUCT §8) ──────────────────────────────────

/// The top-level shape of a `model_configs/*.yaml` file.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct YamlModelConfigsFile {
    #[serde(default)]
    custom_auto_models: Vec<YamlCustomAutoModel>,
}

/// A single custom auto model as authored in YAML.
///
/// `routing` is polymorphic by `type` (a mapping for complexity, a list for
/// prompt) and `default` is a sibling used only by prompt, so it is parsed as a
/// raw value and interpreted in [`YamlCustomAutoModel::into_domain`].
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct YamlCustomAutoModel {
    name: String,
    #[serde(rename = "type")]
    model_type: String,
    #[serde(default)]
    default: Option<String>,
    #[serde(default)]
    routing: serde_yaml::Value,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct YamlComplexityRouting {
    #[serde(default)]
    easy: Option<String>,
    #[serde(default)]
    medium: Option<String>,
    #[serde(default)]
    hard: Option<String>,
}

impl YamlCustomAutoModel {
    fn into_domain(self) -> Result<CustomAutoModel, String> {
        let name = self.name.trim().to_owned();
        if name.is_empty() {
            return Err("custom auto model `name` is empty".to_owned());
        }
        let routing = match self.model_type.as_str() {
            "complexity" => {
                let default_model = self
                    .default
                    .map(|d| d.trim().to_owned())
                    .filter(|d| !d.is_empty())
                    // complexity also requires a catch-all default.
                    .ok_or_else(|| {
                        format!("`{name}`: complexity type requires a `default` model")
                    })?;
                let routing: YamlComplexityRouting = if self.routing.is_null() {
                    YamlComplexityRouting::default()
                } else {
                    serde_yaml::from_value(self.routing)
                        .map_err(|e| format!("`{name}`: invalid complexity routing: {e}"))?
                };
                CustomAutoRouting::Complexity(ComplexityRouting {
                    default: default_model,
                    easy: normalize_target(routing.easy),
                    medium: normalize_target(routing.medium),
                    hard: normalize_target(routing.hard),
                })
            }
            "prompt" => {
                let default_model = self
                    .default
                    .map(|d| d.trim().to_owned())
                    .filter(|d| !d.is_empty())
                    // inv. 29: prompt requires a catch-all default.
                    .ok_or_else(|| format!("`{name}`: prompt type requires a `default` model"))?;
                let rules: Vec<YamlPromptRule> = if self.routing.is_null() {
                    Vec::new()
                } else {
                    serde_yaml::from_value(self.routing)
                        .map_err(|e| format!("`{name}`: invalid prompt routing: {e}"))?
                };
                CustomAutoRouting::Prompt(PromptRouting {
                    default_model,
                    rules: rules
                        .into_iter()
                        .map(|r| PromptRule {
                            description: r.description.trim().to_owned(),
                            model: r.model.trim().to_owned(),
                        })
                        .collect(),
                })
            }
            other => {
                return Err(format!(
                    "`{name}`: unknown type `{other}` (expected `complexity` or `prompt`)"
                ));
            }
        };

        let model = CustomAutoModel::new_local(name, routing);
        model.validate()?;
        Ok(model)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct YamlPromptRule {
    description: String,
    model: String,
}

/// Trims a YAML target and drops empties (so an omitted/blank bucket becomes `None`).
fn normalize_target(value: Option<String>) -> Option<String> {
    value.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty())
}

/// Parses the contents of a single `model_configs/*.yaml` file into local custom
/// auto models.
///
/// Returns an error (which the caller surfaces per-file, inv. 10) if the YAML is
/// invalid or any model in the file fails validation; on success returns every
/// model defined in the file (a file may define multiple, inv. 7).
pub fn parse_model_configs_yaml(contents: &str) -> Result<Vec<CustomAutoModel>, String> {
    let file: YamlModelConfigsFile =
        serde_yaml::from_str(contents).map_err(|e| format!("invalid YAML: {e}"))?;
    file.custom_auto_models
        .into_iter()
        .map(YamlCustomAutoModel::into_domain)
        .collect()
}
