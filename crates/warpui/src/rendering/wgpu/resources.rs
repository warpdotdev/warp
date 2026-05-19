pub mod quad;
pub mod uniforms;

use std::cell::RefCell;
use std::collections::HashSet;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use crate::rendering::OnGPUDeviceSelected;
use crate::windowing;
use crate::{r#async::block_on, rendering::GPUPowerPreference};
use anyhow::{anyhow, Result};
use itertools::Itertools;
use lazy_static::lazy_static;
use pathfinder_geometry::vector::Vector2F;
use thiserror::Error;
use version_compare::Version;
use warpui_core::rendering::{GPUBackend, GPUDeviceInfo, GPUDeviceType};
use wgpu::{
    Adapter, Backend, CompositeAlphaMode, CurrentSurfaceTexture, Device, DeviceType, PresentMode,
    Queue, Surface, SurfaceConfiguration,
};

/// A mostly-arbitrary value to use as the height/width of a surface when
/// creating a default surface configuration.
///
/// 4 was chosen here because sometimes drivers care that things are a
/// multiple of 2 or 4, so this seemed like a safe choice, while being
/// small enough that any buffers that get allocated are tiny and quick to
/// create and destroy.
const SURFACE_SIZE_FOR_TESTING: u32 = 4;

lazy_static! {
    /// The minimum supported driver version for lavapipe, the Vulkan version
    /// of Mesa's llvmpipe software renderer.
    ///
    /// While lavapipe is theoretically Vulkan 1.3 compatible starting in version
    /// 22.1.2, in practice, Warp windows don't render properly until 24.0.2.
    static ref MIN_SUPPORTED_LAVAPIPE_VERSION: Version<'static> = Version::from("24.0.2")
        .expect("should not fail to parse version");

    /// The minimum supported driver version for Vulkan-backed Intel UHD integrated graphics.
    ///
    /// Some issues we've seen: PLAT-744 and PLAT-599.
    /// Mesa changelog mentions a fix for flickering on Intel UHD:
    /// https://docs.mesa3d.org/relnotes/21.3.6.html#:~:text=Flickering%20Intel%20Uhd%20620%20Graphics
    static ref MIN_SUPPORTED_INTEL_UHD_VERSION: Version<'static> = Version::from("21.3.6")
        .expect("should not fail to parse version");

    /// Nvidia drivers version 535 have problems with Wayland window managers, e.g. PLAT-667 and
    /// PLAT-674.
    static ref MIN_SUPPORTED_NVIDIA_VERSION: Version<'static> = Version::from("545")
        .expect("should not fail to parse version");

    static ref MAX_SUPPORTED_NVIDIA_VERSION_ON_WINDOWS: Version<'static> = Version::from("572")
        .expect("should not fail to parse version");
}

/// Set of resources needed to render using wgpu.
pub struct Resources {
    pub device: wgpu::Device,
    pub device_lost: Arc<AtomicBool>,
    pub queue: Queue,
    pub adapter: Adapter,
    pub surface: Surface<'static>,
    pub surface_config: RefCell<SurfaceConfiguration>,
    pub supported_backends: Vec<wgpu::Backend>,
    uniforms: uniforms::Uniforms,
    quad: quad::Resources,
}

impl Resources {
    /// Attempts to construct a new instance of [`Resources`] via the provided `window_handle`.
    pub fn new(
        window_handle: impl Into<wgpu::SurfaceTarget<'static>> + wgpu::rwh::HasDisplayHandle,
        gpu_power_preference: GPUPowerPreference,
        backend_preference: Option<wgpu::Backend>,
        on_gpu_device_selected: &OnGPUDeviceSelected,
        initial_surface_size: Vector2F,
        downrank_non_nvidia_vulkan_adapters: bool,
    ) -> Result<Self> {
        let windowing_system = window_handle.display_handle()?.as_raw().try_into().ok();

        let instance = super::get_wgpu_instance();
        let surface = instance.create_surface(window_handle)?;

        let backends = super::wgpu_backend_options();
        // All of the WGPU initialization functions are asynchronous. For simplicity while
        // prototyping, we just use `block_on` to force them to be synchronous.
        block_on(async {
            let (adapter, device, queue, surface_config, supported_backends) = select_adapter(
                &instance,
                &surface,
                backends,
                backend_preference,
                gpu_power_preference,
                initial_surface_size,
                windowing_system,
                downrank_non_nvidia_vulkan_adapters,
            )
            .await
            .ok_or_else(|| anyhow!("No usable wgpu adapter was found"))?;
            let adapter_info = adapter.get_info();

            log::info!(
                "Using {:?} {:?} ({}) for rendering new window.",
                adapter_info.backend,
                adapter_info.device_type,
                adapter_info.name,
            );

            on_gpu_device_selected(device_info_from_adapter_info(adapter_info));

            let uniforms = uniforms::Uniforms::new(&device);
            let quad = quad::Resources::new(&device);

            let device_lost = Arc::new(AtomicBool::new(false));

            let device_lost_clone = device_lost.clone();
            device.set_device_lost_callback(move |device_lost_reason, message| {
                device_lost_clone.store(true, Ordering::SeqCst);
                log::warn!("The current device is lost. Reason: {device_lost_reason:?}. Message: {message}")
            });

            Ok(Self {
                device,
                device_lost,
                queue,
                adapter,
                surface,
                surface_config: surface_config.into(),
                supported_backends: supported_backends.into_iter().collect(),
                uniforms,
                quad,
            })
        })
    }

    pub fn uniform_bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        self.uniforms.bind_group_layout()
    }

    pub fn configure_render_pass<'a>(
        &'a self,
        render_pass: &mut wgpu::RenderPass<'a>,
        drawable_size: Vector2F,
    ) {
        self.uniforms
            .configure_render_pass(render_pass, drawable_size, self);
        self.quad.configure_render_pass(render_pass);
    }

    /// Updates the size of the underlying surface.
    pub fn update_surface_size(&self, size: Vector2F) -> Result<(), SurfaceConfigureError> {
        if size.x() > 0. && size.y() > 0. {
            let mut surface_config = self.surface_config.borrow_mut();
            surface_config.width = size.x() as u32;
            surface_config.height = size.y() as u32;
            block_on(configure_surface(
                &self.surface,
                &self.device,
                &surface_config,
            ))
        } else {
            Ok(())
        }
    }

    /// Gets the next surface texture to render to.
    pub fn get_surface_texture(&self) -> Result<wgpu::SurfaceTexture, GetSurfaceTextureError> {
        let Resources {
            surface,
            device,
            surface_config,
            ..
        } = self;

        let error = match get_surface_texture(surface) {
            Ok(texture) => return Ok(texture),
            Err(error) => error,
        };

        log::warn!("Encountered error while getting the next swap chain texture: {error:#}");
        match error {
            GetSurfaceTextureError::Timeout
            | GetSurfaceTextureError::Validation
            | GetSurfaceTextureError::Occluded
            | GetSurfaceTextureError::ConfigurationError(_) => {
                // Skip this frame and hope it resolves itself by the next one.
                log::info!("Skipping rendering the current frame...");
                Err(error)
            }
            GetSurfaceTextureError::Lost | GetSurfaceTextureError::Outdated => {
                block_on(configure_surface(surface, device, &surface_config.borrow()))
                    .map_err(GetSurfaceTextureError::ConfigurationError)?;

                match get_surface_texture(surface) {
                    Ok(texture) => {
                        log::info!("Successfully recreated the swap chain");
                        Ok(texture)
                    }
                    Err(e) => {
                        log::warn!("Failed to recreate the swap chain: {e:#}");
                        Err(e)
                    }
                }
            }
        }
    }
}

