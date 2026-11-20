use std::iter::FusedIterator;

use crate::Layout;

/// Maximum tensor rank supported by [`NdIter`].
pub const MAX_DIMS: usize = 8;

/// Multi-dimensional iterator that walks `N` tensor layouts simultaneously.
///
/// Internally, adjacent dimensions are merged whenever their strides allow it,
/// reducing the number of outer iterations. Each call to [`Iterator::next`] yields
/// `[usize; N]`, one base offset per layout, for one inner slice of `inner_size`
/// elements. Callers iterate that slice using `inner_strides`:
///
/// ```ignore
/// let nd_iter = NdIter::new([lhs_l, rhs_l]);
/// let inner_size = nd_iter.inner_size;
/// let [inner_ls, inner_rs] = nd_iter.inner_strides;
/// for [lhs_off, rhs_off] in nd_iter {
///     for i in 0..inner_size {
///         let lhs = lhs_buf[lhs_off + i * inner_ls];
///         let rhs = rhs_buf[rhs_off + i * inner_rs];
///         ...
///     }
/// }
/// ```
///
/// Note: `inner_strides[n]` can be 1 contiguous, but also 0 (broadcast/scalar) or > 1 (non-contiguous),
/// so callers must not assume contiguous element access within a slice.
pub struct NdIter<const N: usize> {
    pub inner_size: usize,
    /// Per-layout element stride within each inner slice: `element_offset = base + i * stride`.
    pub inner_strides: [usize; N],

    outer_dims: [usize; MAX_DIMS],
    outer_strides: [[usize; MAX_DIMS]; N],
    outer_len: usize,

    start_offsets: [usize; N],
    offsets: [usize; N],
    coords: [usize; MAX_DIMS],
    remaining: usize,
}

impl<const N: usize> NdIter<N> {
    pub fn new(layouts: [&Layout; N]) -> NdIter<N> {
        let dims = layouts[0].dims();
        debug_assert!(
            dims.len() <= MAX_DIMS,
            "rank {} exceeds MAX_DIMS={}",
            dims.len(),
            MAX_DIMS
        );
        #[cfg(debug_assertions)]
        for l in &layouts {
            debug_assert_eq!(l.dims(), dims);
        }

        let rank = dims.len();
        let mut out_dims = [0usize; MAX_DIMS];
        let mut out_strides = [[0usize; MAX_DIMS]; N];
        let mut out_len;

        if rank == 0 {
            out_dims[0] = 1;
            out_len = 1;
        } else {
            out_dims[0] = dims[0];
            for n in 0..N {
                out_strides[n][0] = layouts[n].stride()[0];
            }
            out_len = 1;

            for (i, d) in dims.iter().enumerate().take(rank).skip(1) {
                let top = out_len - 1;
                let last_d = out_dims[top];

                let (can_merge, use_inner) = if last_d == 1 {
                    (true, true)
                } else if *d == 1 {
                    (true, false)
                } else {
                    let can_merge =
                        (0..N).all(|n| out_strides[n][top] == layouts[n].stride()[i] * d);
                    (can_merge, true)
                };
                if can_merge {
                    out_dims[top] = last_d * d;
                    if use_inner {
                        for n in 0..N {
                            out_strides[n][top] = layouts[n].stride()[i];
                        }
                    }
                } else {
                    out_dims[out_len] = *d;
                    for n in 0..N {
                        out_strides[n][out_len] = layouts[n].stride()[i];
                    }
                    out_len += 1;
                }
            }
        }

        // Update inner strides
        let inner_idx = out_len - 1;
        let inner_size = out_dims[inner_idx];
        let mut inner_strides = [0usize; N];
        for n in 0..N {
            inner_strides[n] = out_strides[n][inner_idx];
        }

        // Update outer dims
        let outer_len = inner_idx;
        let mut outer_dims = [0usize; MAX_DIMS];
        outer_dims[..outer_len].copy_from_slice(&out_dims[..outer_len]);

        // Update outer strides
        let mut outer_strides = [[0usize; MAX_DIMS]; N];
        for n in 0..N {
            outer_strides[n][..outer_len].copy_from_slice(&out_strides[n][..outer_len]);
        }

        // Update offsets
        let mut offsets = [0usize; N];
        for n in 0..N {
            offsets[n] = layouts[n].start_offset();
        }

        // Number of outer blocks (product of all dims except the innermost).
        let remaining = out_dims[..outer_len].iter().product();

        NdIter {
            inner_size,
            inner_strides,
            outer_dims,
            outer_strides,
            outer_len,
            start_offsets: offsets,
            offsets,
            coords: [0; MAX_DIMS],
            remaining,
        }
    }
}

