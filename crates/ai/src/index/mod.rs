mod file_outline;
pub mod locations;
pub const DEFAULT_SYNC_REQUESTS_PER_MIN: u32 = 600;

#[allow(dead_code)]
pub mod full_source_code_embedding;

#[cfg(feature = "local_fs")]
pub use file_outline::build_outline;
pub use file_outline::{Outline, Symbol};
pub use repo_metadata::{BuildTreeError, DirectoryEntry, Entry, FileId, FileMetadata};

cfg_if::cfg_if! {
    if #[cfg(feature = "local_fs")] {
        pub use repo_metadata::entry::{is_git_internal_path, should_watch_directory_in_git_path};
        pub use repo_metadata::matches_gitignores;
    }
}

#[cfg(feature = "local_fs")]
use native::*;
#[cfg(not(feature = "local_fs"))]
use wasm::*;

#[cfg(feature = "local_fs")]
mod native {
    use std::thread::available_parallelism;

    pub(super) const MAX_PARALLEL_THREADS: usize = 2;

    fn create_thread_pool() -> Option<rayon::ThreadPool> {
        let num_threads = available_parallelism()
            .map(|parallelism| (parallelism.get() / 2).clamp(1, MAX_PARALLEL_THREADS))
            .unwrap_or(MAX_PARALLEL_THREADS);

        rayon::ThreadPoolBuilder::new()
            .thread_name(|index| format!("warp-code-indexing-{index}"))
            .num_threads(num_threads)
            .build()
            .ok()
    }

    lazy_static::lazy_static! {
        pub(super) static ref THREADPOOL: Option<rayon::ThreadPool> = create_thread_pool();
    }
}

#[cfg(not(feature = "local_fs"))]
mod wasm {
    lazy_static::lazy_static! {
        pub(super) static ref THREADPOOL: Option<rayon::ThreadPool> = None;
    }
}