fn device_info_from_adapter_info(adapter_info: wgpu::AdapterInfo) -> GPUDeviceInfo {
    let device_type = match adapter_info.device_type {
        DeviceType::Other => GPUDeviceType::Other,
        DeviceType::IntegratedGpu => GPUDeviceType::IntegratedGpu,
        DeviceType::DiscreteGpu => GPUDeviceType::DiscreteGpu,
        DeviceType::VirtualGpu => GPUDeviceType::VirtualGpu,
        DeviceType::Cpu => GPUDeviceType::Cpu,
    };
    let backend = match adapter_info.backend {
        Backend::Noop => GPUBackend::Empty,
        Backend::Vulkan => GPUBackend::Vulkan,
        Backend::Metal => GPUBackend::Metal,
        Backend::Dx12 => GPUBackend::Dx12,
        Backend::Gl => GPUBackend::Gl,
        Backend::BrowserWebGpu => GPUBackend::BrowserWebGpu,
    };
    GPUDeviceInfo {
        device_type,
        device_name: adapter_info.name,
        driver_name: adapter_info.driver,
        driver_info: adapter_info.driver_info,
        backend,
    }
}

/// Selects the adapter to use to render to the given surface.
///
/// The adapter is selected from the set of adapters that support the given
/// backends, and priority is determined by the power preference.
///
/// This is inspired by the implementation of `request_adapter` in `wgpu_core`:
/// https://github.com/gfx-rs/wgpu/blob/badb3c88ea29acb159d333e2f60b1cc305bbd512/wgpu-core/src/instance.rs#L857
#[allow(clippy::too_many_arguments)]
#[cfg_attr(target_family = "wasm", allow(unused_variables))]
async fn select_adapter(
    instance: &wgpu::Instance,
    surface: &wgpu::Surface<'static>,
    backends: wgpu::Backends,
    backend_preference: Option<wgpu::Backend>,
    gpu_power_preference: GPUPowerPreference,
    initial_surface_size: Vector2F,
    windowing_system: Option<windowing::System>,
    downrank_non_nvidia_vulkan_adapters: bool,
) -> Option<(
    Adapter,
    Device,
    Queue,
    SurfaceConfiguration,
    HashSet<wgpu::Backend>,
)> {
    cfg_if::cfg_if! {
        if #[cfg(target_family = "wasm")] {
            let power_preference = match gpu_power_preference {
                GPUPowerPreference::LowPower => wgpu::PowerPreference::LowPower,
                GPUPowerPreference::HighPerformance => wgpu::PowerPreference::HighPerformance,
            };
            let request_adapter_options = wgpu::RequestAdapterOptions {
                power_preference,
                force_fallback_adapter: false,
                compatible_surface: Some(surface),
            };

            let adapter = instance.request_adapter(&request_adapter_options).await.ok()?;
            let adapters = [adapter].into_iter();
        } else {
            let adapters = instance
                .enumerate_adapters(backends)
                .await
                .into_iter();
            }
    }

    log::info!("Enabled wgpu backends: {backends:?}");

    log::info!("Available wgpu adapters (in priority order):");

    let sorted_adapters = sort_adapters(
        adapters.collect(),
        backend_preference,
        &gpu_power_preference,
        windowing_system,
        downrank_non_nvidia_vulkan_adapters,
    );

    let adapters = sorted_adapters
        // Filter out any unsupported adapters and log information about each one.
        .filter(|adapter| is_supported_adapter(adapter, surface))
        // While we don't strictly need to collect the iterator into a vector,
        // this ensures we log adapter information for all adapters.  (Omitting
        // this means the iterator is lazily evaluated, and we'll only print
        // adapter information up until the point where we find a working one.)
        .collect_vec();

    let supported_backends = adapters
        .iter()
        .map(|adapter| adapter.get_info().backend)
        .collect::<HashSet<_>>();

    for adapter in adapters {
        if let Some((device, queue, surface_config)) =
            initialize_device(&adapter, surface, initial_surface_size).await
        {
            return Some((adapter, device, queue, surface_config, supported_backends));
        }
    }

    None
}

