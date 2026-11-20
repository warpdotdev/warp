use super::{Cpu, CpuBF16, CpuF16};
use core::arch::wasm32::*;
use half::{bf16, f16};

pub struct CurrentCpu {}

const STEP: usize = 16;
const EPR: usize = 4;
const ARR: usize = STEP / EPR;

impl Cpu for CurrentCpu {
    type Unit = v128;
    type Array = [v128; ARR];

    const STEP: usize = STEP;
    const EPR: usize = EPR;
    const ARR: usize = ARR;

    unsafe fn zero() -> Self::Unit {
        f32x4_splat(0.0)
    }

    unsafe fn zero_array() -> Self::Array {
        [Self::zero(); ARR]
    }

    unsafe fn from_f32(v: f32) -> Self::Unit {
        f32x4_splat(v)
    }

    unsafe fn load(mem_addr: *const f32) -> Self::Unit {
        v128_load(mem_addr as *mut v128)
    }

    unsafe fn vec_add(a: Self::Unit, b: Self::Unit) -> Self::Unit {
        f32x4_add(a, b)
    }

    unsafe fn vec_fma(a: Self::Unit, b: Self::Unit, c: Self::Unit) -> Self::Unit {
        f32x4_add(f32x4_mul(b, c), a)
    }

    unsafe fn vec_store(mem_addr: *mut f32, a: Self::Unit) {
        v128_store(mem_addr as *mut v128, a);
    }

    unsafe fn vec_reduce(mut x: Self::Array, y: *mut f32) {
        for i in 0..ARR / 2 {
            x[2 * i] = f32x4_add(x[2 * i], x[2 * i + 1]);
        }
        for i in 0..ARR / 4 {
            x[4 * i] = f32x4_add(x[4 * i], x[4 * i + 2]);
        }
        for i in 0..ARR / 8 {
            x[8 * i] = f32x4_add(x[8 * i], x[8 * i + 4]);
        }
        *y = f32x4_extract_lane::<0>(x[0])
            + f32x4_extract_lane::<1>(x[0])
            + f32x4_extract_lane::<2>(x[0])
            + f32x4_extract_lane::<3>(x[0]);
    }
}

// WebAssembly SIMD has no half-precision arithmetic, so widen f16 lanes to
// f32x4 as the AVX software fallback and the NEON fallback do.
pub struct CurrentCpuF16 {}

impl CpuF16 for CurrentCpuF16 {
    type Unit = v128;
    type Array = [v128; ARR];

    const STEP: usize = STEP;
    const EPR: usize = EPR;
    const ARR: usize = ARR;

    unsafe fn zero() -> Self::Unit {
        <CurrentCpu as Cpu>::zero()
    }

    unsafe fn zero_array() -> Self::Array {
        <CurrentCpu as Cpu>::zero_array()
    }

    unsafe fn from_f32(v: f32) -> Self::Unit {
        <CurrentCpu as Cpu>::from_f32(v)
    }

    unsafe fn load(mem_addr: *const f16) -> Self::Unit {
        let values = [
            (*mem_addr).to_f32(),
            (*mem_addr.add(1)).to_f32(),
            (*mem_addr.add(2)).to_f32(),
            (*mem_addr.add(3)).to_f32(),
        ];
        v128_load(values.as_ptr().cast())
    }

    unsafe fn vec_add(a: Self::Unit, b: Self::Unit) -> Self::Unit {
        <CurrentCpu as Cpu>::vec_add(a, b)
    }

    unsafe fn vec_fma(a: Self::Unit, b: Self::Unit, c: Self::Unit) -> Self::Unit {
        <CurrentCpu as Cpu>::vec_fma(a, b, c)
    }

    unsafe fn vec_store(mem_addr: *mut f16, a: Self::Unit) {
        let mut values = [0f32; EPR];
        v128_store(values.as_mut_ptr().cast(), a);
        for (i, value) in values.into_iter().enumerate() {
            *mem_addr.add(i) = f16::from_f32(value);
        }
    }

    unsafe fn vec_reduce(x: Self::Array, y: *mut f32) {
        <CurrentCpu as Cpu>::vec_reduce(x, y)
    }
}

// bf16 is the upper half of an f32. Widen its bits before doing arithmetic in
// f32x4, then use half's scalar conversion to preserve round-to-even on store.
pub struct CurrentCpuBF16 {}

impl CpuBF16 for CurrentCpuBF16 {
    type Unit = v128;
    type Array = [v128; ARR];

    const STEP: usize = STEP;
    const EPR: usize = EPR;
    const ARR: usize = ARR;

    unsafe fn zero() -> Self::Unit {
        <CurrentCpu as Cpu>::zero()
    }

    unsafe fn zero_array() -> Self::Array {
        <CurrentCpu as Cpu>::zero_array()
    }

    unsafe fn from_f32(v: f32) -> Self::Unit {
        <CurrentCpu as Cpu>::from_f32(v)
    }

    unsafe fn load(mem_addr: *const bf16) -> Self::Unit {
        let values = v128_load64_zero(mem_addr.cast());
        u32x4_shl(u32x4_extend_low_u16x8(values), 16)
    }

    unsafe fn vec_add(a: Self::Unit, b: Self::Unit) -> Self::Unit {
        <CurrentCpu as Cpu>::vec_add(a, b)
    }

