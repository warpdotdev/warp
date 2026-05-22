use std::mem::transmute;
use std::path::Path;

use thiserror::Error;
use warp_util::path::TargetDirError;
use windows::core::{s, HRESULT, HSTRING, PCWSTR};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Console::{COORD, HPCON};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

const CREATE_PSUEDOCONSOLE_FN_NAME: &str = "CreatePsuedoConsole";
const RESIZE_PSUEDOCONSOLE_FN_NAME: &str = "ResizePsuedoConsole";
const CLOSE_PSUEDOCONSOLE_FN_NAME: &str = "ClosePsuedoConsole";
const SHOW_HIDE_PSUEDOCONSOLE_FN_NAME: &str = "ShowHidePseudoConsole";
const RELEASE_PSUEDOCONSOLE_FN_NAME: &str = "ReleasePseudoConsole";

type CreatePseudoConsoleFn =
    unsafe extern "system" fn(COORD, HANDLE, HANDLE, u32, *mut HPCON) -> HRESULT;
type ResizePseudoConsoleFn = unsafe extern "system" fn(HPCON, COORD) -> HRESULT;
type ClosePseudoConsoleFn = unsafe extern "system" fn(HPCON);
type ShowHidePseudoConsoleFn = unsafe extern "system" fn(HPCON, bool) -> HRESULT;
type ReleasePseudoConsoleFn = unsafe extern "system" fn(HPCON) -> HRESULT;

pub struct ConptyApi {
    /// Function pointer for CreatePseudoConsole.
    create: CreatePseudoConsoleFn,
    /// Function pointer for ResizePseudoConsole.
    resize: ResizePseudoConsoleFn,
    /// Function pointer for ClosePseudoConsole.
    close: ClosePseudoConsoleFn,
    /// Function pointer for ShowHidePseudoConsole.
    show_hide: ShowHidePseudoConsoleFn,
    /// Function pointer for ReleasePseudoConsole.
    release: ReleasePseudoConsoleFn,
}

#[derive(Error, Debug)]
pub enum ConptyApiError {
    #[error("Failed to construct target directory: {0}")]
    NoTargetDirectory(#[from] TargetDirError),
    #[error(
        "Failed to load ConPTY library module: {windows_error:#}. DLL file exists: {dll_file_exists:?}"
    )]
    LoadLibraryFailed {
        #[source]
        windows_error: windows::core::Error,
        dll_file_exists: Result<bool, std::io::Error>,
    },
    #[error("Failed to get procedure address for {fn_name:?}")]
    GetProcAddressFailed { fn_name: String },
}

impl ConptyApi {
    pub(super) unsafe fn load() -> Result<Self, ConptyApiError> {
        type LoadedFn = unsafe extern "system" fn() -> isize;

        let hstring = HSTRING::from("conpty.dll");
        let dll_file_path = PCWSTR::from_raw(hstring.as_ptr());

        let conpty_module = match LoadLibraryW(dll_file_path) {
            Ok(conpty_module) => conpty_module,
            Err(windows_error) => {
                let dll_file_exists = Path::new("./conpty.dll").try_exists();
                return Err(ConptyApiError::LoadLibraryFailed {
                    windows_error,
                    dll_file_exists,
                });
            }
        };
        let Some(create) = GetProcAddress(conpty_module, s!("CreatePseudoConsole"))
            .map(|create_fn| transmute::<LoadedFn, CreatePseudoConsoleFn>(create_fn))
        else {
            return Err(ConptyApiError::GetProcAddressFailed {
                fn_name: CREATE_PSUEDOCONSOLE_FN_NAME.to_string(),
            });
        };
        let Some(resize) = GetProcAddress(conpty_module, s!("ResizePseudoConsole"))
            .map(|resize_fn| transmute::<LoadedFn, ResizePseudoConsoleFn>(resize_fn))
        else {
            return Err(ConptyApiError::GetProcAddressFailed {
                fn_name: RESIZE_PSUEDOCONSOLE_FN_NAME.to_string(),
            });
        };
        let Some(close) = GetProcAddress(conpty_module, s!("ClosePseudoConsole"))
            .map(|close_fn| transmute::<LoadedFn, ClosePseudoConsoleFn>(close_fn))
        else {
            return Err(ConptyApiError::GetProcAddressFailed {
                fn_name: CLOSE_PSUEDOCONSOLE_FN_NAME.to_string(),
            });
        };
        let Some(show_hide) = GetProcAddress(conpty_module, s!("ConptyShowHidePseudoConsole"))
            .map(|show_hide_fn| transmute::<LoadedFn, ShowHidePseudoConsoleFn>(show_hide_fn))
        else {
            return Err(ConptyApiError::GetProcAddressFailed {
                fn_name: SHOW_HIDE_PSUEDOCONSOLE_FN_NAME.to_string(),
            });
        };
        let Some(release) = GetProcAddress(conpty_module, s!("ConptyReleasePseudoConsole"))
            .map(|release_fn| transmute::<LoadedFn, ReleasePseudoConsoleFn>(release_fn))
        else {
            return Err(ConptyApiError::GetProcAddressFailed {
                fn_name: RELEASE_PSUEDOCONSOLE_FN_NAME.to_string(),
            });
        };
        Ok(ConptyApi {
            create,
            resize,
            close,
            show_hide,
            release,
        })
    }

    pub(super) unsafe fn create(
        &self,
        size: COORD,
        mut pipe: HANDLE,
        flags: u32,
    ) -> Result<HPCON, windows::core::Error> {
        let mut pty_handle = HPCON::default();
        let result = (self.create)(size, pipe, pipe, flags, &mut pty_handle)
            .ok()
            .map(|_| pty_handle);
        // Explicitly free our end of the pipe, giving the pseudoconsole sole
        // ownership of it.
        windows::core::Free::free(&mut pipe);
        result
    }

    pub(super) unsafe fn resize(
        &self,
        pty_handle: HPCON,
        size: COORD,
    ) -> Result<(), windows::core::Error> {
        (self.resize)(pty_handle, size).ok()
    }

    pub(super) unsafe fn close(&self, pty_handle: HPCON) {
        (self.close)(pty_handle)
    }

    pub(super) unsafe fn show_hide(
        &self,
        pty_handle: HPCON,
        visible: bool,
    ) -> windows::core::Result<()> {
        (self.show_hide)(pty_handle, visible).ok()
    }

    pub(super) unsafe fn release(&self, pty_handle: HPCON) -> windows::core::Result<()> {
        (self.release)(pty_handle).ok()
    }
}
