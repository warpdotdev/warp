//! The section layout of the session-search popup, expressed as priority
//! tiers.
//!
//! The mixer sorts ascending on `(priority_tier, score, source_order)` and the
//! search bar then reverses that list for a `TopDown` palette, so a **higher
//! tier renders higher**. Every section therefore occupies two tiers: one for
//! its header and one, immediately below, for its rows.
//!
//! The direction is the part that inverts silently — swapping two constants
//! compiles, renders, and puts the headers under their rows. `tiers_tests.rs`
//! is the assertion that catches it.

/// Header above the rows matched by name (task, project or directory).
pub const NAME_SEPARATOR_TIER: u8 = 5;

/// A session matched by name. The only rows Phase 1 emits.
pub const NAME_ROW_TIER: u8 = 4;

/// Header above the rows matched inside transcript text (Phase 2).
pub const CONTENT_SEPARATOR_TIER: u8 = 3;

/// A session matched by transcript content (Phase 2).
pub const CONTENT_ROW_TIER: u8 = 2;

/// Title of the name section's header.
pub const NAME_SEPARATOR_TITLE: &str = "Names";

/// Title of the transcript-content section's header (Phase 2). Defined here so
/// both sections' identity lives in one place; user-facing copy is the one spot
/// allowed to say "conversation".
pub const CONTENT_SEPARATOR_TITLE: &str = "In conversation text";

#[cfg(test)]
#[path = "tiers_tests.rs"]
mod tests;
