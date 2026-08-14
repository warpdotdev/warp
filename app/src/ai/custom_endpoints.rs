//! Selects the persistence backend for custom LLM endpoint definitions and
//! bridges `AISettings::custom_endpoints` into `ApiKeyManager`.
//!
//! GUI v1 keeps its existing monolithic `AiApiKeys` custom-endpoint storage.
//! The TUI stores only definitions in `settings.toml`; API keys move to a
//! second secure-storage entry (`CustomEndpointApiKeys`) in the active
//! surface's existing secure-storage service. All parsing, validation,
//! identity derivation, and join/reconciliation logic lives in
//! `crates/ai/src/custom_endpoints.rs`; this module only selects the source
//! and forwards `AISettings` changes into `ApiKeyManager`.

use ai::api_keys::{ApiKeyManager, CustomEndpointPersistenceMode};
use settings::Setting as _;
use warpui_core::{ModelContext, SingletonEntity as _};

use crate::LaunchMode;
use crate::settings::{AISettings, AISettingsChangedEvent};

/// Selects the authoritative persistence backend for custom endpoint
/// definitions and credentials, following the execution-profile precedent
/// (`ProfileSource`): one shared model, a surface-selected persistence
/// source, and no scattered `LaunchMode` checks outside this selection point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CustomEndpointSource {
    /// Complete `CustomEndpoint` values (definition + key) live together in
    /// the monolithic `AiApiKeys` secure-storage blob. Used by the GUI and
    /// test launches in v1.
    LegacySecureBlob,
    /// Endpoint definitions live in `AISettings::custom_endpoints`; only the
    /// API key is stored, in `CustomEndpointApiKeys`. Used by the TUI.
    SettingsCollection,
}

impl CustomEndpointSource {
    /// Resolves the persistence backend for this launch.
    pub fn for_launch_mode(launch_mode: &LaunchMode) -> Self {
        match launch_mode {
            LaunchMode::Tui { .. } => Self::SettingsCollection,
            LaunchMode::App { .. }
            | LaunchMode::Test { .. }
            | LaunchMode::CommandLine { .. }
            | LaunchMode::RemoteServerProxy
            | LaunchMode::RemoteServerDaemon { .. } => Self::LegacySecureBlob,
        }
    }

    pub fn persistence_mode(self) -> CustomEndpointPersistenceMode {
        match self {
            Self::LegacySecureBlob => CustomEndpointPersistenceMode::Monolithic,
            Self::SettingsCollection => CustomEndpointPersistenceMode::Split,
        }
    }
}

/// Bridges `AISettings::custom_endpoints` into `ApiKeyManager` for
/// `SettingsCollection`-sourced launches. Kept separate from
/// `app/src/ai/tui_api_keys.rs`, which is limited to TUI cross-process
/// revision-file notification.
pub(crate) trait CustomEndpointDefinitionsCoordinator {
    /// Subscribes to settings changes and seeds the initial definitions
    /// (`SettingsCollection` source only; a no-op otherwise). Takes the
    /// already-resolved [`CustomEndpointSource`] rather than a `LaunchMode` so
    /// test harnesses that construct `ApiKeyManager` directly in `Split` mode
    /// (with no `LaunchMode` on hand) can wire the same bridge.
    ///
    /// `startup_settings_parse_succeeded` reports whether `settings.toml`
    /// parsed as valid TOML on this launch (regardless of whether
    /// `custom_endpoints` itself is present) — see the seeding comment below
    /// for why this, and not merely "is the value present", decides whether an
    /// absent value is treated as authoritative.
    fn subscribe_to_custom_endpoint_definitions(
        &mut self,
        source: CustomEndpointSource,
        startup_settings_parse_succeeded: bool,
        ctx: &mut ModelContext<Self>,
    ) where
        Self: Sized;
}

impl CustomEndpointDefinitionsCoordinator for ApiKeyManager {
    fn subscribe_to_custom_endpoint_definitions(
        &mut self,
        source: CustomEndpointSource,
        startup_settings_parse_succeeded: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        if source != CustomEndpointSource::SettingsCollection {
            return;
        }

        // Seed from the current value whenever either (a) it was explicitly
        // set, or (b) the file's TOML parsed successfully this launch, even if
        // `custom_endpoints` itself is absent from it. Case (b) matters
        // because an absent-but-successfully-parsed value is authoritatively
        // empty — e.g. the user deleted their last endpoint and restarted — and
        // must still orphan-clean any stored key. Only a full-file parse
        // failure skips seeding: `AISettings` then falls back to a cached or
        // default snapshot that does not reflect the user's actual file, so
        // composing an "empty" collection from it would wrongly orphan-clean
        // every stored key. The settings-change subscription below still
        // fires for every subsequent explicit change regardless.
        let is_explicitly_set = AISettings::as_ref(ctx)
            .custom_endpoints
            .is_value_explicitly_set();
        if is_explicitly_set || startup_settings_parse_succeeded {
            let definitions = AISettings::as_ref(ctx).custom_endpoints.value().clone();
            self.set_custom_endpoint_definitions(definitions, ctx);
        }

        ctx.subscribe_to_model(&AISettings::handle(ctx), |manager, _, event, ctx| {
            if matches!(
                event,
                AISettingsChangedEvent::CustomEndpointDefinitions { .. }
            ) {
                let definitions = AISettings::as_ref(ctx).custom_endpoints.value().clone();
                manager.set_custom_endpoint_definitions(definitions, ctx);
            }
        });
    }
}

#[cfg(test)]
#[path = "custom_endpoints_tests.rs"]
mod tests;
