//! Custom auto models: user-defined "auto" models (named routers) that resolve to
//! a concrete model per task.
//!
//! This module holds the portable definition for **local** (YAML-authored) custom
//! auto models: each file under `~/.warp/custom_model_routers/` defines exactly
//! one router (see [`parse_model_config_yaml`]), surfaced in the model picker as
//! synthetic [`LLMInfo`] entries, and serialized inline into outbound agent
//! requests (`Request.Settings.custom_model_routers`).
//!
//! Cloud/team custom routers arrive as regular `LLMInfo` entries in the
//! available-LLMs fetch with their own server-assigned IDs, and do not need a
//! client-side registry entry.

use std::path::{Path, PathBuf};

/// Nested proto types live under the snake_cased parent message module, exactly
/// like `custom_model_providers::{CustomModelProvider, CustomModel}`.
use api::request::settings::custom_model_routers as proto;
use serde::{Deserialize, Serialize};
use warp_multi_agent_api as api;

use super::llms::{LLMContextWindow, LLMId, LLMInfo, LLMProvider, LLMUsageMetadata};

/// The `config_key` prefix for local (YAML-authored) custom model routers.
pub const LOCAL_CUSTOM_ROUTER_PREFIX: &str = "custom-router:local:";

/// The routing strategy for a custom model router.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CustomModelRouting {
    /// Route by Warp-determined task complexity.
    Complexity(ComplexityRouting),
    /// Route by classifying the prompt against user-authored categories.
    Prompt(PromptRouting),
}

/// Complexity routing: each bucket maps to a concrete model id. The required
/// `default` is the catch-all used when a bucket is omitted or task complexity
/// cannot be determined. Omitted buckets fall back to `default`.
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

/// Prompt routing: each rule maps to a model that should be used for
/// prompts that match that rule.
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

/// A local (YAML-authored) custom model router. Bundles the picker display
/// info and the routing definition together so `LLMPreferences` only needs one
/// collection rather than two parallel vecs.
#[derive(Clone, Debug, PartialEq)]
pub struct CustomModelRouter {
    pub info: LLMInfo,
    pub routing: CustomModelRouting,
    /// The file this router was loaded from. Used to surface validation errors
    /// (e.g. unknown target model IDs) as toasts with an "Open file" link.
    pub source_path: Option<PathBuf>,
}

impl CustomModelRouter {
    /// Builds a local (YAML-sourced) custom model router, computing the
    /// picker [`LLMInfo`] inline so callers never need to call a separate
    /// `to_llm_info()` step.
    ///
    /// `source_path` is the file the router was loaded from; when provided it
    /// appears in the description so the user knows where to edit the config.
    pub fn new_local(
        name: String,
        routing: CustomModelRouting,
        source_path: Option<&Path>,
    ) -> Self {
        let config_key = format!("{LOCAL_CUSTOM_ROUTER_PREFIX}{name}");
        let routing_kind = match &routing {
            CustomModelRouting::Complexity(_) => "Routes by task complexity",
            CustomModelRouting::Prompt(_) => "Routes by prompt content",
        };
        let description = match source_path {
            Some(path) => {
                format!(
                    "{routing_kind} · {}",
                    warp_core::paths::home_relative_path(path)
                )
            }
            None => routing_kind.to_owned(),
        };
        let info = LLMInfo {
            display_name: name.clone(),
            base_model_name: name,
            id: config_key.into(),
            reasoning_level: None,
            usage_metadata: LLMUsageMetadata {
                request_multiplier: 1,
                credit_multiplier: None,
            },
            description: Some(description),
            disable_reason: None,
            vision_supported: true,
            spec: None,
            provider: LLMProvider::Unknown,
            host_configs: Default::default(),
            discount_percentage: None,
            context_window: LLMContextWindow::default(),
        };
        Self {
            info,
            routing,
            source_path: source_path.map(|p| p.to_path_buf()),
        }
    }

    /// The `config_key` that identifies this router in the picker and request
    /// registry. Equal to `info.id`.
    pub fn config_key(&self) -> String {
        self.info.id.as_str().to_owned()
    }

    /// The picker [`LLMId`] for this router (equal to its `config_key`).
    pub fn llm_id(&self) -> LLMId {
        self.info.id.clone()
    }

    /// Returns all routing target model IDs defined in this router (required
    /// defaults and any optional bucket/rule targets that are set). Used to
    /// validate that every target is a known concrete model.
    pub fn all_targets(&self) -> Vec<&str> {
        match &self.routing {
            CustomModelRouting::Complexity(c) => std::iter::once(c.default.as_str())
                .chain(c.easy.as_deref())
                .chain(c.medium.as_deref())
                .chain(c.hard.as_deref())
                .collect(),
            CustomModelRouting::Prompt(p) => std::iter::once(p.default_model.as_str())
                .chain(p.rules.iter().map(|r| r.model.as_str()))
                .collect(),
        }
    }