/// Sorts adapters according to user preference, stability, and performance.
///
/// All sorts performed here should be stable, ensuring that the relative ordering of previous
/// sorting steps is preserved.
pub(super) fn sort_adapters(
    adapters: Vec<wgpu::Adapter>,
    backend_preference: Option<wgpu::Backend>,
    gpu_power_preference: &GPUPowerPreference,
    windowing_system: Option<windowing::System>,
    downrank_non_nvidia_vulkan_adapters: bool,
) -> impl Iterator<Item = wgpu::Adapter> {
    adapters
        .into_iter()
        // Sort adapters by backend priority.
        .sorted_by_cached_key(|adapter| adapter_backend_sort_func(adapter, backend_preference))
        .sorted_by_cached_key(adapter_supported_features)
        // Sort adapters based on low/high power preferences.
        .sorted_by_cached_key(power_preference_adapter_sort_func(gpu_power_preference))
        // Sort adapters that we know have some issues towards the end of the list.
        .sorted_by_cached_key(|adapter| {
            adapter_stability_sort_func(
                adapter,
                windowing_system,
                downrank_non_nvidia_vulkan_adapters,
            )
        })
}

/// Returns whether or not a particular adapter is supported and can be used
/// for rendering.
fn is_supported_adapter(adapter: &wgpu::Adapter, surface: &wgpu::Surface) -> bool {
    let can_present = adapter.is_surface_supported(surface);

    let supported_texture_format = surface
        .get_default_config(adapter, SURFACE_SIZE_FOR_TESTING, SURFACE_SIZE_FOR_TESTING)
        .map(|config| config.format);
    let supported_alpha_modes = surface.get_capabilities(adapter).alpha_modes;

    // Log information about the adapter (to assist with debugging).
    let info = adapter.get_info();
    let device_type = &info.device_type;
    let device_name = &info.name;
    let backend = &info.backend;
    let driver = if info.driver.is_empty() {
        "Unknown"
    } else {
        &info.driver
    };
    let driver_info = if info.driver_info.is_empty() {
        String::new()
    } else {
        format!(" ({})", info.driver_info)
    };
    log::info!("{device_type:?}: {device_name}\n\tBackend: {backend:?}\n\tDriver: {driver}{driver_info}\n\tCan present: {can_present}\n\tSupported texture format: {supported_texture_format:?}\n\tSupported alpha mode: {supported_alpha_modes:?}");

    can_present && supported_texture_format.is_some()
}

