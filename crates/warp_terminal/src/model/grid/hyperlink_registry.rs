//! Per-grid registry that interns OSC 8 hyperlinks behind small integer
//! handles, so each cell stores a 4-byte `HyperlinkId` instead of cloning
//! the URI string.
//!
//! Two design points worth knowing about (see `specs/GH6393/tech.md` §3e):
//!
//! 1. **Bounded.** The registry refuses interns past `MAX_DISTINCT_ENTRIES`
//!    and returns `None`. The URI byte cap (`MAX_URI_BYTES`) is enforced
//!    earlier in the parser before allocating the URI `String`, so a
//!    1 GB OSC 8 sequence never produces a 1 GB allocation; this module
//!    asserts the same cap defensively as a backstop.
//! 2. **No reclamation.** Entries are never freed while the registry is
//!    alive; the registry's lifetime is the grid's lifetime. This avoids
//!    the use-after-free / leak hazards that a refcounted scheme would
//!    have to handle across cell overwrite, RLE split/merge in
//!    `FlatStorage`, scrollback eviction, reflow, and deserialization.
//!    Worst case per grid: `MAX_DISTINCT_ENTRIES` * `MAX_URI_BYTES` ≈ 16 MB.

use std::collections::HashMap;
use std::num::NonZeroU32;

use get_size::GetSize;
use serde::{Deserialize, Serialize};

use crate::model::ansi::control_sequence_parameters::{Hyperlink, MAX_URI_BYTES};

/// Maximum number of distinct hyperlinks a single registry will hold.
/// Past this cap, [`HyperlinkRegistry::intern`] returns `None`; existing
/// entries continue to resolve.
pub const MAX_DISTINCT_ENTRIES: usize = 4096;

/// Opaque, dense, non-zero handle into a [`HyperlinkRegistry`]. Stored in
/// each [`crate::model::grid::cell::Cell`] that's part of an OSC 8 span.
///
/// `NonZeroU32` lets `Option<HyperlinkId>` fit in 4 bytes thanks to niche
/// optimization, which keeps the registry handle compact alongside the
/// other per-cell attributes.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct HyperlinkId(NonZeroU32);

impl HyperlinkId {
    /// Returns the underlying integer value of this id. Intended for callers
    /// that need a stable string identity for the hyperlink (e.g. caching a
    /// rendered cell position by id for integration tests); other callers
    /// should treat ids as opaque.
    #[inline]
    pub fn get(self) -> u32 {
        self.0.get()
    }
}

impl GetSize for HyperlinkId {}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct HyperlinkRegistry {
    /// Reverse map: hyperlink → id. Lets `intern` dedupe.
    by_link: HashMap<Hyperlink, HyperlinkId>,
    /// Forward array: id → hyperlink. The id's `NonZeroU32` value is
    /// `index_in_by_id + 1`.
    by_id: Vec<Hyperlink>,
}

