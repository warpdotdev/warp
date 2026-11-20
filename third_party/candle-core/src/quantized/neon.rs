#![allow(clippy::needless_range_loop)]

use super::k_quants::{
    BlockQ2K, BlockQ3K, BlockQ4K, BlockQ4_0, BlockQ5K, BlockQ6K, BlockQ8K, BlockQ8_0, QK8_0, QK_K,
};
#[cfg(target_arch = "aarch64")]
use super::repack::BlockQ8_0x4;
#[cfg(target_arch = "aarch64")]
use super::repack::{BlockQ4Kx8, BlockQ4_0x4};
#[cfg(target_arch = "aarch64")]
use super::repack::{BlockQ5Kx8, BlockQ6Kx8, BlockQ8Kx4};
use byteorder::{ByteOrder, LittleEndian};
use half::f16;

#[allow(unused_imports)]
#[cfg(target_arch = "arm")]
use core::arch::arm::*;

#[allow(unused_imports)]
#[cfg(target_arch = "aarch64")]
use core::arch::aarch64::*;

#[inline]
#[target_feature(enable = "dotprod")]
unsafe fn vdotq_s32(a: int8x16_t, b: int8x16_t) -> int32x4_t {
    let mut acc: int32x4_t = vdupq_n_s32(0);
    core::arch::asm!(
        "sdot {acc:v}.4s, {a:v}.16b, {b:v}.16b",
        acc = inout(vreg) acc,
        a = in(vreg) a,
        b = in(vreg) b,
        options(nostack, nomem),
    );
    acc
}

/// Two SDOT ops into one accumulator
#[inline]
#[target_feature(enable = "dotprod")]
unsafe fn vdotq_s32_pair(a0: int8x16_t, b0: int8x16_t, a1: int8x16_t, b1: int8x16_t) -> int32x4_t {
    let mut acc: int32x4_t = vdupq_n_s32(0);
    core::arch::asm!(
        "sdot {acc:v}.4s, {a0:v}.16b, {b0:v}.16b",
        "sdot {acc:v}.4s, {a1:v}.16b, {b1:v}.16b",
        acc = inout(vreg) acc,
        a0 = in(vreg) a0,
        b0 = in(vreg) b0,
        a1 = in(vreg) a1,
        b1 = in(vreg) b1,
        options(nostack, nomem),
    );
    acc
}

/// Accumulating SDOT: acc += dot4(a, b) for each lane.
#[cfg(target_arch = "aarch64")]
#[inline]
#[target_feature(enable = "dotprod")]
unsafe fn sdot_acc(acc: int32x4_t, a: int8x16_t, b: int8x16_t) -> int32x4_t {
    let mut out = acc;
    core::arch::asm!(
        "sdot {out:v}.4s, {a:v}.16b, {b:v}.16b",
        out = inout(vreg) out,
        a   = in(vreg) a,
        b   = in(vreg) b,
        options(nostack, nomem),
    );
    out
}

#[cfg(target_arch = "aarch64")]
#[inline(always)]
unsafe fn sdot_laneq_s32<const LANE: i32>(acc: int32x4_t, a: int8x16_t, b: int8x16_t) -> int32x4_t {
    sdot_acc(
        acc,
        a,
        vreinterpretq_s8_s32(vdupq_laneq_s32::<LANE>(vreinterpretq_s32_s8(b))),
    )
}

#[cfg(target_arch = "aarch64")]
#[inline]
#[target_feature(enable = "i8mm")]
unsafe fn smmla_s32(acc: int32x4_t, a: int8x16_t, b: int8x16_t) -> int32x4_t {
    let mut out = acc;
    core::arch::asm!(
        "smmla {out:v}.4s, {a:v}.16b, {b:v}.16b",
        out = inout(vreg) out,
        a = in(vreg) a,
        b = in(vreg) b,
        options(nostack, nomem),
    );
    out
}

#[cfg(target_arch = "aarch64")]
#[inline(always)]
unsafe fn decode_q4kx8_scales(scales_in: *const u8) -> (int16x8_t, int16x8_t) {
    const KMASK1: u32 = 0x3f3f3f3f;
    const KMASK2: u32 = 0x0f0f0f0f;
    const KMASK3: u32 = 0x03030303;
    // Direct unaligned reads avoid the copy_nonoverlapping stack round-trip.
    let sm0 = (scales_in as *const u32).read_unaligned();
    let sm1 = (scales_in.add(4) as *const u32).read_unaligned();
    let sm2 = (scales_in.add(8) as *const u32).read_unaligned();
    let mins_0_3 = sm1 & KMASK1;
    let mins_4_7 = ((sm2 >> 4) & KMASK2) | (((sm1 >> 6) & KMASK3) << 4);
    let out_mins = vreinterpretq_s16_u16(vmovl_u8(vreinterpret_u8_u32(vcreate_u32(
        (mins_0_3 as u64) | ((mins_4_7 as u64) << 32),
    ))));
    let sc_0 = sm0 & KMASK1;
    let sc_1 = (sm2 & KMASK2) | (((sm0 >> 6) & KMASK3) << 4);
    let out_scales = vmovl_s8(vreinterpret_s8_u8(vreinterpret_u8_u32(vcreate_u32(
        (sc_0 as u64) | ((sc_1 as u64) << 32),
    ))));
    (out_mins, out_scales)
}

#[inline(always)]
unsafe fn merge_signed_max(
    abs_a: float32x4_t,
    smax_a: float32x4_t,
    abs_b: float32x4_t,
    smax_b: float32x4_t,
) -> (float32x4_t, float32x4_t) {
    (
        vmaxq_f32(abs_a, abs_b),
        vbslq_f32(vcgtq_f32(abs_b, abs_a), smax_b, smax_a),
    )
}

#[inline(always)]
pub(crate) fn vec_dot_q4_0_q8_0(n: usize, xs: &[BlockQ4_0], ys: &[BlockQ8_0]) -> f32 {
    debug_assert!(
        n.is_multiple_of(QK8_0),
        "vec_dot_q4_0_q8_0: {n} is not divisible by {QK8_0}"
    );
    let nb = n / QK8_0;
    unsafe {
        let mut sumv0 = vdupq_n_f32(0.0f32);
        for i in 0..nb {
            let x0 = &xs[i];
            let y0 = &ys[i];

            let m4b = vdupq_n_u8(0x0F);
            let s8b = vdupq_n_s8(0x8);

            let v0_0 = vld1q_u8(x0.qs.as_ptr());

            // 4-bit -> 8-bit
            let v0_0l = vreinterpretq_s8_u8(vandq_u8(v0_0, m4b));
            let v0_0h = vreinterpretq_s8_u8(vshrq_n_u8(v0_0, 4));

            // sub 8
            let v0_0ls = vsubq_s8(v0_0l, s8b);
            let v0_0hs = vsubq_s8(v0_0h, s8b);

            // load y
            let v1_0l = vld1q_s8(y0.qs.as_ptr());
            let v1_0h = vld1q_s8(y0.qs.as_ptr().add(16));

            let pl0 = vdotq_s32(v0_0ls, v1_0l);
            let ph0 = vdotq_s32(v0_0hs, v1_0h);
            sumv0 = vmlaq_n_f32(
                sumv0,
                vcvtq_f32_s32(vaddq_s32(pl0, ph0)),
                x0.d.to_f32() * y0.d.to_f32(),
            );
        }
        vaddvq_f32(sumv0)
    }
}

#[inline(always)]
pub(crate) fn vec_dot_q8_0_q8_0(n: usize, xs: &[BlockQ8_0], ys: &[BlockQ8_0]) -> f32 {
    debug_assert!(
        n.is_multiple_of(QK8_0),
        "vec_dot_q8_0_q8_0: {n} is not divisible by {QK8_0}"
    );
    let nb = n / QK8_0;
    unsafe {
        let mut sumv0 = vdupq_n_f32(0.0f32);
        for i in 0..nb {
            let x0 = &xs[i];
            let y0 = &ys[i];

            let x0_0 = vld1q_s8(x0.qs.as_ptr());
            let x0_1 = vld1q_s8(x0.qs.as_ptr().add(16));

            // load y
            let y0_0 = vld1q_s8(y0.qs.as_ptr());
            let y0_1 = vld1q_s8(y0.qs.as_ptr().add(16));

            let p0 = vdotq_s32(x0_0, y0_0);
            let p1 = vdotq_s32(x0_1, y0_1);

            sumv0 = vmlaq_n_f32(
                sumv0,
                vcvtq_f32_s32(vaddq_s32(p0, p1)),
                x0.d.to_f32() * y0.d.to_f32(),
            );
        }
        vaddvq_f32(sumv0)
    }
}

#[cfg(target_arch = "aarch64")]
pub(crate) fn quantize_q8_0(xs: &[f32], ys: &mut [BlockQ8_0]) {
    debug_assert!(xs.len().is_multiple_of(QK8_0));
    debug_assert_eq!(ys.len(), xs.len() / QK8_0);

    let (x_chunks, _) = xs.as_chunks::<QK8_0>();
    for (chunk, y) in x_chunks.iter().zip(ys.iter_mut()) {
        unsafe {
            let p = chunk.as_ptr();
            let v0 = vld1q_f32(p);
            let v1 = vld1q_f32(p.add(4));
            let v2 = vld1q_f32(p.add(8));
            let v3 = vld1q_f32(p.add(12));
            let v4 = vld1q_f32(p.add(16));
            let v5 = vld1q_f32(p.add(20));
            let v6 = vld1q_f32(p.add(24));
            let v7 = vld1q_f32(p.add(28));

            let a01 = vmaxq_f32(vabsq_f32(v0), vabsq_f32(v1));
            let a23 = vmaxq_f32(vabsq_f32(v2), vabsq_f32(v3));
            let a45 = vmaxq_f32(vabsq_f32(v4), vabsq_f32(v5));
            let a67 = vmaxq_f32(vabsq_f32(v6), vabsq_f32(v7));
            let amax = vmaxvq_f32(vmaxq_f32(vmaxq_f32(a01, a23), vmaxq_f32(a45, a67)));
            let d = amax / ((1 << 7) - 1) as f32;
            let id = if d != 0.0 { 1.0 / d } else { 0.0 };
            y.d = f16::from_f32(d);
            let id = vdupq_n_f32(id);

            let s01 = vcombine_s16(
                vqmovn_s32(vcvtaq_s32_f32(vmulq_f32(v0, id))),
                vqmovn_s32(vcvtaq_s32_f32(vmulq_f32(v1, id))),
            );
            let s23 = vcombine_s16(
                vqmovn_s32(vcvtaq_s32_f32(vmulq_f32(v2, id))),
                vqmovn_s32(vcvtaq_s32_f32(vmulq_f32(v3, id))),
            );
            let q0 = vcombine_s8(vqmovn_s16(s01), vqmovn_s16(s23));
            vst1q_s8(y.qs.as_mut_ptr(), q0);

            let s45 = vcombine_s16(
                vqmovn_s32(vcvtaq_s32_f32(vmulq_f32(v4, id))),
                vqmovn_s32(vcvtaq_s32_f32(vmulq_f32(v5, id))),
            );
            let s67 = vcombine_s16(
                vqmovn_s32(vcvtaq_s32_f32(vmulq_f32(v6, id))),
                vqmovn_s32(vcvtaq_s32_f32(vmulq_f32(v7, id))),
            );
            let q1 = vcombine_s8(vqmovn_s16(s45), vqmovn_s16(s67));
            vst1q_s8(y.qs.as_mut_ptr().add(16), q1);
        }
    }
}

#[inline(always)]
pub(crate) fn vec_dot_4_q8_0_q8_0(
    n: usize,
    xs0: &[BlockQ8_0],
    xs1: &[BlockQ8_0],
    xs2: &[BlockQ8_0],
    xs3: &[BlockQ8_0],
    ys: &[BlockQ8_0],
) -> (f32, f32, f32, f32) {
    debug_assert!(
        n.is_multiple_of(QK8_0),
        "vec_dot_4_q8_0_q8_0: {n} is not divisible by {QK8_0}"
    );
    let nb = n / QK8_0;
    unsafe {
        let mut sum0 = vdupq_n_f32(0.0f32);
        let mut sum1 = vdupq_n_f32(0.0f32);
        let mut sum2 = vdupq_n_f32(0.0f32);
        let mut sum3 = vdupq_n_f32(0.0f32);
        for i in 0..nb {
            let y = &ys[i];
            let y0 = vld1q_s8(y.qs.as_ptr());
            let y1 = vld1q_s8(y.qs.as_ptr().add(16));
            let yd = y.d.to_f32();
            let x0 = &xs0[i];
            let x1 = &xs1[i];
            let x2 = &xs2[i];
            let x3 = &xs3[i];
            // Each column uses 2 SDOTs fused into one accumulator via vdotq_s32_pair.
            // All 4 columns share y0/y1 and have independent accumulators.
            let p0 = vdotq_s32_pair(
                vld1q_s8(x0.qs.as_ptr()),
                y0,
                vld1q_s8(x0.qs.as_ptr().add(16)),
                y1,
            );
            let p1 = vdotq_s32_pair(
                vld1q_s8(x1.qs.as_ptr()),
                y0,
                vld1q_s8(x1.qs.as_ptr().add(16)),
                y1,
            );
            let p2 = vdotq_s32_pair(
                vld1q_s8(x2.qs.as_ptr()),
                y0,
                vld1q_s8(x2.qs.as_ptr().add(16)),
                y1,
            );
            let p3 = vdotq_s32_pair(
                vld1q_s8(x3.qs.as_ptr()),
                y0,
                vld1q_s8(x3.qs.as_ptr().add(16)),
                y1,
            );
            sum0 = vmlaq_n_f32(sum0, vcvtq_f32_s32(p0), x0.d.to_f32() * yd);
            sum1 = vmlaq_n_f32(sum1, vcvtq_f32_s32(p1), x1.d.to_f32() * yd);
            sum2 = vmlaq_n_f32(sum2, vcvtq_f32_s32(p2), x2.d.to_f32() * yd);
            sum3 = vmlaq_n_f32(sum3, vcvtq_f32_s32(p3), x3.d.to_f32() * yd);
        }
        (
            vaddvq_f32(sum0),
            vaddvq_f32(sum1),
            vaddvq_f32(sum2),
            vaddvq_f32(sum3),
        )
    }
}

#[cfg(target_arch = "aarch64")]
fn quantize_q8_0x4_interleaved<const BLOCK_LEN: usize>(
    lhs: &[f32],
    k: usize,
    row: usize,
    out: &mut [BlockQ8_0x4],
) {
    let k_in_blocks = k / QK8_0;
    for b in 0..k_in_blocks {
        let mut p = BlockQ8_0x4 {
            d: [f16::ZERO; 4],
            qs: [0; QK8_0 * 4],
        };
        let mut ids = [0f32; 4];
        for r in 0..4 {
            let xs = &lhs[(row + r) * k + b * QK8_0..(row + r) * k + (b + 1) * QK8_0];
            let mut amax = 0f32;
            for &x in xs {
                amax = amax.max(x.abs());
            }
            let d = amax / ((1 << 7) - 1) as f32;
            ids[r] = if d != 0f32 { 1. / d } else { 0. };
            p.d[r] = f16::from_f32(d);
        }
        for r in 0..4 {
            let xs = &lhs[(row + r) * k + b * QK8_0..(row + r) * k + (b + 1) * QK8_0];
            for (j, &x) in xs.iter().enumerate() {
                p.qs[(j / BLOCK_LEN) * BLOCK_LEN * 4 + r * BLOCK_LEN + j % BLOCK_LEN] =
                    (x * ids[r]).round() as i8;
            }
        }
        out[b] = p;
    }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "dotprod")]
unsafe fn vec_dot_4_q4_0x4_q8_0_4x4(n: usize, xs: &[BlockQ4_0x4], ys: &[BlockQ8_0]) -> [f32; 4] {
    debug_assert!(n.is_multiple_of(QK8_0));
    let mut out = [0f32; 4];
    let high_mask = vdupq_n_u8(0xf0);
    let mut acc = vdupq_n_f32(0.);

    for (x, y) in xs.iter().zip(ys.iter()) {
        let bd = load_f16x4(x.d.as_ptr());
        let ad = vdupq_n_f32(y.d.to_f32());
        let a0 = vld1q_s8(y.qs.as_ptr());
        let a1 = vld1q_s8(y.qs.as_ptr().add(16));
        let b0 = vld1q_s8(x.qs.as_ptr());
        let b1 = vld1q_s8(x.qs.as_ptr().add(16));
        let b2 = vld1q_s8(x.qs.as_ptr().add(32));
        let b3 = vld1q_s8(x.qs.as_ptr().add(48));
        let mut ret = vdupq_n_s32(0);

        macro_rules! dot_lane {
            ($lane:literal, $b:expr, $a:expr) => {{
                ret = sdot_laneq_s32::<$lane>(ret, $b, $a);
            }};
        }

        dot_lane!(
            0,
            vreinterpretq_s8_u8(vshlq_n_u8(vreinterpretq_u8_s8(b0), 4)),
            a0
        );
        dot_lane!(
            1,
            vreinterpretq_s8_u8(vshlq_n_u8(vreinterpretq_u8_s8(b1), 4)),
            a0
        );
        dot_lane!(
            2,
            vreinterpretq_s8_u8(vshlq_n_u8(vreinterpretq_u8_s8(b2), 4)),
            a0
        );
        dot_lane!(
            3,
            vreinterpretq_s8_u8(vshlq_n_u8(vreinterpretq_u8_s8(b3), 4)),
            a0
        );
        dot_lane!(
            0,
            vreinterpretq_s8_u8(vandq_u8(vreinterpretq_u8_s8(b0), high_mask)),
            a1
        );
        dot_lane!(
            1,
            vreinterpretq_s8_u8(vandq_u8(vreinterpretq_u8_s8(b1), high_mask)),
            a1
        );
        dot_lane!(
            2,
            vreinterpretq_s8_u8(vandq_u8(vreinterpretq_u8_s8(b2), high_mask)),
            a1
        );
        dot_lane!(
            3,
            vreinterpretq_s8_u8(vandq_u8(vreinterpretq_u8_s8(b3), high_mask)),
            a1
        );

        acc = vmlaq_f32(
            acc,
            vmulq_n_f32(vcvtq_f32_s32(ret), 1. / 16.),
            vmulq_f32(ad, bd),
        );
    }

    vst1q_f32(out.as_mut_ptr(), acc);
    out
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "dotprod")]
unsafe fn vec_dot_4_q4_0x4_q8_0_4x8(n: usize, xs: &[BlockQ4_0x4], ys: &[BlockQ8_0]) -> [f32; 4] {
    debug_assert!(n.is_multiple_of(QK8_0));
    let mut out = [0f32; 4];
    let high_mask = vdupq_n_u8(0xf0);
    let mut acc = vdupq_n_f32(0.);

    for (x, y) in xs.iter().zip(ys.iter()) {
        let bd = load_f16x4(x.d.as_ptr());
        let ad = vdupq_n_f32(y.d.to_f32());
        let a0 = vreinterpretq_s8_s64(vld1q_dup_s64(y.qs.as_ptr() as *const i64));
        let a1 = vreinterpretq_s8_s64(vld1q_dup_s64(y.qs.as_ptr().add(8) as *const i64));
        let a2 = vreinterpretq_s8_s64(vld1q_dup_s64(y.qs.as_ptr().add(16) as *const i64));
        let a3 = vreinterpretq_s8_s64(vld1q_dup_s64(y.qs.as_ptr().add(24) as *const i64));
        let b0 = vld1q_s8(x.qs.as_ptr());
        let b1 = vld1q_s8(x.qs.as_ptr().add(16));
        let b2 = vld1q_s8(x.qs.as_ptr().add(32));
        let b3 = vld1q_s8(x.qs.as_ptr().add(48));
        let mut ret0 = vdupq_n_s32(0);
        let mut ret1 = vdupq_n_s32(0);

        ret0 = sdot_acc(
            ret0,
            vreinterpretq_s8_u8(vshlq_n_u8(vreinterpretq_u8_s8(b0), 4)),
            a0,
        );
        ret1 = sdot_acc(
            ret1,
            vreinterpretq_s8_u8(vshlq_n_u8(vreinterpretq_u8_s8(b1), 4)),
            a0,
        );
        ret0 = sdot_acc(
            ret0,
            vreinterpretq_s8_u8(vshlq_n_u8(vreinterpretq_u8_s8(b2), 4)),
            a1,
        );
        ret1 = sdot_acc(
            ret1,
            vreinterpretq_s8_u8(vshlq_n_u8(vreinterpretq_u8_s8(b3), 4)),
            a1,
        );
        ret0 = sdot_acc(
            ret0,
            vreinterpretq_s8_u8(vandq_u8(vreinterpretq_u8_s8(b0), high_mask)),
            a2,
        );
        ret1 = sdot_acc(
            ret1,
            vreinterpretq_s8_u8(vandq_u8(vreinterpretq_u8_s8(b1), high_mask)),
            a2,
        );
        ret0 = sdot_acc(
            ret0,
            vreinterpretq_s8_u8(vandq_u8(vreinterpretq_u8_s8(b2), high_mask)),
            a3,
        );
        ret1 = sdot_acc(
            ret1,
            vreinterpretq_s8_u8(vandq_u8(vreinterpretq_u8_s8(b3), high_mask)),
            a3,
        );

        let ret = vpaddq_s32(ret0, ret1);
        acc = vmlaq_f32(
            acc,
            vmulq_n_f32(vcvtq_f32_s32(ret), 1. / 16.),
            vmulq_f32(ad, bd),
        );
    }

    vst1q_f32(out.as_mut_ptr(), acc);
    out
}

#[cfg(target_arch = "aarch64")]
#[inline(always)]
fn vec_dot_4_q4_0x4_q8_0(n: usize, xs: &[BlockQ4_0x4], ys: &[BlockQ8_0]) -> [f32; 4] {
    unsafe {
        if crate::cpu::features::get().i8mm {
            vec_dot_4_q4_0x4_q8_0_4x8(n, xs, ys)
        } else {
            vec_dot_4_q4_0x4_q8_0_4x4(n, xs, ys)
        }
    }
}

