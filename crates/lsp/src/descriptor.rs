//! Data types and pure logic for user-configured custom LSP servers
//! (`[[editor.language_servers]]` entries in settings.toml).

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub mod matcher;
pub mod parse;
pub mod placeholder;
pub mod validate;

/// One user-defined custom LSP server entry, parsed and validated from a
/// `[[editor.language_servers]]` table in settings.toml.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LspServerDescriptor {
    /// Unique identifier for this server, e.g. `"ruby-lsp"`. Shown in the
    /// footer's "Enable {name}" button and in logs.
    pub name: String,
    /// Path to the server binary: an absolute path, or a bare name resolved
    /// against `PATH`. Supports `{{...}}` placeholders — `{{workspace_root}}`,
    /// `{{workspace_slug}}`, `{{cache_dir}}`, and `{{env_VAR}}` (any
    /// environment variable) — and leading `~`/`~/` home-directory expansion.
    /// Wrap a placeholder in a third pair of braces (`{{{workspace_root}}}`) to
    /// pass it through verbatim without substitution.
    pub command: String,
    /// Arguments passed to `command` on launch. Each undergoes the same
    /// `{{...}}` placeholder substitution and `~` expansion as `command`.
    #[serde(default)]
    pub args: Vec<String>,
    /// Filename patterns that claim files for this server. Non-empty.
    pub filetypes: Vec<LspFiletypePattern>,
    /// Extra environment variables merged into the server process's
    /// environment. Each value undergoes the same `{{...}}` placeholder
    /// substitution as `command`.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Sent as the LSP `initialize` request's `initializationOptions`. String
    /// leaves undergo the same `{{...}}` placeholder substitution as `command`;
    /// non-string values pass through unchanged.
    pub initialization_options: Option<Value>,
}

/// One element of `LspServerDescriptor::filetypes`. Each filetype is an
/// inline table (TOML: `{ pattern = "*.rb", language_id = "ruby" }`).
/// `pattern` is required; `language_id` is optional.
///
/// Patterns containing glob metacharacters (`*`, `?`, `[`) match
/// case-insensitively; literal basenames match case-sensitively.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LspFiletypePattern {
    /// The pattern string as written by the user (e.g. `"*.rb"`, `"Gemfile"`).
    pub pattern: String,
    /// When `Some`, this is the LSP `languageId` sent for files matched by
    /// this pattern. When `None`, the matcher derives a default from the
    /// matched file's extension (lowercased) or basename.
    pub language_id: Option<String>,
    /// Compiled matcher — a derived runtime artifact, not user-typed.
    ///
    /// Skipped from serde (so it's not written to disk) and from the JSON
    /// schema (so the user-facing schema doesn't expose it). This follows the
    /// same `#[serde(skip)]` pattern used in `grid_storage.rs` for runtime-only
    /// cursor/state fields. `globset::GlobMatcher` doesn't impl `Default`, so
    /// we point `default = ...` at a constructor that produces a never-matching
    /// placeholder. The placeholder is unreachable in production: settings
    /// reload routes through `parse::parse_entries`, which always compiles a
    /// real matcher from `pattern`.
    #[serde(skip, default = "placeholder_matcher")]
    #[schemars(skip)]
    matcher: globset::GlobMatcher,
}

/// Default for the `LspFiletypePattern::matcher` field when constructed via
/// serde without going through `parse::compile_pattern`. Production code
/// paths always supply a real matcher via `parse::parse_entries`; the
/// literal below is a sentinel chosen so the placeholder will not match any
/// real filename if it ever leaks into a match check.
fn placeholder_matcher() -> globset::GlobMatcher {
    globset::GlobBuilder::new("__lsp_descriptor_placeholder_never_matches__")
        .build()
        .expect("static placeholder pattern compiles")
        .compile_matcher()
}

impl LspFiletypePattern {
    /// Constructs a pattern from its already-compiled parts.
    pub(crate) fn from_parts(
        pattern: String,
        language_id: Option<String>,
        matcher: globset::GlobMatcher,
    ) -> Self {
        Self {
            pattern,
            language_id,
            matcher,
        }
    }

    /// Returns `true` if this pattern matches `basename`. Glob patterns
    /// match case-insensitively, literal basenames case-sensitively; that
    /// distinction is baked into the compiled matcher at parse time.
    pub fn is_match(&self, basename: &str) -> bool {
        self.matcher.is_match(basename)
    }
}

// Hand-rolled because `globset::GlobMatcher` does not impl `PartialEq`.
// Two patterns are equal when their user-typed inputs (`pattern` and
// `language_id`) match — the compiled `matcher` is deterministic given
// `pattern`, so comparing inputs is sufficient.
impl PartialEq for LspFiletypePattern {
    fn eq(&self, other: &Self) -> bool {
        self.pattern == other.pattern && self.language_id == other.language_id
    }
}

#[cfg(test)]
#[path = "descriptor_tests.rs"]
mod tests;