    /// Builds the proto registry entry sent in `Request.Settings.custom_model_routers`.
    /// The full routing definition is sent inline with every request.
    pub fn to_proto(&self) -> proto::CustomModelRouter {
        let router = match &self.routing {
            CustomModelRouting::Complexity(c) => {
                proto::custom_model_router::Router::Complexity(proto::ComplexityBasedRouter {
                    default: c.default.clone(),
                    easy: c.easy.clone().unwrap_or_default(),
                    medium: c.medium.clone().unwrap_or_default(),
                    hard: c.hard.clone().unwrap_or_default(),
                })
            }
            CustomModelRouting::Prompt(p) => {
                proto::custom_model_router::Router::Prompt(proto::PromptBasedRouter {
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
        };

        proto::CustomModelRouter {
            config_key: self.config_key(),
            name: self.info.display_name.clone(),
            router: Some(router),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        match &self.routing {
            CustomModelRouting::Complexity(c) => {
                if c.default.trim().is_empty() {
                    return Err(
                        "complexity routing requires a non-empty `default` model".to_owned()
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
            CustomModelRouting::Prompt(p) => {
                if p.default_model.trim().is_empty() {
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

/// Validates a single routing target id: non-empty and concrete.
fn validate_target(model_id: &str) -> Result<(), String> {
    let trimmed = model_id.trim();
    if trimmed.is_empty() {
        return Err("target model id is empty".to_owned());
    }
    if is_auto_target(trimmed) {
        return Err(format!(
            "target `{trimmed}` is an auto model; custom model routers must route to concrete models"
        ));
    }
    Ok(())
}

/// Returns whether a model id refers to an auto/router model (built-in or custom).
/// Custom auto models may not route to these.
pub fn is_auto_target(model_id: &str) -> bool {
    let id = model_id.trim();
    id == "auto"
        || id.starts_with("auto-")
        || id == "cli-agent-auto"
        || id == "computer-use-agent-auto"
        || is_custom_router_id(id)
}

/// Returns whether an id is the `config_key`/`LLMId` of a local custom model router.
pub fn is_custom_router_id(id: &str) -> bool {
    id.starts_with(LOCAL_CUSTOM_ROUTER_PREFIX)
}

/// Returns whether an id is the `config_key` of a local custom model router.
/// Alias for [`is_custom_router_id`]; prefer that in new call sites.
pub fn is_local_custom_router_id(id: &str) -> bool {
    is_custom_router_id(id)
}

/// Describes a `custom_model_routers/` YAML file that failed to parse or validate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelConfigError {
    /// The file name shown in the toast (e.g. `"my_router.yaml"`).
    pub file_name: String,
    /// Full path used by the "Open file" action.
    pub file_path: PathBuf,
    /// The full error from parsing/validation.
    pub error_message: String,
}

// ── Local YAML authoring shape (PRODUCT §8) ──────────────────────────────────

/// A single custom model router as authored in YAML. Each
/// `custom_model_routers/*.yaml` file defines exactly one router at the top level.
///
/// `routing` is polymorphic by `type` (a mapping for complexity, a list for
/// prompt) and `default` is a sibling used only by prompt, so it is parsed as a
/// raw value and interpreted in [`YamlCustomModelRouter::into_domain`].
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct YamlCustomModelRouter {
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

impl YamlCustomModelRouter {
    fn into_domain(self, source_path: Option<&Path>) -> Result<CustomModelRouter, String> {
        let name = self.name.trim().to_owned();
        if name.is_empty() {
            return Err("custom model router `name` is empty".to_owned());
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
                CustomModelRouting::Complexity(ComplexityRouting {
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
                    .ok_or_else(|| format!("`{name}`: prompt type requires a `default` model"))?;
                let rules: Vec<YamlPromptRule> = if self.routing.is_null() {
                    Vec::new()
                } else {
                    serde_yaml::from_value(self.routing)
                        .map_err(|e| format!("`{name}`: invalid prompt routing: {e}"))?
                };
                CustomModelRouting::Prompt(PromptRouting {
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

        let model = CustomModelRouter::new_local(name, routing, source_path);
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

/// Parses the contents of a single custom model router file (one router per file).
///
/// `source_path` is the file the content came from; when provided it is embedded
/// in the router's description so the user can find and edit the file.
///
/// Returns an error if the YAML is invalid or the router fails validation; on
/// success returns the single router defined in the file.
pub fn parse_model_config_yaml(
    contents: &str,
    source_path: Option<&Path>,
) -> Result<CustomModelRouter, String> {
    let router: YamlCustomModelRouter =
        serde_yaml::from_str(contents).map_err(|e| format!("invalid YAML: {e}"))?;
    router.into_domain(source_path)
}
