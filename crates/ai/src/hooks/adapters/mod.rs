use std::sync::{Arc, LazyLock};
use std::time::Duration;

use thiserror::Error;

use crate::hooks::model::{
    BindingCapabilities, HookBindingId, HookBoundary, HookDiagnostic, HookEffects, HookEnvelope,
    HookFormat, HookSourceId,
};
use crate::hooks::runner::HookProcessOutcome;

/// A hook definition as the adapter read it from a source, before trust and activation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredBinding {
    pub id: HookBindingId,
    pub boundary: HookBoundary,
    /// The event name as the vendor's format spells it.
    pub vendor_event: String,
    pub capabilities: BindingCapabilities,
    /// The matcher text from the source, if the definition had one.
    pub matcher: Option<String>,
    /// The command line (or prompt text, for prompt handlers) to run.
    pub command: String,
    pub timeout: Duration,
}

/// A source that parsed successfully. The adapter keeps the validated document so bindings can be
/// derived from it and so a later surface can show or diff it.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedHookConfig {
    pub source: HookSourceId,
    pub format: HookFormat,
    pub document: serde_json::Value,
    /// Problems that did not stop the source from loading.
    pub warnings: Vec<HookDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum HookConfigError {
    #[error("could not parse hook configuration: {0}")]
    Parse(String),
    #[error("invalid hook configuration: {0}")]
    Invalid(String),
}

/// Reads one vendor's configuration format and translates between Warp's canonical model and that
/// vendor's wire shapes. Vendor vocabulary stays behind this trait.
pub trait HookFormatAdapter: Send + Sync {
    fn format(&self) -> HookFormat;

    fn parse(
        &self,
        bytes: &[u8],
        source: &HookSourceId,
    ) -> Result<ParsedHookConfig, HookConfigError>;

    fn bindings(&self, config: &ParsedHookConfig) -> Vec<DeclaredBinding>;

    /// Whether `binding`'s matcher selects this firing.
    fn matches(&self, binding: &DeclaredBinding, envelope: &HookEnvelope) -> bool;

    /// The JSON object the hook receives on stdin.
    fn encode_input(&self, binding: &DeclaredBinding, envelope: &HookEnvelope)
    -> serde_json::Value;

    /// Turns what the process did into typed effects, applying the vendor's exit-code and
    /// malformed-output rules.
    fn decode_output(&self, binding: &DeclaredBinding, outcome: &HookProcessOutcome)
    -> HookEffects;
}

/// One entry in the adapter table.
pub struct HookAdapterDefinition {
    pub format: HookFormat,
    pub adapter: Arc<dyn HookFormatAdapter>,
}

/// Every known format adapter. Formats are added here as their adapters land.
pub static HOOK_ADAPTER_DEFINITIONS: LazyLock<Vec<HookAdapterDefinition>> = LazyLock::new(Vec::new);

/// The adapter for `format`, if one is registered.
pub fn adapter_for(format: HookFormat) -> Option<Arc<dyn HookFormatAdapter>> {
    HOOK_ADAPTER_DEFINITIONS
        .iter()
        .find(|definition| definition.format == format)
        .map(|definition| definition.adapter.clone())
}