#[cfg(target_arch = "aarch64")]
fn matmul_q4_0_x4_gemv(
    (_m, k, n): (usize, usize, usize),
    lhs: &[f32],
    repacked: &[BlockQ4_0x4],
    dst: &mut [f32],
) -> crate::Result<()> {
    debug_assert!(k.is_multiple_of(QK8_0));
    debug_assert!(n.is_multiple_of(4));
    let k_in_blocks = k / QK8_0;
    let n_groups = n / 4;

    thread_local! {
        static LHS_Q4_0_GEMV_SCRATCH: std::cell::RefCell<Vec<u64>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }

    let elem_size = std::mem::size_of::<BlockQ8_0>();
    let required_len = (k_in_blocks * elem_size).div_ceil(8);
    LHS_Q4_0_GEMV_SCRATCH.with(|cell| -> crate::Result<()> {
        let mut scratch = cell.borrow_mut();
        if scratch.len() < required_len {
            scratch.resize(required_len, 0);
        }
        let lhs_b: &mut [BlockQ8_0] = unsafe {
            std::slice::from_raw_parts_mut(scratch.as_mut_ptr() as *mut BlockQ8_0, k_in_blocks)
        };
        <BlockQ8_0 as super::GgmlType>::from_float(lhs, lhs_b);

        let pool = crate::utils::barrier_pool();
        let lhs_ptr = lhs_b.as_ptr() as usize;
        let repacked_ptr = repacked.as_ptr() as usize;
        let dst_ptr = dst.as_mut_ptr() as usize;
        let x4_block_bytes = std::mem::size_of::<BlockQ4_0x4>();

        // equal-sized gemv units: a static split skips the shared cursor, which costs
        // several us per small decode matmul
        pool.execute_static(n_groups, |range| {
            let lhs_row =
                unsafe { std::slice::from_raw_parts(lhs_ptr as *const BlockQ8_0, k_in_blocks) };
            let dst_ptr = dst_ptr as *mut f32;
            for g in range {
                let xs = unsafe {
                    std::slice::from_raw_parts(
                        (repacked_ptr + g * k_in_blocks * x4_block_bytes) as *const BlockQ4_0x4,
                        k_in_blocks,
                    )
                };
                let results = vec_dot_4_q4_0x4_q8_0(k, xs, lhs_row);
                unsafe {
                    std::ptr::copy_nonoverlapping(results.as_ptr(), dst_ptr.add(g * 4), 4);
                }
            }
        });

        Ok(())
    })
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "dotprod")]
unsafe fn store_q4_0x4_4x4(
    dst: *mut f32,
    n: usize,
    row: usize,
    col: usize,
    lhs: &[BlockQ8_0x4],
    rhs: &[BlockQ4_0x4],
    k_in_blocks: usize,
) {
    let mut sum0 = vdupq_n_f32(0.);
    let mut sum1 = vdupq_n_f32(0.);
    let mut sum2 = vdupq_n_f32(0.);
    let mut sum3 = vdupq_n_f32(0.);
    let high_mask = vdupq_n_u8(0xf0);

    for i in 0..k_in_blocks {
        let x = &rhs[i];
        let y = &lhs[i];
        let bd = [
            x.d[0].to_f32(),
            x.d[1].to_f32(),
            x.d[2].to_f32(),
            x.d[3].to_f32(),
        ];
        let bd = vld1q_f32(bd.as_ptr());
        let ad = [
            y.d[0].to_f32(),
            y.d[1].to_f32(),
            y.d[2].to_f32(),
            y.d[3].to_f32(),
        ];

        let mut ret0 = vdupq_n_s32(0);
        let mut ret1 = vdupq_n_s32(0);
        let mut ret2 = vdupq_n_s32(0);
        let mut ret3 = vdupq_n_s32(0);

        macro_rules! dot_rows {
            ($xv:expr, $yv:expr) => {{
                ret0 = sdot_laneq_s32::<0>(ret0, $xv, $yv);
                ret1 = sdot_laneq_s32::<1>(ret1, $xv, $yv);
                ret2 = sdot_laneq_s32::<2>(ret2, $xv, $yv);
                ret3 = sdot_laneq_s32::<3>(ret3, $xv, $yv);
            }};
        }

        for j in 0..QK8_0 / 8 {
            let q4 = vld1q_s8(x.qs.as_ptr().add(j * 16));
            let q4lo = vreinterpretq_s8_u8(vshlq_n_u8(vreinterpretq_u8_s8(q4), 4));
            let q4hi = vreinterpretq_s8_u8(vandq_u8(vreinterpretq_u8_s8(q4), high_mask));
            dot_rows!(q4lo, vld1q_s8(y.qs.as_ptr().add(j * 16)));
            dot_rows!(q4hi, vld1q_s8(y.qs.as_ptr().add((j + 4) * 16)));
        }

        sum0 = vmlaq_f32(
            sum0,
            vmulq_n_f32(vcvtq_f32_s32(ret0), 1. / 16.),
            vmulq_n_f32(bd, ad[0]),
        );
        sum1 = vmlaq_f32(
            sum1,
            vmulq_n_f32(vcvtq_f32_s32(ret1), 1. / 16.),
            vmulq_n_f32(bd, ad[1]),
        );
        sum2 = vmlaq_f32(
            sum2,
            vmulq_n_f32(vcvtq_f32_s32(ret2), 1. / 16.),
            vmulq_n_f32(bd, ad[2]),
        );
        sum3 = vmlaq_f32(
            sum3,
            vmulq_n_f32(vcvtq_f32_s32(ret3), 1. / 16.),
            vmulq_n_f32(bd, ad[3]),
        );
    }

    vst1q_f32(dst.add(row * n + col), sum0);
    vst1q_f32(dst.add((row + 1) * n + col), sum1);
    vst1q_f32(dst.add((row + 2) * n + col), sum2);
    vst1q_f32(dst.add((row + 3) * n + col), sum3);
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "dotprod")]
pub(crate) fn matmul_q4_0_x4(
    (m, k, n): (usize, usize, usize),
    lhs: &[f32],
    repacked: &[BlockQ4_0x4],
    dst: &mut [f32],
) -> crate::Result<()> {
    debug_assert!(k.is_multiple_of(QK8_0));
    debug_assert!(n.is_multiple_of(4));
    if m == 1 {
        return matmul_q4_0_x4_gemv((m, k, n), lhs, repacked, dst);
    }
    debug_assert!(m.is_multiple_of(4));
    let k_in_blocks = k / QK8_0;
    let n_groups = n / 4;
    let row_groups = m / 4;

    thread_local! {
        static LHS_Q4_0_X4_SCRATCH: std::cell::RefCell<Vec<u64>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }

    let elem_size = std::mem::size_of::<BlockQ8_0x4>();
    let required_len = (row_groups * k_in_blocks * elem_size).div_ceil(8);
    LHS_Q4_0_X4_SCRATCH.with(|cell| -> crate::Result<()> {
        let mut scratch = cell.borrow_mut();
        if scratch.len() < required_len {
            scratch.resize(required_len, 0);
        }
        let lhs_x4: &mut [BlockQ8_0x4] = unsafe {
            std::slice::from_raw_parts_mut(
                scratch.as_mut_ptr() as *mut BlockQ8_0x4,
                row_groups * k_in_blocks,
            )
        };
        for group in 0..row_groups {
            quantize_q8_0x4_interleaved::<4>(
                lhs,
                k,
                group * 4,
                &mut lhs_x4[group * k_in_blocks..(group + 1) * k_in_blocks],
            );
        }

        let tiles_total = row_groups * n_groups;
        let pool = crate::utils::barrier_pool();
        let lhs_ptr = lhs_x4.as_ptr() as usize;
        let repacked_ptr = repacked.as_ptr() as usize;
        let dst_ptr = dst.as_mut_ptr() as usize;

        pool.execute_chunked(tiles_total, |range| {
            let lhs_ptr = lhs_ptr as *const BlockQ8_0x4;
            let repacked_ptr = repacked_ptr as *const BlockQ4_0x4;
            let dst_ptr = dst_ptr as *mut f32;
            for tile in range {
                let row_group = tile / n_groups;
                let col_group = tile - row_group * n_groups;
                let row = row_group * 4;
                let col = col_group * 4;
                let lhs_tile = unsafe {
                    std::slice::from_raw_parts(lhs_ptr.add(row_group * k_in_blocks), k_in_blocks)
                };
                let rhs_tile = unsafe {
                    std::slice::from_raw_parts(
                        repacked_ptr.add(col_group * k_in_blocks),
                        k_in_blocks,
                    )
                };
                unsafe { store_q4_0x4_4x4(dst_ptr, n, row, col, lhs_tile, rhs_tile, k_in_blocks) };
            }
        });

        Ok(())
    })
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "i8mm")]
unsafe fn store_q4_0x4_4x4_i8mm(
    dst: *mut f32,
    n: usize,
    row: usize,
    col: usize,
    lhs: &[BlockQ8_0x4],
    rhs: &[BlockQ4_0x4],
    k_in_blocks: usize,
) {
    let high_mask = vdupq_n_u8(0xf0);
    let mut sum0 = vdupq_n_f32(0.);
    let mut sum1 = vdupq_n_f32(0.);
    let mut sum2 = vdupq_n_f32(0.);
    let mut sum3 = vdupq_n_f32(0.);

    for i in 0..k_in_blocks {
        let x = &rhs[i];
        let y = &lhs[i];
        let bd = [
            x.d[0].to_f32(),
            x.d[1].to_f32(),
            x.d[2].to_f32(),
            x.d[3].to_f32(),
        ];
        let bd = vld1q_f32(bd.as_ptr());
        let ad = [
            y.d[0].to_f32(),
            y.d[1].to_f32(),
            y.d[2].to_f32(),
            y.d[3].to_f32(),
        ];

        let mut acc00 = vdupq_n_s32(0);
        let mut acc01 = vdupq_n_s32(0);
        let mut acc10 = vdupq_n_s32(0);
        let mut acc11 = vdupq_n_s32(0);

        for chunk in 0..2 {
            let q4_offset = chunk * 32;
            let q8_lo_offset = chunk * 32;
            let q8_hi_offset = (chunk + 2) * 32;

            let q4_01 = vld1q_s8(x.qs.as_ptr().add(q4_offset));
            let q4_23 = vld1q_s8(x.qs.as_ptr().add(q4_offset + 16));
            let q4lo_01 = vreinterpretq_s8_u8(vshlq_n_u8(vreinterpretq_u8_s8(q4_01), 4));
            let q4lo_23 = vreinterpretq_s8_u8(vshlq_n_u8(vreinterpretq_u8_s8(q4_23), 4));
            let q4hi_01 = vreinterpretq_s8_u8(vandq_u8(vreinterpretq_u8_s8(q4_01), high_mask));
            let q4hi_23 = vreinterpretq_s8_u8(vandq_u8(vreinterpretq_u8_s8(q4_23), high_mask));

            let a01 = vld1q_s8(y.qs.as_ptr().add(q8_lo_offset));
            let a23 = vld1q_s8(y.qs.as_ptr().add(q8_lo_offset + 16));
            acc00 = smmla_s32(acc00, a01, q4lo_01);
            acc01 = smmla_s32(acc01, a01, q4lo_23);
            acc10 = smmla_s32(acc10, a23, q4lo_01);
            acc11 = smmla_s32(acc11, a23, q4lo_23);

            let a01 = vld1q_s8(y.qs.as_ptr().add(q8_hi_offset));
            let a23 = vld1q_s8(y.qs.as_ptr().add(q8_hi_offset + 16));
            acc00 = smmla_s32(acc00, a01, q4hi_01);
            acc01 = smmla_s32(acc01, a01, q4hi_23);
            acc10 = smmla_s32(acc10, a23, q4hi_01);
            acc11 = smmla_s32(acc11, a23, q4hi_23);
        }

        let row0 = vcombine_s32(vget_low_s32(acc00), vget_low_s32(acc01));
        let row1 = vcombine_s32(vget_high_s32(acc00), vget_high_s32(acc01));
        let row2 = vcombine_s32(vget_low_s32(acc10), vget_low_s32(acc11));
        let row3 = vcombine_s32(vget_high_s32(acc10), vget_high_s32(acc11));

        sum0 = vmlaq_f32(
            sum0,
            vmulq_n_f32(vcvtq_f32_s32(row0), 1. / 16.),
            vmulq_n_f32(bd, ad[0]),
        );
        sum1 = vmlaq_f32(
            sum1,
            vmulq_n_f32(vcvtq_f32_s32(row1), 1. / 16.),
            vmulq_n_f32(bd, ad[1]),
        );
        sum2 = vmlaq_f32(
            sum2,
            vmulq_n_f32(vcvtq_f32_s32(row2), 1. / 16.),
            vmulq_n_f32(bd, ad[2]),
        );
        sum3 = vmlaq_f32(
            sum3,
            vmulq_n_f32(vcvtq_f32_s32(row3), 1. / 16.),
            vmulq_n_f32(bd, ad[3]),
        );
    }

    vst1q_f32(dst.add(row * n + col), sum0);
    vst1q_f32(dst.add((row + 1) * n + col), sum1);
    vst1q_f32(dst.add((row + 2) * n + col), sum2);
    vst1q_f32(dst.add((row + 3) * n + col), sum3);
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "i8mm")]
pub(crate) fn matmul_q4_0_x4_i8mm(
    (m, k, n): (usize, usize, usize),
    lhs: &[f32],
    repacked: &[BlockQ4_0x4],
    dst: &mut [f32],
) -> crate::Result<()> {
    debug_assert!(k.is_multiple_of(QK8_0));
    debug_assert!(n.is_multiple_of(4));
    debug_assert!(m.is_multiple_of(4));
    let k_in_blocks = k / QK8_0;
    let n_groups = n / 4;
    let row_groups = m / 4;

    thread_local! {
        static LHS_Q4_0_X4_I8MM_SCRATCH: std::cell::RefCell<Vec<u64>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }

    let elem_size = std::mem::size_of::<BlockQ8_0x4>();
    let required_len = (row_groups * k_in_blocks * elem_size).div_ceil(8);
    LHS_Q4_0_X4_I8MM_SCRATCH.with(|cell| -> crate::Result<()> {
        let mut scratch = cell.borrow_mut();
        if scratch.len() < required_len {
            scratch.resize(required_len, 0);
        }
        let lhs_x4: &mut [BlockQ8_0x4] = unsafe {
            std::slice::from_raw_parts_mut(
                scratch.as_mut_ptr() as *mut BlockQ8_0x4,
                row_groups * k_in_blocks,
            )
        };
        for group in 0..row_groups {
            quantize_q8_0x4_interleaved::<8>(
                lhs,
                k,
                group * 4,
                &mut lhs_x4[group * k_in_blocks..(group + 1) * k_in_blocks],
            );
        }

        let tiles_total = row_groups * n_groups;
        let pool = crate::utils::barrier_pool();
        let lhs_ptr = lhs_x4.as_ptr() as usize;
        let repacked_ptr = repacked.as_ptr() as usize;
        let dst_ptr = dst.as_mut_ptr() as usize;

        pool.execute_chunked(tiles_total, |range| {
            let lhs_ptr = lhs_ptr as *const BlockQ8_0x4;
            let repacked_ptr = repacked_ptr as *const BlockQ4_0x4;
            let dst_ptr = dst_ptr as *mut f32;
            for tile in range {
                let row_group = tile / n_groups;
                let col_group = tile - row_group * n_groups;
                let row = row_group * 4;
                let col = col_group * 4;
                let lhs_tile = unsafe {
                    std::slice::from_raw_parts(lhs_ptr.add(row_group * k_in_blocks), k_in_blocks)
                };
                let rhs_tile = unsafe {
                    std::slice::from_raw_parts(
                        repacked_ptr.add(col_group * k_in_blocks),
                        k_in_blocks,
                    )
                };
                unsafe {
                    store_q4_0x4_4x4_i8mm(dst_ptr, n, row, col, lhs_tile, rhs_tile, k_in_blocks)
                };
            }
        });

        Ok(())
    })
}

#[cfg(target_arch = "aarch64")]
fn quantize_q8k_x4_interleaved<const BLOCK_LEN: usize>(
    lhs: &[f32],
    k: usize,
    row: usize,
    out: &mut [BlockQ8Kx4],
) {
    let k_in_blocks = k / QK_K;
    for b in 0..k_in_blocks {
        let mut p = BlockQ8Kx4 {
            d: [0.; 4],
            qs: [0; QK_K * 4],
            bsums: [0; QK_K / 4],
        };
        let mut src = [[0f32; QK_K]; 4];
        let mut iscale = [0f32; 4];
        for r in 0..4 {
            let xs = &lhs[(row + r) * k + b * QK_K..(row + r) * k + (b + 1) * QK_K];
            let mut max = 0f32;
            let mut amax = 0f32;
            for (j, &x) in xs.iter().enumerate() {
                src[r][j] = x;
                if amax < x.abs() {
                    amax = x.abs();
                    max = x;
                }
            }
            iscale[r] = if amax != 0. { -127. / max } else { 0. };
            p.d[r] = if amax != 0. { 1. / iscale[r] } else { 0. };
        }
        for j in 0..QK_K * 4 {
            let src_offset = (j / (4 * BLOCK_LEN)) * BLOCK_LEN + j % BLOCK_LEN;
            let src_id = (j % (4 * BLOCK_LEN)) / BLOCK_LEN;
            let q = (src[src_id][src_offset] * iscale[src_id]).round() as i8;
            p.qs[j] = q;
            let index = if BLOCK_LEN == 4 {
                (((j & 15) >> 2) << 2) + ((j >> 8) << 4) + ((j >> 6) & 3)
            } else {
                (((j & 31) >> 3) << 2) + ((j >> 8) << 4) + ((j >> 6) & 3)
            };
            p.bsums[index] += q as i16;
        }
        out[b] = p;
    }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "i8mm")]
unsafe fn store_q4kx8_4x8_i8mm(
    dst: *mut f32,
    n: usize,
    row: usize,
    col: usize,
    lhs: &[BlockQ8Kx4],
    rhs: &[BlockQ4Kx8],
    k_in_blocks: usize,
) {
    let m4b = vdupq_n_u8(0x0f);
    let mut acc_f32 = [vdupq_n_f32(0.); 8];

    for b in 0..k_in_blocks {
        let x = &rhs[b];
        let y = &lhs[b];
        let mut bsums_arr = [[0i16; 8]; 4];
        for (r, bsums_row) in bsums_arr.iter_mut().enumerate() {
            for (i, bsums) in bsums_row.iter_mut().enumerate() {
                // quarter-major interleaved layout: [16 * quarter + 4 * row + half-pair]
                let base = 16 * (i / 2) + 4 * r + (2 * i) % 4;
                *bsums = y.bsums[base] + y.bsums[base + 1];
            }
        }

        let mut acc = [vdupq_n_s32(0); 8];
        let mut bias_acc = [vdupq_n_s32(0); 8];

        for sb in 0..QK_K / 64 {
            let (mins_0, scales_0) = decode_q4kx8_scales(x.scales.as_ptr().add(sb * 24));
            let (mins_1, scales_1) = decode_q4kx8_scales(x.scales.as_ptr().add(sb * 24 + 12));
            let q4sb_mins = [mins_0, mins_1];
            let mut q4sb_scales = [[0i16; 8]; 2];
            vst1q_s16(q4sb_scales[0].as_mut_ptr(), scales_0);
            vst1q_s16(q4sb_scales[1].as_mut_ptr(), scales_1);

            let q8_base = y.qs.as_ptr().add(sb * 256);
            let mut q8s = [[vdupq_n_s8(0); 8]; 2];
            let (q8s_0, q8s_1) = q8s.split_at_mut(1);
            for (i, (q8_0, q8_1)) in q8s_0[0].iter_mut().zip(q8s_1[0].iter_mut()).enumerate() {
                *q8_0 = vld1q_s8(q8_base.add(i * 32));
                *q8_1 = vld1q_s8(q8_base.add(i * 32 + 16));
            }

            for cp in 0..4 {
                let qs = [
                    vld1q_u8(x.qs.as_ptr().add(sb * QK_K + 16 * cp)),
                    vld1q_u8(x.qs.as_ptr().add(sb * QK_K + 16 * cp + 64)),
                    vld1q_u8(x.qs.as_ptr().add(sb * QK_K + 16 * cp + 128)),
                    vld1q_u8(x.qs.as_ptr().add(sb * QK_K + 16 * cp + 192)),
                ];

                let mut acc_0 = vdupq_n_s32(0);
                let mut acc_1 = vdupq_n_s32(0);
                let mut acc_2 = vdupq_n_s32(0);
                let mut acc_3 = vdupq_n_s32(0);

                for i in 0..4 {
                    let qs_lo = vreinterpretq_s8_u8(vandq_u8(qs[i], m4b));
                    let qs_hi = vreinterpretq_s8_u8(vshrq_n_u8(qs[i], 4));
                    acc_0 = smmla_s32(acc_0, qs_lo, q8s[0][i]);
                    acc_2 = smmla_s32(acc_2, qs_lo, q8s[1][i]);
                    acc_1 = smmla_s32(acc_1, qs_hi, q8s[0][i + 4]);
                    acc_3 = smmla_s32(acc_3, qs_hi, q8s[1][i + 4]);
                }

                let scale_offset = cp * 2;
                let scale_0 = vcombine_s32(
                    vdup_n_s32(q4sb_scales[0][scale_offset] as i32),
                    vdup_n_s32(q4sb_scales[0][scale_offset + 1] as i32),
                );
                let scale_1 = vcombine_s32(
                    vdup_n_s32(q4sb_scales[1][scale_offset] as i32),
                    vdup_n_s32(q4sb_scales[1][scale_offset + 1] as i32),
                );
                acc[cp] = vmlaq_s32(acc[cp], acc_0, scale_0);
                acc[cp] = vmlaq_s32(acc[cp], acc_1, scale_1);
                acc[cp + 4] = vmlaq_s32(acc[cp + 4], acc_2, scale_0);
                acc[cp + 4] = vmlaq_s32(acc[cp + 4], acc_3, scale_1);
            }

            for q8_row in 0..4 {
                let bsums_vec_lo = vdup_n_s16(bsums_arr[q8_row][sb * 2]);
                let bsums_vec_hi = vdup_n_s16(bsums_arr[q8_row][sb * 2 + 1]);
                bias_acc[2 * q8_row] = vmlal_s16(
                    bias_acc[2 * q8_row],
                    bsums_vec_lo,
                    vget_low_s16(q4sb_mins[0]),
                );
                bias_acc[2 * q8_row] = vmlal_s16(
                    bias_acc[2 * q8_row],
                    bsums_vec_hi,
                    vget_low_s16(q4sb_mins[1]),
                );
                bias_acc[2 * q8_row + 1] = vmlal_s16(
                    bias_acc[2 * q8_row + 1],
                    bsums_vec_lo,
                    vget_high_s16(q4sb_mins[0]),
                );
                bias_acc[2 * q8_row + 1] = vmlal_s16(
                    bias_acc[2 * q8_row + 1],
                    bsums_vec_hi,
                    vget_high_s16(q4sb_mins[1]),
                );
            }
        }

        for item in &mut acc {
            let aux = vzip_s32(vget_low_s32(*item), vget_high_s32(*item));
            *item = vcombine_s32(aux.0, aux.1);
        }
        let reorder_acc = [
            vcombine_s32(vget_low_s32(acc[0]), vget_low_s32(acc[1])),
            vcombine_s32(vget_low_s32(acc[2]), vget_low_s32(acc[3])),
            vcombine_s32(vget_high_s32(acc[0]), vget_high_s32(acc[1])),
            vcombine_s32(vget_high_s32(acc[2]), vget_high_s32(acc[3])),
            vcombine_s32(vget_low_s32(acc[4]), vget_low_s32(acc[5])),
            vcombine_s32(vget_low_s32(acc[6]), vget_low_s32(acc[7])),
            vcombine_s32(vget_high_s32(acc[4]), vget_high_s32(acc[5])),
            vcombine_s32(vget_high_s32(acc[6]), vget_high_s32(acc[7])),
        ];

        for r in 0..4 {
            for g in 0..2 {
                let q4_d = [
                    x.d[g * 4].to_f32(),
                    x.d[g * 4 + 1].to_f32(),
                    x.d[g * 4 + 2].to_f32(),
                    x.d[g * 4 + 3].to_f32(),
                ];
                let q4_dmin = [
                    x.dmin[g * 4].to_f32(),
                    x.dmin[g * 4 + 1].to_f32(),
                    x.dmin[g * 4 + 2].to_f32(),
                    x.dmin[g * 4 + 3].to_f32(),
                ];
                let q8_d = y.d[r];
                let scale = vmulq_n_f32(vld1q_f32(q4_d.as_ptr()), q8_d);
                let dmins = vmulq_n_f32(vld1q_f32(q4_dmin.as_ptr()), q8_d);
                let idx = 2 * r + g;
                acc_f32[idx] =
                    vsubq_f32(acc_f32[idx], vmulq_f32(vcvtq_f32_s32(bias_acc[idx]), dmins));
                acc_f32[idx] = vmlaq_f32(acc_f32[idx], vcvtq_f32_s32(reorder_acc[idx]), scale);
            }
        }
    }

    for r in 0..4 {
        vst1q_f32(dst.add((row + r) * n + col), acc_f32[2 * r]);
        vst1q_f32(dst.add((row + r) * n + col + 4), acc_f32[2 * r + 1]);
    }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "i8mm")]
