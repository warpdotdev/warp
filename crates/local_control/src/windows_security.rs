//! Windows owner-only protections for local-control discovery and credential issuance.
//!
//! Windows has neither Unix permission bits nor kernel-reported socket peer
//! credentials, so local control uses the platform equivalents:
//!
//! - The discovery directory and its records carry a protected DACL that grants
//!   access only to the current user, `SYSTEM`, and `Administrators`. Clients
//!   refuse to follow records whose DACL is inherited or grants any other account.
//! - Each instance's credential broker is an instance-bound named pipe whose DACL
//!   grants access only to the current user and which rejects remote clients.
//! - Both ends of a broker connection identify the process on the other side
//!   through the kernel-reported pipe process ID and compare that process's token
//!   user SID with their own.
//!
//! Like the Unix peer-UID check, these boundaries authenticate the OS account,
//! not the calling application: software already running as the same user remains
//! outside this boundary.
use std::ffi::{OsStr, c_void};
use std::os::windows::ffi::OsStrExt as _;
use std::os::windows::io::RawHandle;
use std::path::Path;

use windows::Win32::Foundation::{CloseHandle, HANDLE, HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
    GetNamedSecurityInfoW, SDDL_REVISION_1, SE_FILE_OBJECT, SetNamedSecurityInfoW,
};
use windows::Win32::Security::{
    ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, DACL_SECURITY_INFORMATION, GetAce,
    GetSecurityDescriptorControl, GetSecurityDescriptorDacl, GetTokenInformation,
    OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
    SE_DACL_PROTECTED, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows::Win32::System::Pipes::{GetNamedPipeClientProcessId, GetNamedPipeServerProcessId};
use windows::Win32::System::SystemServices::{ACCESS_ALLOWED_ACE_TYPE, ACCESS_DENIED_ACE_TYPE};
use windows::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::core::{BOOL, PCWSTR, PWSTR};

use crate::protocol::{ControlError, ErrorCode};

/// Well-known SID of the local `SYSTEM` account.
const LOCAL_SYSTEM_SID: &str = "S-1-5-18";
/// Well-known SID of the built-in `Administrators` group.
const BUILTIN_ADMINISTRATORS_SID: &str = "S-1-5-32-544";

/// Returns the string SID (for example `S-1-5-21-…`) of the user that owns this process.
pub fn current_user_sid() -> Result<String, ControlError> {
    // SAFETY: `GetCurrentProcess` returns a pseudo-handle that never needs closing.
    token_user_sid(unsafe { GetCurrentProcess() })
}

/// Returns the string SID of the user that owns process `pid`.
pub fn process_user_sid(pid: u32) -> Result<String, ControlError> {
    // SAFETY: `OpenProcess` has no memory-safety preconditions; the returned
    // handle is owned by `OwnedHandle`, which closes it exactly once.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
        .map_err(|err| identity_error("open local-control peer process", err))?;
    let process = OwnedHandle(process);
    token_user_sid(process.0)
}

/// Rejects `pid` unless its process token belongs to `expected_user_sid`.
///
/// This is the Windows counterpart of the Unix broker's peer-UID comparison.
pub fn ensure_process_user(pid: u32, expected_user_sid: &str) -> Result<(), ControlError> {
    if process_user_sid(pid)? != expected_user_sid {
        return Err(ControlError::new(
            ErrorCode::UnauthorizedLocalClient,
            "local-control credential broker peer belongs to a different OS user",
        ));
    }
    Ok(())
}

/// Returns the kernel-reported process ID of the client connected to server pipe `pipe`.
pub fn pipe_client_process_id(pipe: RawHandle) -> Result<u32, ControlError> {
    let mut pid = 0;
    // SAFETY: `pipe` is a live pipe handle borrowed from the caller, and `pid`
    // is a valid output location for the duration of the call.
    unsafe { GetNamedPipeClientProcessId(HANDLE(pipe), &mut pid) }
        .map_err(|err| identity_error("identify local-control credential broker peer", err))?;
    Ok(pid)
}

/// Returns the kernel-reported process ID of the server behind client pipe `pipe`.
pub fn pipe_server_process_id(pipe: RawHandle) -> Result<u32, ControlError> {
    let mut pid = 0;
    // SAFETY: `pipe` is a live pipe handle borrowed from the caller, and `pid`
    // is a valid output location for the duration of the call.
    unsafe { GetNamedPipeServerProcessId(HANDLE(pipe), &mut pid) }
        .map_err(|err| identity_error("identify local-control credential broker", err))?;
    Ok(pid)
}

/// Replaces `path`'s DACL with a protected DACL for the current user, `SYSTEM`,
/// and `Administrators`.
///
/// Protecting the DACL stops the path from inheriting broader entries from its
/// parent. Directory entries are marked inheritable so records created inside
/// the directory start out private as well.
pub fn set_private_acl(path: &Path, is_directory: bool) -> Result<(), ControlError> {
    let descriptor =
        SecurityDescriptor::from_sddl(&private_sddl(&current_user_sid()?, is_directory))?;
    let dacl = descriptor.dacl()?;
    let path = to_wide(path);
    // SAFETY: `path` is NUL-terminated and `dacl` points into `descriptor`,
    // which outlives the call.
    unsafe {
        SetNamedSecurityInfoW(
            PCWSTR(path.as_ptr()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(dacl),
            None,
        )
    }
    .ok()
    .map_err(|err| {
        ControlError::with_details(
            ErrorCode::Internal,
            "failed to protect local-control discovery path",
            err.to_string(),
        )
    })
}

/// Verifies that `path` is owned by and grants access only to the current user,
/// `SYSTEM`, or `Administrators`, through a DACL that does not inherit from its parent.
///
/// Deny entries are accepted because they can only narrow access.
pub fn validate_private_acl(path: &Path) -> Result<(), ControlError> {
    let user_sid = current_user_sid()?;
    let allowed = [
        user_sid.as_str(),
        LOCAL_SYSTEM_SID,
        BUILTIN_ADMINISTRATORS_SID,
    ];
    let wide_path = to_wide(path);
    let mut owner = PSID::default();
    let mut dacl: *mut ACL = std::ptr::null_mut();
    let mut raw_descriptor = PSECURITY_DESCRIPTOR::default();
    // SAFETY: `wide_path` is NUL-terminated and every output pointer is valid
    // for the call. The returned descriptor owns the memory `owner` and `dacl`
    // point into, and `SecurityDescriptor` frees it after they are last used.
    unsafe {
        GetNamedSecurityInfoW(
            PCWSTR(wide_path.as_ptr()),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            Some(&mut owner),
            None,
            Some(&mut dacl),
            None,
            &mut raw_descriptor,
        )
    }
    .ok()
    .map_err(|err| {
        ControlError::with_details(
            ErrorCode::UnauthorizedLocalClient,
            "failed to read local-control discovery permissions",
            err.to_string(),
        )
    })?;
    let descriptor = SecurityDescriptor(raw_descriptor);

    let mut control = 0;
    let mut revision = 0;
    // SAFETY: `descriptor` is a valid security descriptor returned above.
    unsafe { GetSecurityDescriptorControl(descriptor.0, &mut control, &mut revision) }
        .map_err(|err| unsafe_acl_error("could not be inspected", Some(err)))?;
    if control & SE_DACL_PROTECTED.0 == 0 {
        return Err(unsafe_acl_error(
            "inherits permissions from its parent",
            None,
        ));
    }
    if !allowed.contains(&sid_to_string(owner)?.as_str()) {
        return Err(unsafe_acl_error("is owned by an unexpected account", None));
    }
    // A NULL DACL grants everyone full access.
    if dacl.is_null() {
        return Err(unsafe_acl_error("has no access control list", None));
    }
    // SAFETY: `dacl` is non-null and points into `descriptor`.
    let ace_count = unsafe { (*dacl).AceCount };
    for index in 0..u32::from(ace_count) {
        let mut ace: *mut c_void = std::ptr::null_mut();
        // SAFETY: `index` is below the DACL's entry count.
        unsafe { GetAce(dacl, index, &mut ace) }
            .map_err(|err| unsafe_acl_error("could not be inspected", Some(err)))?;
        // SAFETY: every ACE starts with an `ACE_HEADER`.
        let header = unsafe { &*ace.cast::<ACE_HEADER>() };
        match u32::from(header.AceType) {
            ACCESS_DENIED_ACE_TYPE => {}
            ACCESS_ALLOWED_ACE_TYPE => {
                // SAFETY: the header identifies this ACE as `ACCESS_ALLOWED_ACE`,
                // whose SID starts at `SidStart`.
                let entry = unsafe { &*ace.cast::<ACCESS_ALLOWED_ACE>() };
                let sid = PSID(std::ptr::from_ref(&entry.SidStart).cast_mut().cast());
                if !allowed.contains(&sid_to_string(sid)?.as_str()) {
                    return Err(unsafe_acl_error(
                        "grants access to an unexpected account",
                        None,
                    ));
                }
            }
            _ => {
                return Err(unsafe_acl_error(
                    "contains an unsupported access control entry",
                    None,
                ));
            }
        }
    }
    Ok(())
}

/// Security attributes for an instance's credential-broker pipe.
///
/// The protected DACL grants access only to the current user, so other OS users
/// cannot open the pipe at all; the broker's peer check remains authoritative.
pub struct BrokerPipeSecurity {
    _descriptor: SecurityDescriptor,
    attributes: SECURITY_ATTRIBUTES,
}

impl BrokerPipeSecurity {
    pub fn new() -> Result<Self, ControlError> {
        let descriptor = SecurityDescriptor::from_sddl(&broker_pipe_sddl(&current_user_sid()?))?;
        let attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            // The descriptor's memory is heap-allocated by the system, so this
            // pointer stays valid when `Self` moves.
            lpSecurityDescriptor: descriptor.0.0,
            bInheritHandle: BOOL::from(false),
        };
        Ok(Self {
            _descriptor: descriptor,
            attributes,
        })
    }

    /// Returns a `SECURITY_ATTRIBUTES` pointer suitable for pipe creation APIs.
    pub fn as_mut_ptr(&mut self) -> *mut c_void {
        std::ptr::from_mut(&mut self.attributes).cast()
    }
}

/// SDDL for a protected DACL that grants full access to the current user,
/// `SYSTEM`, and `Administrators`.
fn private_sddl(user_sid: &str, is_directory: bool) -> String {
    // `OICI` makes directory entries inherit to the records created inside it.
    let inheritance = if is_directory { "OICI" } else { "" };
    format!(
        "D:P(A;{inheritance};FA;;;{user_sid})(A;{inheritance};FA;;;{LOCAL_SYSTEM_SID})(A;{inheritance};FA;;;{BUILTIN_ADMINISTRATORS_SID})"
    )
}

/// SDDL for a protected DACL that grants pipe access only to the current user.
fn broker_pipe_sddl(user_sid: &str) -> String {
    format!("D:P(A;;GA;;;{user_sid})")
}

fn token_user_sid(process: HANDLE) -> Result<String, ControlError> {
    let mut token = HANDLE::default();
    // SAFETY: `process` is a valid process handle and `token` is a valid output
    // location; the token handle is closed by `OwnedHandle`.
    unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) }
        .map_err(|err| identity_error("open local-control process token", err))?;
    let token = OwnedHandle(token);
    let mut length = 0;
    // The first call only reports the buffer size, so its error is expected.
    // SAFETY: a null buffer with zero length is valid for a size query.
    let _ = unsafe { GetTokenInformation(token.0, TokenUser, None, 0, &mut length) };
    if length == 0 {
        return Err(ControlError::new(
            ErrorCode::UnauthorizedLocalClient,
            "failed to read local-control process identity",
        ));
    }
    // A `u64` buffer keeps `TOKEN_USER` suitably aligned.
    let mut buffer = vec![0u64; (length as usize).div_ceil(std::mem::size_of::<u64>())];
    // SAFETY: `buffer` holds at least `length` bytes.
    unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            Some(buffer.as_mut_ptr().cast()),
            length,
            &mut length,
        )
    }
    .map_err(|err| identity_error("read local-control process identity", err))?;
    // SAFETY: `GetTokenInformation(TokenUser)` filled `buffer` with a
    // `TOKEN_USER` whose SID points into the same buffer.
    let token_user = unsafe { &*buffer.as_ptr().cast::<TOKEN_USER>() };
    sid_to_string(token_user.User.Sid)
}

