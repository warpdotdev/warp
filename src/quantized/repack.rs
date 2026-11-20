#[cfg(target_arch = "aarch64")]
use super::GgmlDType;
use super::QuantizedType;
use crate::Result;
use std::sync::OnceLock;

#[cfg(target_arch = "aarch64")]
use super::k_quants::{BlockQ4K, BlockQ4_0, QK4_0};
#[cfg(target_arch = "aarch64")]
use super::k_quants::{BlockQ5K, BlockQ6K, QK_K};
#[cfg(target_arch = "aarch64")]
use super::k_quants::{BlockQ8_0, QK8_0};

#[cfg(target_arch = "aarch64")]
use half::f16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg(target_arch = "aarch64")]
enum PackedKind {
    Q4_0x4,
    Q4Kx8,
    Q5Kx8,
    Q6Kx8,
    Q8_0x4,
}

#[cfg(target_arch = "aarch64")]
pub(crate) enum PackedStorage {
    Q4_0x4(Vec<BlockQ4_0x4>),
    Q4Kx8(Vec<BlockQ4Kx8>),
    Q5Kx8(Vec<BlockQ5Kx8>),
    Q6Kx8(Vec<BlockQ6Kx8>),
    Q8_0x4(Vec<BlockQ8_0x4>),
}

#[allow(dead_code)]
pub(crate) struct PackedCache {
    #[cfg(target_arch = "x86_64")]
    x86: OnceLock<super::repack_x86::PackedX86>,
    #[cfg(target_arch = "x86_64")]
    x86_amx: OnceLock<Vec<super::repack_x86::TileQ4KAmx>>,
    #[cfg(target_arch = "aarch64")]
    q4_0x4: OnceLock<PackedStorage>,
    #[cfg(target_arch = "aarch64")]
    q4kx8: OnceLock<PackedStorage>,
    #[cfg(target_arch = "aarch64")]
    q5kx8: OnceLock<PackedStorage>,
    #[cfg(target_arch = "aarch64")]
    q6kx8: OnceLock<PackedStorage>,
    #[cfg(target_arch = "aarch64")]
    q8_0x4: OnceLock<PackedStorage>,
}

impl PackedCache {
    #[cfg(target_arch = "x86_64")]
    pub(crate) fn x86_get_or_init(
        &self,
        init: impl FnOnce() -> super::repack_x86::PackedX86,
    ) -> &super::repack_x86::PackedX86 {
        self.x86.get_or_init(init)
    }

    pub(crate) fn new() -> Self {
        Self {
            #[cfg(target_arch = "x86_64")]
            x86: OnceLock::new(),
            #[cfg(target_arch = "x86_64")]
            x86_amx: OnceLock::new(),
            #[cfg(target_arch = "aarch64")]
            q4_0x4: OnceLock::new(),
            #[cfg(target_arch = "aarch64")]
            q4kx8: OnceLock::new(),
            #[cfg(target_arch = "aarch64")]
            q5kx8: OnceLock::new(),
            #[cfg(target_arch = "aarch64")]
            q6kx8: OnceLock::new(),
            #[cfg(target_arch = "aarch64")]
            q8_0x4: OnceLock::new(),
        }
    }

    #[cfg(target_arch = "aarch64")]
    fn get_or_init(
        &self,
        kind: PackedKind,
        init: impl FnOnce() -> PackedStorage,
    ) -> &PackedStorage {
        match kind {
            #[cfg(target_arch = "aarch64")]
            PackedKind::Q4_0x4 => self.q4_0x4.get_or_init(init),
            #[cfg(target_arch = "aarch64")]
            PackedKind::Q4Kx8 => self.q4kx8.get_or_init(init),
            #[cfg(target_arch = "aarch64")]
            PackedKind::Q5Kx8 => self.q5kx8.get_or_init(init),
            #[cfg(target_arch = "aarch64")]
            PackedKind::Q6Kx8 => self.q6kx8.get_or_init(init),
            #[cfg(target_arch = "aarch64")]
            PackedKind::Q8_0x4 => self.q8_0x4.get_or_init(init),
        }
    }
}

#[cfg(target_arch = "aarch64")]
#[derive(
    Clone,
    Copy,
    zerocopy::FromBytes,
    zerocopy::IntoBytes,
    zerocopy::KnownLayout,
    zerocopy::Immutable,
)]
#[repr(C)]
pub(crate) struct BlockQ8_0x4 {
    pub(crate) d: [f16; 4],
    pub(crate) qs: [i8; QK8_0 * 4],
}

#[cfg(target_arch = "aarch64")]
const _: () = assert!(std::mem::size_of::<BlockQ8_0x4>() == 136);

#[cfg(target_arch = "aarch64")]
#[derive(
    Clone,
    Copy,
    zerocopy::FromBytes,
    zerocopy::IntoBytes,
    zerocopy::KnownLayout,
    zerocopy::Immutable,
)]
#[repr(C)]
pub(crate) struct BlockQ4_0x4 {
    pub(crate) d: [f16; 4],
    pub(crate) qs: [i8; QK4_0 * 2],
}

#[cfg(target_arch = "aarch64")]
const _: () = assert!(std::mem::size_of::<BlockQ4_0x4>() == 72);

#[cfg(target_arch = "aarch64")]
#[derive(
    Clone,
    Copy,
    zerocopy::FromBytes,
    zerocopy::IntoBytes,
    zerocopy::KnownLayout,
    zerocopy::Immutable,
)]
#[repr(C)]
pub(crate) struct BlockQ4Kx8 {
    pub(crate) d: [f16; 8],
    pub(crate) dmin: [f16; 8],
    pub(crate) scales: [u8; 96],
    pub(crate) qs: [u8; 1024],
}