pub(crate) fn matmul_q4k_x8_i8mm(
    (m, k, n): (usize, usize, usize),
    lhs: &[f32],
    repacked: &[BlockQ4Kx8],
    dst: &mut [f32],
) -> crate::Result<()> {
    debug_assert!(k.is_multiple_of(QK_K));
    debug_assert!(n.is_multiple_of(8));
    debug_assert!(m.is_multiple_of(4));
    let k_in_blocks = k / QK_K;
    let n_groups = n / 8;
    let row_groups = m / 4;

    thread_local! {
        static LHS_Q4K_X8_I8MM_SCRATCH: std::cell::RefCell<Vec<u64>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }

    let elem_size = std::mem::size_of::<BlockQ8Kx4>();
    let required_len = (row_groups * k_in_blocks * elem_size).div_ceil(8);
    LHS_Q4K_X8_I8MM_SCRATCH.with(|cell| -> crate::Result<()> {
        let mut scratch = cell.borrow_mut();
        if scratch.len() < required_len {
            scratch.resize(required_len, 0);
        }
        let lhs_x4: &mut [BlockQ8Kx4] = unsafe {
            std::slice::from_raw_parts_mut(
                scratch.as_mut_ptr() as *mut BlockQ8Kx4,
                row_groups * k_in_blocks,
            )
        };
        for group in 0..row_groups {
            quantize_q8k_x4_interleaved::<8>(
                lhs,
                k,
                group * 4,
                &mut lhs_x4[group * k_in_blocks..(group + 1) * k_in_blocks],
            );
        }

        let tiles_total = row_groups * n_groups;
        let pool = crate::utils::barrier_pool();
        let lhs_ptr = lhs_x4.as_ptr() as usize;
        let repacked_ptr = repacked.as_ptr() as usize;
        let dst_ptr = dst.as_mut_ptr() as usize;

        pool.execute_chunked(tiles_total, |range| {
            let lhs_ptr = lhs_ptr as *const BlockQ8Kx4;
            let repacked_ptr = repacked_ptr as *const BlockQ4Kx8;
            let dst_ptr = dst_ptr as *mut f32;
            for tile in range {
                let row_group = tile / n_groups;
                let col_group = tile - row_group * n_groups;
                let row = row_group * 4;
                let col = col_group * 8;
                let lhs_tile = unsafe {
                    std::slice::from_raw_parts(lhs_ptr.add(row_group * k_in_blocks), k_in_blocks)
                };
                let rhs_tile = unsafe {
                    std::slice::from_raw_parts(
                        repacked_ptr.add(col_group * k_in_blocks),
                        k_in_blocks,
                    )
                };
                unsafe {
                    store_q4kx8_4x8_i8mm(dst_ptr, n, row, col, lhs_tile, rhs_tile, k_in_blocks)
                };
            }
        });

        Ok(())
    })
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "dotprod")]
unsafe fn store_q5kx8_4x8(
    dst: *mut f32,
    n: usize,
    row: usize,
    col: usize,
    lhs: &[BlockQ8Kx4],
    rhs: &[BlockQ5Kx8],
    k_in_blocks: usize,
) {
    let m4b = vdupq_n_u8(0x0f);
    let mone = vdupq_n_u8(1);
    let mtwo = vdupq_n_u8(2);
    let mut acc_f32 = [vdupq_n_f32(0.); 8];

    for b in 0..k_in_blocks {
        let x = &rhs[b];
        let y = &lhs[b];
        let q5_d_0123 = vld1q_f32(
            [
                x.d[0].to_f32(),
                x.d[1].to_f32(),
                x.d[2].to_f32(),
                x.d[3].to_f32(),
            ]
            .as_ptr(),
        );
        let q5_d_4567 = vld1q_f32(
            [
                x.d[4].to_f32(),
                x.d[5].to_f32(),
                x.d[6].to_f32(),
                x.d[7].to_f32(),
            ]
            .as_ptr(),
        );
        let q5_dmin_0123 = vld1q_f32(
            [
                x.dmin[0].to_f32(),
                x.dmin[1].to_f32(),
                x.dmin[2].to_f32(),
                x.dmin[3].to_f32(),
            ]
            .as_ptr(),
        );
        let q5_dmin_4567 = vld1q_f32(
            [
                x.dmin[4].to_f32(),
                x.dmin[5].to_f32(),
                x.dmin[6].to_f32(),
                x.dmin[7].to_f32(),
            ]
            .as_ptr(),
        );

        let sbd_scale_0123 = [
            vmulq_n_f32(q5_d_0123, y.d[0]),
            vmulq_n_f32(q5_d_0123, y.d[1]),
            vmulq_n_f32(q5_d_0123, y.d[2]),
            vmulq_n_f32(q5_d_0123, y.d[3]),
        ];
        let sbd_scale_4567 = [
            vmulq_n_f32(q5_d_4567, y.d[0]),
            vmulq_n_f32(q5_d_4567, y.d[1]),
            vmulq_n_f32(q5_d_4567, y.d[2]),
            vmulq_n_f32(q5_d_4567, y.d[3]),
        ];
        let sbd_min_0123 = [
            vmulq_n_f32(q5_dmin_0123, y.d[0]),
            vmulq_n_f32(q5_dmin_0123, y.d[1]),
            vmulq_n_f32(q5_dmin_0123, y.d[2]),
            vmulq_n_f32(q5_dmin_0123, y.d[3]),
        ];
        let sbd_min_4567 = [
            vmulq_n_f32(q5_dmin_4567, y.d[0]),
            vmulq_n_f32(q5_dmin_4567, y.d[1]),
            vmulq_n_f32(q5_dmin_4567, y.d[2]),
            vmulq_n_f32(q5_dmin_4567, y.d[3]),
        ];

        let mut bsums_arr = [[0i16; 8]; 4];
        for r in 0..4 {
            for i in 0..8 {
                // quarter-major interleaved layout: [16 * quarter + 4 * row + half-pair]
                let base = 16 * (i / 2) + 4 * r + (2 * i) % 4;
                bsums_arr[r][i] = y.bsums[base] + y.bsums[base + 1];
            }
        }

        let mut bias_acc = [vdupq_n_s32(0); 8];
        let mut qh = [[vdupq_n_u8(0); 8]; 2];
        for c in 0..2 {
            for i in 0..8 {
                qh[c][i] = vld1q_u8(x.qh.as_ptr().add(i * 32 + 16 * c));
            }
        }

        for sb in 0..QK_K / 64 {
            let mut acc_lo = [vdupq_n_s32(0); 8];
            let mut acc_hi = [vdupq_n_s32(0); 8];
            let (q5sb_mins_0, q5sb_scales_0) = decode_q4kx8_scales(x.scales.as_ptr().add(sb * 24));
            let (q5sb_mins_1, q5sb_scales_1) =
                decode_q4kx8_scales(x.scales.as_ptr().add(sb * 24 + 12));
            let q5sb_mins = [q5sb_mins_0, q5sb_mins_1];
            let q5sb_scales = [q5sb_scales_0, q5sb_scales_1];

            for k_idx in 0..8 {
                let q8_blk0 = vld1q_s8(y.qs.as_ptr().add(sb * 256 + 16 * k_idx));
                let q8_blk1 = vld1q_s8(y.qs.as_ptr().add(sb * 256 + 16 * k_idx + 128));
                let q5_0123 = vld1q_u8(x.qs.as_ptr().add(sb * QK_K + 32 * k_idx));
                let q5_4567 = vld1q_u8(x.qs.as_ptr().add(sb * QK_K + 32 * k_idx + 16));

                let hbit_lo_0123 = vandq_u8(qh[0][k_idx], mone);
                let hbit_hi_0123 = vshlq_n_u8(vandq_u8(qh[0][k_idx], mtwo), 3);
                qh[0][k_idx] = vshrq_n_u8(qh[0][k_idx], 2);
                let hbit_lo_4567 = vandq_u8(qh[1][k_idx], mone);
                let hbit_hi_4567 = vshlq_n_u8(vandq_u8(qh[1][k_idx], mtwo), 3);
                qh[1][k_idx] = vshrq_n_u8(qh[1][k_idx], 2);

                let q5_0123_lo = vreinterpretq_s8_u8(vorrq_u8(
                    vandq_u8(q5_0123, m4b),
                    vshlq_n_u8(hbit_lo_0123, 4),
                ));
                let q5_0123_hi =
                    vreinterpretq_s8_u8(vorrq_u8(vshrq_n_u8(q5_0123, 4), hbit_hi_0123));

                acc_lo[0] = sdot_laneq_s32::<0>(acc_lo[0], q5_0123_lo, q8_blk0);
                acc_lo[1] = sdot_laneq_s32::<1>(acc_lo[1], q5_0123_lo, q8_blk0);
                acc_lo[2] = sdot_laneq_s32::<2>(acc_lo[2], q5_0123_lo, q8_blk0);
                acc_lo[3] = sdot_laneq_s32::<3>(acc_lo[3], q5_0123_lo, q8_blk0);

                acc_hi[0] = sdot_laneq_s32::<0>(acc_hi[0], q5_0123_hi, q8_blk1);
                acc_hi[1] = sdot_laneq_s32::<1>(acc_hi[1], q5_0123_hi, q8_blk1);
                acc_hi[2] = sdot_laneq_s32::<2>(acc_hi[2], q5_0123_hi, q8_blk1);
                acc_hi[3] = sdot_laneq_s32::<3>(acc_hi[3], q5_0123_hi, q8_blk1);

                let q5_4567_lo = vreinterpretq_s8_u8(vorrq_u8(
                    vandq_u8(q5_4567, m4b),
                    vshlq_n_u8(hbit_lo_4567, 4),
                ));
                let q5_4567_hi =
                    vreinterpretq_s8_u8(vorrq_u8(vshrq_n_u8(q5_4567, 4), hbit_hi_4567));

                acc_lo[4] = sdot_laneq_s32::<0>(acc_lo[4], q5_4567_lo, q8_blk0);
                acc_lo[5] = sdot_laneq_s32::<1>(acc_lo[5], q5_4567_lo, q8_blk0);
                acc_lo[6] = sdot_laneq_s32::<2>(acc_lo[6], q5_4567_lo, q8_blk0);
                acc_lo[7] = sdot_laneq_s32::<3>(acc_lo[7], q5_4567_lo, q8_blk0);

                acc_hi[4] = sdot_laneq_s32::<0>(acc_hi[4], q5_4567_hi, q8_blk1);
                acc_hi[5] = sdot_laneq_s32::<1>(acc_hi[5], q5_4567_hi, q8_blk1);
                acc_hi[6] = sdot_laneq_s32::<2>(acc_hi[6], q5_4567_hi, q8_blk1);
                acc_hi[7] = sdot_laneq_s32::<3>(acc_hi[7], q5_4567_hi, q8_blk1);
            }

            let sc_0123_lo = vget_low_s16(q5sb_scales[0]);
            let sc_4567_lo = vget_high_s16(q5sb_scales[0]);
            let sc_0123_hi = vget_low_s16(q5sb_scales[1]);
            let sc_4567_hi = vget_high_s16(q5sb_scales[1]);
            for r in 0..4 {
                let sumf_0123 = vcvtq_f32_s32(vaddq_s32(
                    vmulq_s32(vmovl_s16(sc_0123_lo), acc_lo[r]),
                    vmulq_s32(vmovl_s16(sc_0123_hi), acc_hi[r]),
                ));
                acc_f32[2 * r] = vmlaq_f32(acc_f32[2 * r], sbd_scale_0123[r], sumf_0123);

                let sumf_4567 = vcvtq_f32_s32(vaddq_s32(
                    vmulq_s32(vmovl_s16(sc_4567_lo), acc_lo[r + 4]),
                    vmulq_s32(vmovl_s16(sc_4567_hi), acc_hi[r + 4]),
                ));
                acc_f32[2 * r + 1] = vmlaq_f32(acc_f32[2 * r + 1], sbd_scale_4567[r], sumf_4567);

                let bsums_vec_lo = vdup_n_s16(bsums_arr[r][sb * 2]);
                let bsums_vec_hi = vdup_n_s16(bsums_arr[r][sb * 2 + 1]);
                bias_acc[2 * r] =
                    vmlal_s16(bias_acc[2 * r], bsums_vec_lo, vget_low_s16(q5sb_mins[0]));
                bias_acc[2 * r] =
                    vmlal_s16(bias_acc[2 * r], bsums_vec_hi, vget_low_s16(q5sb_mins[1]));
                bias_acc[2 * r + 1] = vmlal_s16(
                    bias_acc[2 * r + 1],
                    bsums_vec_lo,
                    vget_high_s16(q5sb_mins[0]),
                );
                bias_acc[2 * r + 1] = vmlal_s16(
                    bias_acc[2 * r + 1],
                    bsums_vec_hi,
                    vget_high_s16(q5sb_mins[1]),
                );
            }
        }

        for r in 0..4 {
            acc_f32[2 * r] = vsubq_f32(
                acc_f32[2 * r],
                vmulq_f32(vcvtq_f32_s32(bias_acc[2 * r]), sbd_min_0123[r]),
            );
            acc_f32[2 * r + 1] = vsubq_f32(
                acc_f32[2 * r + 1],
                vmulq_f32(vcvtq_f32_s32(bias_acc[2 * r + 1]), sbd_min_4567[r]),
            );
        }
    }

    for r in 0..4 {
        vst1q_f32(dst.add((row + r) * n + col), acc_f32[2 * r]);
        vst1q_f32(dst.add((row + r) * n + col + 4), acc_f32[2 * r + 1]);
    }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "i8mm")]
unsafe fn store_q5kx8_4x8_i8mm(
    dst: *mut f32,
    n: usize,
    row: usize,
    col: usize,
    lhs: &[BlockQ8Kx4],
    rhs: &[BlockQ5Kx8],
    k_in_blocks: usize,
) {
    let m4b = vdupq_n_u8(0x0f);
    let mone = vdupq_n_u8(1);
    let mtwo = vdupq_n_u8(2);
    let mut acc_f32 = [vdupq_n_f32(0.); 8];

    for b in 0..k_in_blocks {
        let x = &rhs[b];
        let y = &lhs[b];
        let mut bsums_arr = [[0i16; 8]; 4];
        for (r, bsums_row) in bsums_arr.iter_mut().enumerate() {
            for (i, bsums) in bsums_row.iter_mut().enumerate() {
                // quarter-major interleaved layout: [16 * quarter + 4 * row + half-pair]
                let base = 16 * (i / 2) + 4 * r + (2 * i) % 4;
                *bsums = y.bsums[base] + y.bsums[base + 1];
            }
        }

        let mut acc = [vdupq_n_s32(0); 8];
        let mut bias_acc = [vdupq_n_s32(0); 8];
        let mut qh = [[vdupq_n_u8(0); 4]; 4];
        for (cp, qh_cp) in qh.iter_mut().enumerate() {
            qh_cp[0] = vld1q_u8(x.qh.as_ptr().add(16 * cp));
            qh_cp[1] = vld1q_u8(x.qh.as_ptr().add(16 * cp + 64));
            qh_cp[2] = vld1q_u8(x.qh.as_ptr().add(16 * cp + 128));
            qh_cp[3] = vld1q_u8(x.qh.as_ptr().add(16 * cp + 192));
        }

        for sb in 0..QK_K / 64 {
            let (mins_0, scales_0) = decode_q4kx8_scales(x.scales.as_ptr().add(sb * 24));
            let (mins_1, scales_1) = decode_q4kx8_scales(x.scales.as_ptr().add(sb * 24 + 12));
            let q5sb_mins = [mins_0, mins_1];
            let mut q5sb_scales = [[0i16; 8]; 2];
            vst1q_s16(q5sb_scales[0].as_mut_ptr(), scales_0);
            vst1q_s16(q5sb_scales[1].as_mut_ptr(), scales_1);

            let q8_base = y.qs.as_ptr().add(sb * 256);
            let mut q8s = [[vdupq_n_s8(0); 8]; 2];
            let (q8s_0, q8s_1) = q8s.split_at_mut(1);
            for (i, (q8_0, q8_1)) in q8s_0[0].iter_mut().zip(q8s_1[0].iter_mut()).enumerate() {
                *q8_0 = vld1q_s8(q8_base.add(i * 32));
                *q8_1 = vld1q_s8(q8_base.add(i * 32 + 16));
            }

            for cp in 0..4 {
                let qs = [
                    vld1q_u8(x.qs.as_ptr().add(sb * QK_K + 16 * cp)),
                    vld1q_u8(x.qs.as_ptr().add(sb * QK_K + 16 * cp + 64)),
                    vld1q_u8(x.qs.as_ptr().add(sb * QK_K + 16 * cp + 128)),
                    vld1q_u8(x.qs.as_ptr().add(sb * QK_K + 16 * cp + 192)),
                ];

                let mut acc_0 = vdupq_n_s32(0);
                let mut acc_1 = vdupq_n_s32(0);
                let mut acc_2 = vdupq_n_s32(0);
                let mut acc_3 = vdupq_n_s32(0);

                for i in 0..4 {
                    let hbit_lo = vandq_u8(qh[cp][i], mone);
                    let hbit_hi = vshlq_n_u8(vandq_u8(qh[cp][i], mtwo), 3);
                    qh[cp][i] = vshrq_n_u8(qh[cp][i], 2);

                    let qs_lo =
                        vreinterpretq_s8_u8(vorrq_u8(vandq_u8(qs[i], m4b), vshlq_n_u8(hbit_lo, 4)));
                    let qs_hi = vreinterpretq_s8_u8(vorrq_u8(vshrq_n_u8(qs[i], 4), hbit_hi));

                    acc_0 = smmla_s32(acc_0, qs_lo, q8s[0][i]);
                    acc_2 = smmla_s32(acc_2, qs_lo, q8s[1][i]);
                    acc_1 = smmla_s32(acc_1, qs_hi, q8s[0][i + 4]);
                    acc_3 = smmla_s32(acc_3, qs_hi, q8s[1][i + 4]);
                }

                let scale_offset = cp * 2;
                let scale_0 = vcombine_s32(
                    vdup_n_s32(q5sb_scales[0][scale_offset] as i32),
                    vdup_n_s32(q5sb_scales[0][scale_offset + 1] as i32),
                );
                let scale_1 = vcombine_s32(
                    vdup_n_s32(q5sb_scales[1][scale_offset] as i32),
                    vdup_n_s32(q5sb_scales[1][scale_offset + 1] as i32),
                );
                acc[cp] = vmlaq_s32(acc[cp], acc_0, scale_0);
                acc[cp] = vmlaq_s32(acc[cp], acc_1, scale_1);
                acc[cp + 4] = vmlaq_s32(acc[cp + 4], acc_2, scale_0);
                acc[cp + 4] = vmlaq_s32(acc[cp + 4], acc_3, scale_1);
            }

            for q8_row in 0..4 {
                let bsums_vec_lo = vdup_n_s16(bsums_arr[q8_row][sb * 2]);
                let bsums_vec_hi = vdup_n_s16(bsums_arr[q8_row][sb * 2 + 1]);
                bias_acc[2 * q8_row] = vmlal_s16(
                    bias_acc[2 * q8_row],
                    bsums_vec_lo,
                    vget_low_s16(q5sb_mins[0]),
                );
                bias_acc[2 * q8_row] = vmlal_s16(
                    bias_acc[2 * q8_row],
                    bsums_vec_hi,
                    vget_low_s16(q5sb_mins[1]),
                );
                bias_acc[2 * q8_row + 1] = vmlal_s16(
                    bias_acc[2 * q8_row + 1],
                    bsums_vec_lo,
                    vget_high_s16(q5sb_mins[0]),
                );
                bias_acc[2 * q8_row + 1] = vmlal_s16(
                    bias_acc[2 * q8_row + 1],
                    bsums_vec_hi,
                    vget_high_s16(q5sb_mins[1]),
                );
            }
        }

        for item in &mut acc {
            let aux = vzip_s32(vget_low_s32(*item), vget_high_s32(*item));
            *item = vcombine_s32(aux.0, aux.1);
        }
        let reorder_acc = [
            vcombine_s32(vget_low_s32(acc[0]), vget_low_s32(acc[1])),
            vcombine_s32(vget_low_s32(acc[2]), vget_low_s32(acc[3])),
            vcombine_s32(vget_high_s32(acc[0]), vget_high_s32(acc[1])),
            vcombine_s32(vget_high_s32(acc[2]), vget_high_s32(acc[3])),
            vcombine_s32(vget_low_s32(acc[4]), vget_low_s32(acc[5])),
            vcombine_s32(vget_low_s32(acc[6]), vget_low_s32(acc[7])),
            vcombine_s32(vget_high_s32(acc[4]), vget_high_s32(acc[5])),
            vcombine_s32(vget_high_s32(acc[6]), vget_high_s32(acc[7])),
        ];

        for r in 0..4 {
            for g in 0..2 {
                let q5_d = [
                    x.d[g * 4].to_f32(),
                    x.d[g * 4 + 1].to_f32(),
                    x.d[g * 4 + 2].to_f32(),
                    x.d[g * 4 + 3].to_f32(),
                ];
                let q5_dmin = [
                    x.dmin[g * 4].to_f32(),
                    x.dmin[g * 4 + 1].to_f32(),
                    x.dmin[g * 4 + 2].to_f32(),
                    x.dmin[g * 4 + 3].to_f32(),
                ];
                let q8_d = y.d[r];
                let scale = vmulq_n_f32(vld1q_f32(q5_d.as_ptr()), q8_d);
                let dmins = vmulq_n_f32(vld1q_f32(q5_dmin.as_ptr()), q8_d);
                let idx = 2 * r + g;
                acc_f32[idx] =
                    vsubq_f32(acc_f32[idx], vmulq_f32(vcvtq_f32_s32(bias_acc[idx]), dmins));
                acc_f32[idx] = vmlaq_f32(acc_f32[idx], vcvtq_f32_s32(reorder_acc[idx]), scale);
            }
        }
    }

    for r in 0..4 {
        vst1q_f32(dst.add((row + r) * n + col), acc_f32[2 * r]);
        vst1q_f32(dst.add((row + r) * n + col + 4), acc_f32[2 * r + 1]);
    }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "dotprod")]
