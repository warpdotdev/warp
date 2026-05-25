//! Serde-driven parsing of `[[editor.language_servers]]` entries into
//! `LspServerDescriptor` values, with validation.
//!
//! Input is a `serde_json::Value` array (the settings infrastructure converts
//! TOML to JSON before reaching this layer). Output is a partitioned result:
//! the validated descriptors that should be loaded, plus the errors that
//! caused invalid entries to be dropped.

use std::collections::{BTreeMap, HashSet};

use serde::Deserialize;
use serde_json::Value;

use super::validate::{check_supported_glob_features, LspDescriptorError, LspDescriptorErrorKind};
use super::{LspFiletypePattern, LspServerDescriptor};

/// Result of parsing the `[[editor.language_servers]]` array.
///
/// Invalid entries are dropped while valid entries continue to load. The
/// user-visible surface lists every error.
#[derive(Debug)]
pub struct LspParseResult {
    pub descriptors: Vec<LspServerDescriptor>,
    pub errors: Vec<LspDescriptorError>,
}

/// Parses an array of `[[editor.language_servers]]` entries (as JSON, which is
/// how the settings layer hands them off). Each element should be an object;
/// non-object elements produce a `MissingName` error against an anonymous
/// entry.
pub fn parse_entries(entries: &[Value]) -> LspParseResult {
    let mut descriptors: Vec<LspServerDescriptor> = Vec::with_capacity(entries.len());
    let mut errors: Vec<LspDescriptorError> = Vec::new();

    for entry in entries {
        match parse_single(entry) {
            Ok(descriptor) => descriptors.push(descriptor),
            Err(entry_errors) => errors.extend(entry_errors),
        }
    }

    // Dedupe by `name`, preserving the first-seen entry and reporting later
    // ones as errors.
    let mut seen_names: HashSet<String> = HashSet::new();
    let mut deduped: Vec<LspServerDescriptor> = Vec::with_capacity(descriptors.len());
    for descriptor in descriptors {
        if seen_names.insert(descriptor.name.clone()) {
            deduped.push(descriptor);
        } else {
            errors.push(LspDescriptorError {
                entry_name: Some(descriptor.name),
                kind: LspDescriptorErrorKind::DuplicateName,
            });
        }
    }

    LspParseResult {
        descriptors: deduped,
        errors,
    }
}

fn parse_single(value: &Value) -> Result<LspServerDescriptor, Vec<LspDescriptorError>> {
    // Treat unknown fields as ignored by routing through serde with
    // `#[serde(default)]`. We do not enable `deny_unknown_fields` here so
    // future-added fields stay forward-compatible.
    let raw = match serde_json::from_value::<RawDescriptor>(value.clone()) {
        Ok(raw) => raw,
        Err(e) => {
            // Shape-level deserialize failed — the entry isn't a table, or
            // a field has the wrong type (e.g. `name = 42`). Attribute to
            // the entry's `name` string if one is present and parseable,
            // otherwise report anonymously.
            return Err(vec![LspDescriptorError {
                entry_name: anonymous_name_hint(value),
                kind: LspDescriptorErrorKind::MalformedEntry {
                    reason: e.to_string(),
                },
            }]);
        }
    };

    let mut errors: Vec<LspDescriptorError> = Vec::new();

    let name = match raw.name.as_deref() {
        Some(n) if !n.trim().is_empty() => n.to_string(),
        _ => {
            errors.push(LspDescriptorError {
                entry_name: None,
                kind: LspDescriptorErrorKind::MissingName,
            });
            return Err(errors);
        }
    };

    let command = match raw.command.as_deref() {
        Some(c) if !c.trim().is_empty() => c.to_string(),
        _ => {
            errors.push(LspDescriptorError {
                entry_name: Some(name.clone()),
                kind: LspDescriptorErrorKind::MissingCommand,
            });
            return Err(errors);
        }
    };

    let mut filetypes: Vec<LspFiletypePattern> = Vec::new();
    for raw_pattern in &raw.filetypes {
        match parse_filetype(&name, raw_pattern) {
            Ok(pattern) => filetypes.push(pattern),
            Err(error) => errors.push(error),
        }
    }

    // Distinguish "user wrote []" (report EmptyFiletypes) from "every entry
    // failed validation" (don't double-report — the per-entry errors are
    // already in the list).
    let no_filetype_errors = errors.iter().all(|e| {
        !matches!(
            e.kind,
            LspDescriptorErrorKind::InvalidGlob { .. }
                | LspDescriptorErrorKind::UnsupportedGlobFeature { .. }
        )
    });
    if filetypes.is_empty() && no_filetype_errors {
        errors.push(LspDescriptorError {
            entry_name: Some(name.clone()),
            kind: LspDescriptorErrorKind::EmptyFiletypes,
        });
    }

    if !errors.is_empty() {
        return Err(errors);
    }

    Ok(LspServerDescriptor {
        name,
        command,
        args: raw.args,
        filetypes,
        env: raw.env,
        initialization_options: raw.initialization_options,
    })
}