/// Encode levels of preference for graphics adapters based on features they enable. This takes
/// precedence under the "GPU power preference".
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum AdapterFeatureSet {
    /// No features are hindered by what this adapter supports.
    Full = 0,
    /// Some non-critical features not supported by the adapter.
    MissingMinorFeatures = 1,
}

fn adapter_supported_features(adapter: &Adapter) -> AdapterFeatureSet {
    if adapter_has_rendering_offset_bug(&adapter.get_info()) {
        log::warn!("Deprioritizing OpenGL-backed Intel UHD adapter");
        AdapterFeatureSet::MissingMinorFeatures
    } else {
        AdapterFeatureSet::Full
    }
}

fn is_nvidia_adapter(adapter_info: &wgpu::AdapterInfo) -> bool {
    adapter_info.driver == "NVIDIA"
}

fn is_vulkan_nvidia_adapter(adapter_info: &wgpu::AdapterInfo) -> bool {
    // Only consider Vulkan adapters using the Nvidia driver.
    adapter_info.backend == wgpu::Backend::Vulkan && is_nvidia_adapter(adapter_info)
}

/// Returns whether or not the provided adapter is an unsupported Nvidia driver version for warpui
/// to render properly.
fn is_older_nvidia_adapter(adapter_info: &wgpu::AdapterInfo) -> bool {
    if !is_vulkan_nvidia_adapter(adapter_info) {
        return false;
    }

    let Some(version) = Version::from(&adapter_info.driver_info) else {
        // Log an error so we know this occurred and can improve the logic as-needed.
        log::error!(
            "Unable to parse Vulkan-backed Nvidia adapter version {:?}; de-prioritizing out of an \
            abundance of caution.",
            adapter_info.driver_info
        );
        return true;
    };

    version < *MIN_SUPPORTED_NVIDIA_VERSION
}

/// Returns whether this adapter is a newer Windows NVIDIA adapter using a non-DX12 backend.
/// On NVIDIA drivers 572 and later, the default value of "auto" for the "Vulkan / OpenGL Present
/// Method" can cause crashes when creating multiple windows, so we downrank it.
fn is_newer_nondx12_nvidia_adapter_on_windows(adapter_info: &wgpu::AdapterInfo) -> bool {
    if !cfg!(windows) {
        return false;
    }

    if !is_nvidia_adapter(adapter_info) {
        return false;
    }

    if adapter_info.backend == Backend::Dx12 {
        return false;
    }

    let Some(version) = Version::from(&adapter_info.driver_info) else {
        // Log an error so we know this occurred and can improve the logic as-needed.
        log::error!(
            "Unable to parse Nvidia adapter version {:?} adapter_info.driver_info",
            adapter_info.driver_info
        );
        return false;
    };

    version >= *MAX_SUPPORTED_NVIDIA_VERSION_ON_WINDOWS
}

/// Returns whether this adapter is the integrated OpenGL driver for Windows running in Parallels.
/// It caused problems with theme background images.
/// https://linear.app/warpdotdev/issue/CORE-3692/background-images-broken-in-parallels
fn is_gl_to_metal_adapter_on_windows_in_parallels(adapter_info: &wgpu::AdapterInfo) -> bool {
    cfg!(windows)
        && adapter_info.backend == Backend::Gl
        && adapter_info.device_type == DeviceType::IntegratedGpu
        && adapter_info.driver_info.to_lowercase().contains("metal")
        && adapter_info.name.to_lowercase().starts_with("parallels")
}

/// Returns whether or not the provided adapter is an unsupported Intel UHD Mesa driver version for
/// warpui to render properly. Currently, we limit this to "Intel UHD Graphics 620", but we do have
/// some suspicion that more Intel UHD devices are affected, e.g. PLAT-599 has a "Intel(R) UHD
/// Graphics (TGL GT1)" user seeing the exact same issue.
fn is_older_vulkan_intel_uhd_adapter(adapter_info: &wgpu::AdapterInfo) -> bool {
    if adapter_info.backend != wgpu::Backend::Vulkan
        || adapter_info.device_type != wgpu::DeviceType::IntegratedGpu
        || !adapter_info.name.contains("Intel(R) HD Graphics 620")
    {
        return false;
    }

    mesa_driver_version_is_below_minimum(
        &adapter_info.driver_info,
        &MIN_SUPPORTED_INTEL_UHD_VERSION,
    )
}

