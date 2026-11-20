/// Helper functions to write CPU kernels.
use crate::backend::BackendStorage;
use crate::nditer::NdIter;
use crate::{Error, Layout, Result, WithDType};

type C = super::CpuStorage;
pub trait Map1 {
    fn f<T: WithDType>(&self, vs: &[T], layout: &Layout) -> Result<Vec<T>>;

    fn map(&self, vs: &C, layout: &Layout) -> Result<C> {
        match vs {
            C::U8(vs) => Ok(C::U8(self.f(vs, layout)?)),
            C::U32(vs) => Ok(C::U32(self.f(vs, layout)?)),
            C::I16(vs) => Ok(C::I16(self.f(vs, layout)?)),
            C::I32(vs) => Ok(C::I32(self.f(vs, layout)?)),
            C::I64(vs) => Ok(C::I64(self.f(vs, layout)?)),
            C::BF16(vs) => Ok(C::BF16(self.f(vs, layout)?)),
            C::F16(vs) => Ok(C::F16(self.f(vs, layout)?)),
            C::F32(vs) => Ok(C::F32(self.f(vs, layout)?)),
            C::F64(vs) => Ok(C::F64(self.f(vs, layout)?)),
            C::F8E4M3(vs) => Ok(C::F8E4M3(self.f(vs, layout)?)),
            // Dummy types don't support Map1 operations
            C::F6E2M3(_) => Err(Error::UnsupportedDTypeForOp(vs.dtype(), "map1").bt()),
            C::F6E3M2(_) => Err(Error::UnsupportedDTypeForOp(vs.dtype(), "map1").bt()),
            C::F4(_) => Err(Error::UnsupportedDTypeForOp(vs.dtype(), "map1").bt()),
            C::F8E8M0(_) => Err(Error::UnsupportedDTypeForOp(vs.dtype(), "map1").bt()),
        }
    }
}

pub trait Map1Any {
    fn f<T: WithDType, W: Fn(Vec<T>) -> C>(&self, vs: &[T], layout: &Layout, wrap: W) -> Result<C>;

    fn map(&self, vs: &C, layout: &Layout) -> Result<C> {
        match vs {
            C::U8(vs) => Ok(self.f(vs, layout, C::U8)?),
            C::U32(vs) => Ok(self.f(vs, layout, C::U32)?),
            C::I16(vs) => Ok(self.f(vs, layout, C::I16)?),
            C::I32(vs) => Ok(self.f(vs, layout, C::I32)?),
            C::I64(vs) => Ok(self.f(vs, layout, C::I64)?),
            C::BF16(vs) => Ok(self.f(vs, layout, C::BF16)?),
            C::F16(vs) => Ok(self.f(vs, layout, C::F16)?),
            C::F32(vs) => Ok(self.f(vs, layout, C::F32)?),
            C::F64(vs) => Ok(self.f(vs, layout, C::F64)?),
            C::F8E4M3(vs) => Ok(self.f(vs, layout, C::F8E4M3)?),
            // Dummy types don't support Map1Any operations
            C::F6E2M3(_) => Err(Error::UnsupportedDTypeForOp(vs.dtype(), "map1any").bt()),
            C::F6E3M2(_) => Err(Error::UnsupportedDTypeForOp(vs.dtype(), "map1any").bt()),
            C::F4(_) => Err(Error::UnsupportedDTypeForOp(vs.dtype(), "map1any").bt()),
            C::F8E8M0(_) => Err(Error::UnsupportedDTypeForOp(vs.dtype(), "map1any").bt()),
        }
    }
}

pub trait Map2 {
    const OP: &'static str;
    fn f<T: WithDType>(&self, v1: &[T], l1: &Layout, v2: &[T], l2: &Layout) -> Result<Vec<T>>;

