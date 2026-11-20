//! Traits and methods for CPU-backed Tensors
#![allow(clippy::needless_range_loop)]
pub mod erf;
pub(crate) mod features;
pub mod kernels;

#[allow(unused)]
trait Cpu {
    type Unit;
    type Array;
    const STEP: usize;
    const EPR: usize;
    const ARR: usize = Self::STEP / Self::EPR;

    unsafe fn zero() -> Self::Unit;
    unsafe fn zero_array() -> Self::Array;
    unsafe fn load(mem_addr: *const f32) -> Self::Unit;
    unsafe fn vec_add(a: Self::Unit, b: Self::Unit) -> Self::Unit;
    unsafe fn vec_fma(a: Self::Unit, b: Self::Unit, c: Self::Unit) -> Self::Unit;
    unsafe fn vec_reduce(x: Self::Array, y: *mut f32);
    unsafe fn from_f32(v: f32) -> Self::Unit;
    unsafe fn vec_store(mem_addr: *mut f32, a: Self::Unit);
}

#[allow(unused)]
trait CpuF16 {
    type Unit;
    type Array;
    const STEP: usize;
    const EPR: usize;
    const ARR: usize = Self::STEP / Self::EPR;
    // How many outer loop steps to accumulate in Self::Unit before flushing to f32.
    // Set to a finite value to bound rounding error (16 for example).
    const FLUSH_INTERVAL: usize = usize::MAX;

    unsafe fn zero() -> Self::Unit;
    unsafe fn zero_array() -> Self::Array;
    unsafe fn load(mem_addr: *const f16) -> Self::Unit;
    unsafe fn vec_add(a: Self::Unit, b: Self::Unit) -> Self::Unit;
    unsafe fn vec_fma(a: Self::Unit, b: Self::Unit, c: Self::Unit) -> Self::Unit;
    unsafe fn vec_reduce(x: Self::Array, y: *mut f32);
    unsafe fn from_f32(v: f32) -> Self::Unit;
    unsafe fn vec_store(mem_addr: *mut f16, a: Self::Unit);
}

#[allow(unused)]
trait CpuBF16 {
    type Unit;
    type Array;
    const STEP: usize;
    const EPR: usize;
    const ARR: usize = Self::STEP / Self::EPR;

    unsafe fn zero() -> Self::Unit;
    unsafe fn zero_array() -> Self::Array;
    unsafe fn load(mem_addr: *const bf16) -> Self::Unit;
    unsafe fn vec_add(a: Self::Unit, b: Self::Unit) -> Self::Unit;
    unsafe fn vec_fma(a: Self::Unit, b: Self::Unit, c: Self::Unit) -> Self::Unit;
    unsafe fn vec_reduce(x: Self::Array, y: *mut f32);
    unsafe fn from_f32(v: f32) -> Self::Unit;
    unsafe fn vec_store(mem_addr: *mut bf16, a: Self::Unit);
}

use half::{bf16, f16};

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
pub mod avx;

#[cfg(target_arch = "wasm32")]
#[cfg(target_feature = "simd128")]
pub mod simd128;

#[cfg(any(
    target_arch = "aarch64",
    all(target_arch = "arm", target_feature = "neon")
))]
pub mod neon;

#[inline(always)]
#[cfg(not(all(target_arch = "wasm32", target_feature = "simd128")))]
unsafe fn scalar_vec_dot_f32(a_row: *const f32, b_row: *const f32, c: *mut f32, k: usize) {
    for i in 0..k {
        *c += *a_row.add(i) * (*b_row.add(i));
    }
}

#[inline(always)]
#[cfg(not(all(target_arch = "wasm32", target_feature = "simd128")))]
unsafe fn scalar_vec_sum(row: *const f32, b: *mut f32, k: usize) {
    *b = 0f32;
    for i in 0..k {
        *b += *row.add(i)
    }
}

#[inline(always)]
unsafe fn scalar_vec_dot_f16(a_row: *const f16, b_row: *const f16, c: *mut f32, k: usize) {
    let mut sum = 0.0;
    for i in 0..k {
        sum += (*a_row.add(i)).to_f32() * (*b_row.add(i)).to_f32();
    }
    *c = sum;
}

