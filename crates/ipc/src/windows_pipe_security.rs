//! Builds the `SECURITY_ATTRIBUTES` used to create the Windows named pipes in
//! [`crate::platform`] with a DACL scoped to the creating user (across elevation levels),
//! `LocalSystem`, and `BUILTIN\Administrators`, instead of the OS default DACL.
//!
//! ## Background
//! When a Windows named pipe is created without an explicit security descriptor,
//! [`CreateNamedPipeW`] applies a default DACL that grants full control to `LocalSystem`,
//! `Administrators`, and the *creator owner*, and read-only access to `Everyone`.
//!
//! The "creator owner" is derived from the *token* of the process that created the pipe, not
//! from the human user running it. For a UAC-elevated process, a token's default owner is often
//! the `BUILTIN\Administrators` group rather than the signed-in user's own SID. If Warp's
//! single-instance named pipe is created by an elevated process (for example, the installer's
//! post-install launch) and a later, non-elevated instance of the *same* user tries to connect
//! to forward a `warp://` deep link, the non-elevated token has `BUILTIN\Administrators` filtered
//! out (standard UAC token filtering) and so is left with only the read-only `Everyone` ACE.
//! Connecting for full-duplex I/O then fails with `ERROR_ACCESS_DENIED` (OS error 5). See
//! REV-1546.
//!
//! ## Why creation-time, not post-creation
//! `SetNamedSecurityInfoW`/`GetNamedSecurityInfoW` (the "by name" APIs) only support a fixed set
//! of kernel objects -- semaphores, events, mutexes, waitable timers, and file mappings -- and
//! explicitly do *not* support named pipes (see [`SE_OBJECT_TYPE`]). Calling them on a pipe path
//! fails outright, so patching the DACL in after the pipe is already created and accepting
//! connections is not just a TOCTOU risk, it does not work at all. Instead, the DACL is built
//! into a `SECURITY_ATTRIBUTES` structure that is passed directly to pipe creation
//! (`CreateNamedPipeW`'s `lpSecurityAttributes`, via
//! [`tokio`'s `ServerOptions::create_with_security_attributes_raw`][create_with_security_attributes_raw]),
//! so the pipe never exists under a looser DACL than intended.
//!
//! ## Threat model / chosen ACL
//! We replace the default DACL with one scoped to exactly what's needed to fix the bug, rather
//! than opening the pipe to all local users (a null DACL or an `Everyone`-writable SDDL would let
//! any local user on a shared/multi-user machine inject deep-link URIs -- including auth redirect
//! URIs -- into another user's running Warp instance):
//! - The *signed-in user's own SID*, read from the current process's primary token
//!   (`TokenUser`). This SID is identical for both an elevated and a non-elevated token
//!   belonging to the same login session, so granting access to it (rather than to whatever
//!   "owner" a particular token happens to have) is what actually fixes the elevation mismatch,
//!   while still keeping other local users out.
//! - `LocalSystem` and `BUILTIN\Administrators`, matching the access Windows' own default named
//!   pipe DACL already grants those principals, for consistency with standard OS behavior (e.g.
//!   admin tooling).
//!
//! The signed-in user is granted exactly the two access rights a duplex named-pipe client needs
//! (`GENERIC_READ | GENERIC_WRITE`); `SYSTEM`/`Administrators` are granted `GENERIC_ALL` to match
//! the OS default. `Everyone`/anonymous access is dropped entirely.
//!
//! [`CreateNamedPipeW`]: https://learn.microsoft.com/windows/win32/api/namedpipeapi/nf-namedpipeapi-createnamedpipew
//! [`SE_OBJECT_TYPE`]: https://learn.microsoft.com/windows/win32/api/accctrl/ne-accctrl-se_object_type
//! [create_with_security_attributes_raw]: https://docs.rs/tokio/latest/tokio/net/windows/named_pipe/struct.ServerOptions.html#method.create_with_security_attributes_raw
use std::ffi::c_void;