    fn map(&self, v1: &C, l1: &Layout, v2: &C, l2: &Layout) -> Result<C> {
        match (v1, v2) {
            (C::U8(v1), C::U8(v2)) => Ok(C::U8(self.f(v1, l1, v2, l2)?)),
            (C::U32(v1), C::U32(v2)) => Ok(C::U32(self.f(v1, l1, v2, l2)?)),
            (C::I16(v1), C::I16(v2)) => Ok(C::I16(self.f(v1, l1, v2, l2)?)),
            (C::I32(v1), C::I32(v2)) => Ok(C::I32(self.f(v1, l1, v2, l2)?)),
            (C::I64(v1), C::I64(v2)) => Ok(C::I64(self.f(v1, l1, v2, l2)?)),
            (C::BF16(v1), C::BF16(v2)) => Ok(C::BF16(self.f(v1, l1, v2, l2)?)),
            (C::F16(v1), C::F16(v2)) => Ok(C::F16(self.f(v1, l1, v2, l2)?)),
            (C::F32(v1), C::F32(v2)) => Ok(C::F32(self.f(v1, l1, v2, l2)?)),
            (C::F64(v1), C::F64(v2)) => Ok(C::F64(self.f(v1, l1, v2, l2)?)),
            (C::F8E4M3(v1), C::F8E4M3(v2)) => Ok(C::F8E4M3(self.f(v1, l1, v2, l2)?)),
            _ => Err(Error::DTypeMismatchBinaryOp {
                lhs: v1.dtype(),
                rhs: v2.dtype(),
                op: Self::OP,
            }
            .bt()),
        }
    }
}

pub trait Map2InPlace {
    const OP: &'static str;
    fn f<T: WithDType>(&self, v1: &mut [T], l1: &Layout, v2: &[T], l2: &Layout) -> Result<()>;

    fn map(&self, v1: &mut C, l1: &Layout, v2: &C, l2: &Layout) -> Result<()> {
        match (v1, v2) {
            (C::U8(v1), C::U8(v2)) => self.f(v1, l1, v2, l2)?,
            (C::U32(v1), C::U32(v2)) => self.f(v1, l1, v2, l2)?,
            (C::I16(v1), C::I16(v2)) => self.f(v1, l1, v2, l2)?,
            (C::I32(v1), C::I32(v2)) => self.f(v1, l1, v2, l2)?,
            (C::I64(v1), C::I64(v2)) => self.f(v1, l1, v2, l2)?,
            (C::BF16(v1), C::BF16(v2)) => self.f(v1, l1, v2, l2)?,
            (C::F16(v1), C::F16(v2)) => self.f(v1, l1, v2, l2)?,
            (C::F32(v1), C::F32(v2)) => self.f(v1, l1, v2, l2)?,
            (C::F64(v1), C::F64(v2)) => self.f(v1, l1, v2, l2)?,
            (C::F8E4M3(v1), C::F8E4M3(v2)) => self.f(v1, l1, v2, l2)?,
            (v1, v2) => Err(Error::DTypeMismatchBinaryOp {
                lhs: v1.dtype(),
                rhs: v2.dtype(),
                op: Self::OP,
            }
            .bt())?,
        };
        Ok(())
    }
}

pub trait Map2U8 {
    const OP: &'static str;
    fn f<T: WithDType>(&self, v1: &[T], l1: &Layout, v2: &[T], l2: &Layout) -> Result<Vec<u8>>;

    fn map(&self, v1: &C, l1: &Layout, v2: &C, l2: &Layout) -> Result<C> {
        match (v1, v2) {
            (C::U8(v1), C::U8(v2)) => Ok(C::U8(self.f(v1, l1, v2, l2)?)),
            (C::U32(v1), C::U32(v2)) => Ok(C::U8(self.f(v1, l1, v2, l2)?)),
            (C::I16(v1), C::I16(v2)) => Ok(C::U8(self.f(v1, l1, v2, l2)?)),
            (C::I32(v1), C::I32(v2)) => Ok(C::U8(self.f(v1, l1, v2, l2)?)),
            (C::I64(v1), C::I64(v2)) => Ok(C::U8(self.f(v1, l1, v2, l2)?)),
            (C::BF16(v1), C::BF16(v2)) => Ok(C::U8(self.f(v1, l1, v2, l2)?)),
            (C::F16(v1), C::F16(v2)) => Ok(C::U8(self.f(v1, l1, v2, l2)?)),
            (C::F32(v1), C::F32(v2)) => Ok(C::U8(self.f(v1, l1, v2, l2)?)),
            (C::F64(v1), C::F64(v2)) => Ok(C::U8(self.f(v1, l1, v2, l2)?)),
            (C::F8E4M3(v1), C::F8E4M3(v2)) => Ok(C::U8(self.f(v1, l1, v2, l2)?)),
            _ => Err(Error::DTypeMismatchBinaryOp {
                lhs: v1.dtype(),
                rhs: v2.dtype(),
                op: Self::OP,
            }
            .bt()),
        }
    }
}

