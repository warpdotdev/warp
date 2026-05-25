use std::path::Path;

use super::{LspFiletypePattern, LspServerDescriptor};

/// The result of matching a file path against a descriptor's `filetypes`.
///
/// Carries the resolved LSP `languageId` for the matched file alongside a
/// borrow of the winning descriptor. The `languageId` is the explicit
/// `language_id` from the matched pattern if set, otherwise the file's
/// lowercase extension, otherwise the file's literal basename.
#[derive(Debug)]
pub struct LspMatchedDescriptor<'a> {
    pub descriptor: &'a LspServerDescriptor,
    pub language_id: String,
}

/// Returns the first descriptor in `descriptors` whose `filetypes` matches
/// `file_path`. First-in-source-order wins on overlap.
pub fn match_descriptor<'a>(
    descriptors: &'a [LspServerDescriptor],
    file_path: &Path,
) -> Option<LspMatchedDescriptor<'a>> {
    let basename = file_path.file_name()?.to_str()?;
    for descriptor in descriptors {
        if let Some(pattern) = match_filetypes(&descriptor.filetypes, basename) {
            let language_id = resolve_language_id(pattern, basename);
            return Some(LspMatchedDescriptor {
                descriptor,
                language_id,
            });
        }
    }
    None
}

fn match_filetypes<'a>(
    filetypes: &'a [LspFiletypePattern],
    basename: &str,
) -> Option<&'a LspFiletypePattern> {
    filetypes.iter().find(|pattern| pattern.is_match(basename))
}

fn resolve_language_id(pattern: &LspFiletypePattern, basename: &str) -> String {
    if let Some(explicit) = &pattern.language_id {
        return explicit.clone();
    }
    // No explicit override: derive from extension, falling back to basename.
    // `file_name` for `/x/.bashrc` is `.bashrc`; we want the basename in that
    // case (no extension to lowercase).
    if let Some((stem, ext)) = basename.rsplit_once('.') {
        if !stem.is_empty() && !ext.is_empty() {
            return ext.to_ascii_lowercase();
        }
    }
    basename.to_string()
}

#[cfg(test)]
#[path = "matcher_tests.rs"]
mod tests;
