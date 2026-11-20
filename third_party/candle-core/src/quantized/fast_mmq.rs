//! CUDA fast path for GGUF tiled matmul (prompt/prefill phase).
//! Handles batch > 8 (complement to fast_mmvq which handles batch 1-8).

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use super::cuda::{QCudaStorage, MATRIX_ROW_PADDING};
use super::GgmlDType;
use crate::cuda_backend::DeviceId;
use crate::{backend::BackendStorage, CudaDevice, CudaStorage, DType, Result, Shape};

use cudarc::driver::{CudaSlice, DevicePtr};

const QK8_1: usize = 32;
const BLOCK_Q8_1_MMQ_SIZE: usize = 4 * QK8_1 + 4 * 4; // 128 qs + 16 scale bytes = 144

#[inline]
fn pad(p: usize, q: usize) -> usize {
    p.div_ceil(q) * q
}

/// Quant types supported by MMQ kernels (same as MMVQ).
fn supports(dtype: GgmlDType) -> bool {
    matches!(
        dtype,
        GgmlDType::Q4_0
            | GgmlDType::Q4_1
            | GgmlDType::Q5_0
            | GgmlDType::Q5_1
            | GgmlDType::Q8_0
            | GgmlDType::Q2K
            | GgmlDType::Q3K
            | GgmlDType::Q4K
            | GgmlDType::Q5K
            | GgmlDType::Q6K
    )
}

/// qk (block quantization size) per dtype.
fn qk_for(dtype: GgmlDType) -> usize {
    match dtype {
        GgmlDType::Q4_0 | GgmlDType::Q4_1 | GgmlDType::Q5_0 | GgmlDType::Q5_1 | GgmlDType::Q8_0 => {
            32
        }
        GgmlDType::Q2K | GgmlDType::Q3K | GgmlDType::Q4K | GgmlDType::Q5K | GgmlDType::Q6K => 256,
        _ => unreachable!(),
    }
}

// ds_layout mapping: which Q8_1_mmq scale layout to use per weight type.
enum DsLayout {
    D4,
    DS4,
    D2S6,
}

fn ds_layout_for(dtype: GgmlDType) -> DsLayout {
    match dtype {
        GgmlDType::Q4_0 | GgmlDType::Q4_1 => DsLayout::DS4,
        GgmlDType::Q5_0 => DsLayout::D4,
        GgmlDType::Q5_1 => DsLayout::DS4,
        GgmlDType::Q8_0 => DsLayout::D4,
        GgmlDType::Q2K => DsLayout::D2S6,
        GgmlDType::Q3K => DsLayout::D4,
        GgmlDType::Q4K | GgmlDType::Q5K => DsLayout::DS4,
        GgmlDType::Q6K => DsLayout::D4,
        _ => unreachable!(),
    }
}

type QuantizeLauncher = unsafe extern "C" fn(
    x: *const std::ffi::c_void,
    ids: *const i32,
    vy: *mut std::ffi::c_void,
    type_x: i32,
    ne00: i64,
    s01: i64,
    s02: i64,
    s03: i64,
    ne0: i64,
    ne1: i64,
    ne2: i64,
    ne3: i64,
    stream: *mut std::ffi::c_void,
);

fn quantize_launcher(layout: DsLayout) -> QuantizeLauncher {
    use candle_kernels::ffi;
    match layout {
        DsLayout::D4 => ffi::launch_mmq_quantize_q8_1_D4,
        DsLayout::DS4 => ffi::launch_mmq_quantize_q8_1_DS4,
        DsLayout::D2S6 => ffi::launch_mmq_quantize_q8_1_D2S6,
    }
}

type MmqLauncher = unsafe extern "C" fn(
    tmp_fixup: *mut std::ffi::c_void,
    x: *const std::ffi::c_void,
    y: *const std::ffi::c_void,
    dst: *mut std::ffi::c_void,
    ncols_x: i64,
    nrows_x: i64,
    ncols_y: i64,
    stride_row_x: i64,
    stride_col_dst: i64,
    cc: i32,
    nsm: i32,
    smpbo: i64,
    warp_size: i32,
    stream: *mut std::ffi::c_void,
);

fn mmq_launcher(dtype: GgmlDType) -> Option<MmqLauncher> {
    use candle_kernels::ffi;
    let f: MmqLauncher = match dtype {
        GgmlDType::Q4_0 => ffi::launch_mmq_gguf_q4_0,
        GgmlDType::Q4_1 => ffi::launch_mmq_gguf_q4_1,
        GgmlDType::Q5_0 => ffi::launch_mmq_gguf_q5_0,
        GgmlDType::Q5_1 => ffi::launch_mmq_gguf_q5_1,
        GgmlDType::Q8_0 => ffi::launch_mmq_gguf_q8_0,
        GgmlDType::Q2K => ffi::launch_mmq_gguf_q2_k,
        GgmlDType::Q3K => ffi::launch_mmq_gguf_q3_k,
        GgmlDType::Q4K => ffi::launch_mmq_gguf_q4_k,
        GgmlDType::Q5K => ffi::launch_mmq_gguf_q5_k,
        GgmlDType::Q6K => ffi::launch_mmq_gguf_q6_k,
        _ => return None,
    };
    Some(f)
}