impl<const N: usize> Iterator for NdIter<N> {
    type Item = [usize; N];

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let item = self.offsets;
        self.remaining -= 1;

        for k in (0..self.outer_len).rev() {
            self.coords[k] += 1;
            for n in 0..N {
                self.offsets[n] += self.outer_strides[n][k];
            }
            if self.coords[k] < self.outer_dims[k] {
                break;
            }
            self.coords[k] = 0;
            for n in 0..N {
                self.offsets[n] -= self.outer_dims[k] * self.outer_strides[n][k];
            }
        }

        Some(item)
    }

    #[inline]
    fn nth(&mut self, n: usize) -> Option<Self::Item> {
        if n >= self.remaining {
            self.remaining = 0;
            return None;
        }

        let total = self.outer_dims[..self.outer_len].iter().product::<usize>();
        let mut position = total - self.remaining + n;
        self.offsets = self.start_offsets;
        self.coords = [0; MAX_DIMS];

        for k in (0..self.outer_len).rev() {
            let coord = position % self.outer_dims[k];
            position /= self.outer_dims[k];
            self.coords[k] = coord;
            for layout in 0..N {
                self.offsets[layout] += coord * self.outer_strides[layout][k];
            }
        }
        self.remaining -= n;
        self.next()
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<const N: usize> ExactSizeIterator for NdIter<N> {}
impl<const N: usize> FusedIterator for NdIter<N> {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shape::Shape;

    fn layout(dims: &[usize], strides: &[usize]) -> Layout {
        Layout::new(Shape::from(dims.to_vec()), strides.to_vec(), 0)
    }

    #[test]
    fn rank0_scalar() {
        let l = Layout::contiguous(());
        let mut it = NdIter::new([&l, &l]);
        assert_eq!(it.inner_size, 1);
        assert_eq!(it.inner_strides, [0, 0]);
        assert_eq!(it.len(), 1);
        assert_eq!(it.next(), Some([0, 0]));
        assert_eq!(it.next(), None);
    }

    #[test]
    fn rank1_contiguous_single_block() {
        let l = Layout::contiguous(&[5]);
        let mut it = NdIter::new([&l, &l]);
        assert_eq!(it.inner_size, 5);
        assert_eq!(it.inner_strides, [1, 1]);
        assert_eq!(it.len(), 1);
        assert_eq!(it.next(), Some([0, 0]));
        assert_eq!(it.next(), None);
    }

    #[test]
    fn rank2_contiguous_merges_to_one_block() {
        // [3, 4] strides[4, 1] -> fully merged, inner_size=12
        let l = Layout::contiguous(&[3, 4]);
        let it = NdIter::new([&l, &l]);
        assert_eq!(it.inner_size, 12);
        assert_eq!(it.inner_strides, [1, 1]);
        assert_eq!(it.len(), 1);
    }

    #[test]
    fn rank3_contiguous_fully_merged() {
        let l = Layout::contiguous(&[2, 3, 4]);
        let it = NdIter::new([&l]);
        assert_eq!(it.inner_size, 24);
        assert_eq!(it.inner_strides, [1]);
        assert_eq!(it.len(), 1);
    }

    #[test]
    fn rank3_outer_gap_partial_merge() {
        // shape [2, 3, 4]
        // strides [24, 4, 1]
        // dims 1 + 2 merge (4 == 1 * 4), dim 0 stays
        let l = layout(&[2, 3, 4], &[24, 4, 1]);
        let it = NdIter::new([&l]);
        assert_eq!(it.inner_size, 12); // 3 * 4 merged
        assert_eq!(it.inner_strides, [1]);
        assert_eq!(it.len(), 2); // outer: dim 0 has size 2
        let offsets: Vec<_> = it.collect();
        assert_eq!(offsets, vec![[0], [24]]);
    }

    #[test]
    fn rank2_no_merge() {
        // shape [3, 4]
        // strides [1, 3]
        // stride[0] = 1 != stride[1] * dim[1] = 12 -> no merge
        let l = layout(&[3, 4], &[1, 3]);
        let it = NdIter::new([&l, &l]);
        assert_eq!(it.inner_size, 4);
        assert_eq!(it.inner_strides, [3, 3]);
        assert_eq!(it.len(), 3);
        let offsets: Vec<_> = it.collect();
        assert_eq!(offsets, vec![[0, 0], [1, 1], [2, 2]]);
    }

    #[test]
    fn broadcast_zeros_merge() {
        // shape [3, 4]
        // strides [0, 0]
        // 0 == 0 * 4 -> dims merge
        let l = layout(&[3, 4], &[0, 0]);
        let it = NdIter::new([&l, &l]);
        assert_eq!(it.inner_size, 12);
        assert_eq!(it.inner_strides, [0, 0]);
        assert_eq!(it.len(), 1);
    }

    #[test]
    fn mixed_contiguous_and_broadcast_merge() {
        // lhs [4, 1] contiguous
        // rhs [0,0] broadcast
        // both conditions pass -> merge
        let lhs = Layout::contiguous(&[3, 4]);
        let rhs = layout(&[3, 4], &[0, 0]);
        let it = NdIter::new([&lhs, &rhs]);
        assert_eq!(it.inner_size, 12);
        assert_eq!(it.inner_strides, [1, 0]);
        assert_eq!(it.len(), 1);
    }

    #[test]
    fn offsets_lhs_contiguous_rhs_strided() {
        // lhs row-major [2,3] strides [3,1]
        // rhs col-major [2,3] strides [1,2]
        // lhs can merge (3 == 1 * 3)
        // rhs can not (1 ≠ 2 * 3 = 6) -> no merge for the pair
        let lhs = Layout::contiguous(&[2, 3]);
        let rhs = layout(&[2, 3], &[1, 2]);
        let it = NdIter::new([&lhs, &rhs]);
        assert_eq!(it.inner_size, 3);
        assert_eq!(it.inner_strides, [1, 2]);
        assert_eq!(it.len(), 2);
        let offsets: Vec<_> = it.collect();
        assert_eq!(offsets, vec![[0, 0], [3, 1]]);
    }

    #[test]
    fn start_offset_reflected_in_first_iter() {
        let l = Layout::contiguous_with_offset(4, 7);
        let mut it = NdIter::new([&l]);
        assert_eq!(it.next(), Some([7]));
        assert_eq!(it.next(), None);
    }

    #[test]
    fn start_offset_advances_with_outer_dims() {
        // shape [2, 3]
        // strides [4, 1]
        // start_offset=10
        // strides can't merge (4 != 1 * 3)
        // outer blocks at 10 and 14 (10 + 1 * outer_stride)
        let l = Layout::new(Shape::from(vec![2, 3]), vec![4, 1], 10);
        let offsets: Vec<_> = NdIter::new([&l]).collect();
        assert_eq!(offsets, vec![[10], [14]]);
    }

    #[test]
    fn nth_jumps_to_outer_block() {
        let lhs = Layout::new(Shape::from(vec![2, 3, 4]), vec![40, 10, 1], 5);
        let rhs = Layout::new(Shape::from(vec![2, 3, 4]), vec![50, 12, 1], 7);
        let expected: Vec<_> = NdIter::new([&lhs, &rhs]).collect();

        for (n, &offsets) in expected.iter().enumerate() {
            assert_eq!(NdIter::new([&lhs, &rhs]).nth(n), Some(offsets));
        }
        assert_eq!(NdIter::new([&lhs, &rhs]).nth(expected.len()), None);

        let mut iter = NdIter::new([&lhs, &rhs]);
        assert_eq!(iter.next(), Some(expected[0]));
        assert_eq!(iter.nth(2), Some(expected[3]));
        assert_eq!(iter.next(), Some(expected[4]));
        assert_eq!(iter.len(), expected.len() - 5);
    }
}
