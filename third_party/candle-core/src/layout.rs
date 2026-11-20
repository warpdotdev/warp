//! Tensor Layouts including contiguous or sparse strides

use crate::{Error, Result, Shape};

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Layout {
    shape: Shape,
    // The strides are given in number of elements and not in bytes.
    stride: Vec<usize>,
    start_offset: usize,
}

impl Layout {
    pub fn new(shape: Shape, stride: Vec<usize>, start_offset: usize) -> Self {
        Self {
            shape,
            stride,
            start_offset,
        }
    }

    pub fn contiguous_with_offset<S: Into<Shape>>(shape: S, start_offset: usize) -> Self {
        let shape = shape.into();
        let stride = shape.stride_contiguous();
        Self {
            shape,
            stride,
            start_offset,
        }
    }

    pub fn contiguous<S: Into<Shape>>(shape: S) -> Self {
        Self::contiguous_with_offset(shape, 0)
    }

    pub fn dims(&self) -> &[usize] {
        self.shape.dims()
    }

    /// The dimension size for a specified dimension index.
    pub fn dim<D: crate::shape::Dim>(&self, dim: D) -> Result<usize> {
        let dim = dim.to_index(&self.shape, "dim")?;
        Ok(self.dims()[dim])
    }

    pub fn shape(&self) -> &Shape {
        &self.shape
    }

    pub fn stride(&self) -> &[usize] {
        &self.stride
    }

    pub fn start_offset(&self) -> usize {
        self.start_offset
    }

    /// Returns outer stride along `dim` if valid.
    ///
    /// Two conditions must hold:
    ///  1. Inner dims `[dim..]` has standard contiguous strides.
    ///  2. Outer dims `[..dim]` are contiguous among themselves, i.e.
    ///     `stride[k] == dims[k+1] * stride[k+1]` for `k` in `0..dim-1`.
    ///
    /// When the tensor is fully contiguous this returns `Some(dims[dim..].product())`.
    pub(crate) fn outer_stride_for_dim(&self, dim: usize) -> Option<usize> {
        let dims = self.dims();
        let strides = self.stride();

        // 1. Inner `dims[dim..]` must have contiguous strides.
        let mut expected = 1usize;
        for i in (dim..dims.len()).rev() {
            if strides[i] != expected {
                return None;
            }
            expected *= dims[i];
        }

        if dim == 0 {
            // No outer dims.
            // `expected = dims[dim..].product()`
            return Some(expected);
        }

        // 2. Outer `dims[0..dim]` must be internally contiguous.
        let outer_stride = strides[dim - 1];
        let mut expected_outer = outer_stride;
        for k in (0..dim - 1).rev() {
            expected_outer *= dims[k + 1];
            if strides[k] != expected_outer {
                return None;
            }
        }

        Some(outer_stride)
    }

    /// Checks if more than one logical index lands on the same cell (or when we can't prove it does not)
    pub fn has_internal_overlap(&self) -> bool {
        !self.range().is_some_and(|f| f.injective)
    }

    /// Returns range of cells this layout can reach, and wether it reaches all of them.
    /// Returns `None` on arithmetic overflow.
    fn range(&self) -> Option<LayoutRange> {
        // Filter out dims <= 1 as their strides are irrelevant.
        let mut axes: Vec<(usize, usize)> = self
            .dims()
            .iter()
            .zip(self.stride())
            .filter(|(&d, _)| d > 1)
            .map(|(&d, &s)| (d, s))
            .collect();
        axes.sort_unstable_by_key(|&(_, s)| s);

        let mut span = 0usize;
        let mut injective = true;
        let mut dense = true;

        for (d, s) in axes {
            if s <= span {
                // This dim can land on a cell another dim already reaches.
                injective = false;
                dense = false;
            } else if s - span != 1 {
                // Has a gap
                dense = false;
            }
            span = span.checked_add((d - 1).checked_mul(s)?)?;
        }

        let lo = self.start_offset();
        Some(LayoutRange {
            lo,
            hi: lo.checked_add(span)?,
            injective,
            dense,
        })
    }