#[cfg(target_arch = "aarch64")]
const _: () = assert!(std::mem::size_of::<BlockQ4Kx8>() == 1152);

#[cfg(target_arch = "aarch64")]
#[derive(
    Clone,
    Copy,
    zerocopy::FromBytes,
    zerocopy::IntoBytes,
    zerocopy::KnownLayout,
    zerocopy::Immutable,
)]
#[repr(C)]
pub(crate) struct BlockQ5Kx8 {
    pub(crate) d: [f16; 8],
    pub(crate) dmin: [f16; 8],
    pub(crate) scales: [u8; 96],
    pub(crate) qh: [u8; QK_K],
    pub(crate) qs: [u8; QK_K / 2 * 8],
}

#[cfg(target_arch = "aarch64")]
const _: () = assert!(std::mem::size_of::<BlockQ5Kx8>() == 1408);

#[cfg(target_arch = "aarch64")]
#[derive(
    Clone,
    Copy,
    zerocopy::FromBytes,
    zerocopy::IntoBytes,
    zerocopy::KnownLayout,
    zerocopy::Immutable,
)]
#[repr(C)]
pub(crate) struct BlockQ6Kx8 {
    pub(crate) d: [f16; 8],
    pub(crate) scales: [i8; QK_K / 16 * 8],
    pub(crate) ql: [u8; QK_K / 2 * 8],
    pub(crate) qh: [u8; QK_K / 4 * 8],
}

#[cfg(target_arch = "aarch64")]
const _: () = assert!(std::mem::size_of::<BlockQ6Kx8>() == 1680);

#[cfg(target_arch = "aarch64")]
#[derive(
    Clone,
    Copy,
    zerocopy::FromBytes,
    zerocopy::IntoBytes,
    zerocopy::KnownLayout,
    zerocopy::Immutable,
)]
#[repr(C)]
pub(crate) struct BlockQ8Kx4 {
    pub(crate) d: [f32; 4],
    pub(crate) qs: [i8; QK_K * 4],
    pub(crate) bsums: [i16; QK_K / 4],
}

#[cfg(target_arch = "aarch64")]
const _: () = assert!(std::mem::size_of::<BlockQ8Kx4>() == 1168);

pub(crate) fn try_matmul_f32(
    storage: &dyn QuantizedType,
    packed: &PackedCache,
    mkn: (usize, usize, usize),
    lhs: &[f32],
    dst: &mut [f32],
) -> Result<bool> {
    #[cfg(target_arch = "aarch64")]
    {
        let (m, k, n) = mkn;
        let dtype = storage.dtype();
        if let Some(kind) = PackedKind::select(dtype, mkn) {
            let packed = packed.get_or_init(kind, || kind.pack(storage, n));
            kind.matmul(mkn, lhs, packed, dst)?;
            return Ok(true);
        }
        // tiled kernels need m % 4 == 0: run the aligned rows tiled and the tail rows as
        // gemvs on the same packing instead of dropping the whole matmul to the unpacked path
        let Some(kind) = PackedKind::select(dtype, (1, k, n)) else {
            return Ok(false);
        };
        let m_tiled = m & !3;
        if m_tiled > 0 && PackedKind::select(dtype, (m_tiled, k, n)) != Some(kind) {
            return Ok(false);
        }
        let packed = packed.get_or_init(kind, || kind.pack(storage, n));
        let (lhs_tiled, lhs_tail) = lhs.split_at(m_tiled * k);
        let (dst_tiled, dst_tail) = dst.split_at_mut(m_tiled * n);
        if m_tiled > 0 {
            kind.matmul((m_tiled, k, n), lhs_tiled, packed, dst_tiled)?;
        }
        for (lhs_row, dst_row) in lhs_tail.chunks_exact(k).zip(dst_tail.chunks_exact_mut(n)) {
            kind.matmul((1, k, n), lhs_row, packed, dst_row)?;
        }
        Ok(true)
    }

    #[cfg(target_arch = "x86_64")]
    {
        let (m, k, n) = mkn;
        if !super::repack_x86::select(storage.dtype(), m, n, k) {
            return Ok(false);
        }
        let dtype = storage.dtype();
        let lhs_q = super::repack_x86::quantize_lhs(lhs, m, k);
        if m >= 32 && dtype == super::GgmlDType::Q4K && super::repack_x86::amx_available() {
            let tiles = packed
                .x86_amx
                .get_or_init(|| super::repack_x86::pack_q4k_amx(storage, n, k));
            super::repack_x86::matmul_amx_q4k(tiles, &lhs_q, m, k, n, dst);
            return Ok(true);
        }
        let tiles = packed
            .x86
            .get_or_init(|| super::repack_x86::pack(dtype, storage, n, k));
        super::repack_x86::matmul_tiles(tiles, &lhs_q, m, k, n, dst);
        Ok(true)
    }

    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
    {
        let _ = (storage, packed, mkn, lhs, dst);
        Ok(false)
    }
}

