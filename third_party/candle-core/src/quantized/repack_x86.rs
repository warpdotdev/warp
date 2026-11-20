//! x86 repacked quantized kernels: weights tiled [n/16][k-blocks][16 rows] so one
//! vpdpbusd (AVX512-VNNI) produces 16 output columns for 4 k-values per instruction.
//! Activations are BlockQ8K rows; q4k/q6k quants are unsigned so vpdpbusd applies
//! directly with the min/offset terms corrected through the q8k bsums, and q8_0
//! weights are stored offset by +128 with the same correction.

use super::k_quants::{BlockQ4K, BlockQ6K, BlockQ8K, BlockQ8_0, QK8_0, QK_K};
use super::{GgmlType, QuantizedType};
use half::f16;

pub(crate) const TILE_N: usize = 16;

// One k-superblock (256 values) for 16 output rows.
#[repr(C)]
pub(crate) struct TileQ4K {
    // per-subblock (8) per-row (16) effective scales/mins, pre-unpacked from the 6-bit fields
    pub(crate) scales: [[f32; TILE_N]; 8],
    pub(crate) mins: [[f32; TILE_N]; 8],
    // nibble-packed: [k8-group (32)][row (16)][4 bytes]; byte j = q[8g+j] | q[8g+j+4] << 4
    pub(crate) qs: [u8; QK_K / 8 * TILE_N * 4],
}

#[repr(C)]
pub(crate) struct TileQ6K {
    pub(crate) scales: [[f32; TILE_N]; 16],
    // raw 6-bit values as u8 (0..63; real value is v - 32): [k4-group (64)][row (16)][4 bytes]
    pub(crate) qs: [u8; QK_K / 4 * TILE_N * 4],
}

// One 32-value block group for 16 rows of q8_0 weights, stored offset by +128.
#[repr(C)]
pub(crate) struct TileQ8_0 {
    pub(crate) d: [f16; TILE_N],
    pub(crate) qs: [u8; QK8_0 * TILE_N],
}

pub(crate) enum PackedX86 {
    Q4K(Vec<TileQ4K>),
    Q6K(Vec<TileQ6K>),
    Q8_0(Vec<TileQ8_0>),
}

pub(crate) fn select(dtype: super::GgmlDType, m: usize, n: usize, k: usize) -> bool {
    use super::GgmlDType as D;
    let Some(lv) = level() else {
        return false;
    };
    if k == 0 || !n.is_multiple_of(TILE_N) || !k.is_multiple_of(QK_K) {
        return false;
    }
    match dtype {
        D::Q4K => true,
        // Keep Q6K GEMV on compact weights; expanded tiles cost bandwidth
        // without reuse across multiple activation rows.
        D::Q6K => m > 1,
        // pure avx2 maddubs overflows on the +128 u8 range; q8_0 needs real dpbusd
        D::Q8_0 => lv != X86Level::Avx2,
        _ => false,
    }
}

fn q4k_blocks(storage: &dyn QuantizedType) -> &[BlockQ4K] {
    let data = storage.storage_size_in_bytes();
    let ptr = storage.as_ptr() as *const BlockQ4K;
    unsafe { std::slice::from_raw_parts(ptr, data / std::mem::size_of::<BlockQ4K>()) }
}
fn q6k_blocks(storage: &dyn QuantizedType) -> &[BlockQ6K] {
    let data = storage.storage_size_in_bytes();
    let ptr = storage.as_ptr() as *const BlockQ6K;
    unsafe { std::slice::from_raw_parts(ptr, data / std::mem::size_of::<BlockQ6K>()) }
}
fn q8_0_blocks(storage: &dyn QuantizedType) -> &[BlockQ8_0] {
    let data = storage.storage_size_in_bytes();
    let ptr = storage.as_ptr() as *const BlockQ8_0;
    unsafe { std::slice::from_raw_parts(ptr, data / std::mem::size_of::<BlockQ8_0>()) }
}

// ggml's 6-bit scale/min unpack for q4k/q5k superblocks.
fn get_scale_min_k4(j: usize, scales: &[u8; 12]) -> (u8, u8) {
    if j < 4 {
        (scales[j] & 63, scales[j + 4] & 63)
    } else {
        (
            (scales[j + 4] & 0x0F) | ((scales[j - 4] >> 6) << 4),
            (scales[j + 4] >> 4) | ((scales[j] >> 6) << 4),
        )
    }
}

pub(crate) fn pack_q4k(storage: &dyn QuantizedType, n: usize, k: usize) -> PackedX86 {
    let blocks = q4k_blocks(storage);
    let kb = k / QK_K;
    let n_tiles = n / TILE_N;
    let mut out: Vec<TileQ4K> = Vec::with_capacity(n_tiles * kb);
    for t in 0..n_tiles {
        for b in 0..kb {
            let mut tile = TileQ4K {
                scales: [[0.0; TILE_N]; 8],
                mins: [[0.0; TILE_N]; 8],
                qs: [0; QK_K / 8 * TILE_N * 4],
            };
            for r in 0..TILE_N {
                let blk = &blocks[(t * TILE_N + r) * kb + b];
                let d = blk.d.to_f32();
                let dmin = blk.dmin.to_f32();
                for sub in 0..8 {
                    let (sc, mn) = get_scale_min_k4(sub, &blk.scales);
                    tile.scales[sub][r] = d * sc as f32;
                    tile.mins[sub][r] = dmin * mn as f32;
                }
                // q4k qs: 32-byte chunks c hold values [c*64, c*64+32) in the low nibbles
                // and [c*64+32, c*64+64) in the high nibbles
                let mut vals = [0u8; QK_K];
                for c in 0..4 {
                    for j in 0..32 {
                        let byte = blk.qs[c * 32 + j];
                        vals[c * 64 + j] = byte & 0x0F;
                        vals[c * 64 + 32 + j] = byte >> 4;
                    }
                }
                for g in 0..QK_K / 8 {
                    for j in 0..4 {
                        tile.qs[(g * TILE_N + r) * 4 + j] =
                            vals[g * 8 + j] | (vals[g * 8 + j + 4] << 4);
                    }
                }
            }
            out.push(tile);
        }
    }
    PackedX86::Q4K(out)
}

