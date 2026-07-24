//! Module containing the definition of [`ProcThreadAttributeList`], a wrapper struct around
//! Window's [`LPPROC_THREAD_ATTRIBUTE_LIST`].
//! See the Windows documentation for more details: https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-initializeprocthreadattributelist.

use windows::Win32::System::Console::HPCON;
use windows::Win32::System::Threading::{
    DeleteProcThreadAttributeList, InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST,
    PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE, UpdateProcThreadAttribute,
};
pub struct ProcThreadAttributeList {
    data: Box<[u8]>,
}

impl ProcThreadAttributeList {
    pub unsafe fn new() -> windows::core::Result<Self> {
        unsafe {
            let num_attributes = 1;
            let mut bytes_required: usize = 0;

            // Purposefully don't bubble up the error if this fails. Per the Window docs,
            // this should return an error the first time it is called:
            // https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-initializeprocthreadattributelist#remarks
            let _ =
                InitializeProcThreadAttributeList(None, num_attributes, None, &mut bytes_required);

            let mut attribute_list: Box<[u8]> = vec![0; bytes_required].into_boxed_slice();
            let attr_ptr = attribute_list.as_mut_ptr() as *mut _;

            InitializeProcThreadAttributeList(
                Some(LPPROC_THREAD_ATTRIBUTE_LIST(attr_ptr)),
                num_attributes,
                None,
                &mut bytes_required,
            )?;
            Ok(Self {
                data: attribute_list,
            })
        }
    }

    pub fn as_mut_ptr(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        LPPROC_THREAD_ATTRIBUTE_LIST(self.data.as_mut_ptr() as *mut _)
    }

    /// Sets the PTY connection as a startup process attribute using the
    /// `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE` attribute.
    pub fn set_pty_connection(&mut self, con: HPCON) -> windows::core::Result<()> {
        unsafe {
            UpdateProcThreadAttribute(
                self.as_mut_ptr(),
                0,
                PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE as usize,
                Some(con.0 as _),
                size_of::<HPCON>(),
                None,
                None,
            )?
        };

        Ok(())
    }
}

impl Drop for ProcThreadAttributeList {
    fn drop(&mut self) {
        unsafe { DeleteProcThreadAttributeList(self.as_mut_ptr()) };
    }
}