#[cfg(target_arch = "aarch64")]
impl PackedKind {
    fn select(dtype: GgmlDType, (m, _k, n): (usize, usize, usize)) -> Option<Self> {
        let features = crate::cpu::features::get();
        let has_dotprod_gemv = features.dotprod && m == 1;
        let has_tiled_matmul = (features.dotprod || features.i8mm) && m >= 4 && m.is_multiple_of(4);
        match dtype {
            GgmlDType::Q4_0 if n.is_multiple_of(4) && (has_dotprod_gemv || has_tiled_matmul) => {
                Some(Self::Q4_0x4)
            }
            GgmlDType::Q4K if n.is_multiple_of(8) && (has_dotprod_gemv || has_tiled_matmul) => {
                Some(Self::Q4Kx8)
            }
            GgmlDType::Q5K if n.is_multiple_of(8) && (has_dotprod_gemv || has_tiled_matmul) => {
                Some(Self::Q5Kx8)
            }
            GgmlDType::Q6K if n.is_multiple_of(8) && (has_dotprod_gemv || has_tiled_matmul) => {
                Some(Self::Q6Kx8)
            }
            GgmlDType::Q8_0 if n.is_multiple_of(4) && (has_dotprod_gemv || has_tiled_matmul) => {
                Some(Self::Q8_0x4)
            }
            _ => None,
        }
    }

    fn pack(self, storage: &dyn QuantizedType, n: usize) -> PackedStorage {
        let interleave = if crate::cpu::features::get().i8mm {
            8
        } else {
            4
        };
        match self {
            #[cfg(target_arch = "aarch64")]
            Self::Q4_0x4 => {
                let total_blocks =
                    storage.storage_size_in_bytes() / std::mem::size_of::<BlockQ4_0>();
                let blocks = unsafe {
                    std::slice::from_raw_parts(storage.as_ptr() as *const BlockQ4_0, total_blocks)
                };
                PackedStorage::Q4_0x4(pack_to_q4_0x4(blocks, n, interleave))
            }
            #[cfg(target_arch = "aarch64")]
            Self::Q4Kx8 => {
                let total_blocks =
                    storage.storage_size_in_bytes() / std::mem::size_of::<BlockQ4K>();
                let blocks = unsafe {
                    std::slice::from_raw_parts(storage.as_ptr() as *const BlockQ4K, total_blocks)
                };
                PackedStorage::Q4Kx8(pack_to_q4kx8(blocks, n))
            }
            #[cfg(target_arch = "aarch64")]
            Self::Q5Kx8 => {
                let total_blocks =
                    storage.storage_size_in_bytes() / std::mem::size_of::<BlockQ5K>();
                let blocks = unsafe {
                    std::slice::from_raw_parts(storage.as_ptr() as *const BlockQ5K, total_blocks)
                };
                PackedStorage::Q5Kx8(pack_to_q5kx8(blocks, n, interleave))
            }
            #[cfg(target_arch = "aarch64")]
            Self::Q6Kx8 => {
                let total_blocks =
                    storage.storage_size_in_bytes() / std::mem::size_of::<BlockQ6K>();
                let blocks = unsafe {
                    std::slice::from_raw_parts(storage.as_ptr() as *const BlockQ6K, total_blocks)
                };
                PackedStorage::Q6Kx8(pack_to_q6kx8(blocks, n, interleave))
            }
            #[cfg(target_arch = "aarch64")]
            Self::Q8_0x4 => {
                let total_blocks =
                    storage.storage_size_in_bytes() / std::mem::size_of::<BlockQ8_0>();
                let blocks = unsafe {
                    std::slice::from_raw_parts(storage.as_ptr() as *const BlockQ8_0, total_blocks)
                };
                PackedStorage::Q8_0x4(pack_to_q8_0x4(blocks, n, interleave))
            }
        }
    }

    // matmul against one expert's row block inside stacked packed storage; subslices
    // per arm so nothing is copied
    #[cfg(target_arch = "aarch64")]
    fn matmul_expert_range(
        self,
        mkn: (usize, usize, usize),
        lhs: &[f32],
        packed: &PackedStorage,
        expert: usize,
        dst: &mut [f32],
    ) -> Result<()> {
        let (_m, k, n_out) = mkn;
        let features = crate::cpu::features::get();
        macro_rules! sub {
            ($v:expr, $width:expr, $qk:expr) => {{
                let kb = k / $qk;
                let groups = n_out / $width;
                &$v[expert * groups * kb..(expert + 1) * groups * kb]
            }};
        }
        match (self, packed) {
            (Self::Q4Kx8, PackedStorage::Q4Kx8(v)) => {
                let p = sub!(v, 8, QK_K);
                if features.i8mm && mkn.0 >= 4 && mkn.0.is_multiple_of(4) {
                    unsafe { super::neon::matmul_q4k_x8_i8mm(mkn, lhs, p, dst) }
                } else {
                    unsafe { super::k_quants::matmul_q4k_x8(mkn, lhs, p, dst) }
                }
            }
            (Self::Q6Kx8, PackedStorage::Q6Kx8(v)) => {
                super::neon::matmul_q6k_x8(mkn, lhs, sub!(v, 8, QK_K), dst)
            }
            (Self::Q8_0x4, PackedStorage::Q8_0x4(v)) => {
                let p = sub!(v, 4, QK8_0);
                if features.i8mm && mkn.0 >= 4 && mkn.0.is_multiple_of(4) {
                    unsafe { super::neon::matmul_q8_0_x4_i8mm(mkn, lhs, p, dst) }
                } else {
                    unsafe { super::neon::matmul_q8_0_x4(mkn, lhs, p, dst) }
                }
            }
            _ => crate::bail!("unsupported expert-range kind"),
        }
    }