pub(crate) fn pack_q6k(storage: &dyn QuantizedType, n: usize, k: usize) -> PackedX86 {
    let blocks = q6k_blocks(storage);
    let kb = k / QK_K;
    let n_tiles = n / TILE_N;
    let mut out: Vec<TileQ6K> = Vec::with_capacity(n_tiles * kb);
    for t in 0..n_tiles {
        for b in 0..kb {
            let mut tile = TileQ6K {
                scales: [[0.0; TILE_N]; 16],
                qs: [0; QK_K / 4 * TILE_N * 4],
            };
            for r in 0..TILE_N {
                let blk = &blocks[(t * TILE_N + r) * kb + b];
                let d = blk.d.to_f32();
                for sub in 0..16 {
                    tile.scales[sub][r] = d * blk.scales[sub] as f32;
                }
                // q6k: ql 128 bytes (low 4 bits), qh 64 bytes (high 2 bits), in 128-value halves
                let mut vals = [0u8; QK_K];
                for h in 0..2 {
                    let ql = &blk.ql[h * 64..];
                    let qh = &blk.qh[h * 32..];
                    for j in 0..32 {
                        vals[h * 128 + j] = (ql[j] & 0x0F) | ((qh[j] & 3) << 4);
                        vals[h * 128 + j + 32] = (ql[j + 32] & 0x0F) | (((qh[j] >> 2) & 3) << 4);
                        vals[h * 128 + j + 64] = (ql[j] >> 4) | (((qh[j] >> 4) & 3) << 4);
                        vals[h * 128 + j + 96] = (ql[j + 32] >> 4) | (((qh[j] >> 6) & 3) << 4);
                    }
                }
                for g in 0..QK_K / 4 {
                    for j in 0..4 {
                        tile.qs[(g * TILE_N + r) * 4 + j] = vals[g * 4 + j];
                    }
                }
            }
            out.push(tile);
        }
    }
    PackedX86::Q6K(out)
}

pub(crate) fn pack_q8_0(storage: &dyn QuantizedType, n: usize, k: usize) -> PackedX86 {
    let blocks = q8_0_blocks(storage);
    let kb = k / QK8_0;
    let n_tiles = n / TILE_N;
    let mut out: Vec<TileQ8_0> = Vec::with_capacity(n_tiles * kb);
    for t in 0..n_tiles {
        for b in 0..kb {
            let mut tile = TileQ8_0 {
                d: [f16::ZERO; TILE_N],
                qs: [0; QK8_0 * TILE_N],
            };
            for r in 0..TILE_N {
                let blk = &blocks[(t * TILE_N + r) * kb + b];
                tile.d[r] = blk.d;
                for g in 0..QK8_0 / 4 {
                    for j in 0..4 {
                        tile.qs[(g * TILE_N + r) * 4 + j] = (blk.qs[g * 4 + j] as i16 + 128) as u8;
                    }
                }
            }
            out.push(tile);
        }
    }
    PackedX86::Q8_0(out)
}

#[cfg(target_arch = "x86_64")]
mod kernels {
    use super::*;
    use core::arch::x86_64::*;

    // acc[m][16] f32 for up to MT activation rows against one 16-row weight tile
    pub(crate) const MAX_MT: usize = 4;

    #[inline(always)]
    unsafe fn bcast4(act: &[i8], off: usize) -> __m512i {
        _mm512_set1_epi32(std::ptr::read_unaligned(act.as_ptr().add(off) as *const i32))
    }

    // q4k: one superblock tile against `mt` q8k activation blocks. Subblocks are
    // processed in pairs with independent integer accumulators to break the dpbusd
    // latency chain when mt is small (decode gemv).
    #[target_feature(enable = "avx512f,avx512vnni,avx512bw")]
    pub(crate) unsafe fn q4k_tile(tile: &TileQ4K, acts: &[&BlockQ8K], acc: &mut [[__m512; 1]]) {
        let mask = _mm512_set1_epi8(0x0F_u8 as i8);
        for sub2 in 0..4 {
            let sa = sub2 * 2;
            let sb = sa + 1;
            let mut isum_a = [_mm512_setzero_si512(); MAX_MT];
            let mut isum_b = [_mm512_setzero_si512(); MAX_MT];
            for g4 in 0..4 {
                let ga = sa * 4 + g4;
                let gb = sb * 4 + g4;
                let wa = _mm512_loadu_si512(tile.qs.as_ptr().add(ga * TILE_N * 4) as *const _);
                let wb = _mm512_loadu_si512(tile.qs.as_ptr().add(gb * TILE_N * 4) as *const _);
                let lo_a = _mm512_and_si512(wa, mask);
                let hi_a = _mm512_and_si512(_mm512_srli_epi16(wa, 4), mask);
                let lo_b = _mm512_and_si512(wb, mask);
                let hi_b = _mm512_and_si512(_mm512_srli_epi16(wb, 4), mask);
                for (m, act) in acts.iter().enumerate() {
                    isum_a[m] =
                        _mm512_dpbusd_epi32(isum_a[m], lo_a, bcast4(&act.qs, sa * 32 + g4 * 8));
                    isum_a[m] =
                        _mm512_dpbusd_epi32(isum_a[m], hi_a, bcast4(&act.qs, sa * 32 + g4 * 8 + 4));
                    isum_b[m] =
                        _mm512_dpbusd_epi32(isum_b[m], lo_b, bcast4(&act.qs, sb * 32 + g4 * 8));
                    isum_b[m] =
                        _mm512_dpbusd_epi32(isum_b[m], hi_b, bcast4(&act.qs, sb * 32 + g4 * 8 + 4));
                }
            }
            for (sub, isum) in [(sa, &isum_a), (sb, &isum_b)] {
                let scale = _mm512_loadu_ps(tile.scales[sub].as_ptr());
                let min = _mm512_loadu_ps(tile.mins[sub].as_ptr());
                for (m, act) in acts.iter().enumerate() {
                    let dall = act.d;
                    let f = _mm512_cvtepi32_ps(isum[m]);
                    acc[m][0] =
                        _mm512_fmadd_ps(f, _mm512_mul_ps(scale, _mm512_set1_ps(dall)), acc[m][0]);
                    let bsum = (act.bsums[sub * 2] as i32 + act.bsums[sub * 2 + 1] as i32) as f32;
                    acc[m][0] = _mm512_fnmadd_ps(min, _mm512_set1_ps(dall * bsum), acc[m][0]);
                }
            }
        }
    }

