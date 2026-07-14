//! Plain-data option lists both frontends render from. Builders mirror
//! the GUI picker population logic exactly; each is a thin
//! `AppContext`-reading wrapper over a pure core so the mirroring logic
//! is unit-testable without app singletons.

use ai::agent::action::RunAgentsExecutionMode;
use warp_cli::agent::Harness;
use warpui::{AppContext, SingletonEntity};

use super::config_state::{AuthSecretSelection, OrchestrationConfigState};
use super::providers::{
    get_base_model_choices, resolve_default_host_slug, resolve_recent_host_slug,
    ORCHESTRATION_ENV_NONE_LABEL, ORCHESTRATION_WARP_WORKER_HOST,
};
use crate::ai::auth_secret_types::auth_secret_types_for_harness;
use crate::ai::cloud_environments::CloudAmbientAgentEnvironment;
use crate::ai::connected_self_hosted_workers::ConnectedSelfHostedWorkersModel;
use crate::ai::harness_availability::{AuthSecretFetchState, HarnessAvailabilityModel};
use crate::ai::harness_display;
use crate::ai::local_harness_setup::{
    local_harness_is_product_enabled, local_harness_setup_state, LocalHarnessSetupState,
};
use crate::cloud_object::CloudObjectLookup as _;
use crate::LLMPreferences;

const DEFAULT_MODEL_LABEL: &str = "Default model";
/// Label shown in the auth secret picker when no secret is selected
/// (the child agent will inherit credentials from its environment).
pub(crate) const AUTH_SECRET_INHERIT_LABEL: &str = "Skip (advanced)";
const CUSTOM_HOST_LABEL: &str = "Custom host…";
const AUTH_SECRETS_LOAD_FAILED_MESSAGE: &str = "Unable to load secrets";

/// Row id for the Cloud location option.
#[cfg_attr(not(feature = "tui"), allow(dead_code))]
pub const LOCATION_CLOUD_ID: &str = "cloud";
/// Row id for the Local location option.
#[cfg_attr(not(feature = "tui"), allow(dead_code))]
pub const LOCATION_LOCAL_ID: &str = "local";

/// One selectable row in an option snapshot. Carries no GUI types.
#[derive(Debug, Clone, PartialEq)]
pub struct OptionRow {
    pub id: String,
    pub label: String,
    /// Harness identifier for rows representing harnesses; each frontend
    /// maps it to its own icon (GUI `Icon`) or glyph/color (TUI).
    pub harness: Option<Harness>,
    pub badge: Option<OptionBadge>,
    pub disabled_reason: Option<String>,
}

impl OptionRow {
    /// Creates an enabled row with no badge or harness.
    fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            harness: None,
            badge: None,
            disabled_reason: None,
        }
    }
}

/// Secondary marker rendered next to a row's label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionBadge {
    Default,
    Recent,
    Connected,
}

/// Load state of the catalog backing a snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OptionSourceStatus {
    Ready,
    Loading,
    Failed { message: String },
    Empty { message: String },
}

/// Trailing affordance below the option list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OptionFooter {
    /// Free-form text entry (e.g. custom host slug).
    CustomText { label: String },
    /// "New API key…" affordance. Emitted for GUI parity; the TUI ignores
    /// it since resource creation is out of scope there.
    CreateNewAuthSecret,
}

/// A complete option list for one configuration field.
#[derive(Debug, Clone, PartialEq)]
pub struct OptionSnapshot {
    pub rows: Vec<OptionRow>,
    pub selected_id: Option<String>,
    pub status: OptionSourceStatus,
    pub footer: Option<OptionFooter>,
}

impl OptionSnapshot {
    /// A `Ready` snapshot with no footer.
    fn ready(rows: Vec<OptionRow>, selected_id: Option<String>) -> Self {
        Self {
            rows,
            selected_id,
            status: OptionSourceStatus::Ready,
            footer: None,
        }
    }
}

// ── Location ────────────────────────────────────────────────────────