#[inline(always)]
unsafe fn scalar_vec_dot_bf16(a_row: *const bf16, b_row: *const bf16, c: *mut f32, k: usize) {
    let mut sum = 0.0;
    for i in 0..k {
        sum += (*a_row.add(i)).to_f32() * (*b_row.add(i)).to_f32();
    }
    *c = sum;
}

#[inline(always)]
#[cfg(not(all(target_arch = "wasm32", target_feature = "simd128")))]
unsafe fn scalar_vec_add_f16(a_row: *const f16, b_row: *const f16, c: *mut f16, k: usize) {
    for i in 0..k {
        *c.add(i) = *a_row.add(i) + *b_row.add(i);
    }
}

#[inline(always)]
#[cfg(not(all(target_arch = "wasm32", target_feature = "simd128")))]
unsafe fn scalar_vec_add_bf16(a_row: *const bf16, b_row: *const bf16, c: *mut bf16, k: usize) {
    for i in 0..k {
        *c.add(i) = *a_row.add(i) + *b_row.add(i);
    }
}

#[inline(always)]
#[cfg(not(all(target_arch = "wasm32", target_feature = "simd128")))]
unsafe fn scalar_vec_scalar_add_f16(scalar: f16, xs: *const f16, ys: *mut f16, k: usize) {
    for i in 0..k {
        *ys.add(i) = *xs.add(i) + scalar;
    }
}

#[inline(always)]
#[cfg(not(all(target_arch = "wasm32", target_feature = "simd128")))]
unsafe fn scalar_vec_scalar_add_bf16(scalar: bf16, xs: *const bf16, ys: *mut bf16, k: usize) {
    for i in 0..k {
        *ys.add(i) = *xs.add(i) + scalar;
    }
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[inline(always)]
fn use_avx2() -> bool {
    features::get().avx2
}

#[cfg(any(
    target_arch = "aarch64",
    all(target_arch = "arm", target_feature = "neon")
))]
#[inline(always)]
fn use_neon() -> bool {
    features::get().neon
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[inline(always)]
fn use_avx2_f16() -> bool {
    let features = features::get();
    features.avx2 && (!cfg!(target_feature = "f16c") || features.f16c)
}

#[cfg(any(
    target_arch = "aarch64",
    all(target_arch = "arm", target_feature = "neon")
))]
#[inline(always)]
fn use_neon_f16() -> bool {
    let features = features::get();
    features.neon && (!cfg!(target_feature = "fp16") || features.fp16)
}

#[cfg(any(
    target_arch = "aarch64",
    all(target_arch = "arm", target_feature = "neon")
))]
#[inline(always)]
fn use_neon_bf16() -> bool {
    let features = features::get();
    features.neon && (!cfg!(target_feature = "bf16") || features.bf16)
}

#[inline(always)]
pub(crate) unsafe fn vec_dot_f32(a_row: *const f32, b_row: *const f32, c: *mut f32, k: usize) {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if use_avx2() {
        return avx::vec_dot_f32(a_row, b_row, c, k);
    }
    #[cfg(any(
        target_arch = "aarch64",
        all(target_arch = "arm", target_feature = "neon")
    ))]
    if use_neon() {
        return neon::vec_dot_f32(a_row, b_row, c, k);
    }
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        simd128::vec_dot_f32(a_row, b_row, c, k)
    }
    #[cfg(not(all(target_arch = "wasm32", target_feature = "simd128")))]
    {
        scalar_vec_dot_f32(a_row, b_row, c, k)
    }
}

#[inline(always)]
pub(crate) unsafe fn vec_sum(row: *const f32, b: *mut f32, k: usize) {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if use_avx2() {
        return avx::vec_sum(row, b, k);
    }
    #[cfg(any(
        target_arch = "aarch64",
        all(target_arch = "arm", target_feature = "neon")
    ))]
    if use_neon() {
        return neon::vec_sum(row, b, k);
    }
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        simd128::vec_sum(row, b, k)
    }
    #[cfg(not(all(target_arch = "wasm32", target_feature = "simd128")))]
    {
        scalar_vec_sum(row, b, k)
    }
}

#[inline(always)]
pub(crate) unsafe fn vec_dot_f16(a_row: *const f16, b_row: *const f16, c: *mut f32, k: usize) {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if use_avx2_f16() {
        return avx::vec_dot_f16(a_row, b_row, c, k);
    }
    #[cfg(any(
        target_arch = "aarch64",
        all(target_arch = "arm", target_feature = "neon")
    ))]
    if use_neon_f16() {
        return neon::vec_dot_f16(a_row, b_row, c, k);
    }
    scalar_vec_dot_f16(a_row, b_row, c, k)
}