    fn matmul(
        self,
        mkn: (usize, usize, usize),
        lhs: &[f32],
        packed: &PackedStorage,
        dst: &mut [f32],
    ) -> Result<()> {
        let features = crate::cpu::features::get();
        match (self, packed) {
            #[cfg(target_arch = "aarch64")]
            (Self::Q4_0x4, PackedStorage::Q4_0x4(packed)) => {
                if features.i8mm && mkn.0 >= 4 {
                    unsafe { super::neon::matmul_q4_0_x4_i8mm(mkn, lhs, packed, dst) }
                } else {
                    unsafe { super::neon::matmul_q4_0_x4(mkn, lhs, packed, dst) }
                }
            }
            #[cfg(target_arch = "aarch64")]
            (Self::Q4Kx8, PackedStorage::Q4Kx8(packed)) => {
                if features.i8mm && mkn.0 >= 4 {
                    unsafe { super::neon::matmul_q4k_x8_i8mm(mkn, lhs, packed, dst) }
                } else {
                    unsafe { super::k_quants::matmul_q4k_x8(mkn, lhs, packed, dst) }
                }
            }
            #[cfg(target_arch = "aarch64")]
            (Self::Q5Kx8, PackedStorage::Q5Kx8(packed)) => {
                super::neon::matmul_q5k_x8(mkn, lhs, packed, dst)
            }
            #[cfg(target_arch = "aarch64")]
            (Self::Q6Kx8, PackedStorage::Q6Kx8(packed)) => {
                super::neon::matmul_q6k_x8(mkn, lhs, packed, dst)
            }
            #[cfg(target_arch = "aarch64")]
            (Self::Q8_0x4, PackedStorage::Q8_0x4(packed)) => {
                if features.i8mm && mkn.0 >= 4 {
                    unsafe { super::neon::matmul_q8_0_x4_i8mm(mkn, lhs, packed, dst) }
                } else {
                    unsafe { super::neon::matmul_q8_0_x4(mkn, lhs, packed, dst) }
                }
            }
            #[allow(unreachable_patterns)]
            _ => crate::bail!("packed storage kind mismatch"),
        }
    }
}

#[cfg(target_arch = "aarch64")]
fn pack_to_q4kx8(blocks: &[BlockQ4K], n: usize) -> Vec<BlockQ4Kx8> {
    debug_assert!(n.is_multiple_of(8));
    debug_assert_eq!(blocks.len() % n, 0);
    let k_blocks = blocks.len() / n;
    let n_groups = n / 8;
    let mut packed = Vec::with_capacity(n_groups * k_blocks);
    for g in 0..n_groups {
        for b in 0..k_blocks {
            let src: [&BlockQ4K; 8] = std::array::from_fn(|i| &blocks[(g * 8 + i) * k_blocks + b]);
            let mut p = BlockQ4Kx8 {
                d: [f16::ZERO; 8],
                dmin: [f16::ZERO; 8],
                scales: [0; 96],
                qs: [0; 1024],
            };
            for (i, s) in src.iter().enumerate() {
                p.d[i] = s.d;
                p.dmin[i] = s.dmin;
            }
            for i in 0..128usize {
                let src_id = i % 8;
                let src_offset = (i / 8) * 8;
                let dst_offset = i * 8;
                p.qs[dst_offset..dst_offset + 8]
                    .copy_from_slice(&src[src_id].qs[src_offset..src_offset + 8]);
            }
            pack_k_scale_min_x8(&src.map(|s| &s.scales), &mut p.scales);
            packed.push(p);
        }
    }
    packed
}

#[cfg(target_arch = "aarch64")]
fn pack_to_q5kx8(blocks: &[BlockQ5K], n: usize, block_len: usize) -> Vec<BlockQ5Kx8> {
    debug_assert!(n.is_multiple_of(8));
    debug_assert!((QK_K / 2).is_multiple_of(block_len));
    debug_assert!((QK_K / 8).is_multiple_of(block_len));
    debug_assert_eq!(blocks.len() % n, 0);
    let k_blocks = blocks.len() / n;
    let n_groups = n / 8;
    let mut packed = Vec::with_capacity(n_groups * k_blocks);
    for g in 0..n_groups {
        for b in 0..k_blocks {
            let src: [&BlockQ5K; 8] = std::array::from_fn(|i| &blocks[(g * 8 + i) * k_blocks + b]);
            let mut p = BlockQ5Kx8 {
                d: [f16::ZERO; 8],
                dmin: [f16::ZERO; 8],
                scales: [0; 96],
                qh: [0; QK_K],
                qs: [0; QK_K / 2 * 8],
            };
            for (i, s) in src.iter().enumerate() {
                p.d[i] = s.d;
                p.dmin[i] = s.dmin;
            }
            for i in 0..(QK_K / 2 * 8) / block_len {
                let src_id = i % 8;
                let src_offset = (i / 8) * block_len;
                let dst_offset = i * block_len;
                p.qs[dst_offset..dst_offset + block_len]
                    .copy_from_slice(&src[src_id].qs[src_offset..src_offset + block_len]);
            }
            for i in 0..(QK_K / 8 * 8) / block_len {
                let src_id = i % 8;
                let src_offset = (i / 8) * block_len;
                let dst_offset = i * block_len;
                p.qh[dst_offset..dst_offset + block_len]
                    .copy_from_slice(&src[src_id].qh[src_offset..src_offset + block_len]);
            }
            pack_k_scale_min_x8(&src.map(|s| &s.scales), &mut p.scales);
            packed.push(p);
        }
    }
    packed
}