/// Returns true if this is:
/// 1) An Intel UHD 620 Graphics device
/// 2) Using the Vulkan backend
/// 3) On Windows
///
/// We have indication that this specific device is unstable on Windows so we ignore it in the
/// hopes that there is a DX12 or GL version of this adapter that is more stable.
fn is_intel_uhd_620_adapter_on_windows_with_vulkan_backend(
    adapter_info: &wgpu::AdapterInfo,
) -> bool {
    cfg!(windows)
        && adapter_info.backend == Backend::Vulkan
        && adapter_info.device_type == DeviceType::IntegratedGpu
        && (adapter_info.name.contains("Intel(R) UHD Graphics 620")
            || adapter_info.name.contains("Intel(R) HD Graphics 620"))
}

/// Returns whether the given adapter is known to have a rendering offset bug on Windows.
///
/// Certain Intel integrated GPU drivers using the GL backend render the scene at an offset from
/// the window bounds when window decorations are disabled. The offset matches the size of the
/// window decorations (e.g. title bar height). Enabling native window decorations fixes the
/// alignment.
///
/// See: https://github.com/warpdotdev/Warp/issues/6120
pub fn adapter_has_rendering_offset_bug(adapter_info: &wgpu::AdapterInfo) -> bool {
    if !cfg!(windows) {
        return false;
    }

    if adapter_info.backend != Backend::Gl || adapter_info.device_type != DeviceType::IntegratedGpu
    {
        return false;
    }

    // Known affected Intel integrated GPU models. This list is based on user reports from
    // https://github.com/warpdotdev/Warp/issues/6120.
    let affected_models = [
        "Intel(R) HD Graphics 4000",
        "Intel(R) HD Graphics 4400",
        "Intel(R) HD Graphics 4600",
        "Intel(R) HD Graphics 5500",
        "Intel(R) HD Graphics P4600",
        "Intel(R) Iris(TM) Pro Graphics 5200",
        "Intel(R) Iris(TM) Graphics 6100",
    ];

    affected_models
        .iter()
        .any(|model| adapter_info.name.contains(model))
}

/// Returns whether the adapter is the Broadcom V3D Vulkan driver (V3DV).
///
/// The V3D Vulkan driver on Raspberry Pi and similar ARM SBCs have panics and flickering issues.
/// The logs have warnings about these drivers missing the `FULL_DRAW_INDEX_UINT32` downlevel flag
/// but if it's unclear if that is the actual cause. See:
/// https://github.com/warpdotdev/warp/issues/10618
/// https://github.com/warpdotdev/warp/issues/4879
fn is_v3d_vulkan_adapter(adapter_info: &wgpu::AdapterInfo) -> bool {
    cfg!(target_os = "linux")
        && adapter_info.backend == wgpu::Backend::Vulkan
        && adapter_info.driver.contains("V3DV")
        && adapter_info.name.starts_with("V3D 4.2.14.")
}

/// Checks whether the provided adapter info describes a lavapipe
/// (Vulkan llvmpipe) adapter that may not work properly with warpui.
fn is_older_lavapipe_adapter(adapter_info: &wgpu::AdapterInfo) -> bool {
    // Only consider Vulkan adapters using the llvmpipe driver.
    if adapter_info.backend != wgpu::Backend::Vulkan || adapter_info.driver != "llvmpipe" {
        return false;
    }

    mesa_driver_version_is_below_minimum(&adapter_info.driver_info, &MIN_SUPPORTED_LAVAPIPE_VERSION)
}

fn mesa_driver_version_is_below_minimum(info_str: &str, min_version: &Version) -> bool {
    let &[name, version, ..] = info_str.splitn(3, ' ').collect_vec().as_slice() else {
        // Log an error so we know this occurred and can improve the logic as-needed.
        log::error!(
            "Encountered Mesa driver info {info_str:?} with an unexpected format! (too few parts)"
        );
        return false;
    };

    // Perform an extra check that we parsed the driver info string properly.
    if name.trim() != "Mesa" {
        // Log an error so we know this occurred and can improve the logic as-needed.
        log::error!(
            "Encountered Mesa driver info {info_str:?} with an unexpected format! (name != Mesa)"
        );
        return false;
    }

    let manifest = version_compare::Manifest {
        // We only care about major, minor, and patch versions.
        max_depth: Some(3),
        ..Default::default()
    };
    let Some(version) = Version::from_manifest(version, &manifest) else {
        // Log an error so we know this occurred and can improve the logic as-needed.
        log::error!(
            "Unable to parse Mesa version {version:?}; de-prioritizing out of an abundance of caution."
        );
        return true;
    };

    version < *min_version
}