/// Builds the Cloud/Local location options with the current mode selected.
// Only the TUI renders a location page (via `tui_export`); the GUI has
// its own Cloud/Local mode toggle.
#[cfg_attr(not(feature = "tui"), allow(dead_code))]
pub fn location_snapshot(state: &OrchestrationConfigState, _ctx: &AppContext) -> OptionSnapshot {
    let rows = vec![
        OptionRow::new(LOCATION_CLOUD_ID, "Cloud"),
        OptionRow::new(LOCATION_LOCAL_ID, "Local"),
    ];
    let selected = if state.execution_mode.is_remote() {
        LOCATION_CLOUD_ID
    } else {
        LOCATION_LOCAL_ID
    };
    OptionSnapshot::ready(rows, Some(selected.to_string()))
}

// ── Harness ─────────────────────────────────────────────────────────

/// Server-provided harness entry decoupled from `HarnessAvailability` so
/// the pure row builder can be tested directly.
struct HarnessEntryInput {
    harness: Harness,
    display_name: String,
    enabled: bool,
}

/// Builds the harness options, mirroring the GUI's
/// `populate_harness_picker` filtering, ordering, disabled reasons, and
/// selection matching.
pub fn harness_snapshot(state: &OrchestrationConfigState, ctx: &AppContext) -> OptionSnapshot {
    let is_local = !state.execution_mode.is_remote();
    let availability = HarnessAvailabilityModel::as_ref(ctx);
    let entries: Vec<HarnessEntryInput> = availability
        .available_harnesses()
        .iter()
        .map(|entry| HarnessEntryInput {
            harness: entry.harness,
            display_name: entry.display_name.clone(),
            enabled: entry.enabled,
        })
        .collect();
    let target_display = Harness::parse_orchestration_harness(&state.harness_type)
        .map(|harness| availability.display_name_for(harness).to_string());
    build_harness_snapshot(
        entries,
        &state.harness_type,
        target_display,
        is_local,
        &local_harness_setup_state,
    )
}

/// Pure core of [`harness_snapshot`]. `setup_state_for` is injected so
/// tests don't depend on locally installed CLIs.
fn build_harness_snapshot(
    entries: Vec<HarnessEntryInput>,
    initial_harness: &str,
    target_display: Option<String>,
    is_local: bool,
    setup_state_for: &dyn Fn(Harness) -> LocalHarnessSetupState,
) -> OptionSnapshot {
    let resolve_entry_harness = |harness: Harness, display_name: &str| match harness {
        Harness::Unknown => [
            Harness::Oz,
            Harness::Claude,
            Harness::OpenCode,
            Harness::Gemini,
            Harness::Codex,
        ]
        .into_iter()
        .find(|candidate| harness_display::display_name(*candidate) == display_name)
        .unwrap_or(Harness::Unknown),
        harness => harness,
    };
    let setup_is_ready = |harness: Harness| !is_local || setup_state_for(harness).is_selectable();

    // Sort selectable harnesses before disabled ones, preserving
    // relative order within each group.
    // Filter out Gemini — it is not yet supported as a multi-agent
    // harness and causes an infinite "Spawning agents" hang.
    let mut sorted: Vec<_> = entries
        .iter()
        .filter(|entry| {
            let harness = resolve_entry_harness(entry.harness, &entry.display_name);
            harness != Harness::Gemini && (!is_local || local_harness_is_product_enabled(harness))
        })
        .collect();
    sorted.sort_by_key(|entry| {
        let harness = resolve_entry_harness(entry.harness, &entry.display_name);
        !(entry.enabled && setup_is_ready(harness))
    });

    let mut rows: Vec<OptionRow> = Vec::new();
    let mut selected_id: Option<String> = None;
    for entry in sorted {
        let harness = resolve_entry_harness(entry.harness, &entry.display_name);
        let harness_str = harness.to_string();
        let selectable = entry.enabled && setup_is_ready(harness);
        let disabled_reason = if selectable {
            None
        } else {
            let local_setup_state = if is_local {
                Some(setup_state_for(harness))
            } else {
                None
            };
            Some(
                match local_setup_state {
                    Some(LocalHarnessSetupState::MissingHarness { tooltip }) => tooltip,
                    Some(LocalHarnessSetupState::ProductDisabled { message }) => message,
                    Some(LocalHarnessSetupState::Ready) | None => "Disabled by your administrator",
                }
                .to_string(),
            )
        };
        // Match by harness string first, then fall back to matching
        // the display_name against the client-side name for the target
        // harness. This handles stale cache entries where entry.harness
        // is Unknown but entry.display_name is still correct.
        if selected_id.is_none() {
            if harness_str.eq_ignore_ascii_case(initial_harness) {
                selected_id = Some(harness_str.clone());
            } else if let Some(target_display) = &target_display {
                if &entry.display_name == target_display {
                    selected_id = Some(harness_str.clone());
                }
            }
        }
        rows.push(OptionRow {
            id: harness_str,
            // Use the server-provided display_name for the label so stale
            // cache entries (where harness deserializes as Unknown) still
            // show the correct name.
            label: entry.display_name.clone(),
            harness: Some(harness),
            badge: None,
            disabled_reason,
        });
    }
    if rows.is_empty() {
        return OptionSnapshot {
            rows,
            selected_id,
            status: OptionSourceStatus::Empty {
                message: "No harnesses available".to_string(),
            },
            footer: None,
        };
    }
    OptionSnapshot::ready(rows, selected_id)
}

