//! Process-level setup of the daemon: resource limits and core dumps.

use crate::local_tty::ChildResourceLimits;

/// The highest file descriptor limit the daemon raises itself to.
const FD_LIMIT_CAP: libc::rlim_t = 10240;

fn get_rlimit(resource: libc::c_int) -> Option<libc::rlimit> {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `limit` is valid for writes.
    let res = unsafe { libc::getrlimit(resource as _, &mut limit) };
    (res == 0).then_some(limit)
}

fn set_soft_rlimit(resource: libc::c_int, soft: libc::rlim_t) -> std::io::Result<()> {
    let Some(mut limit) = get_rlimit(resource) else {
        return Err(std::io::Error::last_os_error());
    };
    limit.rlim_cur = soft;
    // SAFETY: `limit` is a valid rlimit.
    if unsafe { libc::setrlimit(resource as _, &limit) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// The soft limit to raise `RLIMIT_NOFILE` to: the hard limit, capped.
pub(crate) fn raised_fd_limit(soft: libc::rlim_t, hard: libc::rlim_t) -> libc::rlim_t {
    let target = if hard == libc::RLIM_INFINITY {
        FD_LIMIT_CAP
    } else {
        hard.min(FD_LIMIT_CAP)
    };
    target.max(soft)
}

/// Raises the fd limit, disables core dumps and, on Linux, marks the process
/// non-dumpable so its memory (scrollback) cannot be read through core files
/// or `ptrace`. Returns the limits the daemon inherited, which every shell it
/// spawns restores for itself.
pub(crate) fn harden_daemon_process() -> Option<ChildResourceLimits> {
    let nofile = get_rlimit(libc::RLIMIT_NOFILE as libc::c_int)?;
    let core = get_rlimit(libc::RLIMIT_CORE as libc::c_int)?;
    let raised = raised_fd_limit(nofile.rlim_cur, nofile.rlim_max);
    if raised > nofile.rlim_cur
        && let Err(err) = set_soft_rlimit(libc::RLIMIT_NOFILE as libc::c_int, raised)
    {
        log::warn!("Failed to raise the fd limit: {err}");
    }
    if let Err(err) = set_soft_rlimit(libc::RLIMIT_CORE as libc::c_int, 0) {
        log::warn!("Failed to disable core dumps: {err}");
    }
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        // SAFETY: PR_SET_DUMPABLE takes a plain integer argument.
        if unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) } != 0 {
            log::warn!(
                "Failed to mark the daemon non-dumpable: {}",
                std::io::Error::last_os_error()
            );
        }
    }
    Some(ChildResourceLimits {
        nofile_soft: nofile.rlim_cur as u64,
        core_soft: core.rlim_cur as u64,
    })
}

#[cfg(test)]
#[path = "process_tests.rs"]
mod tests;