pub fn binary_map<T: Copy, U: Copy, F: FnMut(T, T) -> U>(
    lhs_l: &Layout,
    rhs_l: &Layout,
    lhs: &[T],
    rhs: &[T],
    mut f: F,
) -> Vec<U> {
    let el_count = lhs_l.shape().elem_count();
    let mut result: Vec<U> = Vec::with_capacity(el_count);

    let nd_iter = NdIter::new([lhs_l, rhs_l]);
    let inner_size = nd_iter.inner_size;
    let [inner_ls, inner_rs] = nd_iter.inner_strides;

    for [lhs_off, rhs_off] in nd_iter {
        for i in 0..inner_size {
            result.push(f(lhs[lhs_off + i * inner_ls], rhs[rhs_off + i * inner_rs]));
        }
    }

    result
}

// Similar to binary_map but with vectorized variants.
pub fn binary_map_vec<
    T: Copy,
    F: FnMut(T, T) -> T,
    FV: FnMut(&[T], &[T], &mut [T]),
    FSV: FnMut(T, &[T], &mut [T]),
>(
    lhs_l: &Layout,
    rhs_l: &Layout,
    lhs: &[T],
    rhs: &[T],
    mut f: F,
    mut f_vec: FV,
    mut f_scalar_vec: FSV,
) -> Vec<T> {
    let el_count = lhs_l.shape().elem_count();
    let mut ys: Vec<T> = Vec::with_capacity(el_count);
    let ys_to_set = unsafe {
        let s = ys.spare_capacity_mut();
        std::mem::transmute::<&mut [std::mem::MaybeUninit<T>], &mut [T]>(s)
    };

    let nd_iter = NdIter::new([lhs_l, rhs_l]);
    let inner_size = nd_iter.inner_size;
    let [inner_ls, inner_rs] = nd_iter.inner_strides;

    let mut dst_off = 0usize;

    for [lhs_off, rhs_off] in nd_iter {
        match (inner_ls, inner_rs) {
            (1, 1) => f_vec(
                &lhs[lhs_off..lhs_off + inner_size],
                &rhs[rhs_off..rhs_off + inner_size],
                &mut ys_to_set[dst_off..dst_off + inner_size],
            ),
            (1, 0) => {
                let r = rhs[rhs_off];
                f_scalar_vec(
                    r,
                    &lhs[lhs_off..lhs_off + inner_size],
                    &mut ys_to_set[dst_off..dst_off + inner_size],
                );
            }
            (0, 1) => {
                let l = lhs[lhs_off];
                for i in 0..inner_size {
                    ys_to_set[dst_off + i] = f(l, rhs[rhs_off + i]);
                }
            }
            _ => {
                for i in 0..inner_size {
                    ys_to_set[dst_off + i] =
                        f(lhs[lhs_off + i * inner_ls], rhs[rhs_off + i * inner_rs]);
                }
            }
        }
        dst_off += inner_size;
    }

    // SAFETY: all el_count elements have been written in the dispatch loop above.
    unsafe { ys.set_len(el_count) };
    ys
}

