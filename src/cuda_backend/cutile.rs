//! cuTile kernel authoring and CUDA stream interop.
//!
//! ```no_run
//! use candle_core::cutile;
//!
//! #[cutile::module]
//! mod kernels {
//!     use candle_core::cutile;
//!     use cutile::core::*;
//!     use cutile::cutile_compiler;
//!
//!     #[cutile::entry]
//!     unsafe fn noop() {}
//! }
//! ```

use super::cudarc::driver::{CudaSlice, DevicePtr, DevicePtrMut, DeviceRepr, SyncOnDrop};
use super::{CudaDType, CudaDevice, CudaStorage, WrapErr};
use crate::{Error, Layout, Result};
use ::core::ffi::{c_int, c_void};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock};

pub use ::cutile::*;

/// A cuTile launch context backed by Candle's current CUDA stream.
pub struct CutileContext {
    candle_stream: Arc<super::cudarc::driver::CudaStream>,
    cutile_stream: Arc<::cutile::cuda_core::Stream>,
}

impl CutileContext {
    /// Borrows Candle's CUDA context and stream without transferring ownership.
    pub fn new(device: &CudaDevice) -> Result<Self> {
        let candle_stream = device.cuda_stream();
        let candle_context = candle_stream.context();
        candle_context.bind_to_thread().w()?;
        check_device_support(candle_context.ordinal())?;
        let cutile_device = unsafe {
            ::cutile::cuda_core::Device::borrow_with_owner(
                candle_context.cu_ctx() as *mut c_void,
                candle_context.cu_device() as c_int,
                candle_context.ordinal(),
                candle_context.clone(),
            )
        };
        let cutile_stream = unsafe {
            ::cutile::cuda_core::Stream::borrow_with_owner(
                candle_stream.cu_stream() as *mut c_void,
                &cutile_device,
                candle_stream.clone(),
            )
        };
        Ok(Self {
            candle_stream,
            cutile_stream,
        })
    }

    /// The borrowed cuTile stream used by `compile_on` and `async_on`.
    pub fn stream(&self) -> &Arc<::cutile::cuda_core::Stream> {
        &self.cutile_stream
    }

    /// Borrows a typed Candle allocation for a cuTile kernel read.
    pub fn read<'a, T: DeviceRepr>(
        &'a self,
        slice: &'a CudaSlice<T>,
        offset: usize,
    ) -> Result<CutileRead<'a, T>> {
        if offset > slice.len() {
            return Err(Error::msg(format!(
                "cuTile read offset {offset} exceeds allocation length {}",
                slice.len()
            )));
        }
        let (pointer, guard) = slice.device_ptr(&self.candle_stream);
        let byte_offset = offset
            .checked_mul(std::mem::size_of::<T>())
            .ok_or_else(|| Error::msg("cuTile read offset overflow"))?;
        let pointer = pointer
            .checked_add(byte_offset as u64)
            .ok_or_else(|| Error::msg("cuTile read pointer overflow"))?;
        Ok(CutileRead {
            pointer: unsafe {
                ::cutile::cuda_async::device_buffer::DevicePointer::from_cu_deviceptr(pointer)
            },
            _guard: guard,
        })
    }

    /// Borrows a typed Candle allocation for a cuTile kernel write.
    pub fn write<'a, T: DeviceRepr>(
        &'a self,
        slice: &'a mut CudaSlice<T>,
        offset: usize,
    ) -> Result<CutileWrite<'a, T>> {
        if offset > slice.len() {
            return Err(Error::msg(format!(
                "cuTile write offset {offset} exceeds allocation length {}",
                slice.len()
            )));
        }
        let (pointer, guard) = slice.device_ptr_mut(&self.candle_stream);
        let byte_offset = offset
            .checked_mul(std::mem::size_of::<T>())
            .ok_or_else(|| Error::msg("cuTile write offset overflow"))?;
        let pointer = pointer
            .checked_add(byte_offset as u64)
            .ok_or_else(|| Error::msg("cuTile write pointer overflow"))?;
        Ok(CutileWrite {
            pointer: unsafe {
                ::cutile::cuda_async::device_buffer::DevicePointer::from_cu_deviceptr(pointer)
            },
            _guard: guard,
        })
    }

    /// Borrows a typed Candle storage and applies its layout offset.
    pub fn read_storage<'a, T: CudaDType + DeviceRepr + 'a>(
        &'a self,
        storage: &'a CudaStorage,
        layout: &Layout,
    ) -> Result<CutileRead<'a, T>> {
        self.read(storage.as_cuda_slice::<T>()?, layout.start_offset())
    }

    /// Mutably borrows a typed Candle storage and applies its layout offset.
    pub fn write_storage<'a, T: CudaDType + DeviceRepr + 'a>(
        &'a self,
        storage: &'a mut CudaStorage,
        layout: &Layout,
    ) -> Result<CutileWrite<'a, T>> {
        self.write(storage.as_cuda_slice_mut::<T>()?, layout.start_offset())
    }
}