fn sid_to_string(sid: PSID) -> Result<String, ControlError> {
    let mut raw = PWSTR::null();
    // SAFETY: `sid` points to a valid SID and `raw` is a valid output location.
    unsafe { ConvertSidToStringSidW(sid, &mut raw) }
        .map_err(|err| identity_error("format local-control account identity", err))?;
    // SAFETY: `ConvertSidToStringSidW` returned a NUL-terminated string that
    // must be released with `LocalFree`.
    let sid = unsafe { raw.to_string() };
    unsafe { LocalFree(Some(HLOCAL(raw.0.cast()))) };
    sid.map_err(|err| {
        ControlError::with_details(
            ErrorCode::UnauthorizedLocalClient,
            "failed to format local-control account identity",
            err.to_string(),
        )
    })
}

fn to_wide(value: impl AsRef<OsStr>) -> Vec<u16> {
    value.as_ref().encode_wide().chain(Some(0)).collect()
}

fn identity_error(operation: &str, error: windows::core::Error) -> ControlError {
    ControlError::with_details(
        ErrorCode::UnauthorizedLocalClient,
        format!("failed to {operation}"),
        error.to_string(),
    )
}

fn unsafe_acl_error(problem: &str, error: Option<windows::core::Error>) -> ControlError {
    let message = format!("local-control discovery path {problem}");
    match error {
        Some(error) => ControlError::with_details(
            ErrorCode::UnauthorizedLocalClient,
            message,
            error.to_string(),
        ),
        None => ControlError::new(ErrorCode::UnauthorizedLocalClient, message),
    }
}

