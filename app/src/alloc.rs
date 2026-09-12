//! Configuration for the global allocator.

use cfg_if::cfg_if;
use serde::Serialize;

cfg_if! {
    if #[cfg(feature = "dhat_heap_profiling")] {
        #[global_allocator]
        static GLOBAL: dhat::Alloc = dhat::Alloc;
    } else if #[cfg(feature = "jemalloc")] {
        #[global_allocator]
        static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;
    }
}

/// A snapshot of the allocator's own accounting, in bytes.
///
/// OS-level virtual memory counters cannot distinguish memory the application
/// is still using from memory the allocator has freed but not yet returned to
/// the OS -- both are simply resident, dirty pages. These figures can:
/// [`AllocatorStats::allocated`] counts only live allocations, so comparing it
/// against [`AllocatorStats::resident`] separates a genuine leak from memory
/// the allocator is merely holding on to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct AllocatorStats {
    /// Bytes in live allocations still held by the application.
    pub allocated: u64,
    /// Bytes in extents the allocator is actively using to service
    /// allocations. The excess over `allocated` is fragmentation.
    pub active: u64,
    /// Bytes of the allocator's own bookkeeping overhead.
    pub metadata: u64,
    /// Bytes the allocator believes are resident in physical memory. The
    /// excess over `active` is dirty pages awaiting purge.
    pub resident: u64,
    /// Bytes mapped into the process address space.
    pub mapped: u64,
    /// Bytes of virtual memory retained by the allocator instead of being
    /// returned to the OS.
    pub retained: u64,
}

/// Returns a snapshot of the global allocator's statistics.
///
/// Returns `None` for builds that do not use jemalloc, or if the allocator
/// declines to report its statistics.
pub fn allocator_stats() -> Option<AllocatorStats> {
    cfg_if! {
        if #[cfg(feature = "jemalloc")] {
            use tikv_jemalloc_ctl::{epoch, stats};

            // jemalloc caches its statistics and only recomputes them when the
            // epoch is advanced, so skipping this would report the values as
            // they stood at the first ever read.
            if let Err(err) = epoch::advance() {
                log::warn!("Failed to advance jemalloc epoch: {err}");
                return None;
            }

            Some(AllocatorStats {
                allocated: stats::allocated::read().ok()? as u64,
                active: stats::active::read().ok()? as u64,
                metadata: stats::metadata::read().ok()? as u64,
                resident: stats::resident::read().ok()? as u64,
                mapped: stats::mapped::read().ok()? as u64,
                retained: stats::retained::read().ok()? as u64,
            })
        } else {
            None
        }
    }
}

#[cfg(test)]
#[path = "alloc_tests.rs"]
mod tests;
