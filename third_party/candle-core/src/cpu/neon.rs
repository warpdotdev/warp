use super::{Cpu, CpuBF16, CpuF16};
#[cfg(target_arch = "arm")]
use core::arch::arm::*;

#[cfg(target_arch = "aarch64")]
use core::arch::aarch64::*;
use half::{bf16, f16};

pub struct CurrentCpu {}

impl CurrentCpu {
    #[cfg(target_arch = "aarch64")]
    unsafe fn reduce_one(x: float32x4_t) -> f32 {
        vaddvq_f32(x)
    }

    #[cfg(target_arch = "arm")]
    unsafe fn reduce_one(x: float32x4_t) -> f32 {
        vgetq_lane_f32(x, 0) + vgetq_lane_f32(x, 1) + vgetq_lane_f32(x, 2) + vgetq_lane_f32(x, 3)
    }
}

impl Cpu for CurrentCpu {
    type Unit = float32x4_t;
    type Array = [float32x4_t; Self::ARR];

    const STEP: usize = 32;
    const EPR: usize = 4;

    unsafe fn zero() -> Self::Unit {
        vdupq_n_f32(0.0)
    }

    unsafe fn from_f32(x: f32) -> Self::Unit {
        vdupq_n_f32(x)
    }

    unsafe fn zero_array() -> Self::Array {
        [Self::zero(); Self::ARR]
    }

    unsafe fn load(mem_addr: *const f32) -> Self::Unit {
        vld1q_f32(mem_addr)
    }

    unsafe fn vec_add(a: Self::Unit, b: Self::Unit) -> Self::Unit {
        vaddq_f32(a, b)
    }

    unsafe fn vec_fma(a: Self::Unit, b: Self::Unit, c: Self::Unit) -> Self::Unit {
        vfmaq_f32(a, b, c)
    }

    unsafe fn vec_store(mem_addr: *mut f32, a: Self::Unit) {
        vst1q_f32(mem_addr, a);
    }

    unsafe fn vec_reduce(mut x: Self::Array, y: *mut f32) {
        for i in 0..Self::ARR / 2 {
            x[2 * i] = vaddq_f32(x[2 * i], x[2 * i + 1]);
        }
        for i in 0..Self::ARR / 4 {
            x[4 * i] = vaddq_f32(x[4 * i], x[4 * i + 2]);
        }
        for i in 0..Self::ARR / 8 {
            x[8 * i] = vaddq_f32(x[8 * i], x[8 * i + 4]);
        }
        *y = Self::reduce_one(x[0]);
    }
}

mod fp16 {
    use super::super::CpuF16;
    #[cfg(target_arch = "aarch64")]
    use core::arch::aarch64::*;
    #[cfg(target_arch = "arm")]
    use core::arch::arm::*;
    use half::f16;
    use std::arch::asm;

    // Fallback: widen f16 to f32 on load using FCVTL, accumulate in f32.
    #[cfg(not(target_feature = "fp16"))]
    mod inner {
        use super::*;

        pub struct CurrentCpuF16 {}

        impl CpuF16 for CurrentCpuF16 {
            type Unit = float32x4_t;
            type Array = [float32x4_t; Self::ARR];

            const STEP: usize = 32;
            const EPR: usize = 4;

            unsafe fn zero() -> Self::Unit {
                vdupq_n_f32(0.0)
            }

            unsafe fn zero_array() -> Self::Array {
                [Self::zero(); Self::ARR]
            }

            unsafe fn load(mem_addr: *const f16) -> Self::Unit {
                let bits = vld1_u16(mem_addr as *const u16);
                let result: Self::Unit;
                asm!(
                    "fcvtl {dst:v}.4s, {src:v}.4h",
                    src = in(vreg) bits,
                    dst = out(vreg) result,
                    options(pure, nomem, nostack),
                );
                result
            }

            unsafe fn vec_add(a: Self::Unit, b: Self::Unit) -> Self::Unit {
                vaddq_f32(a, b)
            }

            unsafe fn vec_fma(a: Self::Unit, b: Self::Unit, c: Self::Unit) -> Self::Unit {
                vfmaq_f32(a, b, c)
            }

