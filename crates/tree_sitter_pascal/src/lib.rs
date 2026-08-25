//! Tree-sitter grammar for Pascal / Object Pascal / Delphi.
//!
//! `arborium` — where every other language in the `languages` crate gets its
//! grammar — does not ship a Pascal grammar, so the generated parser is vendored
//! under `grammar/` instead. See `README.md` for the upstream revision and how to
//! refresh it.
//!
//! The highlight query lives with the other Warp-authored query data, in
//! `crates/languages/grammars/pascal/highlights.scm`.

use tree_sitter_language::LanguageFn;

unsafe extern "C" {
    fn tree_sitter_pascal() -> *const ();
}

/// The tree-sitter [`LanguageFn`] for Pascal.
///
/// Convert it with `.into()` to get the `arborium::tree_sitter::Language` that the
/// rest of the syntax pipeline expects.
pub const LANGUAGE: LanguageFn = unsafe { LanguageFn::from_raw(tree_sitter_pascal) };