// ── Model ───────────────────────────────────────────────────────────

/// A model choice already resolved to plain strings.
struct ModelChoiceInput {
    id: String,
    label: String,
}

/// Builds the model options for the active harness, mirroring the GUI's
/// `populate_model_picker_for_harness` three catalog branches:
/// - **Oz / empty**: the Warp LLM catalog (auto, then custom, then other
///   models; custom models excluded for Cloud runs).
/// - **Local Codex**: only a "Default model" entry.
/// - **Other non-Oz harnesses**: "Default model" plus the server-provided
///   harness model catalog.
pub fn model_snapshot(state: &OrchestrationConfigState, ctx: &AppContext) -> OptionSnapshot {
    let is_local = !state.execution_mode.is_remote();
    let harness = Harness::parse_orchestration_harness(&state.harness_type);
    match harness {
        Some(Harness::Oz) | None => {
            // Oz / unset: Warp LLM catalog. Custom models excluded for
            // cloud runs (not supported by remote workers).
            // Order: auto models first, then custom models, then other models.
            let llm_prefs = LLMPreferences::as_ref(ctx);
            let (auto_models, rest): (Vec<_>, Vec<_>) =
                get_base_model_choices(llm_prefs, ctx, is_local)
                    .partition(|llm| llm.id.as_str().starts_with("auto"));
            let (custom_models, other_models): (Vec<_>, Vec<_>) = rest
                .into_iter()
                .partition(|llm| llm_prefs.custom_llm_info_for_id(&llm.id).is_some());
            let choices: Vec<ModelChoiceInput> = auto_models
                .into_iter()
                .chain(custom_models)
                .chain(other_models)
                .map(|llm| ModelChoiceInput {
                    id: llm.id.to_string(),
                    label: llm.menu_display_name(),
                })
                .collect();
            build_oz_model_snapshot(choices, &state.model_id)
        }
        Some(Harness::Codex) if is_local => {
            // Local Codex: only "Default model" entry.
            OptionSnapshot::ready(
                vec![OptionRow::new(String::new(), DEFAULT_MODEL_LABEL)],
                Some(String::new()),
            )
        }
        Some(harness) => {
            let models = HarnessAvailabilityModel::as_ref(ctx)
                .models_for(harness)
                .map(|models| {
                    models
                        .iter()
                        .map(|model| ModelChoiceInput {
                            id: model.id.clone(),
                            label: model.display_name.clone(),
                        })
                        .collect::<Vec<_>>()
                });
            build_non_oz_model_snapshot(models, &state.model_id)
        }
    }
}

/// Pure core for the Oz / unset branch of [`model_snapshot`].
fn build_oz_model_snapshot(
    choices: Vec<ModelChoiceInput>,
    initial_model_id: &str,
) -> OptionSnapshot {
    let selected_id = choices
        .iter()
        .find(|choice| choice.id == initial_model_id)
        .map(|choice| choice.id.clone());
    let rows: Vec<OptionRow> = choices
        .into_iter()
        .map(|choice| OptionRow::new(choice.id, choice.label))
        .collect();
    if rows.is_empty() {
        return OptionSnapshot {
            rows,
            selected_id,
            status: OptionSourceStatus::Empty {
                message: "No models available".to_string(),
            },
            footer: None,
        };
    }
    OptionSnapshot::ready(rows, selected_id)
}

