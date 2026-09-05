use std::cmp::Ordering;

pub use warp_terminal::util::extensions::TrimStringExt;

pub trait SliceExt<T: 'static> {
    fn find_insertion_index<'a, F, E>(&'a self, compare: F) -> Result<usize, E>
    where
        F: FnMut(&'a T) -> Result<Ordering, E>;
}

impl<T: 'static> SliceExt<T> for [T] {
    fn find_insertion_index<'a, F, E>(&'a self, mut f: F) -> Result<usize, E>
    where
        F: FnMut(&'a T) -> Result<Ordering, E>,
    {
        use Ordering::*;

        let mut size = self.len();
        if size == 0 {
            return Ok(0);
        }
        let mut base = 0usize;
        while size > 1 {
            let half = size / 2;
            let mid = base + half;
            // mid is always in [0, size), that means mid is >= 0 and < size.
            // mid >= 0: by definition
            // mid < size: mid = size / 2 + size / 4 + size / 8 ...
            let cmp = f(unsafe { self.get_unchecked(mid) })?;
            base = if cmp == Greater { base } else { mid };
            size -= half;
        }
        // base is always in [0, size) because base <= mid.
        let cmp = f(unsafe { self.get_unchecked(base) })?;
        if cmp == Equal {
            Ok(base)
        } else {
            Ok(base + (cmp == Less) as usize)
        }
    }
}
