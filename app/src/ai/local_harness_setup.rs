use warp_cli::agent::Harness;

use crate::features::FeatureFlag;
#[cfg(not(target_family = "wasm"))]
use crate::util::path::resolve_executable;

/// Tooltip shown when a local harness is product-enabled but its CLI is missing.
pub(crate) const LOCAL_HARNESS_INSTALLATION_REQUIRED_TOOLTIP: &str =
    "Install Claude Code to use this local harness.";
pub(crate) const LOCAL_CODEX_HARNESS_INSTALLATION_REQUIRED_TOOLTIP: &str =
    "Install Codex to use this local harness.";
pub(crate) const LOCAL_CODEX_HARNESS_DISABLED_MESSAGE: &str =
    "Local Codex child agents are temporarily disabled.";

/// Client-side readiness for using a harness in local orchestration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LocalHarnessSetupState {
    /// The harness is product-enabled and its required local CLI is installed.
    Ready,
    /// The harness is intentionally unavailable in the product.
    ProductDisabled { message: &'static str },
    /// The harness is product-enabled but the required local CLI is missing.
    MissingHarness { tooltip: &'static str },
}

impl LocalHarnessSetupState {
    /// Returns whether the harness can be selected in local orchestration controls.
    pub(crate) fn is_selectable(self) -> bool {
        matches!(self, Self::Ready)
    }
}

/// Returns the product-level disabled reason for a local harness.
pub(crate) fn local_harness_product_disabled_message(harness: Harness) -> Option<&'static str> {
    match harness {
        Harness::Codex if !local_codex_harness_is_enabled() => {
            Some(LOCAL_CODEX_HARNESS_DISABLED_MESSAGE)
        }
        Harness::Oz | Harness::Claude | Harness::OpenCode | Harness::Gemini | Harness::Unknown => {
            None
        }
        Harness::Codex => None,
    }
}

fn local_codex_harness_is_enabled() -> bool {
    FeatureFlag::LocalClaudeCodexChildHarnesses.is_enabled()
}

/// Returns whether a local harness is exposed by product policy.
pub(crate) fn local_harness_is_product_enabled(harness: Harness) -> bool {
    local_harness_product_disabled_message(harness).is_none()
}

/// Returns the current local setup state for a harness.
pub(crate) fn local_harness_setup_state(harness: Harness) -> LocalHarnessSetupState {
    local_harness_setup_state_with_cli_resolver(harness, local_cli_is_installed)
}

fn local_harness_setup_state_with_cli_resolver(
    harness: Harness,
    cli_is_installed: impl Fn(&str) -> bool,
) -> LocalHarnessSetupState {
    if let Some(message) = local_harness_product_disabled_message(harness) {
        return LocalHarnessSetupState::ProductDisabled { message };
    }

    match harness {
        Harness::Claude if !cli_is_installed("claude") => LocalHarnessSetupState::MissingHarness {
            tooltip: LOCAL_HARNESS_INSTALLATION_REQUIRED_TOOLTIP,
        },
        Harness::Codex if !cli_is_installed("codex") => LocalHarnessSetupState::MissingHarness {
            tooltip: LOCAL_CODEX_HARNESS_INSTALLATION_REQUIRED_TOOLTIP,
        },
        Harness::Oz
        | Harness::Claude
        | Harness::OpenCode
        | Harness::Gemini
        | Harness::Codex
        | Harness::Unknown => LocalHarnessSetupState::Ready,
    }
}

fn local_cli_is_installed(command: &str) -> bool {
    #[cfg(not(target_family = "wasm"))]
    {
        resolve_executable(command).is_some()
    }
    #[cfg(target_family = "wasm")]
    {
        let _ = command;
        false
    }
}

#[cfg(test)]
#[path = "local_harness_setup_tests.rs"]
mod tests;