unsafe fn vec_dot_8_q5kx8_q8k_8x4(n: usize, xs: &[BlockQ5Kx8], ys: &[BlockQ8K]) -> [f32; 8] {
    debug_assert!(n.is_multiple_of(QK_K));
    let mut out = [0f32; 8];
    let m4b = vdupq_n_u8(0x0f);
    let mone = vdupq_n_u8(1);
    let mtwo = vdupq_n_u8(2);
    let mut acc_f32_0 = vdupq_n_f32(0.);
    let mut acc_f32_1 = vdupq_n_f32(0.);

    for (q5, q8) in xs.iter().zip(ys.iter()) {
        let sb_scale_0 = vmulq_f32(load_f16x4(q5.d.as_ptr()), vdupq_n_f32(q8.d));
        let sb_scale_1 = vmulq_f32(load_f16x4(q5.d.as_ptr().add(4)), vdupq_n_f32(q8.d));
        let sb_min_0 = vmulq_f32(load_f16x4(q5.dmin.as_ptr()), vdupq_n_f32(q8.d));
        let sb_min_1 = vmulq_f32(load_f16x4(q5.dmin.as_ptr().add(4)), vdupq_n_f32(q8.d));
        let bsums = vpaddq_s16(
            vld1q_s16(q8.bsums.as_ptr()),
            vld1q_s16(q8.bsums.as_ptr().add(8)),
        );
        let mut bsums_arr = [0i16; 8];
        vst1q_s16(bsums_arr.as_mut_ptr(), bsums);
        let mut qh = [[vdupq_n_u8(0); 8]; 2];
        for c in 0..2 {
            for i in 0..8 {
                qh[c][i] = vld1q_u8(q5.qh.as_ptr().add(i * 32 + 16 * c));
            }
        }
        let mut bias_acc = [vdupq_n_s32(0); 2];

        for sb in 0..QK_K / 64 {
            let mut acc_lo = [vdupq_n_s32(0); 2];
            let mut acc_hi = [vdupq_n_s32(0); 2];
            let (mins_0, scales_0) = decode_q4kx8_scales(q5.scales.as_ptr().add(sb * 24));
            let (mins_1, scales_1) = decode_q4kx8_scales(q5.scales.as_ptr().add(sb * 24 + 12));
            let q5sb_mins = [mins_0, mins_1];
            let q5sb_scales = [scales_0, scales_1];
            let q8_qs = [
                vld1q_s8(q8.qs.as_ptr().add(sb * 64)),
                vld1q_s8(q8.qs.as_ptr().add(sb * 64 + 16)),
                vld1q_s8(q8.qs.as_ptr().add(sb * 64 + 32)),
                vld1q_s8(q8.qs.as_ptr().add(sb * 64 + 48)),
            ];

            for c in 0..2 {
                for i in 0..8 {
                    let q5_col = vld1q_u8(q5.qs.as_ptr().add(sb * QK_K + i * 32 + 16 * c));
                    let hbit_lo = vandq_u8(qh[c][i], mone);
                    let hbit_hi = vshlq_n_u8(vandq_u8(qh[c][i], mtwo), 3);
                    qh[c][i] = vshrq_n_u8(qh[c][i], 2);
                    let q5_lo = vreinterpretq_s8_u8(vsliq_n_u8(vandq_u8(q5_col, m4b), hbit_lo, 4));
                    let q5_hi = vreinterpretq_s8_u8(vorrq_u8(vshrq_n_u8(q5_col, 4), hbit_hi));
                    match i {
                        0 => {
                            acc_lo[c] = sdot_laneq_s32::<0>(acc_lo[c], q5_lo, q8_qs[0]);
                            acc_hi[c] = sdot_laneq_s32::<0>(acc_hi[c], q5_hi, q8_qs[2]);
                        }
                        1 => {
                            acc_lo[c] = sdot_laneq_s32::<1>(acc_lo[c], q5_lo, q8_qs[0]);
                            acc_hi[c] = sdot_laneq_s32::<1>(acc_hi[c], q5_hi, q8_qs[2]);
                        }
                        2 => {
                            acc_lo[c] = sdot_laneq_s32::<2>(acc_lo[c], q5_lo, q8_qs[0]);
                            acc_hi[c] = sdot_laneq_s32::<2>(acc_hi[c], q5_hi, q8_qs[2]);
                        }
                        3 => {
                            acc_lo[c] = sdot_laneq_s32::<3>(acc_lo[c], q5_lo, q8_qs[0]);
                            acc_hi[c] = sdot_laneq_s32::<3>(acc_hi[c], q5_hi, q8_qs[2]);
                        }
                        4 => {
                            acc_lo[c] = sdot_laneq_s32::<0>(acc_lo[c], q5_lo, q8_qs[1]);
                            acc_hi[c] = sdot_laneq_s32::<0>(acc_hi[c], q5_hi, q8_qs[3]);
                        }
                        5 => {
                            acc_lo[c] = sdot_laneq_s32::<1>(acc_lo[c], q5_lo, q8_qs[1]);
                            acc_hi[c] = sdot_laneq_s32::<1>(acc_hi[c], q5_hi, q8_qs[3]);
                        }
                        6 => {
                            acc_lo[c] = sdot_laneq_s32::<2>(acc_lo[c], q5_lo, q8_qs[1]);
                            acc_hi[c] = sdot_laneq_s32::<2>(acc_hi[c], q5_hi, q8_qs[3]);
                        }
                        _ => {
                            acc_lo[c] = sdot_laneq_s32::<3>(acc_lo[c], q5_lo, q8_qs[1]);
                            acc_hi[c] = sdot_laneq_s32::<3>(acc_hi[c], q5_hi, q8_qs[3]);
                        }
                    }
                }
            }

            let sumf_0 = vcvtq_f32_s32(vaddq_s32(
                vmulq_s32(vmovl_s16(vget_low_s16(q5sb_scales[0])), acc_lo[0]),
                vmulq_s32(vmovl_s16(vget_low_s16(q5sb_scales[1])), acc_hi[0]),
            ));
            acc_f32_0 = vfmaq_f32(acc_f32_0, sb_scale_0, sumf_0);
            let sumf_1 = vcvtq_f32_s32(vaddq_s32(
                vmulq_s32(vmovl_s16(vget_high_s16(q5sb_scales[0])), acc_lo[1]),
                vmulq_s32(vmovl_s16(vget_high_s16(q5sb_scales[1])), acc_hi[1]),
            ));
            acc_f32_1 = vfmaq_f32(acc_f32_1, sb_scale_1, sumf_1);

            let bsums_vec_lo = vdup_n_s16(bsums_arr[2 * sb]);
            let bsums_vec_hi = vdup_n_s16(bsums_arr[2 * sb + 1]);
            bias_acc[0] = vmlal_s16(bias_acc[0], bsums_vec_lo, vget_low_s16(q5sb_mins[0]));
            bias_acc[0] = vmlal_s16(bias_acc[0], bsums_vec_hi, vget_low_s16(q5sb_mins[1]));
            bias_acc[1] = vmlal_s16(bias_acc[1], bsums_vec_lo, vget_high_s16(q5sb_mins[0]));
            bias_acc[1] = vmlal_s16(bias_acc[1], bsums_vec_hi, vget_high_s16(q5sb_mins[1]));
        }

        acc_f32_0 = vmlsq_f32(acc_f32_0, vcvtq_f32_s32(bias_acc[0]), sb_min_0);
        acc_f32_1 = vmlsq_f32(acc_f32_1, vcvtq_f32_s32(bias_acc[1]), sb_min_1);
    }

    vst1q_f32(out.as_mut_ptr(), acc_f32_0);
    vst1q_f32(out.as_mut_ptr().add(4), acc_f32_1);
    out
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "i8mm")]
unsafe fn vec_dot_8_q5kx8_q8k_8x8(n: usize, xs: &[BlockQ5Kx8], ys: &[BlockQ8K]) -> [f32; 8] {
    debug_assert!(n.is_multiple_of(QK_K));
    let mut out = [0f32; 8];
    let m4b = vdupq_n_u8(0x0f);
    let mone = vdupq_n_u8(1);
    let mtwo = vdupq_n_u8(2);
    let mut acc_f32_0 = vdupq_n_f32(0.);
    let mut acc_f32_1 = vdupq_n_f32(0.);

    for (q5, q8) in xs.iter().zip(ys.iter()) {
        let sb_scale_0 = vmulq_f32(load_f16x4(q5.d.as_ptr()), vdupq_n_f32(q8.d));
        let sb_scale_1 = vmulq_f32(load_f16x4(q5.d.as_ptr().add(4)), vdupq_n_f32(q8.d));
        let sb_min_0 = vmulq_f32(load_f16x4(q5.dmin.as_ptr()), vdupq_n_f32(q8.d));
        let sb_min_1 = vmulq_f32(load_f16x4(q5.dmin.as_ptr().add(4)), vdupq_n_f32(q8.d));
        let bsums = vpaddq_s16(
            vld1q_s16(q8.bsums.as_ptr()),
            vld1q_s16(q8.bsums.as_ptr().add(8)),
        );
        let mut bsums_arr = [0i16; 8];
        vst1q_s16(bsums_arr.as_mut_ptr(), bsums);
        let mut qh = [[vdupq_n_u8(0); 4]; 4];
        for (cp, qh_cp) in qh.iter_mut().enumerate() {
            qh_cp[0] = vld1q_u8(q5.qh.as_ptr().add(16 * cp));
            qh_cp[1] = vld1q_u8(q5.qh.as_ptr().add(16 * cp + 64));
            qh_cp[2] = vld1q_u8(q5.qh.as_ptr().add(16 * cp + 128));
            qh_cp[3] = vld1q_u8(q5.qh.as_ptr().add(16 * cp + 192));
        }
        let mut bias_acc = [vdupq_n_s32(0); 2];

        for sb in 0..QK_K / 64 {
            let mut acc_lo = [vdupq_n_s32(0); 4];
            let mut acc_hi = [vdupq_n_s32(0); 4];
            let (mins_0, scales_0) = decode_q4kx8_scales(q5.scales.as_ptr().add(sb * 24));
            let (mins_1, scales_1) = decode_q4kx8_scales(q5.scales.as_ptr().add(sb * 24 + 12));
            let q5sb_mins = [mins_0, mins_1];
            let q5sb_scales = [scales_0, scales_1];
            let q8_base = q8.qs.as_ptr().add(sb * 64);
            let q8_qs = [
                vreinterpretq_s8_s64(vld1q_dup_s64(q8_base as *const i64)),
                vreinterpretq_s8_s64(vld1q_dup_s64(q8_base.add(8) as *const i64)),
                vreinterpretq_s8_s64(vld1q_dup_s64(q8_base.add(16) as *const i64)),
                vreinterpretq_s8_s64(vld1q_dup_s64(q8_base.add(24) as *const i64)),
                vreinterpretq_s8_s64(vld1q_dup_s64(q8_base.add(32) as *const i64)),
                vreinterpretq_s8_s64(vld1q_dup_s64(q8_base.add(40) as *const i64)),
                vreinterpretq_s8_s64(vld1q_dup_s64(q8_base.add(48) as *const i64)),
                vreinterpretq_s8_s64(vld1q_dup_s64(q8_base.add(56) as *const i64)),
            ];

            for cp in 0..4 {
                for i in 0..4 {
                    let q5_col = vld1q_u8(q5.qs.as_ptr().add(sb * QK_K + i * 64 + 16 * cp));
                    let hbit_lo = vandq_u8(qh[cp][i], mone);
                    let hbit_hi = vshlq_n_u8(vandq_u8(qh[cp][i], mtwo), 3);
                    qh[cp][i] = vshrq_n_u8(qh[cp][i], 2);
                    let q5_lo = vreinterpretq_s8_u8(vsliq_n_u8(vandq_u8(q5_col, m4b), hbit_lo, 4));
                    let q5_hi = vreinterpretq_s8_u8(vorrq_u8(vshrq_n_u8(q5_col, 4), hbit_hi));
                    acc_lo[cp] = sdot_acc(acc_lo[cp], q5_lo, q8_qs[i]);
                    acc_hi[cp] = sdot_acc(acc_hi[cp], q5_hi, q8_qs[i + 4]);
                }
            }

            let sumf_0 = vcvtq_f32_s32(vaddq_s32(
                vmulq_s32(
                    vmovl_s16(vget_low_s16(q5sb_scales[0])),
                    vpaddq_s32(acc_lo[0], acc_lo[1]),
                ),
                vmulq_s32(
                    vmovl_s16(vget_low_s16(q5sb_scales[1])),
                    vpaddq_s32(acc_hi[0], acc_hi[1]),
                ),
            ));
            acc_f32_0 = vfmaq_f32(acc_f32_0, sb_scale_0, sumf_0);
            let sumf_1 = vcvtq_f32_s32(vaddq_s32(
                vmulq_s32(
                    vmovl_s16(vget_high_s16(q5sb_scales[0])),
                    vpaddq_s32(acc_lo[2], acc_lo[3]),
                ),
                vmulq_s32(
                    vmovl_s16(vget_high_s16(q5sb_scales[1])),
                    vpaddq_s32(acc_hi[2], acc_hi[3]),
                ),
            ));
            acc_f32_1 = vfmaq_f32(acc_f32_1, sb_scale_1, sumf_1);

            let bsums_vec_lo = vdup_n_s16(bsums_arr[2 * sb]);
            let bsums_vec_hi = vdup_n_s16(bsums_arr[2 * sb + 1]);
            bias_acc[0] = vmlal_s16(bias_acc[0], bsums_vec_lo, vget_low_s16(q5sb_mins[0]));
            bias_acc[0] = vmlal_s16(bias_acc[0], bsums_vec_hi, vget_low_s16(q5sb_mins[1]));
            bias_acc[1] = vmlal_s16(bias_acc[1], bsums_vec_lo, vget_high_s16(q5sb_mins[0]));
            bias_acc[1] = vmlal_s16(bias_acc[1], bsums_vec_hi, vget_high_s16(q5sb_mins[1]));
        }

        acc_f32_0 = vmlsq_f32(acc_f32_0, vcvtq_f32_s32(bias_acc[0]), sb_min_0);
        acc_f32_1 = vmlsq_f32(acc_f32_1, vcvtq_f32_s32(bias_acc[1]), sb_min_1);
    }

    vst1q_f32(out.as_mut_ptr(), acc_f32_0);
    vst1q_f32(out.as_mut_ptr().add(4), acc_f32_1);
    out
}

#[cfg(target_arch = "aarch64")]
#[inline(always)]
fn vec_dot_8_q5kx8_q8k(n: usize, xs: &[BlockQ5Kx8], ys: &[BlockQ8K]) -> [f32; 8] {
    unsafe {
        if crate::cpu::features::get().i8mm {
            vec_dot_8_q5kx8_q8k_8x8(n, xs, ys)
        } else {
            vec_dot_8_q5kx8_q8k_8x4(n, xs, ys)
        }
    }
}

#[cfg(target_arch = "aarch64")]
fn matmul_q5k_x8_gemv(
    (_m, k, n): (usize, usize, usize),
    lhs: &[f32],
    repacked: &[BlockQ5Kx8],
    dst: &mut [f32],
) -> crate::Result<()> {
    debug_assert!(k.is_multiple_of(QK_K));
    debug_assert!(n.is_multiple_of(8));
    let k_in_blocks = k / QK_K;
    let n_groups = n / 8;

    thread_local! {
        static LHS_Q5K_GEMV_SCRATCH: std::cell::RefCell<Vec<u64>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }

    let elem_size = std::mem::size_of::<BlockQ8K>();
    let required_len = (k_in_blocks * elem_size).div_ceil(8);
    LHS_Q5K_GEMV_SCRATCH.with(|cell| -> crate::Result<()> {
        let mut scratch = cell.borrow_mut();
        if scratch.len() < required_len {
            scratch.resize(required_len, 0);
        }
        let lhs_b: &mut [BlockQ8K] = unsafe {
            std::slice::from_raw_parts_mut(scratch.as_mut_ptr() as *mut BlockQ8K, k_in_blocks)
        };
        <BlockQ8K as super::GgmlType>::from_float(lhs, lhs_b);

        let pool = crate::utils::barrier_pool();
        let lhs_ptr = lhs_b.as_ptr() as usize;
        let repacked_ptr = repacked.as_ptr() as usize;
        let dst_ptr = dst.as_mut_ptr() as usize;
        let x8_block_bytes = std::mem::size_of::<BlockQ5Kx8>();

        // equal-sized gemv units: a static split skips the shared cursor, which costs
        // several us per small decode matmul
        pool.execute_static(n_groups, |range| {
            let lhs_row =
                unsafe { std::slice::from_raw_parts(lhs_ptr as *const BlockQ8K, k_in_blocks) };
            let dst_ptr = dst_ptr as *mut f32;
            for g in range {
                let xs = unsafe {
                    std::slice::from_raw_parts(
                        (repacked_ptr + g * k_in_blocks * x8_block_bytes) as *const BlockQ5Kx8,
                        k_in_blocks,
                    )
                };
                let results = vec_dot_8_q5kx8_q8k(k, xs, lhs_row);
                unsafe {
                    std::ptr::copy_nonoverlapping(results.as_ptr(), dst_ptr.add(g * 8), 8);
                }
            }
        });

        Ok(())
    })
}

#[cfg(target_arch = "aarch64")]
pub(crate) fn matmul_q5k_x8(
    (m, k, n): (usize, usize, usize),
    lhs: &[f32],
    repacked: &[BlockQ5Kx8],
    dst: &mut [f32],
) -> crate::Result<()> {
    debug_assert!(k.is_multiple_of(QK_K));
    debug_assert!(n.is_multiple_of(8));
    let features = crate::cpu::features::get();
    if features.dotprod && m == 1 {
        return matmul_q5k_x8_gemv((m, k, n), lhs, repacked, dst);
    }
    let use_i8mm = features.i8mm;
    debug_assert!(m.is_multiple_of(4));
    let k_in_blocks = k / QK_K;
    let n_groups = n / 8;
    let row_groups = m / 4;

    thread_local! {
        static LHS_Q5K_X8_SCRATCH: std::cell::RefCell<Vec<u64>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }

    let elem_size = std::mem::size_of::<BlockQ8Kx4>();
    let required_len = (row_groups * k_in_blocks * elem_size).div_ceil(8);
    LHS_Q5K_X8_SCRATCH.with(|cell| -> crate::Result<()> {
        let mut scratch = cell.borrow_mut();
        if scratch.len() < required_len {
            scratch.resize(required_len, 0);
        }
        let lhs_x4: &mut [BlockQ8Kx4] = unsafe {
            std::slice::from_raw_parts_mut(
                scratch.as_mut_ptr() as *mut BlockQ8Kx4,
                row_groups * k_in_blocks,
            )
        };
        for group in 0..row_groups {
            if use_i8mm {
                quantize_q8k_x4_interleaved::<8>(
                    lhs,
                    k,
                    group * 4,
                    &mut lhs_x4[group * k_in_blocks..(group + 1) * k_in_blocks],
                );
            } else {
                quantize_q8k_x4_interleaved::<4>(
                    lhs,
                    k,
                    group * 4,
                    &mut lhs_x4[group * k_in_blocks..(group + 1) * k_in_blocks],
                );
            }
        }

        let tiles_total = row_groups * n_groups;
        let pool = crate::utils::barrier_pool();
        let lhs_ptr = lhs_x4.as_ptr() as usize;
        let repacked_ptr = repacked.as_ptr() as usize;
        let dst_ptr = dst.as_mut_ptr() as usize;

        pool.execute_chunked(tiles_total, |range| {
            let lhs_ptr = lhs_ptr as *const BlockQ8Kx4;
            let repacked_ptr = repacked_ptr as *const BlockQ5Kx8;
            let dst_ptr = dst_ptr as *mut f32;
            for tile in range {
                let row_group = tile / n_groups;
                let col_group = tile - row_group * n_groups;
                let row = row_group * 4;
                let col = col_group * 8;
                let lhs_tile = unsafe {
                    std::slice::from_raw_parts(lhs_ptr.add(row_group * k_in_blocks), k_in_blocks)
                };
                let rhs_tile = unsafe {
                    std::slice::from_raw_parts(
                        repacked_ptr.add(col_group * k_in_blocks),
                        k_in_blocks,
                    )
                };
                if use_i8mm {
                    unsafe {
                        store_q5kx8_4x8_i8mm(dst_ptr, n, row, col, lhs_tile, rhs_tile, k_in_blocks)
                    };
                } else {
                    unsafe {
                        store_q5kx8_4x8(dst_ptr, n, row, col, lhs_tile, rhs_tile, k_in_blocks)
                    };
                }
            }
        });

        Ok(())
    })
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "dotprod")]
unsafe fn store_q6kx8_4x8(
    dst: *mut f32,
    n: usize,
    row: usize,
    col: usize,
    lhs: &[BlockQ8Kx4],
    rhs: &[BlockQ6Kx8],
    k_in_blocks: usize,
) {
    let m4b = vdupq_n_u8(0x0f);
    let mask_lo = vdupq_n_u8(0x03);
    let mask_hi = vdupq_n_u8(0x30);
    let m32s = vdupq_n_s8(32);
    let mut acc_f32 = [vdupq_n_f32(0.); 8];

    for b in 0..k_in_blocks {
        let x = &rhs[b];
        let y = &lhs[b];
        let q6_d_0123 = vld1q_f32(
            [
                x.d[0].to_f32(),
                x.d[1].to_f32(),
                x.d[2].to_f32(),
                x.d[3].to_f32(),
            ]
            .as_ptr(),
        );
        let q6_d_4567 = vld1q_f32(
            [
                x.d[4].to_f32(),
                x.d[5].to_f32(),
                x.d[6].to_f32(),
                x.d[7].to_f32(),
            ]
            .as_ptr(),
        );

        let sbd_scale_0123 = [
            vmulq_n_f32(q6_d_0123, y.d[0]),
            vmulq_n_f32(q6_d_0123, y.d[1]),
            vmulq_n_f32(q6_d_0123, y.d[2]),
            vmulq_n_f32(q6_d_0123, y.d[3]),
        ];
        let sbd_scale_4567 = [
            vmulq_n_f32(q6_d_4567, y.d[0]),
            vmulq_n_f32(q6_d_4567, y.d[1]),
            vmulq_n_f32(q6_d_4567, y.d[2]),
            vmulq_n_f32(q6_d_4567, y.d[3]),
        ];

        let mut acc_s32 = [vdupq_n_s32(0); 8];
        for half in 0..2 {
            let ql_base = x.ql.as_ptr().add(half * 512);
            let qh_base = x.qh.as_ptr().add(half * 256);
            for sb in 0..QK_K / 64 {
                let mut acc_lo = [vdupq_n_s32(0); 8];
                let mut acc_hi = [vdupq_n_s32(0); 8];
                let q8_base_l = y.qs.as_ptr().add(half * 512 + sb * 64);
                let q8_base_h = y.qs.as_ptr().add(half * 512 + 256 + sb * 64);
                let ql_off_base = sb * QK_K / 2;
                let qh_off_base = ql_off_base & 255;

                for k_idx in 0..4 {
                    let q8_l = vld1q_s8(q8_base_l.add(16 * k_idx));
                    let q8_h = vld1q_s8(q8_base_h.add(16 * k_idx));
                    let q6_ql_0123 = vld1q_u8(ql_base.add(ql_off_base + k_idx * 32));
                    let q6_ql_4567 = vld1q_u8(ql_base.add(ql_off_base + k_idx * 32 + 16));
                    let mut q6_qh_0123 = vld1q_u8(qh_base.add(qh_off_base + k_idx * 32));
                    let mut q6_qh_4567 = vld1q_u8(qh_base.add(qh_off_base + k_idx * 32 + 16));
                    if sb > 1 {
                        q6_qh_0123 = vshrq_n_u8(q6_qh_0123, 2);
                        q6_qh_4567 = vshrq_n_u8(q6_qh_4567, 2);
                    }

                    let hbit_lo_0123 = vandq_u8(q6_qh_0123, mask_lo);
                    let hbit_hi_0123 = vandq_u8(q6_qh_0123, mask_hi);
                    let hbit_lo_4567 = vandq_u8(q6_qh_4567, mask_lo);
                    let hbit_hi_4567 = vandq_u8(q6_qh_4567, mask_hi);

                    let q6_0123_lo = vsubq_s8(
                        vreinterpretq_s8_u8(vorrq_u8(
                            vandq_u8(q6_ql_0123, m4b),
                            vshlq_n_u8(hbit_lo_0123, 4),
                        )),
                        m32s,
                    );
                    let q6_0123_hi = vsubq_s8(
                        vreinterpretq_s8_u8(vorrq_u8(vshrq_n_u8(q6_ql_0123, 4), hbit_hi_0123)),
                        m32s,
                    );

                    acc_lo[0] = sdot_laneq_s32::<0>(acc_lo[0], q6_0123_lo, q8_l);
                    acc_lo[1] = sdot_laneq_s32::<1>(acc_lo[1], q6_0123_lo, q8_l);
                    acc_lo[2] = sdot_laneq_s32::<2>(acc_lo[2], q6_0123_lo, q8_l);
                    acc_lo[3] = sdot_laneq_s32::<3>(acc_lo[3], q6_0123_lo, q8_l);

                    acc_hi[0] = sdot_laneq_s32::<0>(acc_hi[0], q6_0123_hi, q8_h);
                    acc_hi[1] = sdot_laneq_s32::<1>(acc_hi[1], q6_0123_hi, q8_h);
                    acc_hi[2] = sdot_laneq_s32::<2>(acc_hi[2], q6_0123_hi, q8_h);
                    acc_hi[3] = sdot_laneq_s32::<3>(acc_hi[3], q6_0123_hi, q8_h);

                    let q6_4567_lo = vsubq_s8(
                        vreinterpretq_s8_u8(vorrq_u8(
                            vandq_u8(q6_ql_4567, m4b),
                            vshlq_n_u8(hbit_lo_4567, 4),
                        )),
                        m32s,
                    );
                    let q6_4567_hi = vsubq_s8(
                        vreinterpretq_s8_u8(vorrq_u8(vshrq_n_u8(q6_ql_4567, 4), hbit_hi_4567)),
                        m32s,
                    );

                    acc_lo[4] = sdot_laneq_s32::<0>(acc_lo[4], q6_4567_lo, q8_l);
                    acc_lo[5] = sdot_laneq_s32::<1>(acc_lo[5], q6_4567_lo, q8_l);
                    acc_lo[6] = sdot_laneq_s32::<2>(acc_lo[6], q6_4567_lo, q8_l);
                    acc_lo[7] = sdot_laneq_s32::<3>(acc_lo[7], q6_4567_lo, q8_l);

                    acc_hi[4] = sdot_laneq_s32::<0>(acc_hi[4], q6_4567_hi, q8_h);
                    acc_hi[5] = sdot_laneq_s32::<1>(acc_hi[5], q6_4567_hi, q8_h);
                    acc_hi[6] = sdot_laneq_s32::<2>(acc_hi[6], q6_4567_hi, q8_h);
                    acc_hi[7] = sdot_laneq_s32::<3>(acc_hi[7], q6_4567_hi, q8_h);
                }

                let scale_idx_l = half * 8 + sb;
                let scale_idx_h = half * 8 + sb + 4;
                for g in 0..2 {
                    let base_l = scale_idx_l * 8 + g * 4;
                    let base_h = scale_idx_h * 8 + g * 4;
                    let scale_vec_l = vld1q_s32(
                        [
                            x.scales[base_l] as i32,
                            x.scales[base_l + 1] as i32,
                            x.scales[base_l + 2] as i32,
                            x.scales[base_l + 3] as i32,
                        ]
                        .as_ptr(),
                    );
                    let scale_vec_h = vld1q_s32(
                        [
                            x.scales[base_h] as i32,
                            x.scales[base_h + 1] as i32,
                            x.scales[base_h + 2] as i32,
                            x.scales[base_h + 3] as i32,
                        ]
                        .as_ptr(),
                    );
                    let acc_offset = g * 4;
                    for row in 0..4 {
                        let idx = row * 2 + g;
                        acc_s32[idx] =
                            vmlaq_s32(acc_s32[idx], acc_lo[acc_offset + row], scale_vec_l);
                        acc_s32[idx] =
                            vmlaq_s32(acc_s32[idx], acc_hi[acc_offset + row], scale_vec_h);
                    }
                }
            }
        }

        for r in 0..4 {
            let idx0 = 2 * r;
            let idx1 = 2 * r + 1;
            acc_f32[idx0] = vmlaq_f32(
                acc_f32[idx0],
                vcvtq_f32_s32(acc_s32[idx0]),
                sbd_scale_0123[r],
            );
            acc_f32[idx1] = vmlaq_f32(
                acc_f32[idx1],
                vcvtq_f32_s32(acc_s32[idx1]),
                sbd_scale_4567[r],
            );
        }
    }

    for r in 0..4 {
        vst1q_f32(dst.add((row + r) * n + col), acc_f32[2 * r]);
        vst1q_f32(dst.add((row + r) * n + col + 4), acc_f32[2 * r + 1]);
    }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "i8mm")]
