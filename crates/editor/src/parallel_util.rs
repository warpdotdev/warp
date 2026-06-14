//! Parallelism utilities.

use std::sync::OnceLock;

use rayon::iter::{FromParallelIterator, IntoParallelIterator, ParallelExtend, ParallelIterator};

/// Maximum number of concurrent threads for text layout operations.
///
/// Limiting concurrency prevents unbounded CoreText memory growth when opening large
/// files. Each concurrent layout task holds a `CTFramesetter`, `CTFrame`, and per-line
/// glyph/caret-position vectors in memory simultaneously; capping threads bounds the
/// peak allocation to roughly `TEXT_LAYOUT_MAX_THREADS * <per-task overhead>` instead
/// of `num_cpus * <per-task overhead>`.
const TEXT_LAYOUT_MAX_THREADS: usize = 4;

static TEXT_LAYOUT_POOL: OnceLock<rayon::ThreadPool> = OnceLock::new();

/// Returns the shared, bounded Rayon thread pool used for parallel text layout.
///
/// Callers should wrap parallel layout work in `text_layout_pool().install(|| { ... })`
/// to ensure the bounded pool is used instead of Rayon's global pool.
pub fn text_layout_pool() -> &'static rayon::ThreadPool {
    TEXT_LAYOUT_POOL.get_or_init(|| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(TEXT_LAYOUT_MAX_THREADS)
            .thread_name(|i| format!("warp-text-layout-{i}"))
            .build()
            .expect("failed to build text-layout thread pool")
    })
}

/// `Last` is a helper to extract the last value of a [`ParallelIterator`].
///
/// It can be used with [`ParallelIterator::collect`], [`ParallelIterator::unzip`], and similar
/// methods.
pub struct Last<T> {
    result: Option<T>,
}

impl<T> Last<T> {
    /// Extract the collected value.
    pub fn into_inner(self) -> Option<T> {
        self.result
    }
}

impl<T> Default for Last<T> {
    fn default() -> Self {
        Self { result: None }
    }
}

impl<T: Send> FromParallelIterator<T> for Last<T> {
    fn from_par_iter<I>(par_iter: I) -> Self
    where
        I: IntoParallelIterator<Item = T>,
    {
        let mut last = Self::default();
        last.par_extend(par_iter);
        last
    }
}

impl<T: Send> ParallelExtend<T> for Last<T> {
    fn par_extend<I>(&mut self, par_iter: I)
    where
        I: IntoParallelIterator<Item = T>,
    {
        // The find_last implementation does a bunch of bookkeeping to short-circuit once it finds
        // the most-last match, so rely on that here.
        self.result = par_iter.into_par_iter().find_last(|_| true)
    }
}
