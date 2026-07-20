use std::path::Path;

/// A language handled by one of Warp's built-in language servers.
///
/// This facet is intentionally narrower than the registry's general
/// `language_id` coverage. Adding a variant or selector changes built-in server
/// dispatch and therefore requires an explicit product change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltInLspLanguage {
    Rust,
    Go,
    Python,
    TypeScript,
    TypeScriptReact,
    JavaScript,
    JavaScriptReact,
    C,
    Cpp,
}

impl BuiltInLspLanguage {
    /// Return the LSP `languageId` associated with this built-in language.
    pub const fn language_id(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Go => "go",
            Self::Python => "python",
            Self::TypeScript => "typescript",
            Self::TypeScriptReact => "typescriptreact",
            Self::JavaScript => "javascript",
            Self::JavaScriptReact => "javascriptreact",
            Self::C => "c",
            Self::Cpp => "cpp",
        }
    }
}

/// A language identity and the file selectors understood by its consumers.
///
/// Entries with the same `id` are allowed when the editor and LSP historically
/// supported different selectors. This keeps the migration behavior-preserving
/// without making selectors consumer-specific.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LanguageEntry {
    pub id: &'static str,
    pub extensions: &'static [&'static str],
    pub filenames: &'static [&'static str],
    /// Prefix matches retained only for Warp's existing `Dockerfile.*` and
    /// `Containerfile.*` behavior. General filename patterns are out of scope.
    pub filename_prefixes: &'static [&'static str],
    pub language_id: Option<&'static str>,
    /// Built-in LSP dispatch, kept separate from the broader `language_id`.
    pub built_in_lsp: Option<BuiltInLspLanguage>,
    /// The parser grammar name used by the `languages` crate.
    pub grammar: Option<&'static str>,
}

const NONE: &[&str] = &[];

include!("generated.rs");

/// Resolve a local path to a language entry.
pub fn resolve(path: &Path, first_line: Option<&str>) -> Option<&'static LanguageEntry> {
    resolve_parts(
        path.file_name().and_then(|filename| filename.to_str()),
        path.extension().and_then(|extension| extension.to_str()),
        first_line,
    )
}

/// Resolve pre-extracted filename components to a language entry.
///
/// This form supports normalized paths whose encoding is not necessarily the
/// local platform's encoding.
pub fn resolve_parts(
    filename: Option<&str>,
    extension: Option<&str>,
    first_line: Option<&str>,
) -> Option<&'static LanguageEntry> {
    resolve_parts_matching(filename, extension, first_line, |_| true)
}

/// Resolve the built-in LSP facet for a local path.
///
/// Unlike general language resolution, a filename match without this facet
/// does not mask a lower-precedence extension match. This preserves built-in
/// LSP's historical extension-only dispatch while editor grammar selection
/// keeps filename and legacy prefix precedence.
pub fn resolve_built_in_lsp(path: &Path) -> Option<BuiltInLspLanguage> {
    resolve_parts_matching(
        path.file_name().and_then(|filename| filename.to_str()),
        path.extension().and_then(|extension| extension.to_str()),
        None,
        |entry| entry.built_in_lsp.is_some(),
    )?
    .built_in_lsp
}

fn resolve_parts_matching(
    filename: Option<&str>,
    extension: Option<&str>,
    first_line: Option<&str>,
    matches_consumer: impl Fn(&LanguageEntry) -> bool,
) -> Option<&'static LanguageEntry> {
    if let Some(filename) = filename {
        if let Some(entry) = LANGUAGE_ENTRIES
            .iter()
            .find(|entry| entry.filenames.contains(&filename) && matches_consumer(entry))
        {
            return Some(entry);
        }

        if let Some(entry) = LANGUAGE_ENTRIES.iter().find(|entry| {
            matches_consumer(entry)
                && entry
                    .filename_prefixes
                    .iter()
                    .any(|prefix| filename.starts_with(prefix))
        }) {
            return Some(entry);
        }
    }

    if let Some(extension) = extension
        && let Some(entry) = LANGUAGE_ENTRIES
            .iter()
            .find(|entry| entry.extensions.contains(&extension) && matches_consumer(entry))
    {
        return Some(entry);
    }

    resolve_first_line(first_line, &matches_consumer)
}

fn resolve_first_line(
    first_line: Option<&str>,
    matches_consumer: &impl Fn(&LanguageEntry) -> bool,
) -> Option<&'static LanguageEntry> {
    first_line?;
    // Reserved for shebang/first-line detection in a later change. Keep the
    // consumer filter at this boundary so future matches cannot bypass it.
    let _ = matches_consumer;
    None
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
