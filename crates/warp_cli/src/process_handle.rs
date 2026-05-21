use std::ffi::c_void;
use std::str::FromStr;

use windows::Win32::Foundation::HANDLE;

/// A Windows process handle. This wraps the [`HANDLE`] type to support parsing with `clap`.
#[derive(Clone, Copy, Debug)]
pub struct ProcessHandle(isize);

impl ProcessHandle {
    pub fn into_inner(self) -> HANDLE {
        HANDLE(self.0 as *mut c_void)
    }
}

impl FromStr for ProcessHandle {
    type Err = String;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let pid = raw
            .parse::<isize>()
            .map_err(|e| format!("invalid parent handle: {e}"))?;
        Ok(Self(pid))
    }
}
