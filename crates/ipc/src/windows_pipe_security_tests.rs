//! Exercises the real Windows security APIs end to end: builds
//! [`super::PipeSecurityAttributes`], creates an actual named pipe with them (via the same raw
//! `CreateNamedPipeW` call `tokio`'s `ServerOptions::create_with_security_attributes_raw` makes
//! internally), and reads the pipe's *actual, applied* DACL back via `GetSecurityInfo` on its
//! handle -- the only Get/Set path that works for named pipes (see the module docs on why the
//! "named" APIs don't).
//!
//! Before REV-1546 was fixed, the (broken) `SetNamedSecurityInfoW`-based attempt silently failed
//! on named pipes, so this pipe would have kept the OS default DACL, which grants `Everyone`
//! read access. This test fails in that case and passes once the DACL is actually applied at
//! creation time.
use std::ffi::c_void;

use windows::Win32::Foundation::{CloseHandle, HANDLE, HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::{
    EXPLICIT_ACCESS_W, GetExplicitEntriesFromAclW, GetSecurityInfo, SE_KERNEL_OBJECT,
};
use windows::Win32::Security::{
    ACL, CreateWellKnownSid, DACL_SECURITY_INFORMATION, EqualSid, PSECURITY_DESCRIPTOR, PSID,
    WELL_KNOWN_SID_TYPE, WinBuiltinAdministratorsSid, WinLocalSystemSid, WinWorldSid,
};
use windows::Win32::Storage::FileSystem::PIPE_ACCESS_DUPLEX;
use windows::Win32::System::Pipes::{
    CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES,
};
use windows::core::PCWSTR;

use super::{
    FULL_CONTROL_ACCESS_MASK, PIPE_CLIENT_ACCESS_MASK, PipeSecurityAttributes, current_user_sid,
};

fn well_known(sid_type: WELL_KNOWN_SID_TYPE) -> Vec<u8> {
    unsafe {
        let mut size: u32 = 0;
        let _ = CreateWellKnownSid(sid_type, None, None, &mut size);
        let mut buf = vec![0u8; size as usize];
        CreateWellKnownSid(
            sid_type,
            None,
            Some(PSID(buf.as_mut_ptr() as *mut c_void)),
            &mut size,
        )
        .expect("CreateWellKnownSid");
        buf
    }
}

#[test]
fn pipe_created_with_our_attributes_grants_only_the_expected_principals() {
    let attrs = PipeSecurityAttributes::for_current_user().expect("build security attributes");
    let name = r"\\.\pipe\ipc_windows_pipe_security_test";
    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();

    // Safety: `attrs` outlives this call, and `wide` is a valid NUL-terminated wide string.
    let handle = unsafe {
        CreateNamedPipeW(
            PCWSTR(wide.as_ptr()),
            PIPE_ACCESS_DUPLEX,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE,
            PIPE_UNLIMITED_INSTANCES,
            4096,
            4096,
            0,
            Some(attrs.as_ptr() as *const _),
        )
    };
    assert_ne!(handle, HANDLE::default(), "CreateNamedPipeW should succeed");

    let mut dacl: *mut ACL = std::ptr::null_mut();
    let mut sd = PSECURITY_DESCRIPTOR::default();
    // Safety: `handle` is a live named-pipe handle; `dacl`/`sd` are valid out-pointers.
    unsafe {
        GetSecurityInfo(
            handle,
            SE_KERNEL_OBJECT,
            DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(&mut dacl),
            None,
            Some(&mut sd),
        )
        .ok()
        .expect("GetSecurityInfo should succeed for a named pipe handle");
    }

    let mut count: u32 = 0;
    let mut entries: *mut EXPLICIT_ACCESS_W = std::ptr::null_mut();
    // Safety: `dacl` was just populated by `GetSecurityInfo` above.
    unsafe {
        GetExplicitEntriesFromAclW(dacl, &mut count, &mut entries)
            .ok()
            .expect("GetExplicitEntriesFromAclW");
    }

    assert_eq!(
        count, 3,
        "expected exactly 3 explicit ACEs on the pipe's DACL"
    );

    let user_sid = current_user_sid().expect("current_user_sid");
    let system_sid = well_known(WinLocalSystemSid);
    let admins_sid = well_known(WinBuiltinAdministratorsSid);
    let everyone_sid = well_known(WinWorldSid);

    // Safety: `entries` and `count` were just filled in by `GetExplicitEntriesFromAclW`.
    let entry_slice = unsafe { std::slice::from_raw_parts(entries, count as usize) };
    for entry in entry_slice {
        let entry_sid = PSID(entry.Trustee.ptstrName.0 as *mut c_void);
        // Safety: both `entry_sid` (from the enumerated ACE) and the well-known SID buffers are
        // valid SIDs for the duration of this comparison.
        let matches =
            |sid: &[u8]| unsafe { EqualSid(entry_sid, PSID(sid.as_ptr() as *mut c_void)).is_ok() };

        assert!(
            !matches(&everyone_sid),
            "Everyone must not appear in the pipe's DACL"
        );

        if matches(&user_sid) {
            assert_eq!(entry.grfAccessPermissions, PIPE_CLIENT_ACCESS_MASK);
        } else if matches(&system_sid) || matches(&admins_sid) {
            assert_eq!(entry.grfAccessPermissions, FULL_CONTROL_ACCESS_MASK);
        } else {
            panic!("unexpected principal granted access in the pipe's DACL");
        }
    }

    // Safety: `entries` and `sd` were allocated by the Win32 APIs above and must be freed with
    // `LocalFree`; `handle` is a valid, still-open handle.
    unsafe {
        let _ = LocalFree(Some(HLOCAL(entries as *mut c_void)));
        let _ = LocalFree(Some(HLOCAL(sd.0)));
        let _ = CloseHandle(handle);
    }
}