/// Creates a device and command queue for the given adapter that is guaranteed
/// to be able to create a swapchain for the surface.
async fn initialize_device(
    adapter: &Adapter,
    surface: &Surface<'static>,
    initial_surface_size: Vector2F,
) -> Option<(Device, Queue, SurfaceConfiguration)> {
    log::info!(
        "Verifying adapter \"{}\" is valid...",
        adapter.get_info().name
    );

    // `Limits::downlevel_webgl2_defaults` gives very conservative defaults. We want to keep these
    // limits low in order to make sure we remain compatible with lower-end devices. One exception
    // to this is sizes of textures. `using_resolution` increases the size limits on textures. We
    // need this because users' displays often exceed the downleveled default limits of 2048px.
    // Here, we increase that to the ceiling of what this adapter is capable of.
    let mut limits = wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits());
    // Set a higher minimum number of variables that can be passed between shader stages.
    limits.max_inter_stage_shader_variables = 15;

    limits.max_mesh_output_layers = 0;

    let (device, queue) = match adapter
        .request_device(&wgpu::DeviceDescriptor {
            // Use the broadest/most permissive device requirements
            // so that we can run on as many machines as possible.
            // If we use any WGSL features that aren't included in
            // these defaults, we can add specific overrides as needed.
            required_limits: limits,
            ..Default::default()
        })
        .await
    {
        Ok(device_and_queue) => device_and_queue,
        Err(err) => {
            log::warn!("Failed to create a logical device: {err:#}");
            return None;
        }
    };

    // Ensure that we're able to create a swapchain before we treat the device
    // as valid.
    let Some(surface_config) = create_surface_config(adapter, surface, initial_surface_size) else {
        log::warn!("Failed to get default surface configuration");
        return None;
    };

    match configure_surface(surface, &device, &surface_config).await {
        Ok(_) => Some((device, queue, surface_config)),
        Err(err) => {
            log::warn!("Failed to create swapchain: {err:#}");
            None
        }
    }
}

/// Returns a priority for an adapter based on backend type, to be used as a
/// sort function.
///
/// This matches the order used by wgpu; see:
/// https://github.com/gfx-rs/wgpu/blob/v0.18/wgpu-core/src/instance.rs#L869-L913
#[cfg(not(windows))]
fn adapter_backend_sort_func(
    adapter: &wgpu::Adapter,
    backend_preference: Option<wgpu::Backend>,
) -> usize {
    let backend = adapter.get_info().backend;
    if backend_preference.is_some_and(|pref| pref == backend) {
        return 0;
    }
    match backend {
        wgpu::Backend::Vulkan => 1,
        wgpu::Backend::Metal => 2,
        wgpu::Backend::Dx12 => 3,
        wgpu::Backend::BrowserWebGpu => 4,
        wgpu::Backend::Gl => 5,
        wgpu::Backend::Noop => 6,
    }
}

/// Returns a priority for an adapter based on backend type, to be used as a
/// sort function.
///
/// This prioritizes DX12 on Windows which is more reliable. See this issue:
/// https://github.com/gfx-rs/wgpu/issues/2719
#[cfg(windows)]
fn adapter_backend_sort_func(
    adapter: &wgpu::Adapter,
    backend_preference: Option<wgpu::Backend>,
) -> usize {
    let backend = adapter.get_info().backend;
    if backend_preference.is_some_and(|pref| pref == backend) {
        return 0;
    }
    match backend {
        // On Windows, we prefer DirectX 12 over Vulkan.  Given that no other
        // platform supports DX12 at all, there's no need to condition this
        // ranking on OS.
        wgpu::Backend::Dx12 => 1,
        wgpu::Backend::Vulkan => 2,
        wgpu::Backend::Gl => 3,
        wgpu::Backend::Metal => 4,
        wgpu::Backend::BrowserWebGpu => 5,
        wgpu::Backend::Noop => 6,
    }
}