    unsafe fn vec_fma(a: Self::Unit, b: Self::Unit, c: Self::Unit) -> Self::Unit {
        <CurrentCpu as Cpu>::vec_fma(a, b, c)
    }

    unsafe fn vec_store(mem_addr: *mut bf16, a: Self::Unit) {
        let mut values = [0f32; EPR];
        v128_store(values.as_mut_ptr().cast(), a);
        for (i, value) in values.into_iter().enumerate() {
            *mem_addr.add(i) = bf16::from_f32(value);
        }
    }

    unsafe fn vec_reduce(x: Self::Array, y: *mut f32) {
        <CurrentCpu as Cpu>::vec_reduce(x, y)
    }
}

pub(crate) unsafe fn vec_dot_f32(a_row: *const f32, b_row: *const f32, c: *mut f32, k: usize) {
    let mut sum = CurrentCpu::zero_array();
    let mut i = 0;
    while i + CurrentCpu::STEP <= k {
        for j in 0..CurrentCpu::ARR {
            sum[j] = CurrentCpu::vec_fma(
                sum[j],
                CurrentCpu::load(a_row.add(i + j * CurrentCpu::EPR)),
                CurrentCpu::load(b_row.add(i + j * CurrentCpu::EPR)),
            );
        }
        i += CurrentCpu::STEP;
    }
    CurrentCpu::vec_reduce(sum, c);
    while i < k {
        *c += *a_row.add(i) * *b_row.add(i);
        i += 1;
    }
}

pub(crate) unsafe fn vec_sum(row: *const f32, b: *mut f32, k: usize) {
    let np = k & !(CurrentCpu::STEP - 1);
    let mut sum = CurrentCpu::zero_array();
    let mut x = CurrentCpu::zero_array();
    for i in (0..np).step_by(CurrentCpu::STEP) {
        for j in 0..CurrentCpu::ARR {
            x[j] = CurrentCpu::load(row.add(i + j * CurrentCpu::EPR));
            sum[j] = CurrentCpu::vec_add(sum[j], x[j]);
        }
    }
    CurrentCpu::vec_reduce(sum, b);
    for i in np..k {
        *b += *row.add(i);
    }
}

pub(crate) unsafe fn vec_add_f16(a_row: *const f16, b_row: *const f16, c: *mut f16, k: usize) {
    let mut i = 0;
    while i + CurrentCpuF16::STEP <= k {
        for j in 0..CurrentCpuF16::ARR {
            CurrentCpuF16::vec_store(
                c.add(i + j * CurrentCpuF16::EPR),
                CurrentCpuF16::vec_add(
                    CurrentCpuF16::load(a_row.add(i + j * CurrentCpuF16::EPR)),
                    CurrentCpuF16::load(b_row.add(i + j * CurrentCpuF16::EPR)),
                ),
            );
        }
        i += CurrentCpuF16::STEP;
    }
    for j in i..k {
        *c.add(j) = *a_row.add(j) + *b_row.add(j);
    }
}

pub(crate) unsafe fn vec_add_bf16(a_row: *const bf16, b_row: *const bf16, c: *mut bf16, k: usize) {
    let mut i = 0;
    while i + CurrentCpuBF16::STEP <= k {
        for j in 0..CurrentCpuBF16::ARR {
            CurrentCpuBF16::vec_store(
                c.add(i + j * CurrentCpuBF16::EPR),
                CurrentCpuBF16::vec_add(
                    CurrentCpuBF16::load(a_row.add(i + j * CurrentCpuBF16::EPR)),
                    CurrentCpuBF16::load(b_row.add(i + j * CurrentCpuBF16::EPR)),
                ),
            );
        }
        i += CurrentCpuBF16::STEP;
    }
    for j in i..k {
        *c.add(j) = *a_row.add(j) + *b_row.add(j);
    }
}

pub(crate) unsafe fn vec_scalar_add_f16(scalar: f16, xs: *const f16, ys: *mut f16, k: usize) {
    let sv = CurrentCpuF16::from_f32(scalar.to_f32());
    let mut i = 0;
    while i + CurrentCpuF16::STEP <= k {
        for j in 0..CurrentCpuF16::ARR {
            CurrentCpuF16::vec_store(
                ys.add(i + j * CurrentCpuF16::EPR),
                CurrentCpuF16::vec_add(CurrentCpuF16::load(xs.add(i + j * CurrentCpuF16::EPR)), sv),
            );
        }
        i += CurrentCpuF16::STEP;
    }
    for j in i..k {
        *ys.add(j) = *xs.add(j) + scalar;
    }
}

pub(crate) unsafe fn vec_scalar_add_bf16(scalar: bf16, xs: *const bf16, ys: *mut bf16, k: usize) {
    let sv = CurrentCpuBF16::from_f32(scalar.to_f32());
    let mut i = 0;
    while i + CurrentCpuBF16::STEP <= k {
        for j in 0..CurrentCpuBF16::ARR {
            CurrentCpuBF16::vec_store(
                ys.add(i + j * CurrentCpuBF16::EPR),
                CurrentCpuBF16::vec_add(
                    CurrentCpuBF16::load(xs.add(i + j * CurrentCpuBF16::EPR)),
                    sv,
                ),
            );
        }
        i += CurrentCpuBF16::STEP;
    }
    for j in i..k {
        *ys.add(j) = *xs.add(j) + scalar;
    }
}
