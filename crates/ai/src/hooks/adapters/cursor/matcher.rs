//! Cursor's matcher rule: a regex searched, unanchored and case-sensitive, against a value that
//! depends on the event. `""`, `"*"`, and a missing matcher match everything.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use regex::Regex;

use super::config::{ParsedDefinition, ParsedMatcher};
use super::events::{CursorEventSpec, MatchedValue};
use super::facets::{finished_shell_command, mcp_tool_call, pending_shell_command};
use crate::hooks::adapters::DeclaredBinding;
use crate::hooks::model::{HookEnvelope, HookSourceId};

/// Compiled matchers, by source and then by pattern text. A source's entries are replaced each
/// time it is activated, so a reload does not accumulate patterns, and nothing compiles per
/// envelope.
#[derive(Default)]
pub(super) struct MatcherCache {
    by_source: Mutex<HashMap<HookSourceId, HashMap<String, Arc<Regex>>>>,
}

impl MatcherCache {
    /// Replaces `source`'s entries with the matchers compiled while its definitions were parsed.
    pub(super) fn install(&self, source: &HookSourceId, definitions: &[ParsedDefinition]) {
        let compiled = definitions
            .iter()
            .filter_map(|definition| match &definition.matcher {
                Some(ParsedMatcher::Pattern(regex)) => {
                    Some((regex.as_str().to_owned(), regex.clone()))
                }
                Some(ParsedMatcher::Everything(_)) | None => None,
            })
            .collect();
        self.by_source
            .lock()
            .expect("matcher cache lock")
            .insert(source.clone(), compiled);
    }

    /// The compiled form of `pattern` for a binding of `source`. A binding that was not installed
    /// (one built directly rather than declared from a file) is compiled on first use.
    fn get(&self, source: &HookSourceId, pattern: &str) -> Option<Arc<Regex>> {
        let mut by_source = self.by_source.lock().expect("matcher cache lock");
        let compiled = by_source.entry(source.clone()).or_default();
        if let Some(regex) = compiled.get(pattern) {
            return Some(regex.clone());
        }
        let regex = Arc::new(Regex::new(pattern).ok()?);
        compiled.insert(pattern.to_owned(), regex.clone());
        Some(regex)
    }

    #[cfg(test)]
    pub(super) fn pattern_count(&self, source: &HookSourceId) -> usize {
        self.by_source
            .lock()
            .expect("matcher cache lock")
            .get(source)
            .map_or(0, HashMap::len)
    }
}

/// Cursor treats an empty matcher and a bare `*` as "match everything"; neither is a regex.
pub(super) fn matches_everything(pattern: &str) -> bool {
    pattern.is_empty() || pattern == "*"
}

/// Whether `binding`, a definition of `spec`'s event, selects this firing. An event Warp does not
/// fire yet never matches, and neither does an envelope the event is not about (a `tool.result`
/// for a file read is not a finished shell command).
pub(super) fn binding_matches(
    cache: &MatcherCache,
    spec: &CursorEventSpec,
    binding: &DeclaredBinding,
    envelope: &HookEnvelope,
) -> bool {
    if !spec.fires_in_phase_1 || spec.boundary != Some(envelope.boundary()) {
        return false;
    }
    let Some(value) = matched_value(spec, envelope) else {
        return false;
    };
    match binding.matcher.as_deref() {
        None => true,
        Some(pattern) if matches_everything(pattern) => true,
        Some(pattern) => cache
            .get(&binding.id.source, pattern)
            .is_some_and(|regex| regex.is_match(value)),
    }
}

/// The text `spec`'s matcher is tested against for `envelope`, or `None` when the envelope is
/// not an occurrence of the event.
pub(super) fn matched_value<'a>(
    spec: &CursorEventSpec,
    envelope: &'a HookEnvelope,
) -> Option<&'a str> {
    let payload = &envelope.payload;
    match spec.matched_value {
        MatchedValue::ToolName => payload.tool_name().map(|tool_name| tool_name.as_str()),
        MatchedValue::CommandText => pending_shell_command(payload)
            .or_else(|| finished_shell_command(payload).map(|finished| finished.command)),
        MatchedValue::McpToolName => mcp_tool_call(payload).map(|call| call.name),
        MatchedValue::SubagentType => None,
        MatchedValue::Literal(literal) => Some(literal),
        MatchedValue::Everything => Some(""),
    }
}

#[cfg(test)]
#[path = "matcher_tests.rs"]
mod tests;
