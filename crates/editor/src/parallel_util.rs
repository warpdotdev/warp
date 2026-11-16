//! Parallelism utilities.

use rayon::iter::{FromParallelIterator, IntoParallelIterator, ParallelExtend, ParallelIterator};

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
