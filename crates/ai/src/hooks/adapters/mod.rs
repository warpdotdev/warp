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
    /// How many hook-requested continuations this definition may start in one chain, as the
    /// source declared it after the host cap was applied. `None` means the definition does not
    /// limit itself and only the host cap applies. Only meaningful for bindings that may produce
    /// a continuation.
    pub loop_limit: Option<u32>,
}

/// Runtime choices that change how an adapter reads a hook's output. They come from Warp's own
/// settings rather than from the source, so they are passed per decode instead of being stored
/// on the binding.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DecodePolicy {
    /// The "strict hook policy" setting: honor schema-accepted `ask` values the vendor would
    /// otherwise ignore, and treat every evaluation failure of a hook that can block as a block.
    pub strict: bool,
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
    #[error("could not read hook configuration: {0}")]
    Unreadable(String),
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
    /// malformed-output rules under `policy`.
    fn decode_output(
        &self,
        binding: &DeclaredBinding,
        outcome: &HookProcessOutcome,
        policy: &DecodePolicy,
    ) -> HookEffects;
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