pub fn unary_map<T: Copy, U: Copy, F: FnMut(T) -> U>(
    vs: &[T],
    layout: &Layout,
    mut f: F,
) -> Vec<U> {
    match layout.strided_blocks() {
        crate::StridedBlocks::SingleBlock { start_offset, len } => vs
            [start_offset..start_offset + len]
            .iter()
            .map(|&v| f(v))
            .collect(),
        crate::StridedBlocks::UniformBlocks {
            start_offset,
            block_len,
            count,
            src_stride,
        } => {
            let mut result = Vec::with_capacity(count * block_len);
            if block_len == 1 {
                for i in 0..count {
                    let v = unsafe { vs.get_unchecked(start_offset + i * src_stride) };
                    result.push(f(*v))
                }
            } else {
                for i in 0..count {
                    let src_start = start_offset + i * src_stride;
                    for offset in 0..block_len {
                        let v = unsafe { vs.get_unchecked(src_start + offset) };
                        result.push(f(*v))
                    }
                }
            }
            result
        }
        crate::StridedBlocks::MultipleBlocks {
            block_start_index,
            block_len,
        } => {
            let mut result = Vec::with_capacity(layout.shape().elem_count());
            // Specialize the case where block_len is one to avoid the second loop.
            if block_len == 1 {
                for index in block_start_index {
                    let v = unsafe { vs.get_unchecked(index) };
                    result.push(f(*v))
                }
            } else {
                for index in block_start_index {
                    for offset in 0..block_len {
                        let v = unsafe { vs.get_unchecked(index + offset) };
                        result.push(f(*v))
                    }
                }
            }
            result
        }
    }
}

pub fn unary_map_vec<T: Copy, U: Copy, F: FnMut(T) -> U, FV: FnMut(&[T], &mut [U])>(
    vs: &[T],
    layout: &Layout,
    mut f: F,
    mut f_vec: FV,
) -> Vec<U> {
    match layout.strided_blocks() {
        crate::StridedBlocks::SingleBlock { start_offset, len } => {
            let mut ys: Vec<U> = Vec::with_capacity(len);
            let ys_to_set = ys.spare_capacity_mut();
            let ys_to_set = unsafe {
                std::mem::transmute::<&mut [std::mem::MaybeUninit<U>], &mut [U]>(ys_to_set)
            };
            f_vec(&vs[start_offset..start_offset + len], ys_to_set);
            // SAFETY: values are all set by f_vec.
            unsafe { ys.set_len(len) };
            ys
        }
        crate::StridedBlocks::UniformBlocks {
            start_offset,
            block_len,
            count,
            src_stride,
        } => {
            let el_count = count * block_len;
            if block_len == 1 {
                let mut result = Vec::with_capacity(count);
                for i in 0..count {
                    let v = unsafe { vs.get_unchecked(start_offset + i * src_stride) };
                    result.push(f(*v))
                }
                result
            } else {
                let mut ys: Vec<U> = Vec::with_capacity(el_count);
                let ys_to_set = ys.spare_capacity_mut();
                let ys_to_set = unsafe {
                    std::mem::transmute::<&mut [std::mem::MaybeUninit<U>], &mut [U]>(ys_to_set)
                };
                let mut dst_index = 0;
                for i in 0..count {
                    let src_start = start_offset + i * src_stride;
                    f_vec(
                        &vs[src_start..src_start + block_len],
                        &mut ys_to_set[dst_index..dst_index + block_len],
                    );
                    dst_index += block_len;
                }
                // SAFETY: values are all set by f_vec.
                unsafe { ys.set_len(el_count) };
                ys
            }
        }
        crate::StridedBlocks::MultipleBlocks {
            block_start_index,
            block_len,
        } => {
            let el_count = layout.shape().elem_count();
            // Specialize the case where block_len is one to avoid the second loop.
            if block_len == 1 {
                let mut result = Vec::with_capacity(el_count);
                for index in block_start_index {
                    let v = unsafe { vs.get_unchecked(index) };
                    result.push(f(*v))
                }
                result
            } else {
                let mut ys: Vec<U> = Vec::with_capacity(el_count);
                let ys_to_set = ys.spare_capacity_mut();
                let ys_to_set = unsafe {
                    std::mem::transmute::<&mut [std::mem::MaybeUninit<U>], &mut [U]>(ys_to_set)
                };
                let mut dst_index = 0;
                for src_index in block_start_index {
                    let vs = &vs[src_index..src_index + block_len];
                    let ys = &mut ys_to_set[dst_index..dst_index + block_len];
                    f_vec(vs, ys);
                    dst_index += block_len;
                }
                // SAFETY: values are all set by f_vec.
                unsafe { ys.set_len(el_count) };
                ys
            }
        }
    }
}

// Below this many logical elements the barrier-pool split costs more than it saves.
const PAR_ELEMWISE_MIN: usize = 64 * 1024;
// Per-unit span: big enough to amortize dispatch, small enough to balance.
const PAR_ELEMWISE_CHUNK: usize = 16 * 1024;

