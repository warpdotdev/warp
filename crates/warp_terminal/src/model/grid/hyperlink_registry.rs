//! Per-grid registry that interns OSC 8 hyperlinks behind small integer
//! handles, so each cell stores a 4-byte `HyperlinkId` instead of cloning
//! the URI string.
//!
//! Two design points worth knowing about:
//!
//! 1. **Bounded.** The registry refuses interns past `MAX_DISTINCT_ENTRIES`
//!    and returns `None`. The URI byte cap (`MAX_URI_BYTES`) is enforced
//!    earlier in the parser before allocating the URI `String`; this module
//!    asserts the same cap defensively as a backstop.
//! 2. **No reclamation.** Entries are never freed while the registry is
//!    alive; the registry's lifetime is the grid's lifetime. This avoids
//!    the use-after-free / leak hazards that a refcounted scheme would
//!    have to handle across cell overwrite, RLE split/merge in
//!    `FlatStorage`, scrollback eviction, reflow, and deserialization.

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

impl GetSize for HyperlinkId {}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct HyperlinkRegistry {
    /// Reverse map: hyperlink → id. Lets `intern` dedupe.
    by_link: HashMap<Hyperlink, HyperlinkId>,
    /// Forward array: id → hyperlink. An id's `NonZeroU32` value is its index
    /// in this vec plus one (the `+ 1` keeps the first id non-zero).
    by_id: Vec<Hyperlink>,
    /// Whether we've already logged a warning that the distinct-entries cap
    /// was hit. Untrusted terminal output can emit unlimited unique URIs, so
    /// we warn at most once per registry to avoid log spam. Not serialized:
    /// on restore the new process gets a fresh warning budget.
    #[serde(skip)]
    cap_warning_logged: bool,
}

impl HyperlinkRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Intern a hyperlink. Returns the existing id if `hyperlink` was already
    /// interned, a fresh id if the registry has capacity, or `None` if the
    /// registry has reached `MAX_DISTINCT_ENTRIES` (in which case the caller
    /// should treat the OSC 8 sequence as if it had been malformed and stamp
    /// the visible cells with `None`, so no cell references a missing id).
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
            if !self.cap_warning_logged {
                log::warn!(
                    "HyperlinkRegistry: distinct-entries cap of {MAX_DISTINCT_ENTRIES} reached; dropping new entries (logged once per registry)"
                );
                self.cap_warning_logged = true;
            }
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
#[path = "hyperlink_registry_tests.rs"]
mod tests;