/// Returns a priority for an adapter based on our expectations of its
/// stability.
///
/// This should be used to deprioritize adapters where they _may not_
/// work, but we're not so confident that they are broken that we fully filter
/// them out.  Ultimately, if the user only has one adapter, it's better for
/// us to attempt to use it than for us to give up without trying.
fn adapter_stability_sort_func(
    adapter: &wgpu::Adapter,
    windowing_system: Option<windowing::System>,
    downrank_non_nvidia_vulkan_adapters: bool,
) -> AdapterSupport {
    let adapter_info = adapter.get_info();

    let window_server_is_wayland = matches!(
        windowing_system,
        Some(windowing::System::Wayland) | Some(windowing::System::X11 { is_x_wayland: true })
    );

    if downrank_non_nvidia_vulkan_adapters
        && adapter_info.backend == Backend::Vulkan
        && !is_vulkan_nvidia_adapter(&adapter_info)
    {
        log::info!("Deprioritizing non-NVIDIA Vulkan adapter (the PRIME performance profile is likely enabled)");
        return AdapterSupport::Unsupported;
    }

    if is_v3d_vulkan_adapter(&adapter_info) {
        log::warn!("Deprioritizing Vulkan-backed V3D adapter");
        return AdapterSupport::Unsupported;
    }

    if is_intel_uhd_620_adapter_on_windows_with_vulkan_backend(&adapter_info) {
        log::warn!("Deprioritizing Vulkan-backed Intel UHD 620 adapter");
        return AdapterSupport::SupportedWithIssues;
    }

    if is_older_vulkan_intel_uhd_adapter(&adapter_info) {
        log::warn!(
            "Deprioritizing Vulkan-backed Intel UHD adapter due to Mesa < {} (unsupported)",
            *MIN_SUPPORTED_INTEL_UHD_VERSION
        );
        AdapterSupport::SupportedWithIssues
    }
    // Deprioritize older lavapipe adapters where we have evidence that they are less stable.
    else if is_older_lavapipe_adapter(&adapter_info) {
        log::warn!(
            "Deprioritizing Vulkan-backed llvmpipe adapter due to Mesa < {} (unsupported)",
            *MIN_SUPPORTED_LAVAPIPE_VERSION
        );
        AdapterSupport::Unsupported
    // Same with Nvidia drivers, though this is only an issue with a Wayland window server.
    } else if window_server_is_wayland && is_older_nvidia_adapter(&adapter_info) {
        log::warn!(
            "Deprioritizing Vulkan-backed Nvidia adapter due to version < {} (unsupported).\nSee \
            the \"Graphics\" secion of our docs here: \
            https://docs.warp.dev/help/known-issues#linux-1",
            *MIN_SUPPORTED_NVIDIA_VERSION
        );
        AdapterSupport::Unsupported
    } else if is_newer_nondx12_nvidia_adapter_on_windows(&adapter_info) {
        log::warn!(
            "Deprioritizing non DX12 Nvidia adapter due to version > {} (unsupported). Newer NVIDIA \
            drivers can crash if multiple windows are created if the `Vulkan / OpenGL Present Method\
             NVIDIA setting is set to `Auto` or `Prefer layered on DXGI Swapchain`.",
            *MAX_SUPPORTED_NVIDIA_VERSION_ON_WINDOWS
        );
        AdapterSupport::SupportedWithIssues
    } else if is_gl_to_metal_adapter_on_windows_in_parallels(&adapter_info) {
        log::warn!("Deprioritizing integrated OpenGL Windows Parallels adapter.");
        AdapterSupport::SupportedWithIssues
    } else {
        AdapterSupport::Supported
    }
}

/// Encode levels of preference for graphics adapters based on application stability. This takes
/// precedence over the "GPU power preference". We've seen varying severities of graphics issues on
/// Linux and Windows.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum AdapterSupport {
    /// The adapter has no known issues.
    Supported = 0,
    /// The adapter is somewhat usable, but there have been some problems.
    SupportedWithIssues = 1,
    /// The adapter is basically not viable. Warpui will either crash or not render.
    Unsupported = 2,
}

/// Returns a function that computes the priority for an adapter based on
/// device type, to be used as a sort function.
///
/// This matches the order used by wgpu; see:
/// https://github.com/gfx-rs/wgpu/blob/v0.18/wgpu-core/src/instance.rs#L953-L954
fn power_preference_adapter_sort_func(
    pref: &GPUPowerPreference,
) -> impl FnMut(&wgpu::Adapter) -> usize {
    match pref {
        GPUPowerPreference::LowPower => {
            |adapter: &wgpu::Adapter| match adapter.get_info().device_type {
                wgpu::DeviceType::IntegratedGpu => 0,
                wgpu::DeviceType::DiscreteGpu => 1,
                wgpu::DeviceType::Other => 2,
                wgpu::DeviceType::VirtualGpu => 3,
                wgpu::DeviceType::Cpu => 4,
            }
        }
        GPUPowerPreference::HighPerformance => {
            |adapter: &wgpu::Adapter| match adapter.get_info().device_type {
                wgpu::DeviceType::DiscreteGpu => 0,
                wgpu::DeviceType::IntegratedGpu => 1,
                wgpu::DeviceType::Other => 2,
                wgpu::DeviceType::VirtualGpu => 3,
                wgpu::DeviceType::Cpu => 4,
            }
        }
    }
}

