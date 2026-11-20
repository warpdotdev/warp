//! Windows-specific utilities.

use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};

/// Attaches the current process to the console of the parent process.
///
/// This is useful for command-line interfaces that need to ensure all standard
/// output gets printed correctly when run from a terminal.
pub fn attach_to_parent_console() {
    use windows::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
    let _ = unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
}

/// Helper struct to properly enforce reference counting of the Windows COM library.
///
/// Per the Windows docs (https://learn.microsoft.com/en-us/windows/win32/api/combaseapi/nf-combaseapi-coinitializeex)
/// each call to [`CoInitializeEx`] must be paired with a call to [`CoUninitialize`] in order for
/// the COM library to be gracefully uninitialized.
struct ComInitialized;

impl Drop for ComInitialized {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

thread_local! {
    static COM_INITIALIZED: Result<ComInitialized, windows::core::Error> = {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
            Ok(ComInitialized)
        }
    };
}

/// Initializes COM as a single-threaded apartment on the current thread, once per thread, and
/// keeps it initialized until the thread exits.
pub fn com_initialized() -> Result<(), windows::core::Error> {
    COM_INITIALIZED.with(|initialized| {
        initialized
            .as_ref()
            .map(|_| ())
            .map_err(|error| error.clone())
    })
}