/// Security descriptor allocated by the system and released with `LocalFree`.
struct SecurityDescriptor(PSECURITY_DESCRIPTOR);

impl SecurityDescriptor {
    fn from_sddl(sddl: &str) -> Result<Self, ControlError> {
        let sddl = to_wide(sddl);
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        // SAFETY: `sddl` is NUL-terminated and `descriptor` is a valid output location.
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(sddl.as_ptr()),
                SDDL_REVISION_1,
                &mut descriptor,
                None,
            )
        }
        .map_err(|err| {
            ControlError::with_details(
                ErrorCode::Internal,
                "failed to build local-control security descriptor",
                err.to_string(),
            )
        })?;
        Ok(Self(descriptor))
    }

    fn dacl(&self) -> Result<*mut ACL, ControlError> {
        let mut present = BOOL::default();
        let mut defaulted = BOOL::default();
        let mut dacl: *mut ACL = std::ptr::null_mut();
        // SAFETY: `self.0` is a valid descriptor and every output pointer is valid.
        unsafe { GetSecurityDescriptorDacl(self.0, &mut present, &mut dacl, &mut defaulted) }
            .map_err(|err| {
                ControlError::with_details(
                    ErrorCode::Internal,
                    "failed to read local-control security descriptor",
                    err.to_string(),
                )
            })?;
        if !present.as_bool() || dacl.is_null() {
            return Err(ControlError::new(
                ErrorCode::Internal,
                "local-control security descriptor has no access control list",
            ));
        }
        Ok(dacl)
    }
}

impl Drop for SecurityDescriptor {
    fn drop(&mut self) {
        if !self.0.0.is_null() {
            // SAFETY: the descriptor was allocated by the system with `LocalAlloc`.
            unsafe { LocalFree(Some(HLOCAL(self.0.0))) };
        }
    }
}

struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        // SAFETY: the handle is owned by `self` and closed exactly once.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

#[cfg(test)]
#[path = "windows_security_tests.rs"]
mod tests;
