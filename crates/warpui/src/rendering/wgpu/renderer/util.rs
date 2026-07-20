use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use wgpu::util::BufferInitDescriptor;
use wgpu::{Buffer, BufferAddress, BufferDescriptor, COPY_BUFFER_ALIGNMENT, Device};

use super::Error;

/// Calls the provided function, capturing and returning any validation errors
/// detected by wgpu.
#[must_use]
pub fn with_error_scope<T>(
    device: &wgpu::Device,
    callback: impl FnOnce() -> T,
) -> (T, Option<Error>) {
    let error_scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let ret = callback();
    // On native platforms, the future returned by `pop_error_scope` resolves
    // immediately.  On wasm, it may take longer due to asynchronous browser
    // APIs, but it's necessary to wait here to know if it is safe to continue.
    let error_future = error_scope.pop();
    cfg_if::cfg_if! {
        if #[cfg(target_family = "wasm")] {
            let error = crate::r#async::block_on(error_future);
        } else {
            use futures::FutureExt;
            let error = error_future.now_or_never().expect("always resolves immediately");
        }
    }
    (ret, error.map(Into::into))
}

/// Creates a buffer and initializes it with data, synchronously returning an
/// error if the buffer could not be created successfully.
///
/// This is adapted from [`wgpu::util::DeviceExt::create_buffer_init`], with
/// added logic to check for and return errors from the underlying buffer
/// creation.
pub fn create_buffer_init(
    device: &Device,
    device_lost: &Arc<AtomicBool>,
    descriptor: &BufferInitDescriptor<'_>,
) -> Result<Buffer, super::Error> {
    // Skip mapping if the buffer is zero sized
    if descriptor.contents.is_empty() {
        let wgt_descriptor = BufferDescriptor {
            label: descriptor.label,
            size: 0,
            usage: descriptor.usage,
            mapped_at_creation: false,
        };

        create_buffer(device, &wgt_descriptor)
    } else {
        let unpadded_size = descriptor.contents.len() as BufferAddress;
        // Valid vulkan usage is
        // 1. buffer size must be a multiple of COPY_BUFFER_ALIGNMENT.
        // 2. buffer size must be greater than 0.
        // Therefore we round the value up to the nearest multiple, and ensure it's at least COPY_BUFFER_ALIGNMENT.
        let align_mask = COPY_BUFFER_ALIGNMENT - 1;
        let padded_size = ((unpadded_size + align_mask) & !align_mask).max(COPY_BUFFER_ALIGNMENT);

        let wgt_descriptor = BufferDescriptor {
            label: descriptor.label,
            size: padded_size,
            usage: descriptor.usage,
            mapped_at_creation: true,
        };

        let buffer = create_buffer(device, &wgt_descriptor)?;

        if device_lost.load(Ordering::SeqCst) {
            return Err(super::Error::DeviceLost);
        }

        buffer
            .slice(..)
            .get_mapped_range_mut()
            .slice(..unpadded_size as usize)
            .copy_from_slice(descriptor.contents);
        buffer.unmap();

        Ok(buffer)
    }
}

/// Creates a buffer using the given device and descriptor, synchronously
/// returning an error if the buffer could not be created successfully.
fn create_buffer(device: &Device, desc: &BufferDescriptor<'_>) -> Result<Buffer, Error> {
    let (buffer, error) = with_error_scope(device, || device.create_buffer(desc));

    match error {
        Some(error) => {
            log::warn!("Failed to create wgpu::Buffer: {error:#}");
            Err(error)
        }
        None => Ok(buffer),
    }
}
