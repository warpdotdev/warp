pub mod renderer;
mod resources;
mod shader_types;
mod texture_with_bind_group;

use std::sync::{Arc, LazyLock, Mutex};

use wgpu::wgt::WgpuHasDisplayHandle;

pub use renderer::Renderer;
pub use resources::{adapter_has_rendering_offset_bug, Resources};

use crate::platform::GraphicsBackend;
#[cfg(not(target_family = "wasm"))]
use crate::{rendering::GPUPowerPreference, windowing};

static WGPU_INSTANCE: LazyLock<Mutex<Option<Arc<wgpu::Instance>>>> = LazyLock::new(Mutex::default);

/// Drops and recreates the global shared [`wgpu::Instance`].
pub fn reset_wgpu_instance(display_handle: Box<dyn wgpu::wgt::WgpuHasDisplayHandle>) {
    // Drop the existing wgpu instance.
    {
        let mut instance = WGPU_INSTANCE
            .lock()
            .expect("wgpu instance lock should not be poisoned");
        let _ = instance.take();
    }

    // Create a new one.
    init_wgpu_instance(display_handle);
}

/// Initializes the global wgpu instance.  This MUST be called before [`get_wgpu_instance()`].
pub fn init_wgpu_instance(display_handle: Box<dyn WgpuHasDisplayHandle>) {
    // Check whether DirectComposition should be explicitly disabled on Windows.
    let disable_dcomp = std::env::var("WARP_USE_DIRECT_COMPOSITION")
        .ok()
        .is_some_and(|val| {
            let val = val.to_lowercase();
            val == "0" || val == "false"
        });

    // A helper function to create a wgpu instance with the appropriate configuration.
    let create_instance = move || {
        let dx12_shader_compiler = get_dx12_shader_compiler();
        Arc::new(wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu_backend_options(),
            backend_options: wgpu::BackendOptions {
                dx12: wgpu::Dx12BackendOptions {
                    presentation_system: if disable_dcomp {
                        wgpu::wgt::Dx12SwapchainKind::DxgiFromHwnd
                    } else {
                        wgpu::wgt::Dx12SwapchainKind::DxgiFromVisual
                    },
                    shader_compiler: dx12_shader_compiler.unwrap_or(wgpu::Dx12Compiler::Fxc),
                    ..Default::default()
                },
                ..Default::default()
            },
            flags: wgpu::InstanceFlags::empty(),
            memory_budget_thresholds: Default::default(),
            display: Some(display_handle),
        }))
    };

    // A helper function for initializing the WGPU_INSTANCE static variable.
    //
    // If `lock_acquired_tx` is provided, it will be used to signal when the lock has been acquired, allowing
    // for asynchronous initialization in a dedicated thread while ensuring that `get_wgpu_instance()` cannot
    // race with the initialization.
    let init_static_var = |lock_acquired_tx: Option<std::sync::mpsc::Sender<()>>| {
        let mut instance_lock_guard = WGPU_INSTANCE
            .lock()
            .expect("wgpu instance lock should not be poisoned");

        if let Some(tx) = lock_acquired_tx {
            tx.send(()).expect("Failed to send lock acquired signal");
        }

        instance_lock_guard.get_or_insert_with(|| {
            #[cfg(any(target_os = "linux", target_os = "freebsd"))]
            {
                use crate::windowing::{winit::app::WINDOWING_SYSTEM, WindowingSystem};
                // If the user hasn't enabled (and is making use of) native Wayland
                // support, due to the fact that we force use of X11 in
                // ui/src/windowing/winit/app.rs, we need to make sure wgpu doesn't
                // attempt to configure the instance to use Wayland, as that causes
                // crashes due to a mismatch between the instance and the window
                // handle we pass in later when constructing GPU resources.
                if WINDOWING_SYSTEM
                    .get()
                    .is_some_and(|windowing_system| *windowing_system == WindowingSystem::X11)
                    || std::env::var_os("WAYLAND_DISPLAY").is_none()
                {
                    let old_wayland_display = std::env::var_os("WAYLAND_DISPLAY");
                    std::env::set_var("WAYLAND_DISPLAY", "");
                    let instance = create_instance();
                    match old_wayland_display {
                        Some(wayland_display) => {
                            std::env::set_var("WAYLAND_DISPLAY", wayland_display)
                        }
                        None => std::env::remove_var("WAYLAND_DISPLAY"),
                    };
                    return instance;
                }
            }

            create_instance()
        });
    };

    cfg_if::cfg_if! {
        if #[cfg(target_family = "wasm")] {
            // On wasm, synchronously initialize the wgpu static variable.
            init_static_var(None);
        } else {
            // On other platforms, initialize the wgpu static variable in a separate thread to parallelize
            // wgpu instance initialization with other application initialization.  We block until we have
            // acquired the lock on the instance, ensuring that this function doesn't return until it is
            // safe to call `get_wgpu_instance()`.
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                init_static_var(Some(tx));
            });
            let _ = rx.recv();
        }
    }
}

