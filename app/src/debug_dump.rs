//! This module contains the code path for the [`warp_cli::Command::DumpDebugInfo`] subcommand.
//!
//! This is intended to never be used by a vast majority of users. This is only intended for users
//! who are unable to run Warp and want to provide us, the dev team, with useful debugging
//! information.
#[cfg(not(windows))]
use command::blocking::Command;
use warp_core::channel::ChannelState;
use warpui::windowing;

pub(crate) fn run() -> anyhow::Result<()> {
    println!("Warp version: {:?}", ChannelState::app_version());

    #[cfg(not(windows))]
    {
        let uname = collect_output_or_suggest_install("uname -a");
        println!("uname(1) output: {}", uname.trim_end());
    }

    #[cfg(target_os = "linux")]
    println!(
        "Package type: {:?}",
        crate::autoupdate::linux::UpdateMethod::detect()
    );

    #[cfg_attr(windows, expect(unused_mut))]
    #[cfg_attr(any(target_os = "macos", target_family = "wasm"), expect(unused))]
    let mut windowing_system: Option<windowing::System> = None;

    // On non-macOS platforms, initialize winit and wgpu.
    #[cfg(not(target_os = "macos"))]
    {
        if let Ok(event_loop) = winit::event_loop::EventLoop::new() {
            warpui::rendering::wgpu::init_wgpu_instance(Box::new(
                event_loop.owned_display_handle(),
            ));

            // Log some additional windowing system information on Linux.
            #[cfg(any(target_os = "linux", target_os = "freebsd"))]
            {
                use winit::raw_window_handle::HasDisplayHandle as _;

                if let Ok(display_handle) = event_loop.display_handle() {
                    if let Ok(system) = windowing::System::try_from(display_handle.as_raw()) {
                        println!("Windowing system: {system:?}");
                        windowing_system = Some(system);
                    }
                }

                if let Some(name) = windowing::winit::get_os_window_manager_name() {
                    println!("Window manager name: {}", name.trim_end());
                }
            }
        }
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd", windows))]
    {
        use std::ops::Deref as _;

        use crate::settings::{
            init_private_user_preferences, PreferLowPowerGPU, PreferredGraphicsBackend,
        };
        use settings::Setting as _;
        use warpui::rendering::GPUPowerPreference;

        let user_preferences = init_private_user_preferences();

        let prefer_low_power_gpu =
            PreferLowPowerGPU::read_from_preferences(user_preferences.deref()).unwrap_or_default();
        let gpu_power_preference = if prefer_low_power_gpu {
            GPUPowerPreference::LowPower
        } else {
            GPUPowerPreference::HighPerformance
        };
        let backend_preference =
            PreferredGraphicsBackend::read_from_preferences(user_preferences.deref()).flatten();

        println!("gpu_power_preference: {gpu_power_preference:?}");
        println!("backend_preference: {backend_preference:?}");
        println!("windowing_system: {windowing_system:?}");

        println!("##################################################");
        println!("# wgpu Adapters");
        println!("##################################################");
        warpui::r#async::block_on(warpui::rendering::wgpu::print_wgpu_adapters(
            gpu_power_preference,
            backend_preference,
            windowing_system,
        ));
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        let lspci_info = collect_output_or_suggest_install("lspci");
        println!("##################################################");
        println!("# lspci(8) output");
        println!("##################################################");
        println!("{lspci_info}");

        let vulkan_info = collect_output_or_suggest_install("vulkaninfo --summary");
        println!("##################################################");
        println!("# vulkaninfo(1) output");
        println!("##################################################");
        println!("{vulkan_info}");

        let egl_info = collect_output_or_suggest_install("eglinfo");
        println!("##################################################");
        println!("# eglinfo(1) output");
        println!("##################################################");
        println!("{egl_info}");
    }

    Ok(())
}

#[cfg(not(windows))]
fn collect_output_or_suggest_install(full_command: &str) -> String {
    let redirected_command = format!("{full_command} 2>&1");
    let output = Command::new("sh")
        .args(["-c", &redirected_command])
        .output();
    match output {
        Ok(output) => String::from_utf8(output.stdout).unwrap_or_default(),
        Err(err) => err.to_string(),
    }
}
