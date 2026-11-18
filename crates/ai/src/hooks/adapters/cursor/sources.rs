//! What the adapter keeps about each source between the trait's calls. `parse` is handed the file
//! and `bindings` is handed the document back, so without this a source would be read, and its
//! matchers compiled, once per call and then once more per envelope.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use regex::Regex;
use serde_json::Value;

use super::config::{ParsedDefinition, ParsedMatcher};
use crate::hooks::model::HookSourceId;

#[derive(Default)]
pub(super) struct SourceCache {
    by_source: Mutex<HashMap<HookSourceId, ParsedSource>>,
}

/// One source's most recent parse. A reparse replaces it, so a reload does not accumulate
/// patterns.
#[derive(Default)]
struct ParsedSource {
    /// The document the definitions were read from, so a document from elsewhere is not mistaken
    /// for it. `None` for a source that was never parsed here.
    document: Option<Value>,
    definitions: Vec<ParsedDefinition>,
    /// The definitions' compiled matchers by pattern text, plus any compiled on demand for a
    /// binding that was not declared from a parse.
    matchers: HashMap<String, Arc<Regex>>,
}

impl SourceCache {
    /// Keeps `definitions` as what `document` declares for `source`.
    pub(super) fn remember(
        &self,
        source: &HookSourceId,
        document: &Value,
        definitions: &[ParsedDefinition],
    ) {
        let matchers = definitions
            .iter()
            .filter_map(|definition| match &definition.matcher {
                Some(ParsedMatcher::Pattern(regex)) => {
                    Some((regex.as_str().to_owned(), regex.clone()))
                }
                Some(ParsedMatcher::Everything(_)) | None => None,
            })
            .collect();
        self.by_source.lock().expect("source cache lock").insert(
            source.clone(),
            ParsedSource {
                document: Some(document.clone()),
                definitions: definitions.to_vec(),
                matchers,
            },
        );
    }

    /// The definitions kept for `source`, if they were read from `document`.
    pub(super) fn definitions(
        &self,
        source: &HookSourceId,
        document: &Value,
    ) -> Option<Vec<ParsedDefinition>> {
        let by_source = self.by_source.lock().expect("source cache lock");
        let parsed = by_source.get(source)?;
        (parsed.document.as_ref() == Some(document)).then(|| parsed.definitions.clone())
    }

    /// The compiled form of `pattern` for a binding of `source`. A pattern that was not kept (a
    /// binding built directly rather than declared from a parse) is compiled on first use.
    pub(super) fn regex(&self, source: &HookSourceId, pattern: &str) -> Option<Arc<Regex>> {
        let mut by_source = self.by_source.lock().expect("source cache lock");
        let matchers = &mut by_source.entry(source.clone()).or_default().matchers;
        if let Some(regex) = matchers.get(pattern) {
            return Some(regex.clone());
        }
        let regex = Arc::new(Regex::new(pattern).ok()?);
        matchers.insert(pattern.to_owned(), regex.clone());
        Some(regex)
    }

    #[cfg(test)]
    pub(super) fn pattern_count(&self, source: &HookSourceId) -> usize {
        self.by_source
            .lock()
            .expect("source cache lock")
            .get(source)
            .map_or(0, |parsed| parsed.matchers.len())
    }
}

#[cfg(test)]
#[path = "sources_tests.rs"]
mod tests;