// ---------------------------------------------------------------------------
// Per-device workspaces (grows-only, reused across calls).
// ---------------------------------------------------------------------------

struct WorkspaceSlot {
    slice: CudaSlice<u8>,
    cap: usize,
}

type WsMap = Mutex<HashMap<DeviceId, &'static Mutex<WorkspaceSlot>>>;

static MMQ_WORKSPACE: OnceLock<WsMap> = OnceLock::new();
static FIXUP_WORKSPACE: OnceLock<WsMap> = OnceLock::new();

fn workspace_ensure(
    ws: &'static OnceLock<WsMap>,
    dev: &CudaDevice,
    bytes: usize,
) -> Result<(u64, std::sync::MutexGuard<'static, WorkspaceSlot>)> {
    let map = ws.get_or_init(|| Mutex::new(HashMap::new()));
    let device_key = dev.id();
    let device_mtx: &'static Mutex<WorkspaceSlot> = {
        let mut guard = map.lock().unwrap();
        match guard.get(&device_key).copied() {
            Some(mtx) => mtx,
            None => {
                let slice = unsafe { dev.alloc::<u8>(bytes.max(1))? };
                let leaked = Box::leak(Box::new(Mutex::new(WorkspaceSlot {
                    slice,
                    cap: bytes.max(1),
                })));
                guard.insert(device_key, leaked);
                leaked
            }
        }
    };
    let mut slot = device_mtx.lock().unwrap();
    if slot.cap < bytes {
        slot.slice = unsafe { dev.alloc::<u8>(bytes)? };
        slot.cap = bytes;
    }
    let ptr = slot.slice.device_ptr(slot.slice.stream()).0;
    Ok((ptr, slot))
}

// ---------------------------------------------------------------------------
// Per-device info cache (compute capability, SM count, etc.)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct DeviceInfo {
    cc: i32,
    nsm: i32,
    smpbo: i64,
    warp_size: i32,
}

static DEVICE_INFO: OnceLock<Mutex<HashMap<DeviceId, DeviceInfo>>> = OnceLock::new();

