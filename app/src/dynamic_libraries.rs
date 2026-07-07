use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::OsStrExt as _;

use windows::core::{Result, HSTRING, PWSTR};
use windows::Win32::System::LibraryLoader::SetDllDirectoryW;

pub(super) fn configure_library_loading() {
    if let Err(err) = set_dll_directory("") {
        // Logging isn't initialized yet.
        eprintln!("Error setting DLL directory: {err:#}");
    }
}

fn set_dll_directory<P: ?Sized + AsRef<OsStr>>(path: &P) -> Result<()> {
    let wide_path: Vec<u16> = OsString::from(path)
        .encode_wide()
        .chain(std::iter::once(0)) // Null-terminate
        .collect();

    let hstring = HSTRING::from_wide(&wide_path);
    let string_ptr = PWSTR::from_raw(hstring.as_ptr().cast_mut());
    unsafe { SetDllDirectoryW(string_ptr) }
}