#[inline(always)]
pub(crate) unsafe fn vec_dot_bf16(a_row: *const bf16, b_row: *const bf16, c: *mut f32, k: usize) {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if use_avx2() {
        return avx::vec_dot_bf16(a_row, b_row, c, k);
    }
    #[cfg(any(
        target_arch = "aarch64",
        all(target_arch = "arm", target_feature = "neon")
    ))]
    if use_neon_bf16() {
        return neon::vec_dot_bf16(a_row, b_row, c, k);
    }
    scalar_vec_dot_bf16(a_row, b_row, c, k)
}

#[inline(always)]
pub(crate) unsafe fn vec_add_f16(a_row: *const f16, b_row: *const f16, c: *mut f16, k: usize) {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if use_avx2_f16() {
        return avx::vec_add_f16(a_row, b_row, c, k);
    }
    #[cfg(any(
        target_arch = "aarch64",
        all(target_arch = "arm", target_feature = "neon")
    ))]
    if use_neon_f16() {
        return neon::vec_add_f16(a_row, b_row, c, k);
    }
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        simd128::vec_add_f16(a_row, b_row, c, k)
    }
    #[cfg(not(all(target_arch = "wasm32", target_feature = "simd128")))]
    {
        scalar_vec_add_f16(a_row, b_row, c, k)
    }
}

#[inline(always)]
pub(crate) unsafe fn vec_add_bf16(a_row: *const bf16, b_row: *const bf16, c: *mut bf16, k: usize) {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if use_avx2() {
        return avx::vec_add_bf16(a_row, b_row, c, k);
    }
    #[cfg(any(
        target_arch = "aarch64",
        all(target_arch = "arm", target_feature = "neon")
    ))]
    if use_neon_bf16() {
        return neon::vec_add_bf16(a_row, b_row, c, k);
    }
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        simd128::vec_add_bf16(a_row, b_row, c, k)
    }
    #[cfg(not(all(target_arch = "wasm32", target_feature = "simd128")))]
    {
        scalar_vec_add_bf16(a_row, b_row, c, k)
    }
}

#[inline(always)]
pub(crate) unsafe fn vec_scalar_add_f16(scalar: f16, xs: *const f16, ys: *mut f16, k: usize) {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if use_avx2_f16() {
        return avx::vec_scalar_add_f16(scalar, xs, ys, k);
    }
    #[cfg(any(
        target_arch = "aarch64",
        all(target_arch = "arm", target_feature = "neon")
    ))]
    if use_neon_f16() {
        return neon::vec_scalar_add_f16(scalar, xs, ys, k);
    }
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        simd128::vec_scalar_add_f16(scalar, xs, ys, k)
    }
    #[cfg(not(all(target_arch = "wasm32", target_feature = "simd128")))]
    {
        scalar_vec_scalar_add_f16(scalar, xs, ys, k)
    }
}

#[inline(always)]
pub(crate) unsafe fn vec_mul_bf16(a_row: *const bf16, b_row: *const bf16, c: *mut bf16, k: usize) {
    #[cfg(any(
        target_arch = "aarch64",
        all(target_arch = "arm", target_feature = "neon")
    ))]
    if use_neon_bf16() {
        return neon::vec_mul_bf16(a_row, b_row, c, k);
    }
    for j in 0..k {
        *c.add(j) = *a_row.add(j) * *b_row.add(j);
    }
}

#[inline(always)]
pub(crate) unsafe fn vec_scalar_add_bf16(scalar: bf16, xs: *const bf16, ys: *mut bf16, k: usize) {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if use_avx2() {
        return avx::vec_scalar_add_bf16(scalar, xs, ys, k);
    }
    #[cfg(any(
        target_arch = "aarch64",
        all(target_arch = "arm", target_feature = "neon")
    ))]
    if use_neon_bf16() {
        return neon::vec_scalar_add_bf16(scalar, xs, ys, k);
    }
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        simd128::vec_scalar_add_bf16(scalar, xs, ys, k)
    }
    #[cfg(not(all(target_arch = "wasm32", target_feature = "simd128")))]
    {
        scalar_vec_scalar_add_bf16(scalar, xs, ys, k)
    }
}