    // q6k: values are v-32; vpdpbusd on raw u6 then subtract 32 * bsum per 16-value subblock.
    #[target_feature(enable = "avx512f,avx512vnni,avx512bw")]
    pub(crate) unsafe fn q6k_tile(tile: &TileQ6K, acts: &[&BlockQ8K], acc: &mut [[__m512; 1]]) {
        for sub in 0..16 {
            let mut isum = [_mm512_setzero_si512(); MAX_MT];
            // subblock = 16 k-values = 4 k4-groups
            for g4 in 0..4 {
                let g = sub * 4 + g4;
                let w = _mm512_loadu_si512(tile.qs.as_ptr().add(g * TILE_N * 4) as *const _);
                for (m, act) in acts.iter().enumerate() {
                    let a = bcast4(&act.qs, sub * 16 + g4 * 4);
                    isum[m] = _mm512_dpbusd_epi32(isum[m], w, a);
                }
            }
            let scale = _mm512_loadu_ps(tile.scales[sub].as_ptr());
            for (m, act) in acts.iter().enumerate() {
                let dall = act.d;
                let bsum = act.bsums[sub] as f32;
                let f = _mm512_sub_ps(_mm512_cvtepi32_ps(isum[m]), _mm512_set1_ps(32.0 * bsum));
                acc[m][0] =
                    _mm512_fmadd_ps(f, _mm512_mul_ps(scale, _mm512_set1_ps(dall)), acc[m][0]);
            }
        }
    }

    // q8_0 (+128 offset weights), two 32-value block groups per call with independent
    // integer accumulators so the dpbusd latency chain never serializes at small m.
    #[target_feature(enable = "avx512f,avx512vnni,avx512bw")]
    pub(crate) unsafe fn q8_0_tile2(
        t0: &TileQ8_0,
        t1: &TileQ8_0,
        acts: &[&BlockQ8K],
        sub32: usize,
        acc: &mut [[__m512; 1]],
    ) {
        let mut isum_a = [_mm512_setzero_si512(); MAX_MT];
        let mut isum_b = [_mm512_setzero_si512(); MAX_MT];
        for g4 in 0..8 {
            let wa = _mm512_loadu_si512(t0.qs.as_ptr().add(g4 * TILE_N * 4) as *const _);
            let wb = _mm512_loadu_si512(t1.qs.as_ptr().add(g4 * TILE_N * 4) as *const _);
            for (m, act) in acts.iter().enumerate() {
                isum_a[m] =
                    _mm512_dpbusd_epi32(isum_a[m], wa, bcast4(&act.qs, sub32 * 32 + g4 * 4));
                isum_b[m] =
                    _mm512_dpbusd_epi32(isum_b[m], wb, bcast4(&act.qs, (sub32 + 1) * 32 + g4 * 4));
            }
        }
        for (blk, isum, tile) in [(sub32, &isum_a, t0), (sub32 + 1, &isum_b, t1)] {
            let dw = _mm512_cvtph_ps(_mm256_loadu_si256(tile.d.as_ptr() as *const _));
            for (m, act) in acts.iter().enumerate() {
                let bsum = (act.bsums[blk * 2] as i32 + act.bsums[blk * 2 + 1] as i32) as f32;
                let f = _mm512_sub_ps(_mm512_cvtepi32_ps(isum[m]), _mm512_set1_ps(128.0 * bsum));
                acc[m][0] = _mm512_fmadd_ps(f, _mm512_mul_ps(dw, _mm512_set1_ps(act.d)), acc[m][0]);
            }
        }
    }
}