/// Pure core for the non-Oz branch of [`model_snapshot`]: "Default model"
/// on top, then server-provided models. Unknown or empty selections fall
/// back to "Default model" (empty id).
fn build_non_oz_model_snapshot(
    models: Option<Vec<ModelChoiceInput>>,
    initial_model_id: &str,
) -> OptionSnapshot {
    let mut rows = vec![OptionRow::new(String::new(), DEFAULT_MODEL_LABEL)];
    let mut found_initial = false;
    for model in models.into_iter().flatten() {
        if model.id == initial_model_id {
            found_initial = true;
        }
        rows.push(OptionRow::new(model.id, model.label));
    }
    let selected_id = if !initial_model_id.is_empty() && found_initial {
        Some(initial_model_id.to_string())
    } else {
        Some(String::new())
    };
    OptionSnapshot::ready(rows, selected_id)
}

// ── API key ─────────────────────────────────────────────────────────

/// Fetch state reduced to plain data for the pure API-key builder.
enum AuthSecretNamesInput {
    NotLoaded,
    Loaded(Vec<String>),
    Failed,
}

/// Builds the API-key options: "Skip (advanced)" (inherit) plus loaded
/// managed-secret names. Secret values are never included — names only.
/// Status mirrors `AuthSecretFetchState`; the `CreateNewAuthSecret`
/// footer is emitted for harnesses with managed-secret types.
pub fn api_key_snapshot(state: &OrchestrationConfigState, ctx: &AppContext) -> OptionSnapshot {
    let Some(harness) = Harness::parse_orchestration_harness(&state.harness_type) else {
        return OptionSnapshot::ready(Vec::new(), None);
    };
    if harness == Harness::Oz {
        return OptionSnapshot::ready(Vec::new(), None);
    }
    let names = match HarnessAvailabilityModel::as_ref(ctx).auth_secrets_for(harness) {
        AuthSecretFetchState::Loaded(secrets) => {
            AuthSecretNamesInput::Loaded(secrets.iter().map(|s| s.name.clone()).collect())
        }
        AuthSecretFetchState::NotFetched | AuthSecretFetchState::Loading => {
            AuthSecretNamesInput::NotLoaded
        }
        AuthSecretFetchState::Failed(_) => AuthSecretNamesInput::Failed,
    };
    let supports_create_new = !auth_secret_types_for_harness(harness).is_empty();
    build_api_key_snapshot(names, &state.auth_secret_selection, supports_create_new)
}

/// Pure core of [`api_key_snapshot`].
fn build_api_key_snapshot(
    names: AuthSecretNamesInput,
    selection: &AuthSecretSelection,
    supports_create_new: bool,
) -> OptionSnapshot {
    let mut rows = vec![OptionRow::new(String::new(), AUTH_SECRET_INHERIT_LABEL)];
    let status = match names {
        AuthSecretNamesInput::Loaded(names) => {
            for name in names {
                rows.push(OptionRow::new(name.clone(), name));
            }
            OptionSourceStatus::Ready
        }
        AuthSecretNamesInput::NotLoaded => OptionSourceStatus::Loading,
        AuthSecretNamesInput::Failed => OptionSourceStatus::Failed {
            message: AUTH_SECRETS_LOAD_FAILED_MESSAGE.to_string(),
        },
    };
    // The selection derives directly from the edit state. `Named` is kept
    // even while the catalog is loading so a transient refresh never
    // clears it; `Unset`/`CreatingNew` have no selected row.
    let selected_id = match selection {
        AuthSecretSelection::Named(name) => Some(name.clone()),
        AuthSecretSelection::Inherit => Some(String::new()),
        AuthSecretSelection::Unset | AuthSecretSelection::CreatingNew => None,
    };
    OptionSnapshot {
        rows,
        selected_id,
        status,
        footer: supports_create_new.then_some(OptionFooter::CreateNewAuthSecret),
    }
}

// ── Host ────────────────────────────────────────────────────────────