            unsafe fn vec_reduce(mut x: Self::Array, y: *mut f32) {
                for i in 0..Self::ARR / 2 {
                    x[2 * i] = vaddq_f32(x[2 * i], x[2 * i + 1]);
                }
                for i in 0..Self::ARR / 4 {
                    x[4 * i] = vaddq_f32(x[4 * i], x[4 * i + 2]);
                }
                for i in 0..Self::ARR / 8 {
                    x[8 * i] = vaddq_f32(x[8 * i], x[8 * i + 4]);
                }
                *y = vaddvq_f32(x[0]);
            }

            unsafe fn from_f32(v: f32) -> Self::Unit {
                vdupq_n_f32(v)
            }

            unsafe fn vec_store(mem_addr: *mut f16, a: Self::Unit) {
                let bits: uint16x4_t;
                asm!(
                    "fcvtn {dst:v}.4h, {src:v}.4s",
                    src = in(vreg) a,
                    dst = out(vreg) bits,
                    options(pure, nomem, nostack),
                );
                vst1_u16(mem_addr as *mut u16, bits);
            }
        }
    }

    // Optimized: accumulate in f16 using FMLA.8h, flush to f32 periodically.
    #[cfg(target_feature = "fp16")]
    mod inner {
        use super::*;

        pub struct CurrentCpuF16 {}

        impl CurrentCpuF16 {
            fn reduce_one(x: uint16x8_t) -> f32 {
                let result: f32;
                unsafe {
                    asm!(
                        // 1. Widen bottom 4 lanes of f16x8 to a full f32x4 vector
                        "fcvtl {bot:v}.4s, {v:v}.4h",
                        // 2. Widen top 4 lanes of f16x8 to a second f32x4 vector
                        "fcvtl2 {top:v}.4s, {v:v}.8h",
                        // 3. Perform a parallel element-wise vector addition
                        "fadd {bot:v}.4s, {bot:v}.4s, {top:v}.4s",
                        // 4. Pairwise add 4 lanes down to 2 lanes: [A+B, C+D, ?, ?]
                        "faddp {bot:v}.4s, {bot:v}.4s, {bot:v}.4s",
                        // 5. Pairwise add 2 lanes down to 1 scalar: [(A+B)+(C+D), ?, ?, ?]
                        "faddp {bot:v}.4s, {bot:v}.4s, {bot:v}.4s",
                        // 6. Extract the scalar from the lowest lane of the vector
                        "mov {res:s}, {bot:v}.s[0]",
                        v = in(vreg) x,
                        top = lateout(vreg) _,
                        bot = out(vreg) _,
                        res = out(vreg) result,
                        options(nostack, preserves_flags)
                    );
                }
                result
            }
        }

        impl CpuF16 for CurrentCpuF16 {
            // Keep the f16 lane bits in the stable integer vector type. The
            // inline assembly assigns the floating-point interpretation.
            type Unit = uint16x8_t;
            type Array = [uint16x8_t; Self::ARR];

            const STEP: usize = 32;
            const EPR: usize = 8;
            // Flush f16 accumulators to f32 every `FLUSH_INTERVAL` steps to bound rounding error.
            const FLUSH_INTERVAL: usize = 16;

            unsafe fn zero() -> Self::Unit {
                let result: Self::Unit;
                asm!(
                    "movi {v:v}.8h, #0",
                    v = out(vreg) result,
                    options(nostack, pure, nomem)
                );
                result
            }

            unsafe fn zero_array() -> Self::Array {
                [Self::zero(); Self::ARR]
            }

            unsafe fn load(mem_addr: *const f16) -> Self::Unit {
                std::ptr::read_unaligned(mem_addr.cast())
            }

            unsafe fn vec_add(a: Self::Unit, b: Self::Unit) -> Self::Unit {
                let result: Self::Unit;
                asm!(
                    "fadd {v:v}.8h, {a:v}.8h, {b:v}.8h",
                    v = out(vreg) result,
                    a = in(vreg) a,
                    b = in(vreg) b,
                    options(pure, nomem, nostack)
                );
                result
            }

            unsafe fn vec_fma(a: Self::Unit, b: Self::Unit, c: Self::Unit) -> Self::Unit {
                let result: Self::Unit;
                asm!(
                    "fmla {a:v}.8h, {b:v}.8h, {c:v}.8h",
                    a = inout(vreg) a => result,
                    b = in(vreg) b,
                    c = in(vreg) c,
                    options(pure, nomem, nostack)
                );
                result
            }