use windows::Win32::Foundation::{CloseHandle, HANDLE, HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::{
    EXPLICIT_ACCESS_W, GRANT_ACCESS, NO_MULTIPLE_TRUSTEE, SetEntriesInAclW, TRUSTEE_IS_SID,
    TRUSTEE_IS_UNKNOWN, TRUSTEE_W,
};
use windows::Win32::Security::{
    ACL, CopySid, CreateWellKnownSid, GetLengthSid, GetTokenInformation,
    InitializeSecurityDescriptor, NO_INHERITANCE, PSECURITY_DESCRIPTOR, PSID, SECURITY_ATTRIBUTES,
    SECURITY_DESCRIPTOR, SetSecurityDescriptorDacl, TOKEN_QUERY, TOKEN_USER, TokenUser,
    WELL_KNOWN_SID_TYPE, WinBuiltinAdministratorsSid, WinLocalSystemSid,
};
use windows::Win32::System::SystemServices::SECURITY_DESCRIPTOR_REVISION;
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::core::BOOL;

/// `GENERIC_READ | GENERIC_WRITE`: exactly the access rights a duplex named-pipe client needs.
const PIPE_CLIENT_ACCESS_MASK: u32 = 0x8000_0000 | 0x4000_0000;

/// `GENERIC_ALL`: matches the access Windows' own default named pipe DACL grants to
/// `LocalSystem`/`Administrators`.
const FULL_CONTROL_ACCESS_MASK: u32 = 0x1000_0000;

/// Owns a `SECURITY_ATTRIBUTES` (and the DACL/security descriptor it points to) suitable for
/// passing to named-pipe creation, restricting the pipe to the current user (across elevation
/// levels), `LocalSystem`, and `BUILTIN\Administrators`. See the module docs for the full threat
/// model.
///
/// The DACL/descriptor are kept alive for as long as this value is alive, since Windows only
/// copies the descriptor's *contents* at `CreateNamedPipeW` time, not the pointer -- reusing one
/// `PipeSecurityAttributes` to create every instance of a pipe (the first and all subsequent
/// ones) is the documented pattern for keeping a named pipe's DACL consistent across instances.
pub(crate) struct PipeSecurityAttributes {
    /// From `SetEntriesInAclW`; freed on drop.
    dacl: *mut ACL,
    /// Never read directly -- kept only so its heap allocation (which `attributes` points into)
    /// stays alive for as long as `PipeSecurityAttributes` does, and is freed when it drops.
    #[allow(dead_code)]
    descriptor: Box<SECURITY_DESCRIPTOR>,
    attributes: SECURITY_ATTRIBUTES,
}

// SAFETY: `PipeSecurityAttributes` owns all the memory its raw pointers refer to (the ACL and the
// boxed security descriptor), so moving it across threads is sound. Nothing here is mutated
// through a shared reference.
unsafe impl Send for PipeSecurityAttributes {}
unsafe impl Sync for PipeSecurityAttributes {}

impl PipeSecurityAttributes {
    /// Builds the security attributes described in the module docs.
    pub(crate) fn for_current_user() -> windows::core::Result<Self> {
        let user_sid = current_user_sid()?;
        let system_sid = well_known_sid(WinLocalSystemSid)?;
        let admins_sid = well_known_sid(WinBuiltinAdministratorsSid)?;

        let entries = [
            explicit_access_entry(&user_sid, PIPE_CLIENT_ACCESS_MASK),
            explicit_access_entry(&system_sid, FULL_CONTROL_ACCESS_MASK),
            explicit_access_entry(&admins_sid, FULL_CONTROL_ACCESS_MASK),
        ];

        let mut dacl: *mut ACL = std::ptr::null_mut();
        unsafe {
            SetEntriesInAclW(Some(&entries), None, &mut dacl).ok()?;
        }

        let mut descriptor = Box::new(SECURITY_DESCRIPTOR::default());
        // Safety: `descriptor` is a live, correctly-sized `SECURITY_DESCRIPTOR` we just
        // allocated, and `dacl` was just built above by `SetEntriesInAclW`.
        unsafe {
            InitializeSecurityDescriptor(
                PSECURITY_DESCRIPTOR(descriptor.as_mut() as *mut _ as *mut c_void),
                SECURITY_DESCRIPTOR_REVISION,
            )?;
            SetSecurityDescriptorDacl(
                PSECURITY_DESCRIPTOR(descriptor.as_mut() as *mut _ as *mut c_void),
                true,
                Some(dacl),
                false,
            )?;
        }

        let attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor.as_mut() as *mut SECURITY_DESCRIPTOR as *mut c_void,
            bInheritHandle: BOOL(0),
        };

        Ok(Self {
            dacl,
            descriptor,
            attributes,
        })
    }

    /// Returns a raw pointer to the `SECURITY_ATTRIBUTES`, suitable for passing as
    /// `lpSecurityAttributes` to pipe-creation APIs. The pointer is valid for as long as `self`
    /// is alive.
    pub(crate) fn as_ptr(&self) -> *mut c_void {
        &self.attributes as *const SECURITY_ATTRIBUTES as *mut c_void
    }
}