unsafe fn store_q6kx8_4x8_i8mm(
    dst: *mut f32,
    n: usize,
    row: usize,
    col: usize,
    lhs: &[BlockQ8Kx4],
    rhs: &[BlockQ6Kx8],
    k_in_blocks: usize,
) {
    let m4b = vdupq_n_u8(0x0f);
    let mask_lo = vdupq_n_u8(0x03);
    let mask_hi = vdupq_n_u8(0x30);
    let m32s = vdupq_n_s8(32);
    let mut acc_f32 = [vdupq_n_f32(0.); 8];

    for b in 0..k_in_blocks {
        let x = &rhs[b];
        let y = &lhs[b];
        let mut acc = [vdupq_n_s32(0); 8];

        for half in 0..2 {
            let ql_base = x.ql.as_ptr().add(half * 512);
            let qh_base = x.qh.as_ptr().add(half * 256);
            for sb in 0..QK_K / 64 {
                let q8_base_l = y.qs.as_ptr().add(half * 512 + sb * 64);
                let q8_base_h = y.qs.as_ptr().add(half * 512 + 256 + sb * 64);
                let q8_l_01 = [vld1q_s8(q8_base_l), vld1q_s8(q8_base_l.add(32))];
                let q8_l_23 = [vld1q_s8(q8_base_l.add(16)), vld1q_s8(q8_base_l.add(48))];
                let q8_h_01 = [vld1q_s8(q8_base_h), vld1q_s8(q8_base_h.add(32))];
                let q8_h_23 = [vld1q_s8(q8_base_h.add(16)), vld1q_s8(q8_base_h.add(48))];

                let ql_off_base = sb * QK_K / 2;
                let qh_off_base = ql_off_base & 255;

                for cp in 0..4 {
                    let mut q6_qh_0 = vld1q_u8(qh_base.add(qh_off_base + 16 * cp));
                    let mut q6_qh_1 = vld1q_u8(qh_base.add(qh_off_base + 64 + 16 * cp));
                    if sb > 1 {
                        q6_qh_0 = vshrq_n_u8(q6_qh_0, 2);
                        q6_qh_1 = vshrq_n_u8(q6_qh_1, 2);
                    }

                    let q6_ql_0 = vld1q_u8(ql_base.add(ql_off_base + 16 * cp));
                    let q6_ql_1 = vld1q_u8(ql_base.add(ql_off_base + 64 + 16 * cp));

                    let q6_l0 = vsubq_s8(
                        vreinterpretq_s8_u8(vorrq_u8(
                            vandq_u8(q6_ql_0, m4b),
                            vshlq_n_u8(vandq_u8(q6_qh_0, mask_lo), 4),
                        )),
                        m32s,
                    );
                    let q6_l1 = vsubq_s8(
                        vreinterpretq_s8_u8(vorrq_u8(
                            vandq_u8(q6_ql_1, m4b),
                            vshlq_n_u8(vandq_u8(q6_qh_1, mask_lo), 4),
                        )),
                        m32s,
                    );
                    let q6_h0 = vsubq_s8(
                        vreinterpretq_s8_u8(vorrq_u8(
                            vshrq_n_u8(q6_ql_0, 4),
                            vandq_u8(q6_qh_0, mask_hi),
                        )),
                        m32s,
                    );
                    let q6_h1 = vsubq_s8(
                        vreinterpretq_s8_u8(vorrq_u8(
                            vshrq_n_u8(q6_ql_1, 4),
                            vandq_u8(q6_qh_1, mask_hi),
                        )),
                        m32s,
                    );

                    let mut sb_acc_0l = smmla_s32(vdupq_n_s32(0), q6_l0, q8_l_01[0]);
                    sb_acc_0l = smmla_s32(sb_acc_0l, q6_l1, q8_l_01[1]);
                    let mut sb_acc_0h = smmla_s32(vdupq_n_s32(0), q6_h0, q8_h_01[0]);
                    sb_acc_0h = smmla_s32(sb_acc_0h, q6_h1, q8_h_01[1]);
                    let mut sb_acc_1l = smmla_s32(vdupq_n_s32(0), q6_l0, q8_l_23[0]);
                    sb_acc_1l = smmla_s32(sb_acc_1l, q6_l1, q8_l_23[1]);
                    let mut sb_acc_1h = smmla_s32(vdupq_n_s32(0), q6_h0, q8_h_23[0]);
                    sb_acc_1h = smmla_s32(sb_acc_1h, q6_h1, q8_h_23[1]);

                    let scale_idx_l = half * 8 + sb;
                    let scale_idx_h = half * 8 + sb + 4;
                    let s0 = x.scales[scale_idx_l * 8 + cp * 2] as i32;
                    let s1 = x.scales[scale_idx_l * 8 + cp * 2 + 1] as i32;
                    let s2 = x.scales[scale_idx_h * 8 + cp * 2] as i32;
                    let s3 = x.scales[scale_idx_h * 8 + cp * 2 + 1] as i32;
                    let scale_vec_l = vcombine_s32(vdup_n_s32(s0), vdup_n_s32(s1));
                    let scale_vec_h = vcombine_s32(vdup_n_s32(s2), vdup_n_s32(s3));
                    acc[cp] = vmlaq_s32(acc[cp], sb_acc_0l, scale_vec_l);
                    acc[cp] = vmlaq_s32(acc[cp], sb_acc_0h, scale_vec_h);
                    acc[cp + 4] = vmlaq_s32(acc[cp + 4], sb_acc_1l, scale_vec_l);
                    acc[cp + 4] = vmlaq_s32(acc[cp + 4], sb_acc_1h, scale_vec_h);
                }
            }
        }

        for item in &mut acc {
            let aux = vzip_s32(vget_low_s32(*item), vget_high_s32(*item));
            *item = vcombine_s32(aux.0, aux.1);
        }
        let reorder_acc = [
            vcombine_s32(vget_low_s32(acc[0]), vget_low_s32(acc[1])),
            vcombine_s32(vget_low_s32(acc[2]), vget_low_s32(acc[3])),
            vcombine_s32(vget_high_s32(acc[0]), vget_high_s32(acc[1])),
            vcombine_s32(vget_high_s32(acc[2]), vget_high_s32(acc[3])),
            vcombine_s32(vget_low_s32(acc[4]), vget_low_s32(acc[5])),
            vcombine_s32(vget_low_s32(acc[6]), vget_low_s32(acc[7])),
            vcombine_s32(vget_high_s32(acc[4]), vget_high_s32(acc[5])),
            vcombine_s32(vget_high_s32(acc[6]), vget_high_s32(acc[7])),
        ];

        for r in 0..4 {
            for g in 0..2 {
                let q6_d = [
                    x.d[g * 4].to_f32(),
                    x.d[g * 4 + 1].to_f32(),
                    x.d[g * 4 + 2].to_f32(),
                    x.d[g * 4 + 3].to_f32(),
                ];
                let scale = vmulq_n_f32(vld1q_f32(q6_d.as_ptr()), y.d[r]);
                let idx = 2 * r + g;
                acc_f32[idx] = vmlaq_f32(acc_f32[idx], vcvtq_f32_s32(reorder_acc[idx]), scale);
            }
        }
    }

    for r in 0..4 {
        vst1q_f32(dst.add((row + r) * n + col), acc_f32[2 * r]);
        vst1q_f32(dst.add((row + r) * n + col + 4), acc_f32[2 * r + 1]);
    }
}

#[cfg(target_arch = "aarch64")]
#[inline(always)]
unsafe fn load_f16x4(ptr: *const f16) -> float32x4_t {
    let raw = vld1_u64(ptr as *const u64);
    let mut result: float32x4_t;
    core::arch::asm!(
        "fcvtl {out:v}.4s, {inp:v}.4h",
        inp = in(vreg) raw,
        out = out(vreg) result,
        options(nostack, nomem),
    );
    result
}

#[cfg(target_arch = "aarch64")]
#[inline(always)]
unsafe fn load_i32x2(a: i32, b: i32) -> int32x2_t {
    vld1_s32([a, b].as_ptr())
}

#[cfg(target_arch = "aarch64")]
#[inline(always)]
unsafe fn load_i32x4(a: i32, b: i32, c: i32, d: i32) -> int32x4_t {
    vld1q_s32([a, b, c, d].as_ptr())
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "i8mm")]
unsafe fn vec_dot_8_q6kx8_q8k_8x8(n: usize, xs: &[BlockQ6Kx8], ys: &[BlockQ8K]) -> [f32; 8] {
    debug_assert!(n.is_multiple_of(QK_K));
    let mut out = [0f32; 8];
    let m4b = vdupq_n_u8(0x0f);
    let mask_lo = vdupq_n_u8(0x03);
    let mask_hi = vdupq_n_u8(0x30);
    let mut acc_f32_0 = vdupq_n_f32(0.0);
    let mut acc_f32_1 = vdupq_n_f32(0.0);

    for (q6, q8) in xs.iter().zip(ys.iter()) {
        let sb_scale_0 = vmulq_f32(load_f16x4(q6.d.as_ptr()), vdupq_n_f32(q8.d));
        let sb_scale_1 = vmulq_f32(load_f16x4(q6.d.as_ptr().add(4)), vdupq_n_f32(q8.d));
        let mut acc = [vdup_n_s32(0); 4];
        let mut q6_scales = [0i16; 16 * 8];

        for i in 0..16 {
            let scales = vmovl_s8(vld1_s8(q6.scales.as_ptr().add(i * 8)));
            vst1q_s16(q6_scales.as_mut_ptr().add(i * 8), scales);
        }

        let mut bias = [0i32; 8];
        for i in 0..16 {
            let bsum = q8.bsums[i] as i32;
            for c in 0..8 {
                bias[c] += bsum * q6_scales[i * 8 + c] as i32;
            }
        }

        for half in 0..2 {
            let ql_base = q6.ql.as_ptr().add(half * 512);
            let qh_base = q6.qh.as_ptr().add(half * 256);
            for sb in 0..QK_K / 64 {
                let q8_base_l = q8.qs.as_ptr().add(half * 128 + sb * 16);
                let q8_base_h = q8_base_l.add(64);
                let q8_l = [
                    vreinterpretq_s8_s64(vld1q_dup_s64(q8_base_l as *const i64)),
                    vreinterpretq_s8_s64(vld1q_dup_s64(q8_base_l.add(8) as *const i64)),
                ];
                let q8_h = [
                    vreinterpretq_s8_s64(vld1q_dup_s64(q8_base_h as *const i64)),
                    vreinterpretq_s8_s64(vld1q_dup_s64(q8_base_h.add(8) as *const i64)),
                ];
                let ql_off_base = sb * QK_K / 2;
                let qh_off_base = ql_off_base & 255;
                let q6_ql_0 = vld1q_u8_x4(ql_base.add(ql_off_base));
                let q6_ql_1 = vld1q_u8_x4(ql_base.add(ql_off_base + 64));
                let q6_qh_0_raw = vld1q_u8_x4(qh_base.add(qh_off_base));
                let q6_qh_1_raw = vld1q_u8_x4(qh_base.add(qh_off_base + 64));
                let q6_qh_0 = if sb > 1 {
                    (
                        vshrq_n_u8(q6_qh_0_raw.0, 2),
                        vshrq_n_u8(q6_qh_0_raw.1, 2),
                        vshrq_n_u8(q6_qh_0_raw.2, 2),
                        vshrq_n_u8(q6_qh_0_raw.3, 2),
                    )
                } else {
                    (q6_qh_0_raw.0, q6_qh_0_raw.1, q6_qh_0_raw.2, q6_qh_0_raw.3)
                };
                let q6_qh_1 = if sb > 1 {
                    (
                        vshrq_n_u8(q6_qh_1_raw.0, 2),
                        vshrq_n_u8(q6_qh_1_raw.1, 2),
                        vshrq_n_u8(q6_qh_1_raw.2, 2),
                        vshrq_n_u8(q6_qh_1_raw.3, 2),
                    )
                } else {
                    (q6_qh_1_raw.0, q6_qh_1_raw.1, q6_qh_1_raw.2, q6_qh_1_raw.3)
                };
                let q6_ql = [
                    q6_ql_0.0, q6_ql_0.1, q6_ql_0.2, q6_ql_0.3, q6_ql_1.0, q6_ql_1.1, q6_ql_1.2,
                    q6_ql_1.3,
                ];
                let q6_qh = [
                    q6_qh_0.0, q6_qh_0.1, q6_qh_0.2, q6_qh_0.3, q6_qh_1.0, q6_qh_1.1, q6_qh_1.2,
                    q6_qh_1.3,
                ];

                for cp in 0..4 {
                    let q6_l0 = vreinterpretq_s8_u8(vorrq_u8(
                        vandq_u8(q6_ql[cp], m4b),
                        vshlq_n_u8(vandq_u8(q6_qh[cp], mask_lo), 4),
                    ));
                    let q6_l1 = vreinterpretq_s8_u8(vorrq_u8(
                        vandq_u8(q6_ql[cp + 4], m4b),
                        vshlq_n_u8(vandq_u8(q6_qh[cp + 4], mask_lo), 4),
                    ));
                    let q6_h0 = vreinterpretq_s8_u8(vorrq_u8(
                        vshrq_n_u8(q6_ql[cp], 4),
                        vandq_u8(q6_qh[cp], mask_hi),
                    ));
                    let q6_h1 = vreinterpretq_s8_u8(vorrq_u8(
                        vshrq_n_u8(q6_ql[cp + 4], 4),
                        vandq_u8(q6_qh[cp + 4], mask_hi),
                    ));
                    let mut sb_acc_l = vdupq_n_s32(0);
                    sb_acc_l = sdot_acc(sb_acc_l, q6_l0, q8_l[0]);
                    sb_acc_l = sdot_acc(sb_acc_l, q6_l1, q8_l[1]);
                    let mut sb_acc_h = vdupq_n_s32(0);
                    sb_acc_h = sdot_acc(sb_acc_h, q6_h0, q8_h[0]);
                    sb_acc_h = sdot_acc(sb_acc_h, q6_h1, q8_h[1]);
                    let sum_l = vpadd_s32(vget_low_s32(sb_acc_l), vget_high_s32(sb_acc_l));
                    let sum_h = vpadd_s32(vget_low_s32(sb_acc_h), vget_high_s32(sb_acc_h));
                    let scale_idx_l = half * 8 + sb;
                    let scale_idx_h = half * 8 + sb + 4;
                    let scale_l = load_i32x2(
                        q6_scales[scale_idx_l * 8 + cp * 2] as i32,
                        q6_scales[scale_idx_l * 8 + cp * 2 + 1] as i32,
                    );
                    let scale_h = load_i32x2(
                        q6_scales[scale_idx_h * 8 + cp * 2] as i32,
                        q6_scales[scale_idx_h * 8 + cp * 2 + 1] as i32,
                    );
                    acc[cp] = vmla_s32(acc[cp], sum_l, scale_l);
                    acc[cp] = vmla_s32(acc[cp], sum_h, scale_h);
                }
            }
        }

        acc[0] = vsub_s32(acc[0], load_i32x2(bias[0] << 5, bias[1] << 5));
        acc[1] = vsub_s32(acc[1], load_i32x2(bias[2] << 5, bias[3] << 5));
        acc[2] = vsub_s32(acc[2], load_i32x2(bias[4] << 5, bias[5] << 5));
        acc[3] = vsub_s32(acc[3], load_i32x2(bias[6] << 5, bias[7] << 5));
        let w_01 = vmul_f32(vcvt_f32_s32(acc[0]), vget_low_f32(sb_scale_0));
        let w_23 = vmul_f32(vcvt_f32_s32(acc[1]), vget_high_f32(sb_scale_0));
        let w_45 = vmul_f32(vcvt_f32_s32(acc[2]), vget_low_f32(sb_scale_1));
        let w_67 = vmul_f32(vcvt_f32_s32(acc[3]), vget_high_f32(sb_scale_1));
        acc_f32_0 = vaddq_f32(acc_f32_0, vcombine_f32(w_01, w_23));
        acc_f32_1 = vaddq_f32(acc_f32_1, vcombine_f32(w_45, w_67));
    }

    vst1q_f32(out.as_mut_ptr(), acc_f32_0);
    vst1q_f32(out.as_mut_ptr().add(4), acc_f32_1);
    out
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "dotprod")]
unsafe fn vec_dot_8_q6kx8_q8k_8x4(n: usize, xs: &[BlockQ6Kx8], ys: &[BlockQ8K]) -> [f32; 8] {
    debug_assert!(n.is_multiple_of(QK_K));
    let mut out = [0f32; 8];
    let m4b = vdupq_n_u8(0x0f);
    let mask_lo = vdupq_n_u8(0x03);
    let mask_hi = vdupq_n_u8(0x30);
    let mut acc_f32_0 = vdupq_n_f32(0.0);
    let mut acc_f32_1 = vdupq_n_f32(0.0);

    for (q6, q8) in xs.iter().zip(ys.iter()) {
        let sb_scale_0 = vmulq_f32(load_f16x4(q6.d.as_ptr()), vdupq_n_f32(q8.d));
        let sb_scale_1 = vmulq_f32(load_f16x4(q6.d.as_ptr().add(4)), vdupq_n_f32(q8.d));
        let mut acc = [vdupq_n_s32(0); 2];
        let mut q6_scales = [0i16; 16 * 8];

        for i in 0..16 {
            let scales = vmovl_s8(vld1_s8(q6.scales.as_ptr().add(i * 8)));
            vst1q_s16(q6_scales.as_mut_ptr().add(i * 8), scales);
        }

        let mut bias = [0i32; 8];
        for i in 0..16 {
            let bsum = q8.bsums[i] as i32;
            for c in 0..8 {
                bias[c] += bsum * q6_scales[i * 8 + c] as i32;
            }
        }

        for half in 0..2 {
            let ql_base = q6.ql.as_ptr().add(half * 512);
            let qh_base = q6.qh.as_ptr().add(half * 256);
            for sb in 0..QK_K / 64 {
                let q8_base_l = q8.qs.as_ptr().add(half * 128 + sb * 16);
                let q8_base_h = q8_base_l.add(64);
                let q8_l = [
                    vreinterpretq_s8_s32(vld1q_dup_s32(q8_base_l as *const i32)),
                    vreinterpretq_s8_s32(vld1q_dup_s32(q8_base_l.add(4) as *const i32)),
                    vreinterpretq_s8_s32(vld1q_dup_s32(q8_base_l.add(8) as *const i32)),
                    vreinterpretq_s8_s32(vld1q_dup_s32(q8_base_l.add(12) as *const i32)),
                ];
                let q8_h = [
                    vreinterpretq_s8_s32(vld1q_dup_s32(q8_base_h as *const i32)),
                    vreinterpretq_s8_s32(vld1q_dup_s32(q8_base_h.add(4) as *const i32)),
                    vreinterpretq_s8_s32(vld1q_dup_s32(q8_base_h.add(8) as *const i32)),
                    vreinterpretq_s8_s32(vld1q_dup_s32(q8_base_h.add(12) as *const i32)),
                ];
                let ql_off_base = sb * QK_K / 2;
                let qh_off_base = ql_off_base & 255;
                let q6_ql_0 = vld1q_u8_x4(ql_base.add(ql_off_base));
                let q6_ql_1 = vld1q_u8_x4(ql_base.add(ql_off_base + 64));
                let q6_qh_0_raw = vld1q_u8_x4(qh_base.add(qh_off_base));
                let q6_qh_1_raw = vld1q_u8_x4(qh_base.add(qh_off_base + 64));
                let q6_qh_0 = if sb > 1 {
                    (
                        vshrq_n_u8(q6_qh_0_raw.0, 2),
                        vshrq_n_u8(q6_qh_0_raw.1, 2),
                        vshrq_n_u8(q6_qh_0_raw.2, 2),
                        vshrq_n_u8(q6_qh_0_raw.3, 2),
                    )
                } else {
                    (q6_qh_0_raw.0, q6_qh_0_raw.1, q6_qh_0_raw.2, q6_qh_0_raw.3)
                };
                let q6_qh_1 = if sb > 1 {
                    (
                        vshrq_n_u8(q6_qh_1_raw.0, 2),
                        vshrq_n_u8(q6_qh_1_raw.1, 2),
                        vshrq_n_u8(q6_qh_1_raw.2, 2),
                        vshrq_n_u8(q6_qh_1_raw.3, 2),
                    )
                } else {
                    (q6_qh_1_raw.0, q6_qh_1_raw.1, q6_qh_1_raw.2, q6_qh_1_raw.3)
                };
                let q6_ql = [
                    q6_ql_0.0, q6_ql_0.1, q6_ql_0.2, q6_ql_0.3, q6_ql_1.0, q6_ql_1.1, q6_ql_1.2,
                    q6_ql_1.3,
                ];
                let q6_qh = [
                    q6_qh_0.0, q6_qh_0.1, q6_qh_0.2, q6_qh_0.3, q6_qh_1.0, q6_qh_1.1, q6_qh_1.2,
                    q6_qh_1.3,
                ];

                for g in 0..2 {
                    let mut sb_acc_l = vdupq_n_s32(0);
                    let mut sb_acc_h = vdupq_n_s32(0);
                    for chunk in 0..4 {
                        let idx = chunk * 2 + g;
                        let q6_l = vreinterpretq_s8_u8(vorrq_u8(
                            vandq_u8(q6_ql[idx], m4b),
                            vshlq_n_u8(vandq_u8(q6_qh[idx], mask_lo), 4),
                        ));
                        let q6_h = vreinterpretq_s8_u8(vorrq_u8(
                            vshrq_n_u8(q6_ql[idx], 4),
                            vandq_u8(q6_qh[idx], mask_hi),
                        ));
                        sb_acc_l = sdot_acc(sb_acc_l, q6_l, q8_l[chunk]);
                        sb_acc_h = sdot_acc(sb_acc_h, q6_h, q8_h[chunk]);
                    }

                    let scale_idx_l = half * 8 + sb;
                    let scale_idx_h = half * 8 + sb + 4;
                    let scale_l = load_i32x4(
                        q6_scales[scale_idx_l * 8 + g * 4] as i32,
                        q6_scales[scale_idx_l * 8 + g * 4 + 1] as i32,
                        q6_scales[scale_idx_l * 8 + g * 4 + 2] as i32,
                        q6_scales[scale_idx_l * 8 + g * 4 + 3] as i32,
                    );
                    let scale_h = load_i32x4(
                        q6_scales[scale_idx_h * 8 + g * 4] as i32,
                        q6_scales[scale_idx_h * 8 + g * 4 + 1] as i32,
                        q6_scales[scale_idx_h * 8 + g * 4 + 2] as i32,
                        q6_scales[scale_idx_h * 8 + g * 4 + 3] as i32,
                    );
                    acc[g] = vmlaq_s32(acc[g], sb_acc_l, scale_l);
                    acc[g] = vmlaq_s32(acc[g], sb_acc_h, scale_h);
                }
            }
        }

        acc[0] = vsubq_s32(
            acc[0],
            load_i32x4(bias[0] << 5, bias[1] << 5, bias[2] << 5, bias[3] << 5),
        );
        acc[1] = vsubq_s32(
            acc[1],
            load_i32x4(bias[4] << 5, bias[5] << 5, bias[6] << 5, bias[7] << 5),
        );
        acc_f32_0 = vfmaq_f32(acc_f32_0, vcvtq_f32_s32(acc[0]), sb_scale_0);
        acc_f32_1 = vfmaq_f32(acc_f32_1, vcvtq_f32_s32(acc[1]), sb_scale_1);
    }

    vst1q_f32(out.as_mut_ptr(), acc_f32_0);
    vst1q_f32(out.as_mut_ptr().add(4), acc_f32_1);
    out
}

#[cfg(target_arch = "aarch64")]
#[inline(always)]
pub(crate) fn vec_dot_8_q6kx8_q8k(n: usize, xs: &[BlockQ6Kx8], ys: &[BlockQ8K]) -> [f32; 8] {
    unsafe {
        if crate::cpu::features::get().i8mm {
            vec_dot_8_q6kx8_q8k_8x8(n, xs, ys)
        } else {
            vec_dot_8_q6kx8_q8k_8x4(n, xs, ys)
        }
    }
}

#[cfg(target_arch = "aarch64")]
fn matmul_q6k_x8_gemv(
    (m, k, n): (usize, usize, usize),
    lhs: &[f32],
    repacked: &[BlockQ6Kx8],
    dst: &mut [f32],
) -> crate::Result<()> {
    debug_assert_eq!(m, 1);
    debug_assert!(k.is_multiple_of(QK_K));
    debug_assert!(n.is_multiple_of(8));
    let k_in_blocks = k / QK_K;
    let n_groups = n / 8;

    thread_local! {
        static LHS_Q6K_GEMV_SCRATCH: std::cell::RefCell<Vec<u64>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }

    let elem_size = std::mem::size_of::<BlockQ8K>();
    let required_len = (k_in_blocks * elem_size).div_ceil(8);
    LHS_Q6K_GEMV_SCRATCH.with(|cell| -> crate::Result<()> {
        let mut scratch = cell.borrow_mut();
        if scratch.len() < required_len {
            scratch.resize(required_len, 0);
        }
        let lhs_b: &mut [BlockQ8K] = unsafe {
            std::slice::from_raw_parts_mut(scratch.as_mut_ptr() as *mut BlockQ8K, k_in_blocks)
        };
        <BlockQ8K as super::GgmlType>::from_float(lhs, lhs_b);

        let pool = crate::utils::barrier_pool();
        let lhs_ptr = lhs_b.as_ptr() as usize;
        let repacked_ptr = repacked.as_ptr() as usize;
        let dst_ptr = dst.as_mut_ptr() as usize;
        let x8_block_bytes = std::mem::size_of::<BlockQ6Kx8>();

        // equal-sized gemv units: a static split skips the shared cursor, which costs
        // several us per small decode matmul
        pool.execute_static(n_groups, |range| {
            let lhs_row =
                unsafe { std::slice::from_raw_parts(lhs_ptr as *const BlockQ8K, k_in_blocks) };
            let dst_ptr = dst_ptr as *mut f32;
            for g in range {
                let xs = unsafe {
                    std::slice::from_raw_parts(
                        (repacked_ptr + g * k_in_blocks * x8_block_bytes) as *const BlockQ6Kx8,
                        k_in_blocks,
                    )
                };
                let results = vec_dot_8_q6kx8_q8k(k, xs, lhs_row);
                unsafe {
                    std::ptr::copy_nonoverlapping(results.as_ptr(), dst_ptr.add(g * 8), 8);
                }
            }
        });

        Ok(())
    })
}