            unsafe fn vec_reduce(x: Self::Array, y: *mut f32) {
                let mut sum = 0.0f32;
                for item in x {
                    sum += Self::reduce_one(item);
                }
                *y = sum;
            }

            unsafe fn from_f32(v: f32) -> Self::Unit {
                let result: Self::Unit;
                asm!(
                    "fcvt {tmp:h}, {src:s}",
                    "fmov {w:w}, {tmp:h}",
                    "dup {dst:v}.8h, {w:w}",
                    src = in(vreg) v,
                    tmp = out(vreg) _,
                    w = out(reg) _,
                    dst = out(vreg) result,
                    options(nostack, pure, nomem),
                );
                result
            }

            unsafe fn vec_store(mem_addr: *mut f16, a: Self::Unit) {
                asm!(
                    "st1 {{ {vec:v}.8h }}, [{ptr}]",
                    vec = in(vreg) a,
                    ptr = in(reg) mem_addr,
                    options(nostack, preserves_flags)
                );
            }
        }
    }

    pub use inner::CurrentCpuF16;
}

pub use fp16::CurrentCpuF16;

mod bfloat16 {
    use super::super::CpuBF16;
    use core::arch::aarch64::*;
    use half::bf16;

    // Fallback: accumulate in f32 using SHLL to widen bf16 to f32 on load.
    #[cfg(not(target_feature = "bf16"))]
    mod inner {
        use super::*;

        pub struct CurrentCpuBF16 {}

        impl CpuBF16 for CurrentCpuBF16 {
            type Unit = float32x4_t;
            type Array = [float32x4_t; Self::ARR];

            const STEP: usize = 32;
            const EPR: usize = 4;

            unsafe fn zero() -> Self::Unit {
                vdupq_n_f32(0.0)
            }

            unsafe fn zero_array() -> Self::Array {
                [Self::zero(); Self::ARR]
            }

            unsafe fn load(mem_addr: *const bf16) -> Self::Unit {
                // bf16 is the top 16 bits of f32; shift left by 16 to reconstruct f32 bits.
                let i = vld1_u16(mem_addr as *const u16);
                let s = vshll_n_u16::<16>(i);
                vreinterpretq_f32_u32(s)
            }

            unsafe fn vec_add(a: Self::Unit, b: Self::Unit) -> Self::Unit {
                vaddq_f32(a, b)
            }

            unsafe fn vec_fma(a: Self::Unit, b: Self::Unit, c: Self::Unit) -> Self::Unit {
                vfmaq_f32(a, b, c)
            }

            unsafe fn vec_reduce(mut x: Self::Array, y: *mut f32) {
                for i in 0..Self::ARR / 2 {
                    x[2 * i] = vaddq_f32(x[2 * i], x[2 * i + 1]);
                }
                for i in 0..Self::ARR / 4 {
                    x[4 * i] = vaddq_f32(x[4 * i], x[4 * i + 2]);
                }
                for i in 0..Self::ARR / 8 {
                    x[8 * i] = vaddq_f32(x[8 * i], x[8 * i + 4]);
                }
                *y = vaddvq_f32(x[0]);
            }

            unsafe fn from_f32(v: f32) -> Self::Unit {
                vdupq_n_f32(v)
            }

            unsafe fn vec_store(mem_addr: *mut bf16, a: Self::Unit) {
                // Extract top 16 bits of each f32 lane, which is the bf16 representation.
                let s = vreinterpretq_u32_f32(a);
                let h = vshrn_n_u32::<16>(s);
                vst1_u16(mem_addr as *mut u16, h);
            }
        }
    }

    // Optimized: BFDOT accumulates bf16 pairs directly into f32 lanes.
    #[cfg(target_feature = "bf16")]
    mod inner {
        use super::*;
        use std::arch::asm;

        pub struct CurrentCpuBF16 {}

        impl CpuBF16 for CurrentCpuBF16 {
            type Unit = uint16x8_t;
            type Array = [uint16x8_t; Self::ARR];

            const STEP: usize = 32;
            const EPR: usize = 8;

            unsafe fn zero() -> Self::Unit {
                vreinterpretq_u16_f32(vdupq_n_f32(0.0))
            }

