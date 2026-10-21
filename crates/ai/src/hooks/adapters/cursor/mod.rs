//! The adapter for Cursor's `hooks.json` format. It reads every option and all 21 event names,
//! declares a binding with capabilities and failure semantics for each definition, matches,
//! encodes the stdin payload Cursor documents for the events Warp fires, and decodes output
//! under Cursor's exit-code and schema rules. Events Warp does not fire are parsed, reported,
//! and never matched.

mod config;
mod decode;
mod encode;
mod events;
mod facets;
mod matcher;
#[cfg(test)]
mod test_support;

use self::config::{parse_cursor_document, parse_cursor_file};
use self::events::CursorEvent;
use self::matcher::MatcherCache;
use crate::hooks::adapters::{
    DeclaredBinding, DecodePolicy, HookConfigError, HookFormatAdapter, ParsedHookConfig,
};
use crate::hooks::model::{
    DiagnosticSeverity, HookDiagnostic, HookEffects, HookEnvelope, HookFormat, HookSourceId,
};
use crate::hooks::runner::HookProcessOutcome;

#[derive(Default)]
pub struct CursorAdapter {
    matchers: MatcherCache,
}

impl CursorAdapter {
    pub fn new() -> Self {
        Self::default()
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

    fn bindings(&self, config: &ParsedHookConfig) -> Vec<DeclaredBinding> {
        match parse_cursor_document(&config.document) {
            Ok(parsed) => events::declare_bindings(&parsed.definitions, &config.source),
            Err(error) => {
                log::error!(
                    "hook source {} no longer parses, though it was accepted earlier: {error}",
                    config.source
                );
                Vec::new()
            }
        }
    }

    fn matches(&self, binding: &DeclaredBinding, envelope: &HookEnvelope) -> bool {
        event_of(binding).is_some_and(|event| {
            matcher::binding_matches(&self.matchers, event.spec(), binding, envelope)
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