#[cfg(target_arch = "aarch64")]
pub(crate) fn matmul_q6k_x8(
    (m, k, n): (usize, usize, usize),
    lhs: &[f32],
    repacked: &[BlockQ6Kx8],
    dst: &mut [f32],
) -> crate::Result<()> {
    debug_assert!(k.is_multiple_of(QK_K));
    debug_assert!(n.is_multiple_of(8));
    let features = crate::cpu::features::get();
    if features.dotprod && m == 1 {
        return matmul_q6k_x8_gemv((m, k, n), lhs, repacked, dst);
    }
    let use_i8mm = features.i8mm;
    debug_assert!(m.is_multiple_of(4));
    let k_in_blocks = k / QK_K;
    let n_groups = n / 8;
    let row_groups = m / 4;

    thread_local! {
        static LHS_Q6K_X8_SCRATCH: std::cell::RefCell<Vec<u64>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }

    let elem_size = std::mem::size_of::<BlockQ8Kx4>();
    let required_len = (row_groups * k_in_blocks * elem_size).div_ceil(8);
    LHS_Q6K_X8_SCRATCH.with(|cell| -> crate::Result<()> {
        let mut scratch = cell.borrow_mut();
        if scratch.len() < required_len {
            scratch.resize(required_len, 0);
        }
        let lhs_x4: &mut [BlockQ8Kx4] = unsafe {
            std::slice::from_raw_parts_mut(
                scratch.as_mut_ptr() as *mut BlockQ8Kx4,
                row_groups * k_in_blocks,
            )
        };
        for group in 0..row_groups {
            if use_i8mm {
                quantize_q8k_x4_interleaved::<8>(
                    lhs,
                    k,
                    group * 4,
                    &mut lhs_x4[group * k_in_blocks..(group + 1) * k_in_blocks],
                );
            } else {
                quantize_q8k_x4_interleaved::<4>(
                    lhs,
                    k,
                    group * 4,
                    &mut lhs_x4[group * k_in_blocks..(group + 1) * k_in_blocks],
                );
            }
        }

        let tiles_total = row_groups * n_groups;
        let pool = crate::utils::barrier_pool();
        let lhs_ptr = lhs_x4.as_ptr() as usize;
        let repacked_ptr = repacked.as_ptr() as usize;
        let dst_ptr = dst.as_mut_ptr() as usize;

        pool.execute_chunked(tiles_total, |range| {
            let lhs_ptr = lhs_ptr as *const BlockQ8Kx4;
            let repacked_ptr = repacked_ptr as *const BlockQ6Kx8;
            let dst_ptr = dst_ptr as *mut f32;
            for tile in range {
                let row_group = tile / n_groups;
                let col_group = tile - row_group * n_groups;
                let row = row_group * 4;
                let col = col_group * 8;
                let lhs_tile = unsafe {
                    std::slice::from_raw_parts(lhs_ptr.add(row_group * k_in_blocks), k_in_blocks)
                };
                let rhs_tile = unsafe {
                    std::slice::from_raw_parts(
                        repacked_ptr.add(col_group * k_in_blocks),
                        k_in_blocks,
                    )
                };
                if use_i8mm {
                    unsafe {
                        store_q6kx8_4x8_i8mm(dst_ptr, n, row, col, lhs_tile, rhs_tile, k_in_blocks)
                    };
                } else {
                    unsafe {
                        store_q6kx8_4x8(dst_ptr, n, row, col, lhs_tile, rhs_tile, k_in_blocks)
                    };
                }
            }
        });

        Ok(())
    })
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "dotprod")]
unsafe fn store_q8_0x4_4x4(
    dst: *mut f32,
    n: usize,
    row: usize,
    col: usize,
    lhs: &[BlockQ8_0x4],
    rhs: &[BlockQ8_0x4],
    k_in_blocks: usize,
) {
    let mut sum0 = vdupq_n_f32(0.);
    let mut sum1 = vdupq_n_f32(0.);
    let mut sum2 = vdupq_n_f32(0.);
    let mut sum3 = vdupq_n_f32(0.);

    for i in 0..k_in_blocks {
        let x = &rhs[i];
        let y = &lhs[i];
        let bd = [
            x.d[0].to_f32(),
            x.d[1].to_f32(),
            x.d[2].to_f32(),
            x.d[3].to_f32(),
        ];
        let bd = vld1q_f32(bd.as_ptr());
        let ad = [
            y.d[0].to_f32(),
            y.d[1].to_f32(),
            y.d[2].to_f32(),
            y.d[3].to_f32(),
        ];

        let mut ret0 = vdupq_n_s32(0);
        let mut ret1 = vdupq_n_s32(0);
        let mut ret2 = vdupq_n_s32(0);
        let mut ret3 = vdupq_n_s32(0);

        macro_rules! dot_rows {
            ($xv:expr, $yv:expr) => {{
                ret0 = sdot_laneq_s32::<0>(ret0, $xv, $yv);
                ret1 = sdot_laneq_s32::<1>(ret1, $xv, $yv);
                ret2 = sdot_laneq_s32::<2>(ret2, $xv, $yv);
                ret3 = sdot_laneq_s32::<3>(ret3, $xv, $yv);
            }};
        }

        let y0 = vld1q_s8_x4(y.qs.as_ptr());
        let x0 = vld1q_s8_x4(x.qs.as_ptr());
        dot_rows!(x0.0, y0.0);
        dot_rows!(x0.1, y0.1);
        dot_rows!(x0.2, y0.2);
        dot_rows!(x0.3, y0.3);

        let y1 = vld1q_s8_x4(y.qs.as_ptr().add(64));
        let x1 = vld1q_s8_x4(x.qs.as_ptr().add(64));
        dot_rows!(x1.0, y1.0);
        dot_rows!(x1.1, y1.1);
        dot_rows!(x1.2, y1.2);
        dot_rows!(x1.3, y1.3);

        sum0 = vmlaq_f32(sum0, vcvtq_f32_s32(ret0), vmulq_n_f32(bd, ad[0]));
        sum1 = vmlaq_f32(sum1, vcvtq_f32_s32(ret1), vmulq_n_f32(bd, ad[1]));
        sum2 = vmlaq_f32(sum2, vcvtq_f32_s32(ret2), vmulq_n_f32(bd, ad[2]));
        sum3 = vmlaq_f32(sum3, vcvtq_f32_s32(ret3), vmulq_n_f32(bd, ad[3]));
    }

    vst1q_f32(dst.add(row * n + col), sum0);
    vst1q_f32(dst.add((row + 1) * n + col), sum1);
    vst1q_f32(dst.add((row + 2) * n + col), sum2);
    vst1q_f32(dst.add((row + 3) * n + col), sum3);
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "dotprod")]
unsafe fn vec_dot_4_q8_0x4_q8_0_4x4(n: usize, xs: &[BlockQ8_0x4], ys: &[BlockQ8_0]) -> [f32; 4] {
    debug_assert!(n.is_multiple_of(QK8_0));
    let mut out = [0f32; 4];
    let mut acc = vdupq_n_f32(0.);

    for (x, y) in xs.iter().zip(ys.iter()) {
        let bd = load_f16x4(x.d.as_ptr());
        let ad = vdupq_n_f32(y.d.to_f32());
        let y0 = vld1q_s8(y.qs.as_ptr());
        let y1 = vld1q_s8(y.qs.as_ptr().add(16));
        let x0 = vld1q_s8_x4(x.qs.as_ptr());
        let x1 = vld1q_s8_x4(x.qs.as_ptr().add(64));
        let mut ret = vdupq_n_s32(0);

        ret = sdot_laneq_s32::<0>(ret, x0.0, y0);
        ret = sdot_laneq_s32::<1>(ret, x0.1, y0);
        ret = sdot_laneq_s32::<2>(ret, x0.2, y0);
        ret = sdot_laneq_s32::<3>(ret, x0.3, y0);
        ret = sdot_laneq_s32::<0>(ret, x1.0, y1);
        ret = sdot_laneq_s32::<1>(ret, x1.1, y1);
        ret = sdot_laneq_s32::<2>(ret, x1.2, y1);
        ret = sdot_laneq_s32::<3>(ret, x1.3, y1);

        acc = vmlaq_f32(acc, vcvtq_f32_s32(ret), vmulq_f32(ad, bd));
    }

    vst1q_f32(out.as_mut_ptr(), acc);
    out
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "dotprod")]
unsafe fn vec_dot_4_q8_0x4_q8_0_4x8(n: usize, xs: &[BlockQ8_0x4], ys: &[BlockQ8_0]) -> [f32; 4] {
    debug_assert!(n.is_multiple_of(QK8_0));
    let mut out = [0f32; 4];
    let mut acc = vdupq_n_f32(0.);

    for (x, y) in xs.iter().zip(ys.iter()) {
        let bd = load_f16x4(x.d.as_ptr());
        let ad = vdupq_n_f32(y.d.to_f32());
        let a0 = vreinterpretq_s8_s64(vld1q_dup_s64(y.qs.as_ptr() as *const i64));
        let a1 = vreinterpretq_s8_s64(vld1q_dup_s64(y.qs.as_ptr().add(8) as *const i64));
        let a2 = vreinterpretq_s8_s64(vld1q_dup_s64(y.qs.as_ptr().add(16) as *const i64));
        let a3 = vreinterpretq_s8_s64(vld1q_dup_s64(y.qs.as_ptr().add(24) as *const i64));
        let x0 = vld1q_s8_x4(x.qs.as_ptr());
        let x1 = vld1q_s8_x4(x.qs.as_ptr().add(64));
        let mut ret0 = vdupq_n_s32(0);
        let mut ret1 = vdupq_n_s32(0);

        ret0 = sdot_acc(ret0, x0.0, a0);
        ret1 = sdot_acc(ret1, x0.1, a0);
        ret0 = sdot_acc(ret0, x0.2, a1);
        ret1 = sdot_acc(ret1, x0.3, a1);
        ret0 = sdot_acc(ret0, x1.0, a2);
        ret1 = sdot_acc(ret1, x1.1, a2);
        ret0 = sdot_acc(ret0, x1.2, a3);
        ret1 = sdot_acc(ret1, x1.3, a3);

        let ret = vpaddq_s32(ret0, ret1);
        acc = vmlaq_f32(acc, vcvtq_f32_s32(ret), vmulq_f32(ad, bd));
    }

    vst1q_f32(out.as_mut_ptr(), acc);
    out
}

#[cfg(target_arch = "aarch64")]
#[inline(always)]
pub(crate) fn vec_dot_4_q8_0x4_q8_0(n: usize, xs: &[BlockQ8_0x4], ys: &[BlockQ8_0]) -> [f32; 4] {
    unsafe {
        if crate::cpu::features::get().i8mm {
            vec_dot_4_q8_0x4_q8_0_4x8(n, xs, ys)
        } else {
            vec_dot_4_q8_0x4_q8_0_4x4(n, xs, ys)
        }
    }
}

#[cfg(target_arch = "aarch64")]
fn matmul_q8_0_x4_gemv(
    (_m, k, n): (usize, usize, usize),
    lhs: &[f32],
    repacked: &[BlockQ8_0x4],
    dst: &mut [f32],
) -> crate::Result<()> {
    debug_assert!(k.is_multiple_of(QK8_0));
    debug_assert!(n.is_multiple_of(4));
    let k_in_blocks = k / QK8_0;
    let n_groups = n / 4;

    thread_local! {
        static LHS_Q8_0_GEMV_SCRATCH: std::cell::RefCell<Vec<u64>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }

    let elem_size = std::mem::size_of::<BlockQ8_0>();
    let required_len = (k_in_blocks * elem_size).div_ceil(8);
    LHS_Q8_0_GEMV_SCRATCH.with(|cell| -> crate::Result<()> {
        let mut scratch = cell.borrow_mut();
        if scratch.len() < required_len {
            scratch.resize(required_len, 0);
        }
        let lhs_b: &mut [BlockQ8_0] = unsafe {
            std::slice::from_raw_parts_mut(scratch.as_mut_ptr() as *mut BlockQ8_0, k_in_blocks)
        };
        <BlockQ8_0 as super::GgmlType>::from_float(lhs, lhs_b);

        let pool = crate::utils::barrier_pool();
        let lhs_ptr = lhs_b.as_ptr() as usize;
        let repacked_ptr = repacked.as_ptr() as usize;
        let dst_ptr = dst.as_mut_ptr() as usize;
        let x4_block_bytes = std::mem::size_of::<BlockQ8_0x4>();

        // equal-sized gemv units: a static split skips the shared cursor, which costs
        // several us per small decode matmul
        pool.execute_static(n_groups, |range| {
            let lhs_row =
                unsafe { std::slice::from_raw_parts(lhs_ptr as *const BlockQ8_0, k_in_blocks) };
            let dst_ptr = dst_ptr as *mut f32;
            for g in range {
                let xs = unsafe {
                    std::slice::from_raw_parts(
                        (repacked_ptr + g * k_in_blocks * x4_block_bytes) as *const BlockQ8_0x4,
                        k_in_blocks,
                    )
                };
                let results = vec_dot_4_q8_0x4_q8_0(k, xs, lhs_row);
                unsafe {
                    std::ptr::copy_nonoverlapping(results.as_ptr(), dst_ptr.add(g * 4), 4);
                }
            }
        });

        Ok(())
    })
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "dotprod")]
pub(crate) fn matmul_q8_0_x4(
    (m, k, n): (usize, usize, usize),
    lhs: &[f32],
    repacked: &[BlockQ8_0x4],
    dst: &mut [f32],
) -> crate::Result<()> {
    debug_assert!(k.is_multiple_of(QK8_0));
    debug_assert!(n.is_multiple_of(4));
    if m == 1 {
        return matmul_q8_0_x4_gemv((m, k, n), lhs, repacked, dst);
    }
    debug_assert!(m.is_multiple_of(4));
    let k_in_blocks = k / QK8_0;
    let n_groups = n / 4;
    let row_groups = m / 4;

    thread_local! {
        static LHS_X4_SCRATCH: std::cell::RefCell<Vec<u64>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }

    let elem_size = std::mem::size_of::<BlockQ8_0x4>();
    let required_len = (row_groups * k_in_blocks * elem_size).div_ceil(8);
    LHS_X4_SCRATCH.with(|cell| -> crate::Result<()> {
        let mut scratch = cell.borrow_mut();
        if scratch.len() < required_len {
            scratch.resize(required_len, 0);
        }
        let lhs_x4: &mut [BlockQ8_0x4] = unsafe {
            std::slice::from_raw_parts_mut(
                scratch.as_mut_ptr() as *mut BlockQ8_0x4,
                row_groups * k_in_blocks,
            )
        };
        for group in 0..row_groups {
            quantize_q8_0x4_interleaved::<4>(
                lhs,
                k,
                group * 4,
                &mut lhs_x4[group * k_in_blocks..(group + 1) * k_in_blocks],
            );
        }

        let tiles_total = row_groups * n_groups;
        let pool = crate::utils::barrier_pool();
        let lhs_ptr = lhs_x4.as_ptr() as usize;
        let repacked_ptr = repacked.as_ptr() as usize;
        let dst_ptr = dst.as_mut_ptr() as usize;

        pool.execute_chunked(tiles_total, |range| {
            let lhs_ptr = lhs_ptr as *const BlockQ8_0x4;
            let repacked_ptr = repacked_ptr as *const BlockQ8_0x4;
            let dst_ptr = dst_ptr as *mut f32;
            for tile in range {
                let row_group = tile / n_groups;
                let col_group = tile - row_group * n_groups;
                let row = row_group * 4;
                let col = col_group * 4;
                let lhs_tile = unsafe {
                    std::slice::from_raw_parts(lhs_ptr.add(row_group * k_in_blocks), k_in_blocks)
                };
                let rhs_tile = unsafe {
                    std::slice::from_raw_parts(
                        repacked_ptr.add(col_group * k_in_blocks),
                        k_in_blocks,
                    )
                };
                unsafe { store_q8_0x4_4x4(dst_ptr, n, row, col, lhs_tile, rhs_tile, k_in_blocks) };
            }
        });

        Ok(())
    })
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "i8mm")]
unsafe fn store_q8_0x4_4x4_i8mm(
    dst: *mut f32,
    n: usize,
    row: usize,
    col: usize,
    lhs: &[BlockQ8_0x4],
    rhs: &[BlockQ8_0x4],
    k_in_blocks: usize,
) {
    let mut sum0 = vdupq_n_f32(0.);
    let mut sum1 = vdupq_n_f32(0.);
    let mut sum2 = vdupq_n_f32(0.);
    let mut sum3 = vdupq_n_f32(0.);

    for i in 0..k_in_blocks {
        let x = &rhs[i];
        let y = &lhs[i];
        let bd = [
            x.d[0].to_f32(),
            x.d[1].to_f32(),
            x.d[2].to_f32(),
            x.d[3].to_f32(),
        ];
        let bd = vld1q_f32(bd.as_ptr());
        let ad = [
            y.d[0].to_f32(),
            y.d[1].to_f32(),
            y.d[2].to_f32(),
            y.d[3].to_f32(),
        ];

        let mut acc00 = vdupq_n_s32(0);
        let mut acc01 = vdupq_n_s32(0);
        let mut acc10 = vdupq_n_s32(0);
        let mut acc11 = vdupq_n_s32(0);

        for chunk in 0..4 {
            let offset = chunk * 32;
            let a01 = vld1q_s8(y.qs.as_ptr().add(offset));
            let a23 = vld1q_s8(y.qs.as_ptr().add(offset + 16));
            let b01 = vld1q_s8(x.qs.as_ptr().add(offset));
            let b23 = vld1q_s8(x.qs.as_ptr().add(offset + 16));

            acc00 = smmla_s32(acc00, a01, b01);
            acc01 = smmla_s32(acc01, a01, b23);
            acc10 = smmla_s32(acc10, a23, b01);
            acc11 = smmla_s32(acc11, a23, b23);
        }

        let row0 = vcombine_s32(vget_low_s32(acc00), vget_low_s32(acc01));
        let row1 = vcombine_s32(vget_high_s32(acc00), vget_high_s32(acc01));
        let row2 = vcombine_s32(vget_low_s32(acc10), vget_low_s32(acc11));
        let row3 = vcombine_s32(vget_high_s32(acc10), vget_high_s32(acc11));

        sum0 = vmlaq_f32(sum0, vcvtq_f32_s32(row0), vmulq_n_f32(bd, ad[0]));
        sum1 = vmlaq_f32(sum1, vcvtq_f32_s32(row1), vmulq_n_f32(bd, ad[1]));
        sum2 = vmlaq_f32(sum2, vcvtq_f32_s32(row2), vmulq_n_f32(bd, ad[2]));
        sum3 = vmlaq_f32(sum3, vcvtq_f32_s32(row3), vmulq_n_f32(bd, ad[3]));
    }

    vst1q_f32(dst.add(row * n + col), sum0);
    vst1q_f32(dst.add((row + 1) * n + col), sum1);
    vst1q_f32(dst.add((row + 2) * n + col), sum2);
    vst1q_f32(dst.add((row + 3) * n + col), sum3);
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "i8mm")]
pub(crate) fn matmul_q8_0_x4_i8mm(
    (m, k, n): (usize, usize, usize),
    lhs: &[f32],
    repacked: &[BlockQ8_0x4],
    dst: &mut [f32],
) -> crate::Result<()> {
    debug_assert!(k.is_multiple_of(QK8_0));
    debug_assert!(n.is_multiple_of(4));
    debug_assert!(m.is_multiple_of(4));
    let k_in_blocks = k / QK8_0;
    let n_groups = n / 4;
    let row_groups = m / 4;

    thread_local! {
        static LHS_X4_I8MM_SCRATCH: std::cell::RefCell<Vec<u64>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }

    let elem_size = std::mem::size_of::<BlockQ8_0x4>();
    let required_len = (row_groups * k_in_blocks * elem_size).div_ceil(8);
    LHS_X4_I8MM_SCRATCH.with(|cell| -> crate::Result<()> {
        let mut scratch = cell.borrow_mut();
        if scratch.len() < required_len {
            scratch.resize(required_len, 0);
        }
        let lhs_x4: &mut [BlockQ8_0x4] = unsafe {
            std::slice::from_raw_parts_mut(
                scratch.as_mut_ptr() as *mut BlockQ8_0x4,
                row_groups * k_in_blocks,
            )
        };
        for group in 0..row_groups {
            quantize_q8_0x4_interleaved::<8>(
                lhs,
                k,
                group * 4,
                &mut lhs_x4[group * k_in_blocks..(group + 1) * k_in_blocks],
            );
        }

        let tiles_total = row_groups * n_groups;
        let pool = crate::utils::barrier_pool();
        let lhs_ptr = lhs_x4.as_ptr() as usize;
        let repacked_ptr = repacked.as_ptr() as usize;
        let dst_ptr = dst.as_mut_ptr() as usize;
        pool.execute_chunked(tiles_total, |range| {
            let lhs_ptr = lhs_ptr as *const BlockQ8_0x4;
            let repacked_ptr = repacked_ptr as *const BlockQ8_0x4;
            let dst_ptr = dst_ptr as *mut f32;
            for tile in range {
                let row_group = tile / n_groups;
                let col_group = tile - row_group * n_groups;
                let row = row_group * 4;
                let col = col_group * 4;
                let lhs_tile = unsafe {
                    std::slice::from_raw_parts(lhs_ptr.add(row_group * k_in_blocks), k_in_blocks)
                };
                let rhs_tile = unsafe {
                    std::slice::from_raw_parts(
                        repacked_ptr.add(col_group * k_in_blocks),
                        k_in_blocks,
                    )
                };
                unsafe {
                    store_q8_0x4_4x4_i8mm(dst_ptr, n, row, col, lhs_tile, rhs_tile, k_in_blocks)
                }
            }
        });
        Ok(())
    })
}

#[inline(always)]
pub(crate) fn vec_dot_q8k_q8k(n: usize, xs: &[BlockQ8K], ys: &[BlockQ8K]) -> f32 {
    debug_assert!(
        n.is_multiple_of(QK_K),
        "vec_dot_q8k_q8k: {n} is not divisible by {QK_K}"
    );
    let mut sumf = 0f32;
    for (xs, ys) in xs.iter().zip(ys.iter()) {
        unsafe {
            let mut sum_i = vdupq_n_s32(0);
            let scale = xs.d * ys.d;
            let xs = xs.qs.as_ptr();
            let ys = ys.qs.as_ptr();
            for i in (0..QK_K).step_by(16) {
                let xs = vld1q_s8(xs.add(i));
                let ys = vld1q_s8(ys.add(i));
                let xy = vdotq_s32(xs, ys);
                sum_i = vaddq_s32(sum_i, xy)
            }
            sumf += vaddvq_s32(sum_i) as f32 * scale
        }
    }
    sumf
}

#[inline(always)]
pub(crate) fn vec_dot_q6k_q8k(n: usize, xs: &[BlockQ6K], ys: &[BlockQ8K]) -> f32 {
    debug_assert!(
        n.is_multiple_of(QK_K),
        "vec_dot_q6k_q8k: {n} is not divisible by {QK_K}"
    );
    let mut sum = 0f32;
    unsafe {
        let m4b = vdupq_n_u8(0xF);

        let mone = vdupq_n_u8(3);

        for (x, y) in xs.iter().zip(ys.iter()) {
            let d_all = x.d.to_f32();

            let mut q6 = x.ql.as_ptr();
            let mut qh = x.qh.as_ptr();
            let mut q8 = y.qs.as_ptr();

            let mut scale = x.scales.as_ptr();

            let q8sums = vld1q_s16_x2(y.bsums.as_ptr());
            let scales = vld1q_s8(scale);
            let q6scales = int16x8x2_t(
                vmovl_s8(vget_low_s8(scales)),
                vmovl_s8(vget_high_s8(scales)),
            );

            let prod = vaddq_s32(
                vaddq_s32(
                    vmull_s16(vget_low_s16(q8sums.0), vget_low_s16(q6scales.0)),
                    vmull_s16(vget_high_s16(q8sums.0), vget_high_s16(q6scales.0)),
                ),
                vaddq_s32(
                    vmull_s16(vget_low_s16(q8sums.1), vget_low_s16(q6scales.1)),
                    vmull_s16(vget_high_s16(q8sums.1), vget_high_s16(q6scales.1)),
                ),
            );
            let isum_mins = vaddvq_s32(prod);

            let mut isum = 0i32;

            for _j in 0..QK_K / 128 {
                let qhbits = vld1q_u8_x2(qh);
                qh = qh.add(32);
                let q6bits = vld1q_u8_x4(q6);
                q6 = q6.add(64);
                let q8bytes = vld1q_s8_x4(q8);
                q8 = q8.add(64);

                let q6h_0 = vshlq_n_u8(vandq_u8(mone, qhbits.0), 4);
                let q6h_1 = vshlq_n_u8(vandq_u8(mone, qhbits.1), 4);
                let shifted = vshrq_n_u8(qhbits.0, 2);
                let q6h_2 = vshlq_n_u8(vandq_u8(mone, shifted), 4);
                let shifted = vshrq_n_u8(qhbits.1, 2);
                let q6h_3 = vshlq_n_u8(vandq_u8(mone, shifted), 4);

                let q6bytes_0 = vreinterpretq_s8_u8(vorrq_u8(vandq_u8(q6bits.0, m4b), q6h_0));
                let q6bytes_1 = vreinterpretq_s8_u8(vorrq_u8(vandq_u8(q6bits.1, m4b), q6h_1));
                let q6bytes_2 = vreinterpretq_s8_u8(vorrq_u8(vandq_u8(q6bits.2, m4b), q6h_2));
                let q6bytes_3 = vreinterpretq_s8_u8(vorrq_u8(vandq_u8(q6bits.3, m4b), q6h_3));

                let p0 = vdotq_s32(q6bytes_0, q8bytes.0);
                let p1 = vdotq_s32(q6bytes_1, q8bytes.1);
                let (scale0, scale1) = (*scale as i32, *scale.add(1) as i32);
                isum += vaddvq_s32(p0) * scale0 + vaddvq_s32(p1) * scale1;
                scale = scale.add(2);

                let p2 = vdotq_s32(q6bytes_2, q8bytes.2);
                let p3 = vdotq_s32(q6bytes_3, q8bytes.3);
                let (scale0, scale1) = (*scale as i32, *scale.add(1) as i32);
                isum += vaddvq_s32(p2) * scale0 + vaddvq_s32(p3) * scale1;
                scale = scale.add(2);

                let q8bytes = vld1q_s8_x4(q8);
                q8 = q8.add(64);

                let shifted = vshrq_n_u8(qhbits.0, 4);
                let q6h_0 = vshlq_n_u8(vandq_u8(mone, shifted), 4);
                let shifted = vshrq_n_u8(qhbits.1, 4);
                let q6h_1 = vshlq_n_u8(vandq_u8(mone, shifted), 4);
                let shifted = vshrq_n_u8(qhbits.0, 6);
                let q6h_2 = vshlq_n_u8(vandq_u8(mone, shifted), 4);
                let shifted = vshrq_n_u8(qhbits.1, 6);
                let q6h_3 = vshlq_n_u8(vandq_u8(mone, shifted), 4);

                let q6bytes_0 = vreinterpretq_s8_u8(vorrq_u8(vshrq_n_u8(q6bits.0, 4), q6h_0));
                let q6bytes_1 = vreinterpretq_s8_u8(vorrq_u8(vshrq_n_u8(q6bits.1, 4), q6h_1));
                let q6bytes_2 = vreinterpretq_s8_u8(vorrq_u8(vshrq_n_u8(q6bits.2, 4), q6h_2));
                let q6bytes_3 = vreinterpretq_s8_u8(vorrq_u8(vshrq_n_u8(q6bits.3, 4), q6h_3));

                let p0 = vdotq_s32(q6bytes_0, q8bytes.0);
                let p1 = vdotq_s32(q6bytes_1, q8bytes.1);
                let (scale0, scale1) = (*scale as i32, *scale.add(1) as i32);
                isum += vaddvq_s32(p0) * scale0 + vaddvq_s32(p1) * scale1;
                scale = scale.add(2);

                let p2 = vdotq_s32(q6bytes_2, q8bytes.2);
                let p3 = vdotq_s32(q6bytes_3, q8bytes.3);
                let (scale0, scale1) = (*scale as i32, *scale.add(1) as i32);
                isum += vaddvq_s32(p2) * scale0 + vaddvq_s32(p3) * scale1;
                scale = scale.add(2);
            }
            sum += d_all * y.d * ((isum - 32 * isum_mins) as f32);
        }
    }
    sum
}