pub fn unary_map_vec_par<
    T: Copy + Send + Sync,
    U: Copy + Send + Sync,
    F: Fn(T) -> U + Sync,
    FV: Fn(&[T], &mut [U]) + Sync,
>(
    vs: &[T],
    layout: &Layout,
    f: F,
    f_vec: FV,
) -> Vec<U> {
    if let crate::StridedBlocks::SingleBlock { start_offset, len } = layout.strided_blocks() {
        if len >= PAR_ELEMWISE_MIN {
            let src = &vs[start_offset..start_offset + len];
            let mut ys: Vec<U> = Vec::with_capacity(len);
            let ys_ptr = ys.as_mut_ptr() as usize;
            let n_units = len.div_ceil(PAR_ELEMWISE_CHUNK);
            crate::utils::barrier_pool().execute_chunked(n_units, |range| {
                let ys_ptr = ys_ptr as *mut U;
                for unit in range {
                    let lo = unit * PAR_ELEMWISE_CHUNK;
                    let hi = len.min(lo + PAR_ELEMWISE_CHUNK);
                    let dst = unsafe { std::slice::from_raw_parts_mut(ys_ptr.add(lo), hi - lo) };
                    f_vec(&src[lo..hi], dst);
                }
            });
            // SAFETY: every element is written by exactly one unit.
            unsafe { ys.set_len(len) };
            return ys;
        }
    }
    unary_map_vec(vs, layout, f, f_vec)
}

#[allow(clippy::too_many_arguments)]
pub fn binary_map_vec_par<
    T: Copy + Send + Sync,
    F: Fn(T, T) -> T + Sync,
    FV: Fn(&[T], &[T], &mut [T]) + Sync,
    FSV: Fn(T, &[T], &mut [T]) + Sync,