fn create_surface_config(
    adapter: &Adapter,
    surface: &Surface,
    initial_surface_size: Vector2F,
) -> Option<SurfaceConfiguration> {
    let mut config = surface.get_default_config(
        adapter,
        initial_surface_size.x() as u32,
        initial_surface_size.y() as u32,
    )?;
    // Make sure we're not using an sRGB format.
    config.format = config.format.remove_srgb_suffix();

    let caps = surface.get_capabilities(adapter);
    // COPY_SRC is only needed to support integration test frame capture via
    // request_frame_capture. It is not required for normal rendering.
    #[cfg(feature = "integration_tests")]
    if caps.usages.contains(wgpu::TextureUsages::COPY_SRC) {
        config.usage |= wgpu::TextureUsages::COPY_SRC;
    }

    // Use a non-vsync presentation mode for reduced input delay.  This could
    // cause visual tearing on present, but we're ok with paying that cost to
    // improve responsiveness.
    config.present_mode = PresentMode::AutoNoVsync;

    // Explicitly request a non-opaque alpha compositing mode, if available.
    // Without this, transparent surfaces don't work on native Wayland.
    if caps
        .alpha_modes
        .contains(&CompositeAlphaMode::PostMultiplied)
        && adapter.get_info().backend != wgpu::Backend::Dx12
    {
        config.alpha_mode = CompositeAlphaMode::PostMultiplied;
    } else if caps
        .alpha_modes
        .contains(&CompositeAlphaMode::PreMultiplied)
    {
        config.alpha_mode = CompositeAlphaMode::PreMultiplied;
    } else if caps.alpha_modes.contains(&CompositeAlphaMode::Inherit) {
        config.alpha_mode = CompositeAlphaMode::Inherit;
    } else {
        config.alpha_mode = CompositeAlphaMode::Auto;
    }

    Some(config)
}

#[derive(Error, Debug)]
pub enum GetSurfaceTextureError {
    #[error("Timeout while getting next surface texture")]
    Timeout,
    #[error("Window is occluded and cannot be presented to")]
    Occluded,
    #[error("Surface configuration outdated")]
    Outdated,
    #[error("Device lost")]
    Lost,
    #[error("Validation error")]
    Validation,
    #[error("Failed to configure surface")]
    ConfigurationError(SurfaceConfigureError),
}

fn get_surface_texture(
    surface: &Surface<'_>,
) -> Result<wgpu::SurfaceTexture, GetSurfaceTextureError> {
    let error = match surface.get_current_texture() {
        CurrentSurfaceTexture::Success(texture) | CurrentSurfaceTexture::Suboptimal(texture) => {
            return Ok(texture)
        }
        CurrentSurfaceTexture::Timeout => GetSurfaceTextureError::Timeout,
        CurrentSurfaceTexture::Occluded => GetSurfaceTextureError::Occluded,
        CurrentSurfaceTexture::Outdated => GetSurfaceTextureError::Outdated,
        CurrentSurfaceTexture::Lost => GetSurfaceTextureError::Lost,
        CurrentSurfaceTexture::Validation => GetSurfaceTextureError::Validation,
    };
    Err(error)
}

/// Represents an error that occurred when configuring a surface.
#[derive(Error, Debug)]
pub enum SurfaceConfigureError {
    #[error("Failed to configure surface: {source:#}\n\nDesired configuration: {config:#?}")]
    Error {
        /// The underlying error.
        #[source]
        source: wgpu::Error,
        /// The desired configuration.
        config: SurfaceConfiguration,
    },
}

/// Configures the provided surface.
async fn configure_surface(
    surface: &Surface<'_>,
    device: &Device,
    surface_config: &SurfaceConfiguration,
) -> Result<(), SurfaceConfigureError> {
    let error_scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    surface.configure(device, surface_config);
    match error_scope.pop().await {
        Some(err) => Err(SurfaceConfigureError::Error {
            source: err,
            config: surface_config.clone(),
        }),
        None => Ok(()),
    }
}

#[cfg(test)]
#[path = "resources_tests.rs"]
mod tests;
