//! Configuration for the global allocator.

use cfg_if::cfg_if;

cfg_if! {
    if #[cfg(feature = "dhat_heap_profiling")] {
        #[global_allocator]
        static GLOBAL: dhat::Alloc = dhat::Alloc;
    } else if #[cfg(feature = "jemalloc")] {
        #[global_allocator]
        static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;
    }
}

/// Asks the allocator to return every page it is holding but not using back to
/// the OS, reporting whether a purge was actually performed.
///
/// jemalloc only advances its dirty-page decay while the process is
/// allocating, and we ship it without `background_thread`, so an idle Warp can
/// sit indefinitely on gigabytes of freed-but-unreturned pages. Those pages are
/// cold, so macOS compresses them, and they keep counting towards
/// `phys_footprint` -- the number Activity Monitor shows and the number the
/// excessive-memory check thresholds on.
///
/// This call is blocking and walks every arena, so it belongs on a background
/// thread rather than the main thread.
pub fn purge_unused_pages() -> bool {
    #[cfg(feature = "jemalloc")]
    {
        // `MALLCTL_ARENAS_ALL` from jemalloc's public API: applies the
        // operation to every arena at once.
        const ALL_ARENAS: u32 = 4096;

        let name = std::ffi::CString::new(format!("arena.{ALL_ARENAS}.purge"))
            .expect("mallctl name should not contain an interior NUL");

        // SAFETY: `arena.<i>.purge` is a write-only mallctl that takes no
        // value, so jemalloc requires all four value pointers to be null.
        let result = unsafe {
            tikv_jemalloc_sys::mallctl(
                name.as_ptr(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                0,
            )
        };

        if result != 0 {
            log::warn!("Failed to purge unused allocator pages: mallctl returned {result}");
            return false;
        }

        true
    }

    #[cfg(not(feature = "jemalloc"))]
    false
}

#[cfg(test)]
#[path = "alloc_tests.rs"]
mod tests;