#[cfg(target_arch = "aarch64")]
fn pack_to_q6kx8(blocks: &[BlockQ6K], n: usize, block_len: usize) -> Vec<BlockQ6Kx8> {
    debug_assert!(n.is_multiple_of(8));
    debug_assert!((QK_K / 2).is_multiple_of(block_len));
    debug_assert!((QK_K / 4).is_multiple_of(block_len));
    debug_assert_eq!(blocks.len() % n, 0);
    let k_blocks = blocks.len() / n;
    let n_groups = n / 8;
    let mut packed = Vec::with_capacity(n_groups * k_blocks);
    for g in 0..n_groups {
        for b in 0..k_blocks {
            let src: [&BlockQ6K; 8] = std::array::from_fn(|i| &blocks[(g * 8 + i) * k_blocks + b]);
            let mut p = BlockQ6Kx8 {
                d: [f16::ZERO; 8],
                scales: [0; QK_K / 16 * 8],
                ql: [0; QK_K / 2 * 8],
                qh: [0; QK_K / 4 * 8],
            };
            for (i, s) in src.iter().enumerate() {
                p.d[i] = s.d;
            }
            for i in 0..(QK_K / 2 * 8) / block_len {
                let src_id = i % 8;
                let src_offset = (i / 8) * block_len;
                let dst_offset = i * block_len;
                p.ql[dst_offset..dst_offset + block_len]
                    .copy_from_slice(&src[src_id].ql[src_offset..src_offset + block_len]);
            }
            for i in 0..(QK_K / 4 * 8) / block_len {
                let src_id = i % 8;
                let src_offset = (i / 8) * block_len;
                let dst_offset = i * block_len;
                p.qh[dst_offset..dst_offset + block_len]
                    .copy_from_slice(&src[src_id].qh[src_offset..src_offset + block_len]);
            }
            for (col, s) in src.iter().enumerate() {
                for j in 0..QK_K / 16 {
                    p.scales[j * 8 + col] = s.scales[j];
                }
            }
            packed.push(p);
        }
    }
    packed
}

#[cfg(target_arch = "aarch64")]
fn pack_k_scale_min_x8(src: &[&[u8; 12]; 8], dst: &mut [u8; 96]) {
    for i in 0..4usize {
        let mut s = [0u8; 8];
        let mut m = [0u8; 8];
        for j in 0..8 {
            s[j] = src[j][i] & 63;
            m[j] = src[j][i + 4] & 63;
        }
        let b12 = i * 12;
        dst[b12] = (s[0] & 63) + ((s[4] & 48) << 2);
        dst[b12 + 1] = (s[1] & 63) + ((s[5] & 48) << 2);
        dst[b12 + 2] = (s[2] & 63) + ((s[6] & 48) << 2);
        dst[b12 + 3] = (s[3] & 63) + ((s[7] & 48) << 2);
        dst[b12 + 4] = (m[0] & 63) + ((m[4] & 48) << 2);
        dst[b12 + 5] = (m[1] & 63) + ((m[5] & 48) << 2);
        dst[b12 + 6] = (m[2] & 63) + ((m[6] & 48) << 2);
        dst[b12 + 7] = (m[3] & 63) + ((m[7] & 48) << 2);
        dst[b12 + 8] = (s[4] & 15) + ((m[4] & 15) << 4);
        dst[b12 + 9] = (s[5] & 15) + ((m[5] & 15) << 4);
        dst[b12 + 10] = (s[6] & 15) + ((m[6] & 15) << 4);
        dst[b12 + 11] = (s[7] & 15) + ((m[7] & 15) << 4);
    }
    for i in 0..4usize {
        let mut s = [0u8; 8];
        let mut m = [0u8; 8];
        for j in 0..8 {
            s[j] = ((src[j][i] & 192) >> 2) | (src[j][i + 8] & 15);
            m[j] = ((src[j][i + 4] & 192) >> 2) | ((src[j][i + 8] & 240) >> 4);
        }
        let b12 = i * 12 + 48;
        dst[b12] = (s[0] & 63) + ((s[4] & 48) << 2);
        dst[b12 + 1] = (s[1] & 63) + ((s[5] & 48) << 2);
        dst[b12 + 2] = (s[2] & 63) + ((s[6] & 48) << 2);
        dst[b12 + 3] = (s[3] & 63) + ((s[7] & 48) << 2);
        dst[b12 + 4] = (m[0] & 63) + ((m[4] & 48) << 2);
        dst[b12 + 5] = (m[1] & 63) + ((m[5] & 48) << 2);
        dst[b12 + 6] = (m[2] & 63) + ((m[6] & 48) << 2);
        dst[b12 + 7] = (m[3] & 63) + ((m[7] & 48) << 2);
        dst[b12 + 8] = (s[4] & 15) + ((m[4] & 15) << 4);
        dst[b12 + 9] = (s[5] & 15) + ((m[5] & 15) << 4);
        dst[b12 + 10] = (s[6] & 15) + ((m[6] & 15) << 4);
        dst[b12 + 11] = (s[7] & 15) + ((m[7] & 15) << 4);
    }
}