/// Builds the host options in the GUI host picker's order: workspace
/// default (badged), warp, connected worker hosts (badged), the recent
/// custom slug (badged), then a custom-host text-entry footer.
pub fn host_snapshot(state: &OrchestrationConfigState, ctx: &AppContext) -> OptionSnapshot {
    let default_host = resolve_default_host_slug(ctx);
    let recent_host = resolve_recent_host_slug(ctx);
    let mut connected_hosts = ConnectedSelfHostedWorkersModel::as_ref(ctx)
        .worker_hosts_excluding(default_host.as_deref());
    connected_hosts.sort();
    connected_hosts.dedup();
    let current = match &state.execution_mode {
        RunAgentsExecutionMode::Remote { worker_host, .. } if !worker_host.trim().is_empty() => {
            worker_host.trim().to_string()
        }
        RunAgentsExecutionMode::Remote { .. } | RunAgentsExecutionMode::Local => {
            ORCHESTRATION_WARP_WORKER_HOST.to_string()
        }
    };
    build_host_snapshot(default_host, recent_host, connected_hosts, &current)
}

/// Pure core of [`host_snapshot`].
fn build_host_snapshot(
    default_host: Option<String>,
    recent_host: Option<String>,
    connected_hosts: Vec<String>,
    current: &str,
) -> OptionSnapshot {
    let mut rows: Vec<OptionRow> = Vec::new();
    let mut known_slugs: Vec<String> = Vec::new();
    if let Some(slug) = default_host.filter(|s| !s.trim().is_empty()) {
        rows.push(OptionRow {
            badge: Some(OptionBadge::Default),
            ..OptionRow::new(slug.clone(), slug.clone())
        });
        known_slugs.push(slug);
    }
    rows.push(OptionRow::new(
        ORCHESTRATION_WARP_WORKER_HOST,
        ORCHESTRATION_WARP_WORKER_HOST,
    ));
    known_slugs.push(ORCHESTRATION_WARP_WORKER_HOST.to_string());

    for slug in connected_hosts {
        if slug.trim().is_empty()
            || known_slugs
                .iter()
                .any(|known| known.eq_ignore_ascii_case(&slug))
        {
            continue;
        }
        rows.push(OptionRow {
            badge: Some(OptionBadge::Connected),
            ..OptionRow::new(slug.clone(), slug.clone())
        });
        known_slugs.push(slug);
    }
    if let Some(slug) = recent_host.filter(|s| !s.trim().is_empty()) {
        if !known_slugs
            .iter()
            .any(|known| known.eq_ignore_ascii_case(&slug))
        {
            rows.push(OptionRow {
                badge: Some(OptionBadge::Recent),
                ..OptionRow::new(slug.clone(), slug)
            });
        }
    }
    OptionSnapshot {
        rows,
        selected_id: Some(current.to_string()),
        status: OptionSourceStatus::Ready,
        footer: Some(OptionFooter::CustomText {
            label: CUSTOM_HOST_LABEL.to_string(),
        }),
    }
}

// ── Environment ─────────────────────────────────────────────────────

/// Builds the environment options: "Empty environment" plus existing
/// environments sorted by name, mirroring the GUI environment picker.
pub fn environment_snapshot(state: &OrchestrationConfigState, ctx: &AppContext) -> OptionSnapshot {
    let all_envs = CloudAmbientAgentEnvironment::get_all(ctx);
    let mut sorted_envs: Vec<(String, String)> = all_envs
        .iter()
        .map(|env| (env.id.uid(), env.model().string_model.name.clone()))
        .collect();
    sorted_envs.sort_by(|a, b| a.1.cmp(&b.1));
    let current = match &state.execution_mode {
        RunAgentsExecutionMode::Remote { environment_id, .. } => environment_id.clone(),
        RunAgentsExecutionMode::Local => String::new(),
    };
    build_environment_snapshot(sorted_envs, &current)
}

/// Pure core of [`environment_snapshot`]; `envs` must already be sorted
/// by display name.
fn build_environment_snapshot(envs: Vec<(String, String)>, current: &str) -> OptionSnapshot {
    let mut rows = vec![OptionRow::new(String::new(), ORCHESTRATION_ENV_NONE_LABEL)];
    let mut selected_id = current.is_empty().then(String::new);
    for (env_id, env_name) in envs {
        if env_id == current {
            selected_id = Some(env_id.clone());
        }
        rows.push(OptionRow::new(env_id, env_name));
    }
    OptionSnapshot::ready(rows, selected_id)
}

#[cfg(test)]
#[path = "snapshots_tests.rs"]
mod tests;