/// Four Q6K dot products sharing one Q8K load.
#[inline(always)]
pub(crate) fn vec_dot_4_q6k_q8k(
    n: usize,
    xs0: &[BlockQ6K],
    xs1: &[BlockQ6K],
    xs2: &[BlockQ6K],
    xs3: &[BlockQ6K],
    ys: &[BlockQ8K],
) -> (f32, f32, f32, f32) {
    debug_assert!(
        n.is_multiple_of(QK_K),
        "vec_dot_4_q6k_q8k: {n} is not divisible by {QK_K}"
    );

    let mut sum0 = 0f32;
    let mut sum1 = 0f32;
    let mut sum2 = 0f32;
    let mut sum3 = 0f32;

    unsafe {
        let m4b = vdupq_n_u8(0xF);
        let mone = vdupq_n_u8(3);

        for ((((x0, x1), x2), x3), y) in xs0
            .iter()
            .zip(xs1.iter())
            .zip(xs2.iter())
            .zip(xs3.iter())
            .zip(ys.iter())
        {
            let yd = y.d;

            // Q8K bsums - loaded once for all four columns.
            let q8sums = vld1q_s16_x2(y.bsums.as_ptr());

            // Compute isum_mins for each column: dot(q8sums, column_scales).
            macro_rules! col_isum_mins {
                ($x:ident) => {{
                    let scales_v = vld1q_s8($x.scales.as_ptr());
                    let q6sc = int16x8x2_t(
                        vmovl_s8(vget_low_s8(scales_v)),
                        vmovl_s8(vget_high_s8(scales_v)),
                    );
                    let prod = vaddq_s32(
                        vaddq_s32(
                            vmull_s16(vget_low_s16(q8sums.0), vget_low_s16(q6sc.0)),
                            vmull_s16(vget_high_s16(q8sums.0), vget_high_s16(q6sc.0)),
                        ),
                        vaddq_s32(
                            vmull_s16(vget_low_s16(q8sums.1), vget_low_s16(q6sc.1)),
                            vmull_s16(vget_high_s16(q8sums.1), vget_high_s16(q6sc.1)),
                        ),
                    );
                    vaddvq_s32(prod)
                }};
            }

            let isum_mins0 = col_isum_mins!(x0);
            let isum_mins1 = col_isum_mins!(x1);
            let isum_mins2 = col_isum_mins!(x2);
            let isum_mins3 = col_isum_mins!(x3);

            let mut q6_0 = x0.ql.as_ptr();
            let mut qh_0 = x0.qh.as_ptr();
            let mut sc_0 = x0.scales.as_ptr();
            let mut q6_1 = x1.ql.as_ptr();
            let mut qh_1 = x1.qh.as_ptr();
            let mut sc_1 = x1.scales.as_ptr();
            let mut q6_2 = x2.ql.as_ptr();
            let mut qh_2 = x2.qh.as_ptr();
            let mut sc_2 = x2.scales.as_ptr();
            let mut q6_3 = x3.ql.as_ptr();
            let mut qh_3 = x3.qh.as_ptr();
            let mut sc_3 = x3.scales.as_ptr();
            let mut q8 = y.qs.as_ptr();

            let mut isum0 = 0i32;
            let mut isum1 = 0i32;
            let mut isum2 = 0i32;
            let mut isum3 = 0i32;

            for _j in 0..QK_K / 128 {
                // Load Q8K bytes once - shared across all four columns.
                let q8lo = vld1q_s8_x4(q8);
                q8 = q8.add(64);
                let q8hi = vld1q_s8_x4(q8);
                q8 = q8.add(64);

                // Decode one column's Q6 bits and accumulate into its isum.
                // q8lo/q8hi are shared from the outer scope.
                macro_rules! process_col {
                    ($q6:ident, $qh:ident, $sc:ident, $isum:ident) => {
                        let qhb = vld1q_u8_x2($qh);
                        $qh = $qh.add(32);
                        let q6b = vld1q_u8_x4($q6);
                        $q6 = $q6.add(64);

                        // First half: low nibbles of ql + bits[1:0] and bits[3:2] of qh.
                        let qh00 = vshlq_n_u8(vandq_u8(mone, qhb.0), 4);
                        let qh01 = vshlq_n_u8(vandq_u8(mone, qhb.1), 4);
                        let qh10 = vshlq_n_u8(vandq_u8(mone, vshrq_n_u8(qhb.0, 2)), 4);
                        let qh11 = vshlq_n_u8(vandq_u8(mone, vshrq_n_u8(qhb.1, 2)), 4);

                        let q6b0 = vreinterpretq_s8_u8(vorrq_u8(vandq_u8(q6b.0, m4b), qh00));
                        let q6b1 = vreinterpretq_s8_u8(vorrq_u8(vandq_u8(q6b.1, m4b), qh01));
                        let q6b2 = vreinterpretq_s8_u8(vorrq_u8(vandq_u8(q6b.2, m4b), qh10));
                        let q6b3 = vreinterpretq_s8_u8(vorrq_u8(vandq_u8(q6b.3, m4b), qh11));

                        let p0 = vdotq_s32(q6b0, q8lo.0);
                        let p1 = vdotq_s32(q6b1, q8lo.1);
                        $isum +=
                            vaddvq_s32(p0) * (*$sc as i32) + vaddvq_s32(p1) * (*$sc.add(1) as i32);
                        $sc = $sc.add(2);

                        let p2 = vdotq_s32(q6b2, q8lo.2);
                        let p3 = vdotq_s32(q6b3, q8lo.3);
                        $isum +=
                            vaddvq_s32(p2) * (*$sc as i32) + vaddvq_s32(p3) * (*$sc.add(1) as i32);
                        $sc = $sc.add(2);

                        // Second half: high nibbles of ql + bits[5:4] and bits[7:6] of qh.
                        let qh20 = vshlq_n_u8(vandq_u8(mone, vshrq_n_u8(qhb.0, 4)), 4);
                        let qh21 = vshlq_n_u8(vandq_u8(mone, vshrq_n_u8(qhb.1, 4)), 4);
                        let qh30 = vshlq_n_u8(vandq_u8(mone, vshrq_n_u8(qhb.0, 6)), 4);
                        let qh31 = vshlq_n_u8(vandq_u8(mone, vshrq_n_u8(qhb.1, 6)), 4);

                        let q6b0 = vreinterpretq_s8_u8(vorrq_u8(vshrq_n_u8(q6b.0, 4), qh20));
                        let q6b1 = vreinterpretq_s8_u8(vorrq_u8(vshrq_n_u8(q6b.1, 4), qh21));
                        let q6b2 = vreinterpretq_s8_u8(vorrq_u8(vshrq_n_u8(q6b.2, 4), qh30));
                        let q6b3 = vreinterpretq_s8_u8(vorrq_u8(vshrq_n_u8(q6b.3, 4), qh31));

                        let p0 = vdotq_s32(q6b0, q8hi.0);
                        let p1 = vdotq_s32(q6b1, q8hi.1);
                        $isum +=
                            vaddvq_s32(p0) * (*$sc as i32) + vaddvq_s32(p1) * (*$sc.add(1) as i32);
                        $sc = $sc.add(2);

                        let p2 = vdotq_s32(q6b2, q8hi.2);
                        let p3 = vdotq_s32(q6b3, q8hi.3);
                        $isum +=
                            vaddvq_s32(p2) * (*$sc as i32) + vaddvq_s32(p3) * (*$sc.add(1) as i32);
                        $sc = $sc.add(2);
                    };
                }

                process_col!(q6_0, qh_0, sc_0, isum0);
                process_col!(q6_1, qh_1, sc_1, isum1);
                process_col!(q6_2, qh_2, sc_2, isum2);
                process_col!(q6_3, qh_3, sc_3, isum3);
            }

            sum0 += x0.d.to_f32() * yd * ((isum0 - 32 * isum_mins0) as f32);
            sum1 += x1.d.to_f32() * yd * ((isum1 - 32 * isum_mins1) as f32);
            sum2 += x2.d.to_f32() * yd * ((isum2 - 32 * isum_mins2) as f32);
            sum3 += x3.d.to_f32() * yd * ((isum3 - 32 * isum_mins3) as f32);
        }
    }

    (sum0, sum1, sum2, sum3)
}

#[inline(always)]
pub(crate) fn vec_dot_q5k_q8k(n: usize, xs: &[BlockQ5K], ys: &[BlockQ8K]) -> f32 {
    debug_assert!(
        n.is_multiple_of(QK_K),
        "vec_dot_q5k_q8k: {n} is not divisible by {QK_K}"
    );
    let mut sumf = 0f32;
    let mut utmp = [0u32; 4];
    const KMASK1: u32 = 0x3f3f3f3f;
    const KMASK2: u32 = 0x0f0f0f0f;
    const KMASK3: u32 = 0x03030303;

    unsafe {
        let m4b = vdupq_n_u8(0xF);
        let mone = vdupq_n_u8(1);
        let mtwo = vdupq_n_u8(2);

        for (x, y) in xs.iter().zip(ys.iter()) {
            let d = y.d * x.d.to_f32();
            let dmin = y.d * x.dmin.to_f32();

            let q8sums = vpaddq_s16(
                vld1q_s16(y.bsums.as_ptr()),
                vld1q_s16(y.bsums.as_ptr().add(8)),
            );

            LittleEndian::read_u32_into(&x.scales, &mut utmp[0..3]);

            utmp[3] = ((utmp[2] >> 4) & KMASK2) | (((utmp[1] >> 6) & KMASK3) << 4);
            let uaux = utmp[1] & KMASK1;
            utmp[1] = (utmp[2] & KMASK2) | (((utmp[0] >> 6) & KMASK3) << 4);
            utmp[2] = uaux;
            utmp[0] &= KMASK1;

            let mins8 = vld1_u8((utmp.as_ptr() as *const u8).add(8));
            let mins = vreinterpretq_s16_u16(vmovl_u8(mins8));
            let prod = vaddq_s32(
                vmull_s16(vget_low_s16(q8sums), vget_low_s16(mins)),
                vmull_s16(vget_high_s16(q8sums), vget_high_s16(mins)),
            );
            let sumi_mins = vaddvq_s32(prod);

            let mut scales = utmp.as_ptr() as *const u8;

            let mut q5 = x.qs.as_ptr();
            let mut q8 = y.qs.as_ptr();

            let mut qhbits = vld1q_u8_x2(x.qh.as_ptr());

            let mut sumi = 0i32;

            for _j in 0..QK_K / 64 {
                let q5bits = vld1q_u8_x2(q5);
                q5 = q5.add(32);
                let q8bytes = vld1q_s8_x4(q8);
                q8 = q8.add(64);

                let q5h_0 = vshlq_n_u8(vandq_u8(mone, qhbits.0), 4);
                let q5h_1 = vshlq_n_u8(vandq_u8(mone, qhbits.1), 4);
                let q5h_2 = vshlq_n_u8(vandq_u8(mtwo, qhbits.0), 3);
                let q5h_3 = vshlq_n_u8(vandq_u8(mtwo, qhbits.1), 3);
                qhbits.0 = vshrq_n_u8(qhbits.0, 2);
                qhbits.1 = vshrq_n_u8(qhbits.1, 2);

                let q5bytes_0 = vreinterpretq_s8_u8(vorrq_u8(vandq_u8(q5bits.0, m4b), q5h_0));
                let q5bytes_1 = vreinterpretq_s8_u8(vorrq_u8(vandq_u8(q5bits.1, m4b), q5h_1));
                let q5bytes_2 = vreinterpretq_s8_u8(vorrq_u8(vshrq_n_u8(q5bits.0, 4), q5h_2));
                let q5bytes_3 = vreinterpretq_s8_u8(vorrq_u8(vshrq_n_u8(q5bits.1, 4), q5h_3));

                let p0 = vdotq_s32(q5bytes_0, q8bytes.0);
                let p1 = vdotq_s32(q5bytes_1, q8bytes.1);
                sumi += vaddvq_s32(vaddq_s32(p0, p1)) * *scales as i32;
                scales = scales.add(1);

                let p2 = vdotq_s32(q5bytes_2, q8bytes.2);
                let p3 = vdotq_s32(q5bytes_3, q8bytes.3);
                sumi += vaddvq_s32(vaddq_s32(p2, p3)) * *scales as i32;
                scales = scales.add(1);
            }
            sumf += d * sumi as f32 - dmin * sumi_mins as f32;
        }
    }
    sumf
}

#[inline(always)]
pub(crate) fn vec_dot_q4k_q8k(n: usize, xs: &[BlockQ4K], ys: &[BlockQ8K]) -> f32 {
    debug_assert!(
        n.is_multiple_of(QK_K),
        "vec_dot_q4k_q8k: {n} is not divisible by {QK_K}"
    );
    let mut sumf = 0f32;
    let mut utmp = [0u32; 4];
    let mut scales = [0u8; 16];
    const KMASK1: u32 = 0x3f3f3f3f;
    const KMASK2: u32 = 0x0f0f0f0f;
    const KMASK3: u32 = 0x03030303;

    unsafe {
        let m4b = vdupq_n_u8(0xF);

        for (x, y) in xs.iter().zip(ys.iter()) {
            let d = y.d * x.d.to_f32();
            let dmin = y.d * x.dmin.to_f32();

            let q8sums = vpaddq_s16(
                vld1q_s16(y.bsums.as_ptr()),
                vld1q_s16(y.bsums.as_ptr().add(8)),
            );

            LittleEndian::read_u32_into(&x.scales, &mut utmp[0..3]);

            let mins8 = vld1_u32(
                [
                    utmp[1] & KMASK1,
                    ((utmp[2] >> 4) & KMASK2) | (((utmp[1] >> 6) & KMASK3) << 4),
                ]
                .as_ptr(),
            );
            utmp[1] = (utmp[2] & KMASK2) | (((utmp[0] >> 6) & KMASK3) << 4);
            utmp[0] &= KMASK1;

            let mins = vreinterpretq_s16_u16(vmovl_u8(vreinterpret_u8_u32(mins8)));
            let prod = vaddq_s32(
                vmull_s16(vget_low_s16(q8sums), vget_low_s16(mins)),
                vmull_s16(vget_high_s16(q8sums), vget_high_s16(mins)),
            );
            sumf -= dmin * vaddvq_s32(prod) as f32;

            LittleEndian::write_u32_into(&utmp, &mut scales);

            let mut q4 = x.qs.as_ptr();
            let mut q8 = y.qs.as_ptr();

            let mut sumi1 = 0i32;
            let mut sumi2 = 0i32;

            for j in 0..QK_K / 64 {
                let q4bits = vld1q_u8_x2(q4);
                q4 = q4.add(32);
                let q8bytes = vld1q_s8_x2(q8);
                q8 = q8.add(32);
                let q4bytes = int8x16x2_t(
                    vreinterpretq_s8_u8(vandq_u8(q4bits.0, m4b)),
                    vreinterpretq_s8_u8(vandq_u8(q4bits.1, m4b)),
                );
                let p0 = vdotq_s32(q4bytes.0, q8bytes.0);
                let p1 = vdotq_s32(q4bytes.1, q8bytes.1);
                sumi1 += vaddvq_s32(vaddq_s32(p0, p1)) * scales[2 * j] as i32;

                let q8bytes = vld1q_s8_x2(q8);
                q8 = q8.add(32);
                let q4bytes = int8x16x2_t(
                    vreinterpretq_s8_u8(vshrq_n_u8(q4bits.0, 4)),
                    vreinterpretq_s8_u8(vshrq_n_u8(q4bits.1, 4)),
                );
                let p2 = vdotq_s32(q4bytes.0, q8bytes.0);
                let p3 = vdotq_s32(q4bytes.1, q8bytes.1);
                sumi2 += vaddvq_s32(vaddq_s32(p2, p3)) * scales[2 * j + 1] as i32;
            }
            sumf += d * (sumi1 + sumi2) as f32;
        }
    }
    sumf
}

/// Four Q4K dot products sharing one Q8K load.
#[inline(always)]
pub(crate) fn vec_dot_4_q4k_q8k(
    n: usize,
    xs0: &[BlockQ4K],
    xs1: &[BlockQ4K],
    xs2: &[BlockQ4K],
    xs3: &[BlockQ4K],
    ys: &[BlockQ8K],
) -> (f32, f32, f32, f32) {
    debug_assert!(
        n.is_multiple_of(QK_K),
        "vec_dot_4_q4k_q8k: {n} is not divisible by {QK_K}"
    );

    let mut sum0 = 0f32;
    let mut sum1 = 0f32;
    let mut sum2 = 0f32;
    let mut sum3 = 0f32;

    let mut utmp = [0u32; 4];
    let mut sc0 = [0u8; 16];
    let mut sc1 = [0u8; 16];
    let mut sc2 = [0u8; 16];
    let mut sc3 = [0u8; 16];

    const KMASK1: u32 = 0x3f3f3f3f;
    const KMASK2: u32 = 0x0f0f0f0f;
    const KMASK3: u32 = 0x03030303;

    // Decode Q4K scale+min bytes into an 8-entry scales array and a float32x4x2
    // mins vector. Returns the mins vector so the caller can compute min correction.
    macro_rules! decode_q4k_scales {
        ($x:ident, $sc:ident) => {{
            LittleEndian::read_u32_into(&$x.scales, &mut utmp[0..3]);
            let mins8 = vld1_u32(
                [
                    utmp[1] & KMASK1,
                    ((utmp[2] >> 4) & KMASK2) | (((utmp[1] >> 6) & KMASK3) << 4),
                ]
                .as_ptr(),
            );
            utmp[1] = (utmp[2] & KMASK2) | (((utmp[0] >> 6) & KMASK3) << 4);
            utmp[0] &= KMASK1;
            LittleEndian::write_u32_into(&utmp, &mut $sc);
            vreinterpretq_s16_u16(vmovl_u8(vreinterpret_u8_u32(mins8)))
        }};
    }

    // Accumulate one column's nibble-group contribution into a scalar sumi.
    macro_rules! dot_col {
        ($q4:ident, $sc:ident, $vsum1:ident, $vsum2:ident, $q8lo:ident, $q8hi:ident, $j:ident, $m4b:ident) => {
            let bits = vld1q_u8_x2($q4);
            $q4 = $q4.add(32);
            let q4lo = int8x16x2_t(
                vreinterpretq_s8_u8(vandq_u8(bits.0, $m4b)),
                vreinterpretq_s8_u8(vandq_u8(bits.1, $m4b)),
            );
            $vsum1 = vmlaq_n_s32(
                $vsum1,
                vdotq_s32_pair(q4lo.0, $q8lo.0, q4lo.1, $q8lo.1),
                $sc[2 * $j] as i32,
            );
            let q4hi = int8x16x2_t(
                vreinterpretq_s8_u8(vshrq_n_u8(bits.0, 4)),
                vreinterpretq_s8_u8(vshrq_n_u8(bits.1, 4)),
            );
            $vsum2 = vmlaq_n_s32(
                $vsum2,
                vdotq_s32_pair(q4hi.0, $q8hi.0, q4hi.1, $q8hi.1),
                $sc[2 * $j + 1] as i32,
            );
        };
    }

    unsafe {
        let m4b = vdupq_n_u8(0xF);

        for ((((x0, x1), x2), x3), y) in xs0
            .iter()
            .zip(xs1.iter())
            .zip(xs2.iter())
            .zip(xs3.iter())
            .zip(ys.iter())
        {
            let yd = y.d;

            // Q8K bsums - loaded once for all four columns.
            let q8sums = vpaddq_s16(
                vld1q_s16(y.bsums.as_ptr()),
                vld1q_s16(y.bsums.as_ptr().add(8)),
            );

            // Decode scales and apply min correction for each column.
            let mins0 = decode_q4k_scales!(x0, sc0);
            let mins1 = decode_q4k_scales!(x1, sc1);
            let mins2 = decode_q4k_scales!(x2, sc2);
            let mins3 = decode_q4k_scales!(x3, sc3);

            let d0 = yd * x0.d.to_f32();
            let d1 = yd * x1.d.to_f32();
            let d2 = yd * x2.d.to_f32();
            let d3 = yd * x3.d.to_f32();

            // min correction: sum -= dmin * dot(q8sums, mins)
            macro_rules! min_correct {
                ($mins:ident, $dmin:expr, $sum:ident) => {
                    let prod = vaddq_s32(
                        vmull_s16(vget_low_s16(q8sums), vget_low_s16($mins)),
                        vmull_s16(vget_high_s16(q8sums), vget_high_s16($mins)),
                    );
                    $sum -= $dmin * vaddvq_s32(prod) as f32;
                };
            }
            min_correct!(mins0, yd * x0.dmin.to_f32(), sum0);
            min_correct!(mins1, yd * x1.dmin.to_f32(), sum1);
            min_correct!(mins2, yd * x2.dmin.to_f32(), sum2);
            min_correct!(mins3, yd * x3.dmin.to_f32(), sum3);

            let mut q4_0 = x0.qs.as_ptr();
            let mut q4_1 = x1.qs.as_ptr();
            let mut q4_2 = x2.qs.as_ptr();
            let mut q4_3 = x3.qs.as_ptr();
            let mut q8 = y.qs.as_ptr();

            let mut s0a = vdupq_n_s32(0);
            let mut s0b = vdupq_n_s32(0);
            let mut s1a = vdupq_n_s32(0);
            let mut s1b = vdupq_n_s32(0);
            let mut s2a = vdupq_n_s32(0);
            let mut s2b = vdupq_n_s32(0);
            let mut s3a = vdupq_n_s32(0);
            let mut s3b = vdupq_n_s32(0);

            for j in 0..QK_K / 64 {
                // Load Q8K once and reuse across all four columns.
                let q8lo = vld1q_s8_x2(q8);
                q8 = q8.add(32);
                let q8hi = vld1q_s8_x2(q8);
                q8 = q8.add(32);

                dot_col!(q4_0, sc0, s0a, s0b, q8lo, q8hi, j, m4b);
                dot_col!(q4_1, sc1, s1a, s1b, q8lo, q8hi, j, m4b);
                dot_col!(q4_2, sc2, s2a, s2b, q8lo, q8hi, j, m4b);
                dot_col!(q4_3, sc3, s3a, s3b, q8lo, q8hi, j, m4b);
            }

            sum0 += d0 * vaddvq_s32(vaddq_s32(s0a, s0b)) as f32;
            sum1 += d1 * vaddvq_s32(vaddq_s32(s1a, s1b)) as f32;
            sum2 += d2 * vaddvq_s32(vaddq_s32(s2a, s2b)) as f32;
            sum3 += d3 * vaddvq_s32(vaddq_s32(s3a, s3b)) as f32;
        }
    }

    (sum0, sum1, sum2, sum3)
}

