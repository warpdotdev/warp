use windows::Win32::System::Threading;

#[repr(C)]
#[derive(Default)]
struct RedirectionTrustPolicy {
    flags: u32,
}

/// Warn if the Windows RedirectionGuard process mitigation policy is enforced on this process. When
/// active, symlink and junction traversal to paths created by non-admin users fails with
/// ERROR_UNTRUSTED_MOUNT_POINT (448).
///
/// See https://github.com/warpdotdev/warp/issues/9044
pub fn check_redirection_guard() {
    let mut policy = RedirectionTrustPolicy::default();
    let ok = unsafe {
        Threading::GetProcessMitigationPolicy(
            Threading::GetCurrentProcess(),
            Threading::ProcessRedirectionTrustPolicy,
            &mut policy as *mut _ as *mut std::ffi::c_void,
            std::mem::size_of::<RedirectionTrustPolicy>(),
        )
    };

    if ok.is_ok() && policy.flags & 1 != 0 {
        log::warn!(
            "RedirectionGuard is enforced on this process — symlink/junction traversal to paths \
            created by non-admin users will fail with error 448."
        );
    }
}