impl Drop for PipeSecurityAttributes {
    fn drop(&mut self) {
        if !self.dacl.is_null() {
            // Safety: `dacl` was allocated by `SetEntriesInAclW`, which is documented to require
            // freeing via `LocalFree`.
            unsafe {
                let _ = LocalFree(Some(HLOCAL(self.dacl as *mut c_void)));
            }
        }
    }
}

/// Builds an `EXPLICIT_ACCESS_W` entry granting `access_mask` to `sid`, without inheritance
/// (named pipes have no child objects to inherit to).
///
/// The returned value borrows `sid`; the caller must not let it outlive `sid`.
fn explicit_access_entry(sid: &[u8], access_mask: u32) -> EXPLICIT_ACCESS_W {
    EXPLICIT_ACCESS_W {
        grfAccessPermissions: access_mask,
        grfAccessMode: GRANT_ACCESS,
        grfInheritance: NO_INHERITANCE,
        Trustee: TRUSTEE_W {
            pMultipleTrustee: std::ptr::null_mut(),
            MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_UNKNOWN,
            // Safety: callers (`for_current_user`) keep `sid`'s backing buffer alive for as long
            // as this entry is used, which is synchronously within the same function via
            // `SetEntriesInAclW`.
            ptstrName: windows::core::PWSTR(sid.as_ptr() as *mut u16),
        },
    }
}

/// Returns the SID bytes for the user identified by the calling process's primary token
/// (`TokenUser`). This is stable across elevation: an elevated and a non-elevated token for the
/// same login session report the same user SID here, even though their *token owner* can differ.
fn current_user_sid() -> windows::core::Result<Vec<u8>> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)?;
        let token = ScopedHandle(token);

        let mut required_len: u32 = 0;
        // Expected to "fail" with ERROR_INSUFFICIENT_BUFFER; we only want the required size.
        let _ = GetTokenInformation(token.0, TokenUser, None, 0, &mut required_len);

        let mut buffer = vec![0u8; required_len as usize];
        GetTokenInformation(
            token.0,
            TokenUser,
            Some(buffer.as_mut_ptr() as *mut c_void),
            required_len,
            &mut required_len,
        )?;

        // Safety: `buffer` was sized and filled by `GetTokenInformation` above for `TokenUser`,
        // which is documented to return a `TOKEN_USER` struct, but a `Vec<u8>`'s allocation is
        // not guaranteed to satisfy `TOKEN_USER`'s alignment. Use an unaligned read rather than
        // dereferencing a `*const TOKEN_USER` directly to avoid undefined behavior.
        let token_user: TOKEN_USER = std::ptr::read_unaligned(buffer.as_ptr() as *const TOKEN_USER);
        copy_sid(token_user.User.Sid)
    }
}

/// Returns the SID bytes for the given well-known SID type (e.g. `LocalSystem`,
/// `BUILTIN\Administrators`).
fn well_known_sid(sid_type: WELL_KNOWN_SID_TYPE) -> windows::core::Result<Vec<u8>> {
    unsafe {
        let mut size: u32 = 0;
        // Expected to "fail" because the buffer is too small; we only want the required size.
        let _ = CreateWellKnownSid(sid_type, None, None, &mut size);
        let mut buf = vec![0u8; size as usize];
        CreateWellKnownSid(
            sid_type,
            None,
            Some(PSID(buf.as_mut_ptr() as *mut c_void)),
            &mut size,
        )?;
        Ok(buf)
    }
}

/// # Safety
/// `sid` must be a valid `PSID` for the duration of this call.
unsafe fn copy_sid(sid: PSID) -> windows::core::Result<Vec<u8>> {
    unsafe {
        let sid_len = GetLengthSid(sid);
        let mut sid_bytes = vec![0u8; sid_len as usize];
        CopySid(sid_len, PSID(sid_bytes.as_mut_ptr() as *mut c_void), sid)?;
        Ok(sid_bytes)
    }
}

/// RAII wrapper that closes a `HANDLE` on drop.
struct ScopedHandle(HANDLE);

impl Drop for ScopedHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

#[cfg(test)]
#[path = "windows_pipe_security_tests.rs"]
mod tests;