#[inline(always)]
pub(crate) fn vec_dot_q3k_q8k(n: usize, xs: &[BlockQ3K], ys: &[BlockQ8K]) -> f32 {
    debug_assert!(
        n.is_multiple_of(QK_K),
        "vec_dot_q3k_q8k: {n} is not divisible by {QK_K}"
    );
    let mut sumf = 0f32;
    let mut utmp = [0u32; 4];
    let mut aux = [0u32; 3];
    const KMASK1: u32 = 0x03030303;
    const KMASK2: u32 = 0x0f0f0f0f;

    unsafe {
        let m3b = vdupq_n_u8(0x3);
        let m0 = vdupq_n_u8(1);
        let m1 = vshlq_n_u8(m0, 1);
        let m2 = vshlq_n_u8(m0, 2);
        let m3 = vshlq_n_u8(m0, 3);
        for (x, y) in xs.iter().zip(ys.iter()) {
            let d = y.d * x.d.to_f32();
            let mut q3 = x.qs.as_ptr();
            let qh = x.hmask.as_ptr();
            let mut q8 = y.qs.as_ptr();

            let mut qhbits = vld1q_u8_x2(qh);

            let mut isum = 0i32;

            // Set up scales
            LittleEndian::read_u32_into(&x.scales, &mut aux);

            utmp[3] = ((aux[1] >> 4) & KMASK2) | (((aux[2] >> 6) & KMASK1) << 4);
            utmp[2] = ((aux[0] >> 4) & KMASK2) | (((aux[2] >> 4) & KMASK1) << 4);
            utmp[1] = (aux[1] & KMASK2) | (((aux[2] >> 2) & KMASK1) << 4);
            utmp[0] = (aux[0] & KMASK2) | ((aux[2] & KMASK1) << 4);

            let mut scale = utmp.as_mut_ptr() as *mut i8;
            for j in 0..16 {
                *scale.add(j) -= 32i8
            }

            for j in 0..QK_K / 128 {
                let q3bits = vld1q_u8_x2(q3);
                q3 = q3.add(32);
                let q8bytes_1 = vld1q_s8_x4(q8);
                q8 = q8.add(64);
                let q8bytes_2 = vld1q_s8_x4(q8);
                q8 = q8.add(64);

                let q3h_0 = vshlq_n_u8(vbicq_u8(m0, qhbits.0), 2);
                let q3h_1 = vshlq_n_u8(vbicq_u8(m0, qhbits.1), 2);
                let q3h_2 = vshlq_n_u8(vbicq_u8(m1, qhbits.0), 1);
                let q3h_3 = vshlq_n_u8(vbicq_u8(m1, qhbits.1), 1);

                let q3bytes_0 = vsubq_s8(
                    vreinterpretq_s8_u8(vandq_u8(q3bits.0, m3b)),
                    vreinterpretq_s8_u8(q3h_0),
                );
                let q3bytes_1 = vsubq_s8(
                    vreinterpretq_s8_u8(vandq_u8(q3bits.1, m3b)),
                    vreinterpretq_s8_u8(q3h_1),
                );
                let q3bytes_2 = vsubq_s8(
                    vreinterpretq_s8_u8(vandq_u8(vshrq_n_u8(q3bits.0, 2), m3b)),
                    vreinterpretq_s8_u8(q3h_2),
                );
                let q3bytes_3 = vsubq_s8(
                    vreinterpretq_s8_u8(vandq_u8(vshrq_n_u8(q3bits.1, 2), m3b)),
                    vreinterpretq_s8_u8(q3h_3),
                );

                let p0 = vdotq_s32(q3bytes_0, q8bytes_1.0);
                let p1 = vdotq_s32(q3bytes_1, q8bytes_1.1);
                let p2 = vdotq_s32(q3bytes_2, q8bytes_1.2);
                let p3 = vdotq_s32(q3bytes_3, q8bytes_1.3);
                isum += vaddvq_s32(p0) * *scale as i32
                    + vaddvq_s32(p1) * *scale.add(1) as i32
                    + vaddvq_s32(p2) * *scale.add(2) as i32
                    + vaddvq_s32(p3) * *scale.add(3) as i32;
                scale = scale.add(4);

                let q3h_0 = vbicq_u8(m2, qhbits.0);
                let q3h_1 = vbicq_u8(m2, qhbits.1);
                let q3h_2 = vshrq_n_u8(vbicq_u8(m3, qhbits.0), 1);
                let q3h_3 = vshrq_n_u8(vbicq_u8(m3, qhbits.1), 1);

                let q3bytes_0 = vsubq_s8(
                    vreinterpretq_s8_u8(vandq_u8(vshrq_n_u8(q3bits.0, 4), m3b)),
                    vreinterpretq_s8_u8(q3h_0),
                );
                let q3bytes_1 = vsubq_s8(
                    vreinterpretq_s8_u8(vandq_u8(vshrq_n_u8(q3bits.1, 4), m3b)),
                    vreinterpretq_s8_u8(q3h_1),
                );
                let q3bytes_2 = vsubq_s8(
                    vreinterpretq_s8_u8(vandq_u8(vshrq_n_u8(q3bits.0, 6), m3b)),
                    vreinterpretq_s8_u8(q3h_2),
                );
                let q3bytes_3 = vsubq_s8(
                    vreinterpretq_s8_u8(vandq_u8(vshrq_n_u8(q3bits.1, 6), m3b)),
                    vreinterpretq_s8_u8(q3h_3),
                );

                let p0 = vdotq_s32(q3bytes_0, q8bytes_2.0);
                let p1 = vdotq_s32(q3bytes_1, q8bytes_2.1);
                let p2 = vdotq_s32(q3bytes_2, q8bytes_2.2);
                let p3 = vdotq_s32(q3bytes_3, q8bytes_2.3);
                isum += vaddvq_s32(p0) * *scale as i32
                    + vaddvq_s32(p1) * *scale.add(1) as i32
                    + vaddvq_s32(p2) * *scale.add(2) as i32
                    + vaddvq_s32(p3) * *scale.add(3) as i32;
                scale = scale.add(4);

                if j == 0 {
                    qhbits.0 = vshrq_n_u8(qhbits.0, 4);
                    qhbits.1 = vshrq_n_u8(qhbits.1, 4);
                }
            }
            sumf += d * isum as f32;
        }
    }
    sumf
}

#[inline(always)]
pub(crate) fn vec_dot_q2k_q8k(n: usize, xs: &[BlockQ2K], ys: &[BlockQ8K]) -> f32 {
    debug_assert!(
        n.is_multiple_of(QK_K),
        "vec_dot_q2k_q8k: {n} is not divisible by {QK_K}"
    );
    let mut sumf = 0f32;
    let mut aux = [0u8; 16];

    unsafe {
        let m3 = vdupq_n_u8(0x3);
        let m4 = vdupq_n_u8(0xF);

        for (x, y) in xs.iter().zip(ys.iter()) {
            let d = y.d * x.d.to_f32();
            let dmin = -y.d * x.dmin.to_f32();

            let mut q2 = x.qs.as_ptr();
            let mut q8 = y.qs.as_ptr();
            let sc = x.scales.as_ptr();

            let mins_and_scales = vld1q_u8(sc);
            let scales = vandq_u8(mins_and_scales, m4);
            vst1q_u8(aux.as_mut_ptr(), scales);

            let mins = vshrq_n_u8(mins_and_scales, 4);
            let q8sums = vld1q_s16_x2(y.bsums.as_ptr());
            let mins16 = int16x8x2_t(
                vreinterpretq_s16_u16(vmovl_u8(vget_low_u8(mins))),
                vreinterpretq_s16_u16(vmovl_u8(vget_high_u8(mins))),
            );
            let s0 = vaddq_s32(
                vmull_s16(vget_low_s16(mins16.0), vget_low_s16(q8sums.0)),
                vmull_s16(vget_high_s16(mins16.0), vget_high_s16(q8sums.0)),
            );
            let s1 = vaddq_s32(
                vmull_s16(vget_low_s16(mins16.1), vget_low_s16(q8sums.1)),
                vmull_s16(vget_high_s16(mins16.1), vget_high_s16(q8sums.1)),
            );
            sumf += dmin * vaddvq_s32(vaddq_s32(s0, s1)) as f32;

            let mut isum = 0i32;
            let mut is = 0usize;

            // TODO: dotprod
            for _j in 0..QK_K / 128 {
                let q2bits = vld1q_u8_x2(q2);
                q2 = q2.add(32);

                let q8bytes = vld1q_s8_x2(q8);
                q8 = q8.add(32);
                let mut q2bytes = int8x16x2_t(
                    vreinterpretq_s8_u8(vandq_u8(q2bits.0, m3)),
                    vreinterpretq_s8_u8(vandq_u8(q2bits.1, m3)),
                );
                isum += multiply_accum_with_scale(&aux, is, 0, q2bytes, q8bytes);

                let q8bytes = vld1q_s8_x2(q8);
                q8 = q8.add(32);
                q2bytes.0 = vreinterpretq_s8_u8(vandq_u8(vshrq_n_u8(q2bits.0, 2), m3));
                q2bytes.1 = vreinterpretq_s8_u8(vandq_u8(vshrq_n_u8(q2bits.1, 2), m3));
                isum += multiply_accum_with_scale(&aux, is, 2, q2bytes, q8bytes);

                let q8bytes = vld1q_s8_x2(q8);
                q8 = q8.add(32);
                q2bytes.0 = vreinterpretq_s8_u8(vandq_u8(vshrq_n_u8(q2bits.0, 4), m3));
                q2bytes.1 = vreinterpretq_s8_u8(vandq_u8(vshrq_n_u8(q2bits.1, 4), m3));
                isum += multiply_accum_with_scale(&aux, is, 4, q2bytes, q8bytes);

                let q8bytes = vld1q_s8_x2(q8);
                q8 = q8.add(32);
                q2bytes.0 = vreinterpretq_s8_u8(vandq_u8(vshrq_n_u8(q2bits.0, 6), m3));
                q2bytes.1 = vreinterpretq_s8_u8(vandq_u8(vshrq_n_u8(q2bits.1, 6), m3));
                isum += multiply_accum_with_scale(&aux, is, 6, q2bytes, q8bytes);

                is += 8;
            }
            sumf += d * isum as f32;
        }
    }
    sumf
}

/// Quantize a row of f32 activations into Q8K format
#[inline(always)]
pub(crate) fn quantize_row_q8k(xs: &[f32], ys: &mut [BlockQ8K]) {
    debug_assert!(
        xs.len().is_multiple_of(QK_K),
        "quantize_row_q8k: {} is not a multiple of {QK_K}",
        xs.len()
    );
    unsafe {
        let (x_chunks, _) = xs.as_chunks::<QK_K>();
        for (chunk, y) in x_chunks.iter().zip(ys.iter_mut()) {
            // Find the element with the maximum absolute value, preserving its sign.
            let (mut vabs_max0, mut vsmax0) = (vdupq_n_f32(0.0), vdupq_n_f32(0.0));
            let (mut vabs_max1, mut vsmax1) = (vdupq_n_f32(0.0), vdupq_n_f32(0.0));
            let (mut vabs_max2, mut vsmax2) = (vdupq_n_f32(0.0), vdupq_n_f32(0.0));
            let (mut vabs_max3, mut vsmax3) = (vdupq_n_f32(0.0), vdupq_n_f32(0.0));
            let mut p = chunk.as_ptr();
            for _ in 0..QK_K / 16 {
                let (v0, v1) = (vld1q_f32(p), vld1q_f32(p.add(4)));
                let (v2, v3) = (vld1q_f32(p.add(8)), vld1q_f32(p.add(12)));
                p = p.add(16);
                (vabs_max0, vsmax0) = merge_signed_max(vabs_max0, vsmax0, vabsq_f32(v0), v0);
                (vabs_max1, vsmax1) = merge_signed_max(vabs_max1, vsmax1, vabsq_f32(v1), v1);
                (vabs_max2, vsmax2) = merge_signed_max(vabs_max2, vsmax2, vabsq_f32(v2), v2);
                (vabs_max3, vsmax3) = merge_signed_max(vabs_max3, vsmax3, vabsq_f32(v3), v3);
            }
            // Tree-reduce 4 accumulators to 1.
            let (abs01, smax01) = merge_signed_max(vabs_max0, vsmax0, vabs_max1, vsmax1);
            let (abs23, smax23) = merge_signed_max(vabs_max2, vsmax2, vabs_max3, vsmax3);
            let (abs_v, smax_v) = merge_signed_max(abs01, smax01, abs23, smax23);
            // Cross lane reduce to scalar
            let mask_lohi = vcgt_f32(vget_high_f32(abs_v), vget_low_f32(abs_v));
            let abs_pair = vmax_f32(vget_low_f32(abs_v), vget_high_f32(abs_v));
            let smax_pair = vbsl_f32(mask_lohi, vget_high_f32(smax_v), vget_low_f32(smax_v));
            let max_signed = if vget_lane_f32(abs_pair, 1) > vget_lane_f32(abs_pair, 0) {
                vget_lane_f32(smax_pair, 1)
            } else {
                vget_lane_f32(smax_pair, 0)
            };

            if max_signed == 0.0f32 {
                y.d = 0.0f32;
                y.qs.fill(0);
                y.bsums.fill(0);
                continue;
            }

            let iscale = -127.0f32 / max_signed;
            let vscale = vdupq_n_f32(iscale);

            // Quantize f32 -> i8. Multiply, round-to-nearest, saturating narrow.
            let mut out = y.qs.as_mut_ptr();
            let mut p = chunk.as_ptr();
            for j in 0..QK_K / 16 {
                let f0 = vmulq_f32(vld1q_f32(p), vscale);
                let f1 = vmulq_f32(vld1q_f32(p.add(4)), vscale);
                let f2 = vmulq_f32(vld1q_f32(p.add(8)), vscale);
                let f3 = vmulq_f32(vld1q_f32(p.add(12)), vscale);
                p = p.add(16);
                let s01 = vcombine_s16(
                    vqmovn_s32(vcvtaq_s32_f32(f0)),
                    vqmovn_s32(vcvtaq_s32_f32(f1)),
                );
                let s23 = vcombine_s16(
                    vqmovn_s32(vcvtaq_s32_f32(f2)),
                    vqmovn_s32(vcvtaq_s32_f32(f3)),
                );
                let q = vcombine_s8(vqmovn_s16(s01), vqmovn_s16(s23));
                vst1q_s8(out, q);
                out = out.add(16);
                y.bsums[j] = vaddvq_s32(vpaddlq_s16(vpaddlq_s8(q))) as i16;
            }

            y.d = 1.0f32 / iscale;
        }
    }
}

#[inline(always)]
unsafe fn multiply_accum_with_scale(
    aux: &[u8; 16],
    is: usize,
    index: usize,
    q2bytes: int8x16x2_t,
    q8bytes: int8x16x2_t,
) -> i32 {
    let p1 = vdotq_s32(q2bytes.0, q8bytes.0);
    let p2 = vdotq_s32(q2bytes.1, q8bytes.1);
    vaddvq_s32(p1) * aux[is + index] as i32 + vaddvq_s32(p2) * aux[is + 1 + index] as i32
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "dotprod")]
pub(crate) fn vec_dot_8_q4k_q8k(n: usize, xs: &[BlockQ4Kx8], ys: &[BlockQ8K]) -> [f32; 8] {
    debug_assert!(n.is_multiple_of(QK_K));
    let mut out = [0f32; 8];

    #[inline(always)]
    unsafe fn load_f16x4(ptr: *const half::f16) -> float32x4_t {
        let raw = vld1_u64(ptr as *const u64);
        let mut result: float32x4_t;
        core::arch::asm!(
            "fcvtl {out:v}.4s, {inp:v}.4h",
            inp = in(vreg) raw,
            out = out(vreg) result,
            options(nostack, nomem),
        );
        result
    }

    unsafe {
        let mut vacc_0 = vdupq_n_f32(0.0f32);
        let mut vacc_1 = vdupq_n_f32(0.0f32);
        let m4b = vdupq_n_u8(0x0f);
        for (q4, q8) in xs.iter().zip(ys.iter()) {
            let q8d = q8.d;
            let q8d_v = vdupq_n_f32(q8d);
            let sb_scale_0 = vmulq_f32(load_f16x4(q4.d.as_ptr()), q8d_v);
            let sb_scale_1 = vmulq_f32(load_f16x4(q4.d.as_ptr().add(4)), q8d_v);
            let sb_min_0 = vmulq_f32(load_f16x4(q4.dmin.as_ptr()), q8d_v);
            let sb_min_1 = vmulq_f32(load_f16x4(q4.dmin.as_ptr().add(4)), q8d_v);
            let bsums = vpaddq_s16(
                vld1q_s16(q8.bsums.as_ptr()),
                vld1q_s16(q8.bsums.as_ptr().add(8)),
            );
            let mut bias_0 = vdupq_n_s32(0);
            let mut bias_1 = vdupq_n_s32(0);
            macro_rules! process_sb {
                ($sb:literal) => {{
                    let (mins0, sc0) = decode_q4kx8_scales(q4.scales.as_ptr().add($sb * 24));
                    let (mins1, sc1) = decode_q4kx8_scales(q4.scales.as_ptr().add($sb * 24 + 12));
                    let q8p = q8.qs.as_ptr().add($sb * 64);
                    let q8_0 = vreinterpretq_s8_s64(vld1q_dup_s64(q8p as *const i64));
                    let q8_1 = vreinterpretq_s8_s64(vld1q_dup_s64(q8p.add(8) as *const i64));
                    let q8_2 = vreinterpretq_s8_s64(vld1q_dup_s64(q8p.add(16) as *const i64));
                    let q8_3 = vreinterpretq_s8_s64(vld1q_dup_s64(q8p.add(24) as *const i64));
                    let q8_4 = vreinterpretq_s8_s64(vld1q_dup_s64(q8p.add(32) as *const i64));
                    let q8_5 = vreinterpretq_s8_s64(vld1q_dup_s64(q8p.add(40) as *const i64));
                    let q8_6 = vreinterpretq_s8_s64(vld1q_dup_s64(q8p.add(48) as *const i64));
                    let q8_7 = vreinterpretq_s8_s64(vld1q_dup_s64(q8p.add(56) as *const i64));
                    let q4p = q4.qs.as_ptr().add($sb * QK_K);
                    let s0 = vld1q_u8_x2(q4p);
                    let s0h = vld1q_u8_x2(q4p.add(32));
                    let s1 = vld1q_u8_x2(q4p.add(64));
                    let s1h = vld1q_u8_x2(q4p.add(96));
                    let s2 = vld1q_u8_x2(q4p.add(128));
                    let s2h = vld1q_u8_x2(q4p.add(160));
                    let s3 = vld1q_u8_x2(q4p.add(192));
                    let s3h = vld1q_u8_x2(q4p.add(224));
                    let (b00, b10) = (s0.0, s0.1);
                    let (b20, b30) = (s0h.0, s0h.1);
                    let (b01, b11) = (s1.0, s1.1);
                    let (b21, b31) = (s1h.0, s1h.1);
                    let (b02, b12) = (s2.0, s2.1);
                    let (b22, b32) = (s2h.0, s2h.1);
                    let (b03, b13) = (s3.0, s3.1);
                    let (b23, b33) = (s3h.0, s3h.1);
                    let mut a0 = vdupq_n_s32(0);
                    a0 = sdot_acc(a0, vreinterpretq_s8_u8(vandq_u8(b00, m4b)), q8_0);
                    a0 = sdot_acc(a0, vreinterpretq_s8_u8(vandq_u8(b01, m4b)), q8_1);
                    a0 = sdot_acc(a0, vreinterpretq_s8_u8(vandq_u8(b02, m4b)), q8_2);
                    a0 = sdot_acc(a0, vreinterpretq_s8_u8(vandq_u8(b03, m4b)), q8_3);
                    let mut h0 = vdupq_n_s32(0);
                    h0 = sdot_acc(h0, vreinterpretq_s8_u8(vshrq_n_u8(b00, 4)), q8_4);
                    h0 = sdot_acc(h0, vreinterpretq_s8_u8(vshrq_n_u8(b01, 4)), q8_5);
                    h0 = sdot_acc(h0, vreinterpretq_s8_u8(vshrq_n_u8(b02, 4)), q8_6);
                    h0 = sdot_acc(h0, vreinterpretq_s8_u8(vshrq_n_u8(b03, 4)), q8_7);
                    let mut a1 = vdupq_n_s32(0);
                    a1 = sdot_acc(a1, vreinterpretq_s8_u8(vandq_u8(b10, m4b)), q8_0);
                    a1 = sdot_acc(a1, vreinterpretq_s8_u8(vandq_u8(b11, m4b)), q8_1);
                    a1 = sdot_acc(a1, vreinterpretq_s8_u8(vandq_u8(b12, m4b)), q8_2);
                    a1 = sdot_acc(a1, vreinterpretq_s8_u8(vandq_u8(b13, m4b)), q8_3);
                    let mut h1 = vdupq_n_s32(0);
                    h1 = sdot_acc(h1, vreinterpretq_s8_u8(vshrq_n_u8(b10, 4)), q8_4);
                    h1 = sdot_acc(h1, vreinterpretq_s8_u8(vshrq_n_u8(b11, 4)), q8_5);
                    h1 = sdot_acc(h1, vreinterpretq_s8_u8(vshrq_n_u8(b12, 4)), q8_6);
                    h1 = sdot_acc(h1, vreinterpretq_s8_u8(vshrq_n_u8(b13, 4)), q8_7);
                    let sumf_lo_03 =
                        vcvtq_f32_s32(vmulq_s32(vmovl_s16(vget_low_s16(sc0)), vpaddq_s32(a0, a1)));
                    vacc_0 = vfmaq_f32(vacc_0, sb_scale_0, sumf_lo_03);
                    let sumf_hi_03 =
                        vcvtq_f32_s32(vmulq_s32(vmovl_s16(vget_low_s16(sc1)), vpaddq_s32(h0, h1)));
                    vacc_0 = vfmaq_f32(vacc_0, sb_scale_0, sumf_hi_03);
                    let mut a2 = vdupq_n_s32(0);
                    a2 = sdot_acc(a2, vreinterpretq_s8_u8(vandq_u8(b20, m4b)), q8_0);
                    a2 = sdot_acc(a2, vreinterpretq_s8_u8(vandq_u8(b21, m4b)), q8_1);
                    a2 = sdot_acc(a2, vreinterpretq_s8_u8(vandq_u8(b22, m4b)), q8_2);
                    a2 = sdot_acc(a2, vreinterpretq_s8_u8(vandq_u8(b23, m4b)), q8_3);
                    let mut h2 = vdupq_n_s32(0);
                    h2 = sdot_acc(h2, vreinterpretq_s8_u8(vshrq_n_u8(b20, 4)), q8_4);
                    h2 = sdot_acc(h2, vreinterpretq_s8_u8(vshrq_n_u8(b21, 4)), q8_5);
                    h2 = sdot_acc(h2, vreinterpretq_s8_u8(vshrq_n_u8(b22, 4)), q8_6);
                    h2 = sdot_acc(h2, vreinterpretq_s8_u8(vshrq_n_u8(b23, 4)), q8_7);
                    let mut a3 = vdupq_n_s32(0);
                    a3 = sdot_acc(a3, vreinterpretq_s8_u8(vandq_u8(b30, m4b)), q8_0);
                    a3 = sdot_acc(a3, vreinterpretq_s8_u8(vandq_u8(b31, m4b)), q8_1);
                    a3 = sdot_acc(a3, vreinterpretq_s8_u8(vandq_u8(b32, m4b)), q8_2);
                    a3 = sdot_acc(a3, vreinterpretq_s8_u8(vandq_u8(b33, m4b)), q8_3);
                    let mut h3 = vdupq_n_s32(0);
                    h3 = sdot_acc(h3, vreinterpretq_s8_u8(vshrq_n_u8(b30, 4)), q8_4);
                    h3 = sdot_acc(h3, vreinterpretq_s8_u8(vshrq_n_u8(b31, 4)), q8_5);
                    h3 = sdot_acc(h3, vreinterpretq_s8_u8(vshrq_n_u8(b32, 4)), q8_6);
                    h3 = sdot_acc(h3, vreinterpretq_s8_u8(vshrq_n_u8(b33, 4)), q8_7);
                    let sumf_lo_47 =
                        vcvtq_f32_s32(vmulq_s32(vmovl_s16(vget_high_s16(sc0)), vpaddq_s32(a2, a3)));
                    vacc_1 = vfmaq_f32(vacc_1, sb_scale_1, sumf_lo_47);
                    let sumf_hi_47 =
                        vcvtq_f32_s32(vmulq_s32(vmovl_s16(vget_high_s16(sc1)), vpaddq_s32(h2, h3)));
                    vacc_1 = vfmaq_f32(vacc_1, sb_scale_1, sumf_hi_47);
                    let bl = vdup_n_s16(vgetq_lane_s16::<{ $sb * 2 }>(bsums));
                    let bh = vdup_n_s16(vgetq_lane_s16::<{ $sb * 2 + 1 }>(bsums));
                    bias_0 = vmlal_s16(bias_0, bl, vget_low_s16(mins0));
                    bias_0 = vmlal_s16(bias_0, bh, vget_low_s16(mins1));
                    bias_1 = vmlal_s16(bias_1, bl, vget_high_s16(mins0));
                    bias_1 = vmlal_s16(bias_1, bh, vget_high_s16(mins1));
                }};
            }
            process_sb!(0);
            process_sb!(1);
            process_sb!(2);
            process_sb!(3);
            vacc_0 = vmlsq_f32(vacc_0, vcvtq_f32_s32(bias_0), sb_min_0);
            vacc_1 = vmlsq_f32(vacc_1, vcvtq_f32_s32(bias_1), sb_min_1);
        }
        vst1q_f32(out.as_mut_ptr(), vacc_0);
        vst1q_f32(out.as_mut_ptr().add(4), vacc_1);
    }
    out
}