/// Fails when the installed `tileiras` cannot target the device, if it can be probed.
fn check_device_support(ordinal: usize) -> Result<()> {
    use ::cutile::cutile_compiler::cuda_tile_runtime_utils::{get_gpu_name, tileiras_binary};
    let tileiras = tileiras_binary();
    let Some(supported) = tileiras_gpu_names(&tileiras) else {
        return Ok(());
    };
    ensure_gpu_supported(ordinal, &get_gpu_name(ordinal), &tileiras, &supported)
}

fn ensure_gpu_supported(
    ordinal: usize,
    gpu_name: &str,
    tileiras: &Path,
    supported: &[String],
) -> Result<()> {
    if supported.iter().any(|name| name == gpu_name) {
        return Ok(());
    }
    Err(Error::msg(format!(
        "cuTile cannot target CUDA device {ordinal}: it is {gpu_name}, but {} supports only {}. \
         Use a supported GPU or a CUDA toolkit whose tileiras supports {gpu_name}.",
        tileiras.display(),
        supported.join(", "),
    )))
}

type GpuNames = Option<Arc<[String]>>;

/// Architectures accepted by `tileiras --gpu-name`, probed once per binary.
fn tileiras_gpu_names(tileiras: &Path) -> GpuNames {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, GpuNames>>> = OnceLock::new();
    let mut cache = CACHE
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(cached) = cache.get(tileiras) {
        return cached.clone();
    }
    let names = probe_tileiras_gpu_names(tileiras);
    cache.insert(tileiras.to_path_buf(), names.clone());
    names
}

fn probe_tileiras_gpu_names(tileiras: &Path) -> GpuNames {
    let output = Command::new(tileiras).arg("--help").output().ok()?;
    let mut help = String::from_utf8_lossy(&output.stdout).into_owned();
    help.push_str(&String::from_utf8_lossy(&output.stderr));
    let names = parse_tileiras_gpu_names(&help);
    (!names.is_empty()).then(|| names.into())
}

/// Extracts the `=sm_XY` choices listed under `--gpu-name` in `tileiras --help`.
fn parse_tileiras_gpu_names(help: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut in_gpu_name = false;
    for line in help.lines() {
        let line = line.trim_start();
        if line.starts_with("--gpu-name") {
            in_gpu_name = true;
        } else if in_gpu_name {
            match line
                .strip_prefix('=')
                .and_then(|rest| rest.split_whitespace().next())
            {
                Some(name) => names.push(name.to_string()),
                None => in_gpu_name = false,
            }
        }
    }
    names
}

/// A typed cuTile pointer that records a Candle read after it is dropped.
#[must_use = "keep the pointer guard alive until the cuTile launch is enqueued"]
pub struct CutileRead<'a, T> {
    pointer: ::cutile::cuda_async::device_buffer::DevicePointer<T>,
    _guard: SyncOnDrop<'a>,
}

impl<T> CutileRead<'_, T> {
    /// Returns the pointer accepted by raw-pointer cuTile entry functions.
    pub fn device_pointer(&self) -> ::cutile::cuda_async::device_buffer::DevicePointer<T> {
        unsafe {
            ::cutile::cuda_async::device_buffer::DevicePointer::from_cu_deviceptr(
                self.pointer.cu_deviceptr(),
            )
        }
    }
}

/// A typed cuTile pointer that records a Candle write after it is dropped.
#[must_use = "keep the pointer guard alive until the cuTile launch is enqueued"]
pub struct CutileWrite<'a, T> {
    pointer: ::cutile::cuda_async::device_buffer::DevicePointer<T>,
    _guard: SyncOnDrop<'a>,
}

impl<T> CutileWrite<'_, T> {
    /// Returns the pointer accepted by raw-pointer cuTile entry functions.
    pub fn device_pointer(&self) -> ::cutile::cuda_async::device_buffer::DevicePointer<T> {
        unsafe {
            ::cutile::cuda_async::device_buffer::DevicePointer::from_cu_deviceptr(
                self.pointer.cu_deviceptr(),
            )
        }
    }
}

/// Converts a cuTile error or panic into a Candle error.
pub fn kernel<T, E: std::fmt::Display>(
    operation: &str,
    f: impl FnOnce() -> std::result::Result<T, E>,
) -> Result<T> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(Error::msg(format!("cuTile {operation}: {error}"))),
        Err(payload) => {
            let message = payload
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| payload.downcast_ref::<&'static str>().copied())
                .unwrap_or("non-string panic");
            Err(Error::msg(format!(
                "cuTile {operation} panicked: {message}"
            )))
        }
    }
}