// Full matmul over the packed tiles: lhs is m rows already quantized to BlockQ8K,
// parallelized over (m-tiles x n-tiles) on the barrier pool by the caller's chunker.
// Per-unit bodies live in #[target_feature] fns so intrinsics inline under portable
// (target-cpu=generic) release builds; closures cannot carry the attribute.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f,avx512vnni,avx512bw")]
#[allow(clippy::too_many_arguments)]
unsafe fn matmul_unit_512(
    packed: &PackedX86,
    lhs_q: &[BlockQ8K],
    kb: usize,
    nt: usize,
    m0: usize,
    mt: usize,
    dst_ptr: *mut f32,
    n: usize,
) {
    use core::arch::x86_64::*;
    let mut act_refs: [&BlockQ8K; kernels::MAX_MT] = [&lhs_q[m0 * kb]; kernels::MAX_MT];
    let mut acc = [[_mm512_setzero_ps(); 1]; kernels::MAX_MT];
    for b in 0..kb {
        for (i, a) in act_refs.iter_mut().enumerate().take(mt) {
            *a = &lhs_q[(m0 + i) * kb + b];
        }
        match packed {
            PackedX86::Q4K(tiles) => {
                kernels::q4k_tile(&tiles[nt * kb + b], &act_refs[..mt], &mut acc)
            }
            PackedX86::Q6K(tiles) => {
                kernels::q6k_tile(&tiles[nt * kb + b], &act_refs[..mt], &mut acc)
            }
            PackedX86::Q8_0(tiles) => {
                // q8_0 tiles cover 32 k-values each; 8 tiles per q8k superblock
                for s in (0..8).step_by(2) {
                    kernels::q8_0_tile2(
                        &tiles[nt * (kb * 8) + b * 8 + s],
                        &tiles[nt * (kb * 8) + b * 8 + s + 1],
                        &act_refs[..mt],
                        s,
                        &mut acc,
                    );
                }
            }
        }
    }
    for (i, row) in acc[..mt].iter().enumerate() {
        _mm512_storeu_ps(dst_ptr.add((m0 + i) * n + nt * TILE_N), row[0]);
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
#[allow(clippy::too_many_arguments)]
unsafe fn matmul_unit_256(
    packed: &PackedX86,
    lhs_q: &[BlockQ8K],
    kb: usize,
    nt: usize,
    m0: usize,
    mt: usize,
    dst_ptr: *mut f32,
    n: usize,
    vnni: bool,
) {
    use core::arch::x86_64::*;
    let mut act_refs: [&BlockQ8K; kernels::MAX_MT] = [&lhs_q[m0 * kb]; kernels::MAX_MT];
    let mut acc = [[_mm256_setzero_ps(); 2]; kernels::MAX_MT];
    for b in 0..kb {
        for (i, a) in act_refs.iter_mut().enumerate().take(mt) {
            *a = &lhs_q[(m0 + i) * kb + b];
        }
        match packed {
            PackedX86::Q4K(tiles) => {
                let t = &tiles[nt * kb + b];
                if vnni {
                    kernels256::q4k_tile_avxvnni(t, &act_refs[..mt], &mut acc)
                } else {
                    kernels256::q4k_tile_avx2(t, &act_refs[..mt], &mut acc)
                }
            }
            PackedX86::Q6K(tiles) => {
                let t = &tiles[nt * kb + b];
                if vnni {
                    kernels256::q6k_tile_avxvnni(t, &act_refs[..mt], &mut acc)
                } else {
                    kernels256::q6k_tile_avx2(t, &act_refs[..mt], &mut acc)
                }
            }
            PackedX86::Q8_0(tiles) => {
                for s in (0..8).step_by(2) {
                    kernels256::q8_0_tile2_avxvnni(
                        &tiles[nt * (kb * 8) + b * 8 + s],
                        &tiles[nt * (kb * 8) + b * 8 + s + 1],
                        &act_refs[..mt],
                        s,
                        &mut acc,
                    );
                }
            }
        }
    }
    for (i, row) in acc[..mt].iter().enumerate() {
        _mm256_storeu_ps(dst_ptr.add((m0 + i) * n + nt * TILE_N), row[0]);
        _mm256_storeu_ps(dst_ptr.add((m0 + i) * n + nt * TILE_N + 8), row[1]);
    }
}

pub(crate) fn matmul_tiles(
    packed: &PackedX86,
    lhs_q: &[BlockQ8K],
    m: usize,
    k: usize,
    n: usize,
    dst: &mut [f32],
) {
    #[cfg(target_arch = "x86_64")]
    {
        let lv = level().expect("select() gated");
        let kb = k / QK_K;
        let n_tiles = n / TILE_N;
        let mt_step = kernels::MAX_MT;
        let dst_ptr = dst.as_mut_ptr() as usize;
        let total_units = n_tiles * m.div_ceil(mt_step);
        let m_units = m.div_ceil(mt_step);
        crate::utils::barrier_pool().execute_chunked(total_units, |range| {
            let dst_ptr = dst_ptr as *mut f32;
            for unit in range {
                let nt = unit / m_units;
                let m0 = (unit % m_units) * mt_step;
                let mt = (m - m0).min(mt_step);
                unsafe {
                    if lv == X86Level::Avx512Vnni {
                        matmul_unit_512(packed, lhs_q, kb, nt, m0, mt, dst_ptr, n);
                    } else {
                        matmul_unit_256(
                            packed,
                            lhs_q,
                            kb,
                            nt,
                            m0,
                            mt,
                            dst_ptr,
                            n,
                            lv == X86Level::AvxVnni,
                        );
                    }
                }
            }
        });
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = (packed, lhs_q, m, k, n, dst);
        unreachable!("x86 repack kernels on non-x86 arch");
    }
}

pub(crate) fn pack(
    dtype: super::GgmlDType,
    storage: &dyn QuantizedType,
    n: usize,
    k: usize,
) -> PackedX86 {
    use super::GgmlDType as D;
    match dtype {
        D::Q4K => pack_q4k(storage, n, k),
        D::Q6K => pack_q6k(storage, n, k),
        D::Q8_0 => pack_q8_0(storage, n, k),
        _ => unreachable!("select() gates dtypes"),
    }
}

// Quantize lhs rows to BlockQ8K, parallelizing only when there are multiple rows.
pub(crate) fn quantize_lhs(lhs: &[f32], m: usize, k: usize) -> Vec<BlockQ8K> {
    let kb = k / QK_K;
    let mut out = vec![BlockQ8K::zeros(); m * kb];
    if m == 1 {
        BlockQ8K::from_float(&lhs[..k], &mut out);
        return out;
    }
    let out_ptr = out.as_mut_ptr() as usize;
    crate::utils::barrier_pool().execute_chunked(m, |range| {
        let out_ptr = out_ptr as *mut BlockQ8K;
        for r in range {
            let row = &lhs[r * k..(r + 1) * k];
            let dst = unsafe { std::slice::from_raw_parts_mut(out_ptr.add(r * kb), kb) };
            BlockQ8K::from_float(row, dst);
        }
    });
    out
}

// Fused multi-projection gemv: one lhs quantization and one barrier region across
// all projections sharing the same activations (qkv, gate/up). 512-bit only; the
// per-unit body reuses matmul_unit_512 so intrinsics inline in portable builds.
pub(crate) fn gemv_fused(
    parts: &[(&PackedX86, usize)],
    lhs_q: &[BlockQ8K],
    m: usize,
    k: usize,
    dsts: &mut [Vec<f32>],
) {
    #[cfg(target_arch = "x86_64")]
    {
        let kb = k / QK_K;
        let mt_step = kernels::MAX_MT;
        let m_units = m.div_ceil(mt_step);
        // (n, dst ptr, unit offset)
        let mut metas: Vec<(usize, usize, usize)> = Vec::with_capacity(parts.len());
        let mut total = 0usize;
        for (i, (_p, n)) in parts.iter().enumerate() {
            metas.push((*n, dsts[i].as_mut_ptr() as usize, total));
            total += (n / TILE_N) * m_units;
        }
        let parts_ref = parts;
        crate::utils::barrier_pool().execute_chunked(total, |range| {
            for unit in range {
                let pi = metas
                    .iter()
                    .rposition(|(_n, _d, off)| unit >= *off)
                    .unwrap();
                let (n, dst_ptr, off) = metas[pi];
                let local = unit - off;
                let nt = local / m_units;
                let m0 = (local % m_units) * mt_step;
                let mt = (m - m0).min(mt_step);
                unsafe {
                    matmul_unit_512(
                        parts_ref[pi].0,
                        lhs_q,
                        kb,
                        nt,
                        m0,
                        mt,
                        dst_ptr as *mut f32,
                        n,
                    );
                }
            }
        });
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = (parts, lhs_q, m, k, dsts);
        unreachable!("x86 repack kernels on non-x86 arch");
    }
}

// AMX-int8 path for large-m (prefill) q4k matmuls: weights unpacked to i8 in tile
// layout (B tile = 8 rows x 64 bytes = 32k x 16n VNNI-paired), activations repacked
// per 16-row block, tdpbssd accumulating 16x16 i32 per 32-k subblock so the q4k
// per-subblock scales apply exactly.
#[repr(C)]
pub(crate) struct TileQ4KAmx {
    pub(crate) scales: [[f32; TILE_N]; 8],
    pub(crate) mins: [[f32; TILE_N]; 8],
    // [subblock (8)][k4-row (8)][16 cols * 4 k] unsigned 4-bit values as i8
    pub(crate) qs: [u8; QK_K * TILE_N],
}

pub(crate) fn amx_available() -> bool {
    use std::sync::OnceLock;
    static OK: OnceLock<bool> = OnceLock::new();
    *OK.get_or_init(|| {
        // forced downlevel testing must exercise the 256-bit paths end to end
        if level() != Some(X86Level::Avx512Vnni) {
            return false;
        }
        // is_x86_feature_detected!("amx-int8") is unstable; read CPUID leaf 7 directly
        #[cfg(target_arch = "x86_64")]
        let has_amx = {
            // SAFETY: CPUID leaf 7 is a register query; it does not access memory.
            let info = unsafe { core::arch::x86_64::__cpuid_count(7, 0) };
            info.edx & (1 << 25) != 0 && info.edx & (1 << 24) != 0
        };
        #[cfg(not(target_arch = "x86_64"))]
        let has_amx = false;
        if !has_amx {
            return false;
        }
        // Linux gates AMX tile state behind a per-process permission request; other
        // OSes would need their own enablement, so AMX stays off there.
        #[cfg(target_os = "linux")]
        {
            const ARCH_REQ_XCOMP_PERM: i32 = 0x1023;
            const XFEATURE_XTILEDATA: u64 = 18;
            let r = unsafe {
                libc::syscall(
                    libc::SYS_arch_prctl,
                    ARCH_REQ_XCOMP_PERM,
                    XFEATURE_XTILEDATA,
                )
            };
            r == 0
        }
        #[cfg(not(target_os = "linux"))]
        false
    })
}

pub(crate) fn pack_q4k_amx(storage: &dyn QuantizedType, n: usize, k: usize) -> Vec<TileQ4KAmx> {
    let blocks = q4k_blocks(storage);
    let kb = k / QK_K;
    let n_tiles = n / TILE_N;
    let mut out: Vec<TileQ4KAmx> = Vec::with_capacity(n_tiles * kb);
    for t in 0..n_tiles {
        for b in 0..kb {
            let mut tile = TileQ4KAmx {
                scales: [[0.0; TILE_N]; 8],
                mins: [[0.0; TILE_N]; 8],
                qs: [0; QK_K * TILE_N],
            };
            for r in 0..TILE_N {
                let blk = &blocks[(t * TILE_N + r) * kb + b];
                let d = blk.d.to_f32();
                let dmin = blk.dmin.to_f32();
                for sub in 0..8 {
                    let (sc, mn) = get_scale_min_k4(sub, &blk.scales);
                    tile.scales[sub][r] = d * sc as f32;
                    tile.mins[sub][r] = dmin * mn as f32;
                }
                let mut vals = [0u8; QK_K];
                for c in 0..4 {
                    for j in 0..32 {
                        let byte = blk.qs[c * 32 + j];
                        vals[c * 64 + j] = byte & 0x0F;
                        vals[c * 64 + 32 + j] = byte >> 4;
                    }
                }
                for sub in 0..8 {
                    for krow in 0..8 {
                        for j in 0..4 {
                            tile.qs[((sub * 8 + krow) * TILE_N + r) * 4 + j] =
                                vals[sub * 32 + krow * 4 + j];
                        }
                    }
                }
            }
            out.push(tile);
        }
    }
    out
}

#[cfg(target_arch = "x86_64")]
mod amx {
    use super::*;
    use core::arch::x86_64::*;

    #[repr(C, align(64))]
    struct TileCfg {
        palette: u8,
        start_row: u8,
        _rsvd: [u8; 14],
        colsb: [u16; 16],
        rows: [u8; 16],
    }

    // tmm0 = C (16x16 i32), tmm1 = A (16 x 32 i8), tmm2 = B (8 x 64 i8)
    unsafe fn configure() {
        let mut cfg = TileCfg {
            palette: 1,
            start_row: 0,
            _rsvd: [0; 14],
            colsb: [0; 16],
            rows: [0; 16],
        };
        const _: () = assert!(std::mem::size_of::<TileCfg>() == 64);
        // tmm0-3: C (16x16 i32); tmm4-5: A (16x32 i8); tmm6-7: B (8x64 i8)
        for t in 0..4 {
            cfg.colsb[t] = 64;
            cfg.rows[t] = 16;
        }
        for t in 4..6 {
            cfg.colsb[t] = 32;
            cfg.rows[t] = 16;
        }
        for t in 6..8 {
            cfg.colsb[t] = 64;
            cfg.rows[t] = 8;
        }
        core::arch::asm!(
            "ldtilecfg [{cfg}]",
            cfg = in(reg) &cfg,
            options(nostack)
        );
    }

    thread_local! {
        static AMX_READY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }

    fn ensure_configured() {
        AMX_READY.with(|f| {
            if !f.get() {
                unsafe { configure() };
                f.set(true);
            }
        });
    }

    // One (m16-block, n-tile) product over the full k range.
    // a_pack: [kb][8 subs][16 rows * 32 bytes]; returns via dst rows of n floats.
    // 32m x 32n macro-tile: 4 C tiles per (2 A, 2 B) loads so tdp throughput is not
    // gated on tile loads. a_pack holds 32 m-rows (two 16-row halves interleaved by sub).
    #[target_feature(enable = "avx512f,avx512bw")]
    #[allow(clippy::too_many_arguments)]
    pub(super) unsafe fn q4k_m32(
        tiles: &[TileQ4KAmx],
        nt0: usize,
        n_tiles: usize,
        kb: usize,
        a_pack: &[i8],
        acts: &[&[BlockQ8K]],
        mt: usize,
        dst: *mut f32,
        n: usize,
    ) {
        ensure_configured();
        let two_n = nt0 + 1 < n_tiles;
        let mut c_scratch = [0i32; 4 * 16 * 16];
        let mut facc = [0f32; 32 * 32];
        let a_half = kb * 8 * 16 * 32;
        unsafe {
            for b in 0..kb {
                let t0 = &tiles[nt0 * kb + b];
                let t1 = &tiles[(nt0 + two_n as usize) * kb + b];
                for sub in 0..8 {
                    let a0 = a_pack.as_ptr().add((b * 8 + sub) * 16 * 32);
                    let a1 = a_pack.as_ptr().add(a_half + (b * 8 + sub) * 16 * 32);
                    let b0 = t0.qs.as_ptr().add(sub * 8 * TILE_N * 4);
                    let b1 = t1.qs.as_ptr().add(sub * 8 * TILE_N * 4);
                    core::arch::asm!(
                        "tilezero tmm0",
                        "tilezero tmm1",
                        "tilezero tmm2",
                        "tilezero tmm3",
                        "tileloadd tmm4, [{a0} + {sa}*1]",
                        "tileloadd tmm6, [{b0} + {sb}*1]",
                        "tdpbssd tmm0, tmm4, tmm6",
                        "tileloadd tmm7, [{b1} + {sb}*1]",
                        "tdpbssd tmm1, tmm4, tmm7",
                        "tileloadd tmm5, [{a1} + {sa}*1]",
                        "tdpbssd tmm2, tmm5, tmm6",
                        "tdpbssd tmm3, tmm5, tmm7",
                        "tilestored [{c} + {sc}*1], tmm0",
                        "tilestored [{c} + {sc}*1 + 1024], tmm1",
                        "tilestored [{c} + {sc}*1 + 2048], tmm2",
                        "tilestored [{c} + {sc}*1 + 3072], tmm3",
                        a0 = in(reg) a0,
                        a1 = in(reg) a1,
                        b0 = in(reg) b0,
                        b1 = in(reg) b1,
                        c = in(reg) c_scratch.as_mut_ptr(),
                        sa = in(reg) 32usize,
                        sb = in(reg) 64usize,
                        sc = in(reg) 64usize,
                        options(nostack)
                    );
                    for half in 0..2 {
                        let rows = if half == 0 {
                            mt.min(16)
                        } else {
                            mt.saturating_sub(16)
                        };
                        if rows == 0 {
                            continue;
                        }
                        for ntl in 0..1 + two_n as usize {
                            let tile = if ntl == 0 { t0 } else { t1 };
                            let scale = _mm512_loadu_ps(tile.scales[sub].as_ptr());
                            let min = _mm512_loadu_ps(tile.mins[sub].as_ptr());
                            let cbase = (half * 2 + ntl) * 256;
                            for mi in 0..rows {
                                let act = &acts[half * 16 + mi][b];
                                let dall = act.d;
                                let ci = _mm512_loadu_si512(
                                    c_scratch.as_ptr().add(cbase + mi * 16) as *const _,
                                );
                                let cf = _mm512_cvtepi32_ps(ci);
                                let fo = (half * 16 + mi) * 32 + ntl * 16;
                                let f = _mm512_loadu_ps(facc.as_ptr().add(fo));
                                let bsum = (act.bsums[sub * 2] as i32
                                    + act.bsums[sub * 2 + 1] as i32)
                                    as f32;
                                let f = _mm512_fmadd_ps(
                                    cf,
                                    _mm512_mul_ps(scale, _mm512_set1_ps(dall)),
                                    f,
                                );
                                let f = _mm512_fnmadd_ps(min, _mm512_set1_ps(dall * bsum), f);
                                _mm512_storeu_ps(facc.as_mut_ptr().add(fo), f);
                            }
                        }
                    }
                }
            }
            let n_cols = 16 * (1 + two_n as usize);
            for mi in 0..mt {
                std::ptr::copy_nonoverlapping(
                    facc.as_ptr().add(mi * 32),
                    dst.add(mi * n + nt0 * TILE_N),
                    n_cols,
                );
            }
        }
    }
}

// AMX matmul: m >= 32, q4k only. acts are per-row BlockQ8K slices.
pub(crate) fn matmul_amx_q4k(
    tiles: &[TileQ4KAmx],
    lhs_q: &[BlockQ8K],
    m: usize,
    k: usize,
    n: usize,
    dst: &mut [f32],
) {
    #[cfg(target_arch = "x86_64")]
    {
        let kb = k / QK_K;
        let n_tiles = n / TILE_N;
        let m_blocks = m.div_ceil(32);
        let dst_ptr = dst.as_mut_ptr() as usize;
        let lhs_ref = lhs_q;
        crate::utils::barrier_pool().execute_chunked(m_blocks, |range| {
            let dst_ptr = dst_ptr as *mut f32;
            // per-thread activation repack buffer (two 16-row halves), reused across n
            let a_half = kb * 8 * 16 * 32;
            let mut a_pack = vec![0i8; 2 * a_half];
            let mut act_rows: Vec<&[BlockQ8K]> = Vec::with_capacity(32);
            for mb in range {
                let m0 = mb * 32;
                let mt = (m - m0).min(32);
                act_rows.clear();
                for i in 0..mt {
                    act_rows.push(&lhs_ref[(m0 + i) * kb..(m0 + i + 1) * kb]);
                }
                a_pack.fill(0);
                for (i, row) in act_rows.iter().enumerate() {
                    let base = (i / 16) * a_half;
                    let r = i % 16;
                    for b in 0..kb {
                        let qs = &row[b].qs;
                        for sub in 0..8 {
                            let dst_off = base + ((b * 8 + sub) * 16 + r) * 32;
                            a_pack[dst_off..dst_off + 32]
                                .copy_from_slice(&qs[sub * 32..sub * 32 + 32]);
                        }
                    }
                }
                let mut nt = 0;
                while nt < n_tiles {
                    unsafe {
                        amx::q4k_m32(
                            tiles,
                            nt,
                            n_tiles,
                            kb,
                            &a_pack,
                            &act_rows,
                            mt,
                            dst_ptr.add(m0 * n),
                            n,
                        )
                    };
                    nt += 2;
                }
            }
        });
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = (tiles, lhs_q, m, k, n, dst);
        unreachable!()
    }
}

// 256-bit kernel tier for CPUs without AVX512: AVX-VNNI (vpdpbusd ymm) or pure AVX2
// (maddubs+madd, safe for the u4/u6 x i8 operand ranges). Same tile layout, each
// 16-column group processed as two ymm halves.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum X86Level {
    Avx512Vnni,
    AvxVnni,
    Avx2,
}

pub(crate) fn level() -> Option<X86Level> {
    use std::sync::OnceLock;
    static LEVEL: OnceLock<Option<X86Level>> = OnceLock::new();
    *LEVEL.get_or_init(|| {
        let force_avx2 = std::env::var("MISTRALRS_FORCE_AVX2").as_deref() == Ok("1");
        let force_vnni = std::env::var("MISTRALRS_FORCE_AVXVNNI").as_deref() == Ok("1");
        if !force_avx2
            && !force_vnni
            && is_x86_feature_detected!("avx512f")
            && is_x86_feature_detected!("avx512vnni")
        {
            return Some(X86Level::Avx512Vnni);
        }
        if !force_avx2
            && (force_vnni || is_x86_feature_detected!("avxvnni"))
            && is_x86_feature_detected!("avx2")
        {
            return Some(X86Level::AvxVnni);
        }
        if is_x86_feature_detected!("avx2") && is_x86_feature_detected!("fma") {
            return Some(X86Level::Avx2);
        }
        None
    })
}

#[cfg(target_arch = "x86_64")]
mod kernels256 {
    use super::kernels::MAX_MT;
    use super::*;
    use core::arch::x86_64::*;

    #[target_feature(enable = "avx2")]
    unsafe fn bcast4_256(act: &[i8], off: usize) -> __m256i {
        _mm256_set1_epi32(std::ptr::read_unaligned(act.as_ptr().add(off) as *const i32))
    }

    #[target_feature(enable = "avx2,avxvnni")]
    unsafe fn dp_vnni(acc: __m256i, w_u8: __m256i, a_i8: __m256i) -> __m256i {
        _mm256_dpbusd_avx_epi32(acc, w_u8, a_i8)
    }

    // dpbusd stand-in for pure AVX2: maddubs (u8 x i8 -> i16 pairs) then madd against ones.
    #[target_feature(enable = "avx2")]
    unsafe fn dp_avx2(acc: __m256i, w_u8: __m256i, a_i8: __m256i) -> __m256i {
        let prod16 = _mm256_maddubs_epi16(w_u8, a_i8);
        _mm256_add_epi32(acc, _mm256_madd_epi16(prod16, _mm256_set1_epi16(1)))
    }

    macro_rules! q4k_tile_256 {
        ($fname:ident, $dp:path, $feat:literal) => {
            #[target_feature(enable = $feat)]
            pub(crate) unsafe fn $fname(
                tile: &TileQ4K,
                acts: &[&BlockQ8K],
                acc: &mut [[__m256; 2]],
            ) {
                let mask = _mm256_set1_epi8(0x0F_u8 as i8);
                for sub in 0..8 {
                    let mut isum = [[_mm256_setzero_si256(); 2]; MAX_MT];
                    for g4 in 0..4 {
                        let g = sub * 4 + g4;
                        for h in 0..2 {
                            let w = _mm256_loadu_si256(
                                tile.qs.as_ptr().add(g * TILE_N * 4 + h * 32) as *const _,
                            );
                            let lo = _mm256_and_si256(w, mask);
                            let hi = _mm256_and_si256(_mm256_srli_epi16(w, 4), mask);
                            for (m, act) in acts.iter().enumerate() {
                                let a_lo = bcast4_256(&act.qs, sub * 32 + g4 * 8);
                                let a_hi = bcast4_256(&act.qs, sub * 32 + g4 * 8 + 4);
                                isum[m][h] = $dp(isum[m][h], lo, a_lo);
                                isum[m][h] = $dp(isum[m][h], hi, a_hi);
                            }
                        }
                    }
                    for h in 0..2 {
                        let scale = _mm256_loadu_ps(tile.scales[sub].as_ptr().add(h * 8));
                        let min = _mm256_loadu_ps(tile.mins[sub].as_ptr().add(h * 8));
                        for (m, act) in acts.iter().enumerate() {
                            let dall = act.d;
                            let f = _mm256_cvtepi32_ps(isum[m][h]);
                            acc[m][h] = _mm256_fmadd_ps(
                                f,
                                _mm256_mul_ps(scale, _mm256_set1_ps(dall)),
                                acc[m][h],
                            );
                            let bsum =
                                (act.bsums[sub * 2] as i32 + act.bsums[sub * 2 + 1] as i32) as f32;
                            acc[m][h] =
                                _mm256_fnmadd_ps(min, _mm256_set1_ps(dall * bsum), acc[m][h]);
                        }
                    }
                }
            }
        };
    }
    q4k_tile_256!(q4k_tile_avxvnni, dp_vnni, "avx2,avxvnni,fma");
    q4k_tile_256!(q4k_tile_avx2, dp_avx2, "avx2,fma");

    macro_rules! q6k_tile_256 {
        ($fname:ident, $dp:path, $feat:literal) => {
            #[target_feature(enable = $feat)]
            pub(crate) unsafe fn $fname(
                tile: &TileQ6K,
                acts: &[&BlockQ8K],
                acc: &mut [[__m256; 2]],
            ) {
                for sub in 0..16 {
                    let mut isum = [[_mm256_setzero_si256(); 2]; MAX_MT];
                    for g4 in 0..4 {
                        let g = sub * 4 + g4;
                        for h in 0..2 {
                            let w = _mm256_loadu_si256(
                                tile.qs.as_ptr().add(g * TILE_N * 4 + h * 32) as *const _,
                            );
                            for (m, act) in acts.iter().enumerate() {
                                let a = bcast4_256(&act.qs, sub * 16 + g4 * 4);
                                isum[m][h] = $dp(isum[m][h], w, a);
                            }
                        }
                    }
                    for h in 0..2 {
                        let scale = _mm256_loadu_ps(tile.scales[sub].as_ptr().add(h * 8));
                        for (m, act) in acts.iter().enumerate() {
                            let dall = act.d;
                            let bsum = act.bsums[sub] as f32;
                            let f = _mm256_sub_ps(
                                _mm256_cvtepi32_ps(isum[m][h]),
                                _mm256_set1_ps(32.0 * bsum),
                            );
                            acc[m][h] = _mm256_fmadd_ps(
                                f,
                                _mm256_mul_ps(scale, _mm256_set1_ps(dall)),
                                acc[m][h],
                            );
                        }
                    }
                }
            }
        };
    }
    q6k_tile_256!(q6k_tile_avxvnni, dp_vnni, "avx2,avxvnni,fma");
    q6k_tile_256!(q6k_tile_avx2, dp_avx2, "avx2,fma");

    // q8_0 needs true dpbusd (u8 range overflows maddubs' i16 pairs), so vnni only.
    #[target_feature(enable = "avx2,avxvnni,fma")]
    pub(crate) unsafe fn q8_0_tile2_avxvnni(
        t0: &TileQ8_0,
        t1: &TileQ8_0,
        acts: &[&BlockQ8K],
        sub32: usize,
        acc: &mut [[__m256; 2]],
    ) {
        let mut isum_a = [[_mm256_setzero_si256(); 2]; MAX_MT];
        let mut isum_b = [[_mm256_setzero_si256(); 2]; MAX_MT];
        for g4 in 0..8 {
            for h in 0..2 {
                let wa =
                    _mm256_loadu_si256(t0.qs.as_ptr().add(g4 * TILE_N * 4 + h * 32) as *const _);
                let wb =
                    _mm256_loadu_si256(t1.qs.as_ptr().add(g4 * TILE_N * 4 + h * 32) as *const _);
                for (m, act) in acts.iter().enumerate() {
                    isum_a[m][h] = _mm256_dpbusd_avx_epi32(
                        isum_a[m][h],
                        wa,
                        bcast4_256(&act.qs, sub32 * 32 + g4 * 4),
                    );
                    isum_b[m][h] = _mm256_dpbusd_avx_epi32(
                        isum_b[m][h],
                        wb,
                        bcast4_256(&act.qs, (sub32 + 1) * 32 + g4 * 4),
                    );
                }
            }
        }
        for (blk, isum, tile) in [(sub32, &isum_a, t0), (sub32 + 1, &isum_b, t1)] {
            for h in 0..2 {
                let dw = _mm256_cvtph_ps(_mm_loadu_si128(tile.d.as_ptr().add(h * 8) as *const _));
                for (m, act) in acts.iter().enumerate() {
                    let bsum = (act.bsums[blk * 2] as i32 + act.bsums[blk * 2 + 1] as i32) as f32;
                    let f =
                        _mm256_sub_ps(_mm256_cvtepi32_ps(isum[m][h]), _mm256_set1_ps(128.0 * bsum));
                    acc[m][h] =
                        _mm256_fmadd_ps(f, _mm256_mul_ps(dw, _mm256_set1_ps(act.d)), acc[m][h]);
                }
            }
        }
    }
}