>(
    lhs_l: &Layout,
    rhs_l: &Layout,
    lhs: &[T],
    rhs: &[T],
    f: F,
    f_vec: FV,
    f_scalar_vec: FSV,
) -> Vec<T> {
    let el_count = lhs_l.shape().elem_count();
    if el_count < PAR_ELEMWISE_MIN {
        return binary_map_vec(lhs_l, rhs_l, lhs, rhs, f, f_vec, f_scalar_vec);
    }

    let nd_iter = NdIter::new([lhs_l, rhs_l]);
    let inner_size = nd_iter.inner_size;
    let [lhs_stride, rhs_stride] = nd_iter.inner_strides;
    debug_assert!(inner_size > 0);

    let mut ys: Vec<T> = Vec::with_capacity(el_count);
    let ys_ptr = ys.as_mut_ptr() as usize;
    let n_units = el_count.div_ceil(PAR_ELEMWISE_CHUNK);
    crate::utils::barrier_pool().execute_chunked(n_units, |range| {
        let ys_ptr = ys_ptr as *mut T;
        for unit in range {
            let mut dst_offset = unit * PAR_ELEMWISE_CHUNK;
            let hi = el_count.min(dst_offset + PAR_ELEMWISE_CHUNK);
            let mut inner_offset = dst_offset % inner_size;
            let mut nd_iter = NdIter::new([lhs_l, rhs_l]);
            let mut offsets = nd_iter
                .nth(dst_offset / inner_size)
                .expect("logical output offset must map to an NdIter block");

            while dst_offset < hi {
                let len = (inner_size - inner_offset).min(hi - dst_offset);
                let lhs_offset = offsets[0] + inner_offset * lhs_stride;
                let rhs_offset = offsets[1] + inner_offset * rhs_stride;
                // SAFETY: work units own disjoint output ranges, all written before set_len.
                let dst = unsafe { std::slice::from_raw_parts_mut(ys_ptr.add(dst_offset), len) };

                match (lhs_stride, rhs_stride) {
                    (1, 1) => f_vec(
                        &lhs[lhs_offset..lhs_offset + len],
                        &rhs[rhs_offset..rhs_offset + len],
                        dst,
                    ),
                    (1, 0) => {
                        f_scalar_vec(rhs[rhs_offset], &lhs[lhs_offset..lhs_offset + len], dst)
                    }
                    _ => {
                        for (i, dst) in dst.iter_mut().enumerate() {
                            *dst = f(
                                lhs[lhs_offset + i * lhs_stride],
                                rhs[rhs_offset + i * rhs_stride],
                            );
                        }
                    }
                }

                dst_offset += len;
                inner_offset = 0;
                if dst_offset < hi {
                    offsets = nd_iter
                        .next()
                        .expect("logical output range must stay within NdIter blocks");
                }
            }
        }
    });
    // SAFETY: every element is written by exactly one work unit.
    unsafe { ys.set_len(el_count) };
    ys
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Shape;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn binary_map_vec_par_chunks_non_contiguous_broadcasts() {
        const ROWS: usize = 2;
        const COLS: usize = 2 * PAR_ELEMWISE_CHUNK;
        const LHS_ROW_STRIDE: usize = COLS + 1;

        let mut lhs = vec![i64::MAX; ROWS * LHS_ROW_STRIDE];
        for row in 0..ROWS {
            for col in 0..COLS {
                lhs[row * LHS_ROW_STRIDE + col] = (row * COLS + col) as i64;
            }
        }
        let rhs = [11i64, 29];
        let shape = Shape::from((ROWS, COLS));
        let lhs_l = Layout::new(shape.clone(), vec![LHS_ROW_STRIDE, 1], 0);
        let rhs_l = Layout::new(shape, vec![1, 0], 0);

        let calls = AtomicUsize::new(0);
        let elements = AtomicUsize::new(0);
        let result = binary_map_vec_par(
            &lhs_l,
            &rhs_l,
            &lhs,
            &rhs,
            |lhs, rhs| lhs - rhs,
            |_, _, _| panic!("broadcast layout must not use the two-vector kernel"),
            |scalar, xs, ys| {
                assert!(xs.len() <= PAR_ELEMWISE_CHUNK);
                calls.fetch_add(1, Ordering::Relaxed);
                elements.fetch_add(xs.len(), Ordering::Relaxed);
                for (&x, y) in xs.iter().zip(ys) {
                    *y = x - scalar;
                }
            },
        );

        let expected: Vec<_> = (0..ROWS)
            .flat_map(|row| (0..COLS).map(move |col| (row * COLS + col) as i64 - rhs[row]))
            .collect();
        assert_eq!(result, expected);
        assert_eq!(calls.load(Ordering::Relaxed), 4);
        assert_eq!(elements.load(Ordering::Relaxed), ROWS * COLS);
    }

    #[test]
    fn binary_map_vec_par_handles_two_strided_inputs() {
        const ROWS: usize = 2;
        const COLS: usize = 2 * PAR_ELEMWISE_CHUNK;
        const LHS_ROW_STRIDE: usize = COLS + 1;
        const RHS_ROW_STRIDE: usize = COLS + 2;

        let mut lhs = vec![0i64; ROWS * LHS_ROW_STRIDE];
        let mut rhs = vec![0i64; ROWS * RHS_ROW_STRIDE];
        for row in 0..ROWS {
            for col in 0..COLS {
                lhs[row * LHS_ROW_STRIDE + col] = (row * COLS + col) as i64;
                rhs[row * RHS_ROW_STRIDE + col] = (2 * row * COLS + col) as i64;
            }
        }
        let shape = Shape::from((COLS, ROWS));
        let lhs_l = Layout::new(shape.clone(), vec![1, LHS_ROW_STRIDE], 0);
        let rhs_l = Layout::new(shape, vec![1, RHS_ROW_STRIDE], 0);

        let result = binary_map_vec_par(
            &lhs_l,
            &rhs_l,
            &lhs,
            &rhs,
            |lhs, rhs| lhs - rhs,
            |_, _, _| panic!("strided layout must not use the two-vector kernel"),
            |_, _, _| panic!("strided layout must not use the scalar-vector kernel"),
        );

        let lhs = &lhs;
        let rhs = &rhs;
        let expected: Vec<_> = (0..COLS)
            .flat_map(|col| {
                (0..ROWS).map(move |row| {
                    lhs[row * LHS_ROW_STRIDE + col] - rhs[row * RHS_ROW_STRIDE + col]
                })
            })
            .collect();
        assert_eq!(result, expected);
    }
}