    /// Relation between this layout and another
    pub fn relation(&self, other: &Self) -> LayoutRelation {
        if self == other {
            return LayoutRelation::Identical;
        }

        if self.shape().elem_count() == 0 || other.shape().elem_count() == 0 {
            return LayoutRelation::Disjoint;
        }

        // Extract [`LayoutRange`] from layout.
        let (a, b) = match (self.range(), other.range()) {
            (Some(a), Some(b)) => (a, b),
            _ => return LayoutRelation::Unknown, // address arithmetic overflowed
        };

        if a.separated_from(&b) {
            return LayoutRelation::Disjoint;
        }

        if a.densely_contains(&b) || b.densely_contains(&a) {
            return LayoutRelation::Overlapping;
        }

        // We end up here when layouts ranges overlap and neither is dense, such as disjoint
        // column slices. Figuring out these cases is a bounded integer feasibility problem
        // over the strides. This is NP-hard in general. Cheap at common tensor ranks.
        // If we want to move more cases out of unknown into disjoint/overlapping it can be done using the
        // same approach as numpy: https://github.com/numpy/numpy/blob/main/numpy/_core/src/common/mem_overlap.c
        LayoutRelation::Unknown
    }

    /// Returns the appropriate start and stop offset if the data is stored in a C
    /// contiguous (aka row major) way.
    pub fn contiguous_offsets(&self) -> Option<(usize, usize)> {
        if self.is_contiguous() {
            let start_o = self.start_offset;
            Some((start_o, start_o + self.shape.elem_count()))
        } else {
            None
        }
    }

    /// Returns true if the data is stored in a C contiguous (aka row major) way.
    /// Note that this does not implies that the start offset is 0 or that there are no extra
    /// elements at the end of the storage.
    pub fn is_contiguous(&self) -> bool {
        self.shape.is_contiguous(&self.stride)
    }

    /// Returns true if the data is stored in a Fortran contiguous (aka column major) way.
    pub fn is_fortran_contiguous(&self) -> bool {
        self.shape.is_fortran_contiguous(&self.stride)
    }

    pub fn is_scalar(&self) -> bool {
        let dims = self.dims();
        dims.is_empty() || dims.iter().all(|d| *d == 1)
    }

    /// Returns true if the data is actually a scalar during broadcast
    pub fn is_scalar_broadcast(&self) -> bool {
        self.stride().iter().all(|s| *s == 0)
    }

    pub fn is_scalar_like(&self) -> bool {
        self.is_scalar() || self.is_scalar_broadcast()
    }

    pub fn narrow(&self, dim: usize, start: usize, len: usize) -> Result<Self> {
        let dims = self.shape().dims();
        if dim >= dims.len() {
            Err(Error::DimOutOfRange {
                shape: self.shape().clone(),
                dim: dim as i32,
                op: "narrow",
            }
            .bt())?
        }
        if start + len > dims[dim] {
            Err(Error::NarrowInvalidArgs {
                shape: self.shape.clone(),
                dim,
                start,
                len,
                msg: "start + len > dim_len",
            }
            .bt())?
        }
        let mut dims = dims.to_vec();
        dims[dim] = len;
        Ok(Self {
            shape: Shape::from(dims),
            stride: self.stride.clone(),
            start_offset: self.start_offset + self.stride[dim] * start,
        })
    }

    pub fn transpose(&self, dim1: usize, dim2: usize) -> Result<Self> {
        let rank = self.shape.rank();
        if rank <= dim1 || rank <= dim2 {
            Err(Error::UnexpectedNumberOfDims {
                expected: usize::max(dim1, dim2),
                got: rank,
                shape: self.shape().clone(),
            }
            .bt())?
        }
        let mut stride = self.stride().to_vec();
        let mut dims = self.shape().dims().to_vec();
        dims.swap(dim1, dim2);
        stride.swap(dim1, dim2);
        Ok(Self {
            shape: Shape::from(dims),
            stride,
            start_offset: self.start_offset,
        })
    }