/// Best-effort extraction of an entry's `name` field directly from the raw
/// JSON, used when full deserialization failed but we'd still like to
/// attribute the error to a recognizable entry. Returns `None` if the value
/// isn't an object, has no `name` key, or `name` isn't a string.
fn anonymous_name_hint(value: &Value) -> Option<String> {
    value
        .get("name")
        .and_then(|n| n.as_str())
        .map(|s| s.to_string())
}

/// Converts one raw `filetypes` entry into a compiled `LspFiletypePattern`.
/// Each entry is an inline table `{ pattern = "...", language_id = "..." }`;
/// `pattern` is required (serde rejects entries without it) and `language_id`
/// is optional. `descriptor_name` is used solely for error attribution.
fn parse_filetype(
    descriptor_name: &str,
    raw: &RawFiletype,
) -> Result<LspFiletypePattern, LspDescriptorError> {
    let pattern = raw.pattern.clone();
    let language_id = raw.language_id.clone();
    let matcher = compile_pattern(descriptor_name, &pattern)?;
    Ok(LspFiletypePattern::from_parts(
        pattern,
        language_id,
        matcher,
    ))
}

/// Compiles a user-provided pattern into a `globset::GlobMatcher`. Patterns
/// without glob metacharacters (`*`, `?`, `[`) are passed through with any
/// remaining globset-special characters (`{`, `}`, `,`, `\`, `]`) escaped, so
/// they match as an exact case-sensitive basename. Glob patterns compile
/// directly and match case-insensitively. This collapses the two pattern
/// forms onto a single match path without changing the documented behavior.
fn compile_pattern(
    descriptor_name: &str,
    pattern: &str,
) -> Result<globset::GlobMatcher, LspDescriptorError> {
    let is_glob = has_glob_metacharacter(pattern);

    if is_glob {
        if let Some(kind) = check_supported_glob_features(pattern) {
            return Err(LspDescriptorError {
                entry_name: Some(descriptor_name.to_string()),
                kind,
            });
        }
    }

    let compiled_source = if is_glob {
        pattern.to_string()
    } else {
        escape_globset_special_chars(pattern)
    };

    globset::GlobBuilder::new(&compiled_source)
        .case_insensitive(is_glob)
        .literal_separator(true)
        .backslash_escape(true)
        .build()
        .map(|glob| glob.compile_matcher())
        .map_err(|e| LspDescriptorError {
            entry_name: Some(descriptor_name.to_string()),
            kind: LspDescriptorErrorKind::InvalidGlob {
                pattern: pattern.to_string(),
                reason: e.to_string(),
            },
        })
}

/// Returns `true` if the pattern contains any character that classifies it as
/// a glob (`*`, `?`, `[`). Patterns without these characters are treated as
/// literal basenames and matched case-sensitively. Used by `compile_pattern`
/// to decide how to configure the compiled `globset::GlobMatcher`.
fn has_glob_metacharacter(pattern: &str) -> bool {
    pattern.contains(['*', '?', '['])
}

/// Escapes every character that globset would otherwise interpret as syntax
/// (`*`, `?`, `[`, `]`, `{`, `}`, `,`, `\`). The result, when compiled with
/// `backslash_escape(true)`, matches the original string exactly.
fn escape_globset_special_chars(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '*' | '?' | '[' | ']' | '{' | '}' | ',' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[derive(Debug, Deserialize)]
struct RawDescriptor {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    filetypes: Vec<RawFiletype>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    initialization_options: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct RawFiletype {
    pattern: String,
    #[serde(default)]
    language_id: Option<String>,
}

#[cfg(test)]
#[path = "parse_tests.rs"]
mod tests;