#[cfg(target_arch = "aarch64")]
fn pack_to_q4_0x4(blocks: &[BlockQ4_0], n: usize, block_len: usize) -> Vec<BlockQ4_0x4> {
    debug_assert!(n.is_multiple_of(4));
    debug_assert!((QK4_0 / 2).is_multiple_of(block_len));
    debug_assert_eq!(blocks.len() % n, 0);
    let k_blocks = blocks.len() / n;
    let n_groups = n / 4;
    let mut packed = Vec::with_capacity(n_groups * k_blocks);
    for g in 0..n_groups {
        for b in 0..k_blocks {
            let src: [&BlockQ4_0; 4] = std::array::from_fn(|i| &blocks[(g * 4 + i) * k_blocks + b]);
            let mut p = BlockQ4_0x4 {
                d: [f16::ZERO; 4],
                qs: [0; QK4_0 * 2],
            };
            for (i, s) in src.iter().enumerate() {
                p.d[i] = s.d;
            }
            for i in 0..(QK4_0 / 2) / block_len {
                let offset = i * block_len;
                let dst_offset = i * block_len * 4;
                for (j, s) in src.iter().enumerate() {
                    let dst_offset = dst_offset + j * block_len;
                    for k in 0..block_len {
                        p.qs[dst_offset + k] = (s.qs[offset + k] ^ 0x88) as i8;
                    }
                }
            }
            packed.push(p);
        }
    }
    packed
}

#[cfg(target_arch = "aarch64")]
fn pack_to_q8_0x4(blocks: &[BlockQ8_0], n: usize, block_len: usize) -> Vec<BlockQ8_0x4> {
    debug_assert!(n.is_multiple_of(4));
    debug_assert!(QK8_0.is_multiple_of(block_len));
    debug_assert_eq!(blocks.len() % n, 0);
    let k_blocks = blocks.len() / n;
    let n_groups = n / 4;
    let mut packed = Vec::with_capacity(n_groups * k_blocks);
    for g in 0..n_groups {
        for b in 0..k_blocks {
            let src: [&BlockQ8_0; 4] = std::array::from_fn(|i| &blocks[(g * 4 + i) * k_blocks + b]);
            let mut p = BlockQ8_0x4 {
                d: [f16::ZERO; 4],
                qs: [0; QK8_0 * 4],
            };
            for (i, s) in src.iter().enumerate() {
                p.d[i] = s.d;
            }
            for i in 0..QK8_0 / block_len {
                let offset = i * block_len;
                let dst_offset = i * block_len * 4;
                for (j, s) in src.iter().enumerate() {
                    let dst_offset = dst_offset + j * block_len;
                    p.qs[dst_offset..dst_offset + block_len]
                        .copy_from_slice(&s.qs[offset..offset + block_len]);
                }
            }
            packed.push(p);
        }
    }
    packed
}

// One lhs quantization and one barrier region for several same-dtype m==1 matmuls
// sharing the lhs (qkv / gate+up in decode), instead of one of each per projection.
#[cfg(target_arch = "aarch64")]
macro_rules! define_gemv_fused {
    ($fname:ident, $blk:ty, $variant:ident, $kernel:path, $w:expr, $lhs_blk:ty, $qk:expr) => {
        #[target_feature(enable = "dotprod")]
        unsafe fn $fname(
            kind: PackedKind,
            k: usize,
            lhs: &[f32],
            parts: &[(&dyn QuantizedType, &PackedCache)],
            dsts: &mut [Vec<f32>],
        ) {
            use super::GgmlType as _;
            let k_in_blocks = k / $qk;

            thread_local! {
                static FUSED_LHS_SCRATCH: std::cell::RefCell<Vec<u64>> =
                    const { std::cell::RefCell::new(Vec::new()) };
            }
            FUSED_LHS_SCRATCH.with(|cell| {
                let mut scratch = cell.borrow_mut();
                let elem_size = std::mem::size_of::<$lhs_blk>();
                let required_len = (k_in_blocks * elem_size).div_ceil(8);
                if scratch.len() < required_len {
                    scratch.resize(required_len, 0);
                }
                let lhs_b: &mut [$lhs_blk] = unsafe {
                    std::slice::from_raw_parts_mut(
                        scratch.as_mut_ptr() as *mut $lhs_blk,
                        k_in_blocks,
                    )
                };
                <$lhs_blk>::from_float(lhs, lhs_b);

                // (packed ptr, n_groups, dst ptr, running offset) per projection
                let mut metas: Vec<(usize, usize, usize, usize)> = Vec::with_capacity(parts.len());
                let mut total = 0usize;
                for (i, (storage, cache)) in parts.iter().enumerate() {
                    let n = dsts[i].len();
                    let ps = cache.get_or_init(kind, || kind.pack(*storage, n));
                    let PackedStorage::$variant(v) = ps else {
                        return;
                    };
                    metas.push((
                        v.as_ptr() as usize,
                        n / $w,
                        dsts[i].as_mut_ptr() as usize,
                        total,
                    ));
                    total += n / $w;
                }

                let blk_bytes = std::mem::size_of::<$blk>();
                let lhs_ptr = lhs_b.as_ptr() as usize;
                let pool = crate::utils::barrier_pool();
                // equal-sized gemv units: static split, as in the single-projection gemvs
                pool.execute_static(total, |range| {
                    let lhs_row: &[$lhs_blk] = unsafe {
                        std::slice::from_raw_parts(lhs_ptr as *const $lhs_blk, k_in_blocks)
                    };
                    for gg in range {
                        let &(pptr, _ng, dptr, off) = metas
                            .iter()
                            .find(|&&(_, ng, _, off)| gg < off + ng)
                            .unwrap();
                        let g = gg - off;
                        let xs = unsafe {
                            std::slice::from_raw_parts(
                                (pptr + g * k_in_blocks * blk_bytes) as *const $blk,
                                k_in_blocks,
                            )
                        };
                        let r = $kernel(k, xs, lhs_row);
                        unsafe {
                            std::ptr::copy_nonoverlapping(
                                r.as_ptr(),
                                (dptr as *mut f32).add(g * $w),
                                $w,
                            );
                        }
                    }
                });
            });
        }
    };
}