    pub fn permute(&self, idxs: &[usize]) -> Result<Self> {
        let is_permutation =
            idxs.len() == self.shape.rank() && (0..idxs.len()).all(|i| idxs.contains(&i));
        if !is_permutation {
            crate::bail!(
                "dimension mismatch in permute, tensor {:?}, dims: {:?}",
                self.dims(),
                idxs
            )
        }
        let stride = self.stride();
        let dims = self.shape().dims();
        let mut perm_stride = stride.to_vec();
        let mut perm_dims = dims.to_vec();
        for (i, &idx) in idxs.iter().enumerate() {
            perm_stride[i] = stride[idx];
            perm_dims[i] = dims[idx];
        }
        Ok(Self {
            shape: Shape::from(perm_dims),
            stride: perm_stride,
            start_offset: self.start_offset,
        })
    }

    pub fn broadcast_as<S: Into<Shape>>(&self, shape: S) -> Result<Self> {
        let shape = shape.into();
        if shape.rank() < self.shape().rank() {
            return Err(Error::BroadcastIncompatibleShapes {
                src_shape: self.shape().clone(),
                dst_shape: shape,
            }
            .bt());
        }
        let added_dims = shape.rank() - self.shape().rank();
        let mut stride = vec![0; added_dims];
        for (&dst_dim, (&src_dim, &src_stride)) in shape.dims()[added_dims..]
            .iter()
            .zip(self.dims().iter().zip(self.stride()))
        {
            let s = if dst_dim == src_dim {
                src_stride
            } else if src_dim != 1 {
                return Err(Error::BroadcastIncompatibleShapes {
                    src_shape: self.shape().clone(),
                    dst_shape: shape,
                }
                .bt());
            } else {
                0
            };
            stride.push(s)
        }
        Ok(Self {
            shape,
            stride,
            start_offset: self.start_offset,
        })
    }

    pub(crate) fn strided_index(&self) -> crate::StridedIndex<'_> {
        crate::StridedIndex::from_layout(self)
    }

    pub(crate) fn strided_blocks(&self) -> crate::StridedBlocks<'_> {
        let mut block_len = 1usize;
        let mut contiguous_dims = 0usize; // Counted from the right.
        for (&stride, &dim) in self.stride().iter().zip(self.dims().iter()).rev() {
            // Size-1 dimensions are trivially contiguous regardless of their stride.
            if dim == 1 {
                contiguous_dims += 1;
                continue;
            }
            if stride != block_len {
                break;
            }
            block_len *= dim;
            contiguous_dims += 1;
        }
        let index_dims = self.dims().len() - contiguous_dims;
        match index_dims {
            0 => crate::StridedBlocks::SingleBlock {
                start_offset: self.start_offset,
                len: block_len,
            },
            1 => crate::StridedBlocks::UniformBlocks {
                start_offset: self.start_offset,
                block_len,
                count: self.dims()[0],
                src_stride: self.stride[0],
            },
            _ => {
                let block_start_index = crate::StridedIndex::new(
                    &self.dims()[..index_dims],
                    &self.stride[..index_dims],
                    self.start_offset,
                );
                crate::StridedBlocks::MultipleBlocks {
                    block_start_index,
                    block_len,
                }
            }
        }
    }
}

/// Describes the range of cells a [`Layout`] can reach within its allocation,
/// and whether it reaches all of them.
struct LayoutRange {
    /// Lowest reachable cell. Equal to layout `start_offset`.
    lo: usize,
    /// Highest reachable cell (inclusive).
    hi: usize,
    /// Proven to map distinct logical indices to distinct cells.
    injective: bool,
    /// Indicates that layout occupies every cell in `lo..=hi`.
    /// Does not necessarily mean that layout is contiguous.
    dense: bool,
}

impl LayoutRange {
    /// Bounding intervals cannot meet, which means these layouts are seperate.
    fn separated_from(&self, other: &Self) -> bool {
        self.hi < other.lo || other.hi < self.lo
    }

    /// If a dense layout contains another's `lo` there is overlap.
    fn densely_contains(&self, other: &Self) -> bool {
        self.dense && other.lo >= self.lo && other.lo <= self.hi
    }
}

/// How two layouts over the same allocation relate.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum LayoutRelation {
    /// Simple assignment. x[i] += x[i]
    Identical,
    /// Completely distinct layouts.
    Disjoint,
    /// Any kind of overlap.
    Overlapping,
    /// Could not prove relation.
    Unknown,
}