/// Helper function to get a [`wgpu::Instance`].
///
/// This should always be used over [`wgpu::Instance::new`] or
/// [`wgpu::Instance::default`] to ensure that configuration is consistent
/// across the app.
fn get_wgpu_instance() -> Arc<wgpu::Instance> {
    WGPU_INSTANCE
        .lock()
        .expect("wgpu instance lock should not be poisoned")
        .as_ref()
        .expect("wgpu instance should have been initialized")
        .clone()
}

/// Returns the set of wgpu backends that we can select from.
fn wgpu_backend_options() -> wgpu::Backends {
    wgpu::Backends::from_env().unwrap_or(wgpu::Backends::all())
}

#[cfg(not(target_family = "wasm"))]
pub async fn print_wgpu_adapters(
    gpu_power_preference: GPUPowerPreference,
    backend_preference: Option<GraphicsBackend>,
    windowing_system: Option<windowing::System>,
) {
    let instance = get_wgpu_instance();
    let backends = wgpu_backend_options();
    let adapters = instance.enumerate_adapters(backends).await;

    let sorted = resources::sort_adapters(
        adapters,
        backend_preference.map(to_wgpu_backend),
        &gpu_power_preference,
        windowing_system,
        // This value is only ever true after failing to render frames, which we never attempt when
        // running in this mode.
        false, /* downrank_non_nvidia_vulkan_adapters */
    );

    for adapter in sorted {
        let info = adapter.get_info();
        let device_type = info.device_type;
        let device_name = info.name;
        let backend = info.backend;
        let driver = if info.driver.is_empty() {
            "?"
        } else {
            &info.driver
        };
        let driver_info = if info.driver_info.is_empty() {
            String::new()
        } else {
            format!(" ({})", info.driver_info)
        };
        println!("{device_type:?}: {device_name}\n\tBackend: {backend:?}\n\tDriver: {driver}{driver_info}");
    }
}

/// Returns `true` if a low power GPU is available for rendering. Typically, this is true for
/// machines with two GPUs -- a dedicated discrete high-performance GPU and a lower power
/// integrated GPU.
#[cfg(not(target_family = "wasm"))]
pub async fn is_low_power_gpu_available() -> bool {
    get_wgpu_instance()
        .enumerate_adapters(::wgpu::Backends::all())
        .await
        .iter()
        .any(|adapter| adapter.get_info().device_type == ::wgpu::DeviceType::IntegratedGpu)
}

#[cfg(target_family = "wasm")]
pub async fn is_low_power_gpu_available() -> bool {
    // We return false here because we only support WebGL (not WebGPU) on the web and the former
    // does not allow configuration of a low or high power GPU.
    false
}

#[cfg(windows)]
fn get_dx12_shader_compiler() -> Option<wgpu::Dx12Compiler> {
    let dxc_path = crate::platform::windows::DXC_PATH.get()?;

    dxc_path
        .as_ref()
        .map(|dxc_path| wgpu::Dx12Compiler::DynamicDxc {
            dxc_path: dxc_path.dxc_path.clone(),
        })
}

#[cfg(not(windows))]
fn get_dx12_shader_compiler() -> Option<wgpu::Dx12Compiler> {
    None
}

/// Converts a [`wgpu::Backend`] to a [`GraphicsBackend`].
#[cfg_attr(target_os = "macos", expect(dead_code))]
pub(crate) fn from_wgpu_backend(backend: wgpu::Backend) -> GraphicsBackend {
    match backend {
        wgpu::Backend::Noop => GraphicsBackend::Empty,
        wgpu::Backend::Vulkan => GraphicsBackend::Vulkan,
        wgpu::Backend::Metal => GraphicsBackend::Metal,
        wgpu::Backend::Dx12 => GraphicsBackend::Dx12,
        wgpu::Backend::Gl => GraphicsBackend::Gl,
        wgpu::Backend::BrowserWebGpu => GraphicsBackend::BrowserWebGpu,
    }
}

/// Converts a [`GraphicsBackend`] to a [`wgpu::Backend`].
pub(crate) fn to_wgpu_backend(backend: GraphicsBackend) -> wgpu::Backend {
    match backend {
        GraphicsBackend::Empty => wgpu::Backend::Noop,
        GraphicsBackend::Dx12 => wgpu::Backend::Dx12,
        GraphicsBackend::Vulkan => wgpu::Backend::Vulkan,
        GraphicsBackend::Gl => wgpu::Backend::Gl,
        GraphicsBackend::Metal => wgpu::Backend::Metal,
        GraphicsBackend::BrowserWebGpu => wgpu::Backend::BrowserWebGpu,
    }
}
