//! The adapter for Cursor's `hooks.json` format. It reads every option and all 21 event names,
//! declares a binding with capabilities and failure semantics for each definition, matches, encodes
//! the stdin payload Cursor documents for the events Warp fires, and decodes output under Cursor's
//! exit-code and schema rules. Events Warp does not fire are parsed, reported, and never matched.

mod config;
mod decode;
mod encode;
mod events;
mod facets;
mod matcher;
mod sources;
#[cfg(test)]
mod test_support;

use self::config::{ParsedDefinition, parse_cursor_document, parse_cursor_file};
use self::events::CursorEvent;
use self::sources::SourceCache;
use crate::hooks::adapters::{
    DeclaredBinding, DecodePolicy, HookConfigError, HookFormatAdapter, ParsedHookConfig,
};
use crate::hooks::model::{
    DiagnosticSeverity, HookDiagnostic, HookEffects, HookEnvelope, HookFormat, HookSourceId,
};
use crate::hooks::runner::HookProcessOutcome;

#[derive(Default)]
pub struct CursorAdapter {
    sources: SourceCache,
}

impl CursorAdapter {
    pub fn new() -> Self {
        Self::default()
    }

    fn reparse(&self, config: &ParsedHookConfig) -> Option<Vec<ParsedDefinition>> {
        match parse_cursor_document(&config.document) {
            Ok(parsed) => {
                self.sources
                    .remember(&config.source, &config.document, &parsed.definitions);
                Some(parsed.definitions)
            }
            Err(error) => {
                log::error!(
                    "hook source {} no longer parses, though it was accepted earlier: {error}",
                    config.source
                );
                None
            }
        }
    }
}

/// The event a binding was declared under. Bindings handed to this adapter always come from it,
/// so a name it does not know is a bug in the caller and is logged.
fn event_of(binding: &DeclaredBinding) -> Option<CursorEvent> {
    let event = CursorEvent::from_name(&binding.vendor_event);
    if event.is_none() {
        log::error!(
            "hook {} was declared under {}, which is not a Cursor event this adapter knows",
            binding.id,
            binding.vendor_event
        );
    }
    event
}

impl HookFormatAdapter for CursorAdapter {
    fn format(&self) -> HookFormat {
        HookFormat::Cursor
    }

    fn parse(
        &self,
        bytes: &[u8],
        source: &HookSourceId,
    ) -> Result<ParsedHookConfig, HookConfigError> {
        let (document, parsed) = parse_cursor_file(bytes)?;
        self.sources
            .remember(source, &document, &parsed.definitions);
        let warnings = parsed
            .warnings
            .into_iter()
            .map(|message| HookDiagnostic {
                source: source.clone(),
                severity: DiagnosticSeverity::Warning,
                message,
            })
            .collect();
        Ok(ParsedHookConfig {
            source: source.clone(),
            format: HookFormat::Cursor,
            document,
            warnings,
        })
    }

    /// Declares bindings from the definitions `parse` kept for the source. A document this
    /// adapter did not parse (another adapter instance did, or the registry changed it) is read
    /// again.
    fn bindings(&self, config: &ParsedHookConfig) -> Vec<DeclaredBinding> {
        let definitions = self
            .sources
            .definitions(&config.source, &config.document)
            .or_else(|| self.reparse(config));
        definitions
            .map(|definitions| events::declare_bindings(&definitions, &config.source))
            .unwrap_or_default()
    }

    fn matches(&self, binding: &DeclaredBinding, envelope: &HookEnvelope) -> bool {
        event_of(binding).is_some_and(|event| {
            matcher::binding_matches(&self.sources, event.spec(), binding, envelope)
        })
    }

    fn encode_input(
        &self,
        binding: &DeclaredBinding,
        envelope: &HookEnvelope,
    ) -> serde_json::Value {
        match event_of(binding) {
            Some(event) => encode::encode_input(event.spec(), envelope),
            None => serde_json::Value::Object(serde_json::Map::new()),
        }
    }

    fn decode_output(
        &self,
        binding: &DeclaredBinding,
        outcome: &HookProcessOutcome,
        policy: &DecodePolicy,
    ) -> HookEffects {
        match event_of(binding) {
            Some(event) => decode::decode_output(event.spec(), binding, outcome, policy),
            None => HookEffects::default(),
        }
    }
}

#[cfg(test)]
#[path = "mod_test.rs"]
mod tests;