            unsafe fn zero_array() -> Self::Array {
                [Self::zero(); Self::ARR]
            }

            unsafe fn load(mem_addr: *const bf16) -> Self::Unit {
                vld1q_u16(mem_addr as *const u16)
            }

            unsafe fn vec_add(a: Self::Unit, b: Self::Unit) -> Self::Unit {
                let result: Self::Unit;
                asm!(
                    "fadd {v:v}.4s, {a:v}.4s, {b:v}.4s",
                    v = out(vreg) result,
                    a = in(vreg) a,
                    b = in(vreg) b,
                    options(pure, nomem, nostack)
                );
                result
            }

            unsafe fn vec_fma(a: Self::Unit, b: Self::Unit, c: Self::Unit) -> Self::Unit {
                // IMPORTANT: `a` is not bf16x8 here. It is 128 bits used as f32x4 (.4s) to accumulate
                // results from bfdot. `b` and `c` are bf16x8 inputs (.8h).
                // bfdot: acc[i] += (f32)b[2i]*c[2i] + (f32)b[2i+1]*c[2i+1]
                let result: Self::Unit;
                asm!(
                    "bfdot {acc:v}.4s, {b:v}.8h, {c:v}.8h",
                    acc = inout(vreg) a => result,
                    b = in(vreg) b,
                    c = in(vreg) c,
                    options(pure, nomem, nostack)
                );
                result
            }

            unsafe fn vec_reduce(x: Self::Array, y: *mut f32) {
                // IMPORTANT: `x` is not bf16x8 here. It is 128 bits used as f32x4.
                // This means that if you use `vec_reduce` where `x` is actually bf16x8 you will get
                // entirely wrong results.
                let mut xf: [float32x4_t; Self::ARR] = std::mem::transmute(x);
                for i in 0..Self::ARR / 2 {
                    xf[2 * i] = vaddq_f32(xf[2 * i], xf[2 * i + 1]);
                }
                for i in 0..Self::ARR / 4 {
                    xf[4 * i] = vaddq_f32(xf[4 * i], xf[4 * i + 2]);
                }
                *y = vaddvq_f32(xf[0]);
            }

            unsafe fn from_f32(v: f32) -> Self::Unit {
                vreinterpretq_u16_f32(vdupq_n_f32(v))
            }

            unsafe fn vec_store(mem_addr: *mut bf16, a: Self::Unit) {
                vst1q_u16(mem_addr as *mut u16, a);
            }
        }
    }

    pub use inner::CurrentCpuBF16;
}

pub use bfloat16::CurrentCpuBF16;

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
        *c += *a_row.add(i) * (*b_row.add(i));
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
        *b += *row.add(i)
    }
}

pub(crate) unsafe fn vec_dot_f16(a_row: *const f16, b_row: *const f16, c: *mut f32, k: usize) {
    let mut sumf = 0.0f32;
    let mut sum = CurrentCpuF16::zero_array();
    let mut steps_since_flush = 0usize;
    let mut i = 0;
    while i + CurrentCpuF16::STEP <= k {
        for j in 0..CurrentCpuF16::ARR {
            sum[j] = CurrentCpuF16::vec_fma(
                sum[j],
                CurrentCpuF16::load(a_row.add(i + j * CurrentCpuF16::EPR)),
                CurrentCpuF16::load(b_row.add(i + j * CurrentCpuF16::EPR)),
            );
        }
        steps_since_flush += 1;
        if steps_since_flush == CurrentCpuF16::FLUSH_INTERVAL {
            let mut partial = 0.0f32;
            CurrentCpuF16::vec_reduce(sum, &mut partial);
            sumf += partial;
            sum = CurrentCpuF16::zero_array();
            steps_since_flush = 0;
        }
        i += CurrentCpuF16::STEP;
    }
    let mut partial = 0.0f32;
    CurrentCpuF16::vec_reduce(sum, &mut partial);
    sumf += partial;
    while i < k {
        sumf += (*a_row.add(i)).to_f32() * (*b_row.add(i)).to_f32();
        i += 1;
    }
    *c = sumf;
}

