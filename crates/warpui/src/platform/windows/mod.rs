use std::os::windows::ffi::OsStrExt as _;

use itertools::Itertools as _;
use warp_errors::report_error;

// Re-export a couple winit types and modules as the concrete implementations
// for Windows.
pub use crate::windowing::winit::app::App;

pub(crate) static DXC_PATH: std::sync::OnceLock<Option<DXCPath>> = std::sync::OnceLock::new();

/// Path to the DXC DLLs to be used to compile DirectX shaders using DXC.
/// See https://github.com/microsoft/DirectXShaderCompiler.
#[derive(Debug)]
pub struct DXCPath {
    pub dxc_path: String,
    pub dxil_path: String,
}

pub trait AppBuilderExt {
    /// Set the AppUserModel ID, which Windows uses to attribute notifications to
    /// our correct application.
    fn set_app_user_model_id(&mut self, app_id: String);

    /// Use DXC (the newer DirectX Shader Compiler) to compile DirectX shaders.
    /// Using DXC requires the dlls within [`DXCPath`] to be available and shipped
    /// alongside the application.=
    fn use_dxc_for_directx_shader_compilation(&mut self, dxc_path: DXCPath);
}

impl AppBuilderExt for super::AppBuilder {
    fn set_app_user_model_id(&mut self, app_id: String) {
        let set_id = unsafe { set_app_user_model_id(app_id) };
        if let Err(err) = set_id {
            report_error!(anyhow::Error::new(err).context("Unable to set Windows AppUserModel ID"));
        }
    }

    fn use_dxc_for_directx_shader_compilation(&mut self, dxc_path: DXCPath) {
        if let Err(e) = DXC_PATH.set(Some(dxc_path)) {
            log::warn!("Failed to set DXC path {e:?}");
        }
    }
}

unsafe fn set_app_user_model_id(app_id: String) -> Result<(), windows::core::Error> {
    unsafe {
        let wide_string = std::ffi::OsStr::new(&app_id)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect_vec();
        windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID(windows::core::PCWSTR(
            wide_string.as_ptr(),
        ))
    }
}