impl HyperlinkRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Intern a hyperlink. Returns the existing id if `hyperlink` was already
    /// interned, a fresh id if the registry has capacity, or `None` if the
    /// registry has reached `MAX_DISTINCT_ENTRIES` (in which case the caller
    /// should treat the OSC 8 sequence as if it had been malformed and stamp
    /// the visible cells with `None`, per the no-dangling-id invariant in
    /// `specs/GH6393/tech.md` §3c).
    pub fn intern(&mut self, hyperlink: Hyperlink) -> Option<HyperlinkId> {
        // Defensive: parser enforces this, but assert here too so a future
        // call site that bypasses the parser still respects the cap.
        if hyperlink.uri.len() > MAX_URI_BYTES {
            return None;
        }

        if let Some(&id) = self.by_link.get(&hyperlink) {
            return Some(id);
        }

        if self.by_id.len() >= MAX_DISTINCT_ENTRIES {
            log::warn!(
                "HyperlinkRegistry: distinct-entries cap of {MAX_DISTINCT_ENTRIES} reached; dropping new entry"
            );
            return None;
        }

        // The next id's wire value is `len + 1` (so the first id is 1, not 0;
        // NonZeroU32 forbids 0). The `as u32` cast is bounded by
        // MAX_DISTINCT_ENTRIES (= 4096) checked above, so it can't truncate.
        let next_value = (self.by_id.len() + 1) as u32;
        let id = HyperlinkId(NonZeroU32::new(next_value).expect("len + 1 is never 0"));
        self.by_id.push(hyperlink.clone());
        self.by_link.insert(hyperlink, id);
        Some(id)
    }

    /// Resolve an id back to the hyperlink it names. Returns `None` if the
    /// id wasn't issued by this registry (e.g. came from a different grid's
    /// registry via a buggy migration path).
    pub fn get(&self, id: HyperlinkId) -> Option<&Hyperlink> {
        let index = id.0.get() as usize - 1;
        self.by_id.get(index)
    }

    /// The current number of distinct entries. Test-only because the
    /// no-reclaim invariant means this only ever grows during the registry's
    /// lifetime — production code shouldn't need to inspect it.
    #[cfg(any(test, feature = "test-util"))]
    pub fn len_for_test(&self) -> usize {
        self.by_id.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(uri: &str) -> Hyperlink {
        Hyperlink {
            id: None,
            uri: uri.to_owned(),
        }
    }

    #[test]
    fn intern_dedupes_same_hyperlink() {
        let mut reg = HyperlinkRegistry::new();
        let a = reg.intern(link("https://example.com")).unwrap();
        let b = reg.intern(link("https://example.com")).unwrap();
        assert_eq!(a, b);
        assert_eq!(reg.len_for_test(), 1);
    }

    #[test]
    fn intern_returns_distinct_ids_for_distinct_uris() {
        let mut reg = HyperlinkRegistry::new();
        let a = reg.intern(link("https://a")).unwrap();
        let b = reg.intern(link("https://b")).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn get_resolves_interned_link() {
        let mut reg = HyperlinkRegistry::new();
        let id = reg.intern(link("https://example.com")).unwrap();
        assert_eq!(reg.get(id).unwrap().uri, "https://example.com");
    }

    #[test]
    fn intern_returns_none_past_distinct_entries_cap() {
        // Override the cap for the test by exhausting it directly. The cap
        // is `pub const`, but we can simulate by interning the cap itself.
        let mut reg = HyperlinkRegistry::new();
        for i in 0..MAX_DISTINCT_ENTRIES {
            assert!(reg
                .intern(link(&format!("https://example.com/{i}")))
                .is_some());
        }
        // Past the cap → None.
        assert!(reg.intern(link("https://overflow.example")).is_none());
        // Existing entries still resolve.
        let first = reg.intern(link("https://example.com/0")).unwrap();
        assert_eq!(reg.get(first).unwrap().uri, "https://example.com/0");
        // `len_for_test` does NOT shrink on cap-hit; the failed intern is a
        // no-op rather than an eviction.
        assert_eq!(reg.len_for_test(), MAX_DISTINCT_ENTRIES);
    }

    #[test]
    fn intern_rejects_uri_above_max_bytes() {
        let mut reg = HyperlinkRegistry::new();
        let big = "x".repeat(MAX_URI_BYTES + 1);
        // Defensive backstop: even if parser was bypassed, the registry won't
        // accept an over-length URI.
        assert!(reg.intern(link(&big)).is_none());
    }

    #[test]
    fn no_reclaim_overwrite_does_not_shrink_registry() {
        // The registry has no API to "remove" or "decrement" an entry; this
        // test pins down that contract by interning, then re-interning the
        // same value (which must reuse the slot, not grow the registry).
        let mut reg = HyperlinkRegistry::new();
        let id = reg.intern(link("https://example.com")).unwrap();
        assert_eq!(reg.len_for_test(), 1);
        let id2 = reg.intern(link("https://example.com")).unwrap();
        assert_eq!(id, id2);
        assert_eq!(reg.len_for_test(), 1);
    }
}