fn get_device_info(dev: &CudaDevice) -> DeviceInfo {
    use cudarc::driver::{result, sys};
    let map = DEVICE_INFO.get_or_init(|| Mutex::new(HashMap::new()));
    let key = dev.id();
    let mut guard = map.lock().unwrap();
    if let Some(info) = guard.get(&key) {
        return *info;
    }
    let cu_device = dev.cuda_stream().context().cu_device();
    let major = unsafe {
        result::device::get_attribute(
            cu_device,
            sys::CUdevice_attribute::CU_DEVICE_ATTRIBUTE_COMPUTE_CAPABILITY_MAJOR,
        )
    }
    .unwrap_or(8);
    let minor = unsafe {
        result::device::get_attribute(
            cu_device,
            sys::CUdevice_attribute::CU_DEVICE_ATTRIBUTE_COMPUTE_CAPABILITY_MINOR,
        )
    }
    .unwrap_or(0);
    let nsm = unsafe {
        result::device::get_attribute(
            cu_device,
            sys::CUdevice_attribute::CU_DEVICE_ATTRIBUTE_MULTIPROCESSOR_COUNT,
        )
    }
    .unwrap_or(1);
    let smpbo = unsafe {
        result::device::get_attribute(
            cu_device,
            sys::CUdevice_attribute::CU_DEVICE_ATTRIBUTE_MAX_SHARED_MEMORY_PER_BLOCK_OPTIN,
        )
    }
    .unwrap_or(49152);
    let warp_size = unsafe {
        result::device::get_attribute(
            cu_device,
            sys::CUdevice_attribute::CU_DEVICE_ATTRIBUTE_WARP_SIZE,
        )
    }
    .unwrap_or(32);
    let info = DeviceInfo {
        cc: major * 100 + minor * 10,
        nsm,
        smpbo: smpbo as i64,
        warp_size,
    };
    guard.insert(key, info);
    info
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Try the fast MMQ path. Returns `Ok(None)` when the fast path is not applicable:
/// - unsupported quant dtype
/// - batch size 0
/// - non-BF16/F16/F32 input
/// - non-contiguous input
pub fn try_fwd(
    qstorage: &QCudaStorage,
    self_shape: &Shape,
    rhs: &CudaStorage,
    rhs_l: &crate::Layout,
) -> Result<Option<(CudaStorage, Shape)>> {
    let w_dtype = qstorage.dtype();
    if !supports(w_dtype) {
        return Ok(None);
    }
    let input_dtype = rhs.dtype();
    if !matches!(input_dtype, DType::BF16 | DType::F16 | DType::F32) {
        return Ok(None);
    }

    let (nrows, ncols) = self_shape.dims2()?;

    let (b_size, k) = match rhs_l.shape().dims() {
        [b, m, k] => (b * m, *k),
        [b, k] => (*b, *k),
        _ => return Ok(None),
    };
    if ncols != k {
        return Ok(None);
    }
    if b_size == 0 {
        return Ok(None);
    }

    let qk = qk_for(w_dtype);
    if k % qk != 0 {
        return Ok(None);
    }

    let (o1, o2) = match rhs_l.contiguous_offsets() {
        Some(offsets) => offsets,
        None => return Ok(None),
    };

    let dev = qstorage.device();
    let stream = dev.cuda_stream();
    let stream_ptr = stream.cu_stream() as *mut std::ffi::c_void;

    // MMQ quantize expects f32 input. Convert if needed.
    let rhs_f32_storage: Option<CudaStorage> = if input_dtype != DType::F32 {
        let layout_for_cast =
            crate::Layout::contiguous_with_offset(rhs_l.shape(), rhs_l.start_offset());
        Some(rhs.to_dtype(&layout_for_cast, DType::F32)?)
    } else {
        None
    };

    // Get the f32 slice (either original or converted)
    let rhs_f32_ref = rhs_f32_storage.as_ref().unwrap_or(rhs);
    let rhs_f32_slice = rhs_f32_ref.as_cuda_slice::<f32>()?;
    // For the converted case, offset is 0 (to_dtype returns contiguous from 0).
    // For the original f32 case, apply the original offsets.
    let rhs_f32_slice = if rhs_f32_storage.is_some() {
        rhs_f32_slice.slice(..b_size * k)
    } else {
        rhs_f32_slice.slice(o1..o2)
    };
    let rhs_ptr = rhs_f32_slice.device_ptr(&stream).0 as *const std::ffi::c_void;

    // Compute padded dimensions
    let k_padded = pad(k, MATRIX_ROW_PADDING);
    // Must also be multiple of 4*QK8_1 = 128 for block_q8_1_mmq
    let k_padded = pad(k_padded, 4 * QK8_1);

    // Workspace for block_q8_1_mmq quantized activations
    let blocks_per_row = k_padded / (4 * QK8_1);
    let workspace_main = b_size * blocks_per_row * BLOCK_Q8_1_MMQ_SIZE;
    // Extra padding for mmq_x_max (128 for MMA path)
    let workspace_extra = 128 * BLOCK_Q8_1_MMQ_SIZE;
    let workspace_bytes = workspace_main + workspace_extra;

    let (scratch_ptr, _workspace_guard) = workspace_ensure(&MMQ_WORKSPACE, dev, workspace_bytes)?;
    let scratch_ptr = scratch_ptr as *mut std::ffi::c_void;

    // Stream-k fixup workspace
    const MMQ_X_MAX: usize = 128;
    const MMQ_Y_MAX: usize = 128;
    const MAX_SMS: usize = 256;
    let fixup_bytes = MAX_SMS * MMQ_X_MAX * MMQ_Y_MAX * std::mem::size_of::<f32>();
    let (fixup_ptr, _fixup_guard) = workspace_ensure(&FIXUP_WORKSPACE, dev, fixup_bytes)?;
    let fixup_ptr = fixup_ptr as *mut std::ffi::c_void;

    let weight_ptr = qstorage.device_ptr()? as *const std::ffi::c_void;
    let stride_row_x = (k / qk) as i64;
    let di = get_device_info(dev);

    let out = unsafe { dev.alloc::<f32>(nrows * b_size)? };
    let stride_col_dst = nrows as i64;

    let out_ptr = out.device_ptr(&stream).0 as *mut std::ffi::c_void;

    unsafe {
        let quantize = quantize_launcher(ds_layout_for(w_dtype));
        quantize(
            rhs_ptr,
            std::ptr::null(),
            scratch_ptr,
            0,
            k as i64,
            k as i64,
            0,
            0,
            k_padded as i64,
            b_size as i64,
            1,
            1,
            stream_ptr,
        );

        let launcher = mmq_launcher(w_dtype).expect("supports() checked");
        launcher(
            fixup_ptr,
            weight_ptr,
            scratch_ptr as *const std::ffi::c_void,
            out_ptr,
            k as i64,
            nrows as i64,
            b_size as i64,
            stride_row_x,
            stride_col_dst,
            di.cc,
            di.nsm,
            di.smpbo,
            di.warp_size,
            stream_ptr,
        );
    }

    let mut out_shape = rhs_l.shape().dims().to_vec();
    out_shape.pop();
    out_shape.push(nrows);

    let out_storage = CudaStorage::wrap_cuda_slice(out, dev.clone());

    if input_dtype == DType::F32 {
        Ok(Some((out_storage, out_shape.into())))
    } else {
        // Cast f32 output back to input dtype
        let out_layout = crate::Layout::contiguous(Shape::from(out_shape.clone()));
        let cast_storage = out_storage.to_dtype(&out_layout, input_dtype)?;
        Ok(Some((cast_storage, out_shape.into())))
    }
}