pub(crate) unsafe fn vec_dot_bf16(a_row: *const bf16, b_row: *const bf16, c: *mut f32, k: usize) {
    let mut sum = CurrentCpuBF16::zero_array();
    let mut i = 0;
    while i + CurrentCpuBF16::STEP <= k {
        for j in 0..CurrentCpuBF16::ARR {
            sum[j] = CurrentCpuBF16::vec_fma(
                sum[j],
                CurrentCpuBF16::load(a_row.add(i + j * CurrentCpuBF16::EPR)),
                CurrentCpuBF16::load(b_row.add(i + j * CurrentCpuBF16::EPR)),
            );
        }
        i += CurrentCpuBF16::STEP;
    }
    CurrentCpuBF16::vec_reduce(sum, c);
    while i < k {
        *c += (*a_row.add(i)).to_f32() * (*b_row.add(i)).to_f32();
        i += 1;
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
    // Widen to f32, add, narrow. The bfdot-specialized CurrentCpuBF16 keeps raw bf16 bits in
    // its Unit and its vec_add interprets them as f32 lanes, so it must not be used here.
    let mut i = 0;
    while i + 8 <= k {
        let a = vld1q_u16(a_row.add(i) as *const u16);
        let b = vld1q_u16(b_row.add(i) as *const u16);
        let a_lo = vreinterpretq_f32_u32(vshll_n_u16::<16>(vget_low_u16(a)));
        let a_hi = vreinterpretq_f32_u32(vshll_high_n_u16::<16>(a));
        let b_lo = vreinterpretq_f32_u32(vshll_n_u16::<16>(vget_low_u16(b)));
        let b_hi = vreinterpretq_f32_u32(vshll_high_n_u16::<16>(b));
        let lo = vshrn_n_u32::<16>(vreinterpretq_u32_f32(vaddq_f32(a_lo, b_lo)));
        let hi = vshrn_n_u32::<16>(vreinterpretq_u32_f32(vaddq_f32(a_hi, b_hi)));
        vst1q_u16(c.add(i) as *mut u16, vcombine_u16(lo, hi));
        i += 8;
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

pub(crate) unsafe fn vec_mul_bf16(a_row: *const bf16, b_row: *const bf16, c: *mut bf16, k: usize) {
    let mut i = 0;
    while i + 8 <= k {
        let a = vld1q_u16(a_row.add(i) as *const u16);
        let b = vld1q_u16(b_row.add(i) as *const u16);
        let a_lo = vreinterpretq_f32_u32(vshll_n_u16::<16>(vget_low_u16(a)));
        let a_hi = vreinterpretq_f32_u32(vshll_high_n_u16::<16>(a));
        let b_lo = vreinterpretq_f32_u32(vshll_n_u16::<16>(vget_low_u16(b)));
        let b_hi = vreinterpretq_f32_u32(vshll_high_n_u16::<16>(b));
        let lo = vshrn_n_u32::<16>(vreinterpretq_u32_f32(vmulq_f32(a_lo, b_lo)));
        let hi = vshrn_n_u32::<16>(vreinterpretq_u32_f32(vmulq_f32(a_hi, b_hi)));
        vst1q_u16(c.add(i) as *mut u16, vcombine_u16(lo, hi));
        i += 8;
    }
    for j in i..k {
        *c.add(j) = *a_row.add(j) * *b_row.add(j);
    }
}

pub(crate) unsafe fn vec_scalar_add_bf16(scalar: bf16, xs: *const bf16, ys: *mut bf16, k: usize) {
    let sv = vdupq_n_f32(scalar.to_f32());
    let mut i = 0;
    while i + 8 <= k {
        let x = vld1q_u16(xs.add(i) as *const u16);
        let x_lo = vreinterpretq_f32_u32(vshll_n_u16::<16>(vget_low_u16(x)));
        let x_hi = vreinterpretq_f32_u32(vshll_high_n_u16::<16>(x));
        let lo = vshrn_n_u32::<16>(vreinterpretq_u32_f32(vaddq_f32(x_lo, sv)));
        let hi = vshrn_n_u32::<16>(vreinterpretq_u32_f32(vaddq_f32(x_hi, sv)));
        vst1q_u16(ys.add(i) as *mut u16, vcombine_u16(lo, hi));
        i += 8;
    }
    for j in i..k {
        *ys.add(j) = *xs.add(j) + scalar;
    }
}
