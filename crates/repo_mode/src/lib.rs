//! Domain types and path rules for Warp repo mode.
//!
//! Pure helpers only — no app/UI dependency. Selection state lives in
//! `app` (`RepoModeModel`).

mod entry;

pub use entry::{
    canonicalize_repo_path, classify_entry_kind, display_name_for_path, is_dead_path, RepoEntry,
    RepoEntryKind,
};