#[cfg(target_arch = "aarch64")]
define_gemv_fused!(
    gemv_fused_q4k,
    BlockQ4Kx8,
    Q4Kx8,
    super::neon::vec_dot_8_q4k_q8k,
    8,
    super::k_quants::BlockQ8K,
    QK_K
);
#[cfg(target_arch = "aarch64")]
define_gemv_fused!(
    gemv_fused_q6k,
    BlockQ6Kx8,
    Q6Kx8,
    super::neon::vec_dot_8_q6kx8_q8k,
    8,
    super::k_quants::BlockQ8K,
    QK_K
);
#[cfg(target_arch = "aarch64")]
define_gemv_fused!(
    gemv_fused_q8_0,
    BlockQ8_0x4,
    Q8_0x4,
    super::neon::vec_dot_4_q8_0x4_q8_0,
    4,
    BlockQ8_0,
    QK8_0
);

#[cfg(target_arch = "aarch64")]
pub(crate) fn try_gemv_fused(
    k: usize,
    lhs: &[f32],
    parts: &[(&dyn QuantizedType, &PackedCache)],
    dsts: &mut [Vec<f32>],
) -> Result<bool> {
    #[cfg(target_arch = "aarch64")]
    {
        if parts.is_empty() || parts.len() != dsts.len() {
            return Ok(false);
        }
        let features = crate::cpu::features::get();
        if !features.dotprod {
            return Ok(false);
        }
        let dtype = parts[0].0.dtype();
        if parts.iter().any(|(s, _)| s.dtype() != dtype) {
            return Ok(false);
        }
        let Some(kind) = PackedKind::select(dtype, (1, k, dsts[0].len())) else {
            return Ok(false);
        };
        for d in dsts.iter() {
            if PackedKind::select(dtype, (1, k, d.len())) != Some(kind) {
                return Ok(false);
            }
        }
        match kind {
            PackedKind::Q4Kx8 if k.is_multiple_of(QK_K) => {
                unsafe { gemv_fused_q4k(kind, k, lhs, parts, dsts) };
                Ok(true)
            }
            PackedKind::Q6Kx8 if k.is_multiple_of(QK_K) => {
                unsafe { gemv_fused_q6k(kind, k, lhs, parts, dsts) };
                Ok(true)
            }
            PackedKind::Q8_0x4 if k.is_multiple_of(QK8_0) => {
                unsafe { gemv_fused_q8_0(kind, k, lhs, parts, dsts) };
                Ok(true)
            }
            _ => Ok(false),
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let _ = (k, lhs, parts, dsts);
        Ok(false)
    }
}

// Indexed (MoE) gemv: each (token, expert) pair runs a gemv against one expert's row
// block inside the packed [n_experts * n_out, k] weights, all in one barrier region.
#[cfg(target_arch = "aarch64")]
macro_rules! define_indexed_gemv {
    ($fname:ident, $blk:ty, $variant:ident, $kernel:path, $w:expr, $lhs_blk:ty, $qk:expr) => {
        #[target_feature(enable = "dotprod")]
        #[allow(clippy::too_many_arguments)]
        unsafe fn $fname(
            kind: PackedKind,
            storage: &dyn QuantizedType,
            cache: &PackedCache,
            n_experts: usize,
            n_out: usize,
            k: usize,
            lhs: &[f32],
            n_rows: usize,
            ids: &[u32],
            topk: usize,
            dst: &mut [f32],
        ) {
            use super::GgmlType as _;
            let k_in_blocks = k / $qk;
            let total_rows = n_experts * n_out;
            let ps = cache.get_or_init(kind, || kind.pack(storage, total_rows));
            let PackedStorage::$variant(packed) = ps else {
                return;
            };

            // quantize each distinct lhs row once
            let mut lhs_q: Vec<$lhs_blk> = Vec::with_capacity(n_rows * k_in_blocks);
            #[allow(clippy::uninit_vec)]
            unsafe {
                lhs_q.set_len(n_rows * k_in_blocks)
            };
            let lhs_q_ptr = lhs_q.as_mut_ptr() as usize;
            let pool = crate::utils::barrier_pool();
            pool.execute_chunked(n_rows, |range| {
                let lhs_q_ptr = lhs_q_ptr as *mut $lhs_blk;
                for r in range {
                    let row = &lhs[r * k..(r + 1) * k];
                    let out = unsafe {
                        std::slice::from_raw_parts_mut(lhs_q_ptr.add(r * k_in_blocks), k_in_blocks)
                    };
                    <$lhs_blk>::from_float(row, out);
                }
            });

            let groups_per_expert = n_out / $w;
            let n_pairs = ids.len();
            let total_units = n_pairs * groups_per_expert;
            let packed_ptr = packed.as_ptr() as usize;
            let lhs_q_ptr = lhs_q.as_ptr() as usize;
            let dst_ptr = dst.as_mut_ptr() as usize;
            let blk_bytes = std::mem::size_of::<$blk>();

            pool.execute_chunked(total_units, |range| {
                let packed_ptr = packed_ptr as *const u8;
                let lhs_q_ptr = lhs_q_ptr as *const $lhs_blk;
                let dst_ptr = dst_ptr as *mut f32;
                for unit in range {
                    let pair = unit / groups_per_expert;
                    let g = unit % groups_per_expert;
                    let expert = ids[pair] as usize;
                    // x may carry one row per token (shared across that token's experts)
                    let row = if n_rows == n_pairs { pair } else { pair / topk };
                    let lhs_row = unsafe {
                        std::slice::from_raw_parts(lhs_q_ptr.add(row * k_in_blocks), k_in_blocks)
                    };
                    let group_global = expert * groups_per_expert + g;
                    let xs = unsafe {
                        std::slice::from_raw_parts(
                            packed_ptr.add(group_global * k_in_blocks * blk_bytes) as *const $blk,
                            k_in_blocks,
                        )
                    };
                    let r = $kernel(k, xs, lhs_row);
                    unsafe {
                        std::ptr::copy_nonoverlapping(
                            r.as_ptr(),
                            dst_ptr.add(pair * n_out + g * $w),
                            $w,
                        );
                    }
                }
            });
        }
    };
}

#[cfg(target_arch = "aarch64")]
define_indexed_gemv!(
    indexed_gemv_q4k,
    BlockQ4Kx8,
    Q4Kx8,
    super::neon::vec_dot_8_q4k_q8k,
    8,
    super::k_quants::BlockQ8K,
    QK_K
);
#[cfg(target_arch = "aarch64")]
define_indexed_gemv!(
    indexed_gemv_q6k,
    BlockQ6Kx8,
    Q6Kx8,
    super::neon::vec_dot_8_q6kx8_q8k,
    8,
    super::k_quants::BlockQ8K,
    QK_K
);
#[cfg(target_arch = "aarch64")]
define_indexed_gemv!(
    indexed_gemv_q8_0,
    BlockQ8_0x4,
    Q8_0x4,
    super::neon::vec_dot_4_q8_0x4_q8_0,
    4,
    BlockQ8_0,
    QK8_0
);

#[cfg(target_arch = "aarch64")]
const INDEXED_BUCKET_MIN_PAIRS: usize = 32;

#[cfg(target_arch = "aarch64")]
#[allow(clippy::too_many_arguments)]
pub(crate) fn try_indexed_gemv(
    storage: &dyn QuantizedType,
    cache: &PackedCache,
    n_experts: usize,
    n_out: usize,
    k: usize,
    lhs: &[f32],
    n_rows: usize,
    ids: &[u32],
    topk: usize,
    dst: &mut [f32],
) -> Result<bool> {
    #[cfg(target_arch = "aarch64")]
    {
        let features = crate::cpu::features::get();
        if !features.dotprod {
            return Ok(false);
        }
        let Some(kind) = PackedKind::select(storage.dtype(), (1, k, n_experts * n_out)) else {
            return Ok(false);
        };
        // prefill: bucket pairs by expert and run one tiled matmul per expert instead
        // of a gemv per (token, expert) pair
        if ids.len() >= INDEXED_BUCKET_MIN_PAIRS
            && matches!(
                kind,
                PackedKind::Q4Kx8 | PackedKind::Q6Kx8 | PackedKind::Q8_0x4
            )
        {
            let ps = cache.get_or_init(kind, || kind.pack(storage, n_experts * n_out));
            let n_pairs = ids.len();
            let mut buckets: Vec<Vec<usize>> = vec![Vec::new(); n_experts];
            for (pair, &e) in ids.iter().enumerate() {
                buckets[e as usize].push(pair);
            }
            let mut gathered: Vec<f32> = Vec::new();
            let mut scratch: Vec<f32> = Vec::new();
            for (e, bucket) in buckets.iter().enumerate() {
                if bucket.is_empty() {
                    continue;
                }
                let cnt = bucket.len();
                let cnt_pad = cnt.next_multiple_of(4);
                gathered.clear();
                gathered.resize(cnt_pad * k, 0.0);
                for (i, &pair) in bucket.iter().enumerate() {
                    let row = if n_rows == n_pairs { pair } else { pair / topk };
                    gathered[i * k..(i + 1) * k].copy_from_slice(&lhs[row * k..(row + 1) * k]);
                }
                scratch.clear();
                scratch.resize(cnt_pad * n_out, 0.0);
                kind.matmul_expert_range((cnt_pad, k, n_out), &gathered, ps, e, &mut scratch)?;
                for (i, &pair) in bucket.iter().enumerate() {
                    dst[pair * n_out..(pair + 1) * n_out]
                        .copy_from_slice(&scratch[i * n_out..(i + 1) * n_out]);
                }
            }
            return Ok(true);
        }
        match kind {
            PackedKind::Q4Kx8 if k.is_multiple_of(QK_K) && n_out.is_multiple_of(8) => {
                unsafe {
                    indexed_gemv_q4k(
                        kind, storage, cache, n_experts, n_out, k, lhs, n_rows, ids, topk, dst,
                    )
                };
                Ok(true)
            }
            PackedKind::Q6Kx8 if k.is_multiple_of(QK_K) && n_out.is_multiple_of(8) => {
                unsafe {
                    indexed_gemv_q6k(
                        kind, storage, cache, n_experts, n_out, k, lhs, n_rows, ids, topk, dst,
                    )
                };
                Ok(true)
            }
            PackedKind::Q8_0x4 if k.is_multiple_of(QK8_0) && n_out.is_multiple_of(4) => {
                unsafe {
                    indexed_gemv_q8_0(
                        kind, storage, cache, n_experts, n_out, k, lhs, n_rows, ids, topk, dst,
                    )
                };
                Ok(true)
            }
            _ => Ok(false),
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let _ = (
            storage, cache, n_experts, n_out, k, lhs, n_rows, ids, topk, dst,
        );
        Ok(false)
    }
}
