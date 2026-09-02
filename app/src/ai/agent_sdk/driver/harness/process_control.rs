//! Best-effort SIGKILL of a third-party harness process group when graceful
//! `/exit` does not complete within the bounded shutdown ladder.

use crate::terminal::model::terminal_model::ShellProcessInfo;

/// SIGKILL the harness only if its process group can be proved to belong to
/// `shell`'s live descendant tree and is not this process's own group.
///
/// If that cannot be proved, skips the signal. The caller still returns on
/// the bounded path; the sandbox is torn down afterward.
pub(super) fn force_kill_harness_if_safe(shell: &ShellProcessInfo) {
    #[cfg(unix)]
    {
        let target = proven_kill_pgid(
            untrusted_foreground_pgid(shell),
            shell.pid,
            current_pgid(),
            &live_tree_pgids(shell.pid),
        );
        match target {
            Some(pgid) => kill_process_group(pgid),
            None => log::warn!(
                "Skipping harness force-kill: no process group could be proved \
                 to belong to the tracked shell's tree"
            ),
        }
    }
    #[cfg(not(unix))]
    {
        let _ = shell;
        log::warn!("Skipping harness force-kill: process-group SIGKILL is Unix-only");
    }
}

/// Returns `candidate_foreground_pgid` or `shell_pid` only when that value is a
/// real process group in `tree_pgids` (the live shell and its descendants) and
/// is not this process's own group.
pub(super) fn proven_kill_pgid(
    candidate_foreground_pgid: Option<u32>,
    shell_pid: u32,
    self_pgid: u32,
    tree_pgids: &[u32],
) -> Option<u32> {
    let is_safe = |pgid: u32| pgid >= 2 && pgid != self_pgid && tree_pgids.contains(&pgid);
    if let Some(pgid) = candidate_foreground_pgid
        && is_safe(pgid)
    {
        return Some(pgid);
    }
    is_safe(shell_pid).then_some(shell_pid)
}

#[cfg(unix)]
fn untrusted_foreground_pgid(shell: &ShellProcessInfo) -> Option<u32> {
    let fd = shell.pty_leader_fd?;
    // SAFETY: `tcgetpgrp` only reads terminal state for `fd`. A closed or
    // reused descriptor can fail or name an unrelated terminal's group; that
    // value is not signaled unless `proven_kill_pgid` accepts it.
    let pgid = unsafe { libc::tcgetpgrp(fd) };
    (pgid > 0).then_some(pgid as u32)
}

#[cfg(unix)]
fn current_pgid() -> u32 {
    // SAFETY: `getpgrp` has no failure mode and only reads this process's group.
    unsafe { libc::getpgrp() as u32 }
}

#[cfg(unix)]
fn live_tree_pgids(shell_pid: u32) -> Vec<u32> {
    use std::collections::HashSet;

    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true, /* remove_dead_processes */
        ProcessRefreshKind::nothing(),
    );
    let shell = Pid::from_u32(shell_pid);
    if system.process(shell).is_none() {
        return Vec::new();
    }

    let mut pgids = HashSet::new();
    if let Some(pgid) = process_group_of(shell) {
        pgids.insert(pgid);
    }
    for pid in descendants_of(&system, shell) {
        if let Some(pgid) = process_group_of(pid) {
            pgids.insert(pgid);
        }
    }
    pgids.into_iter().collect()
}

#[cfg(unix)]
fn descendants_of(
    system: &sysinfo::System,
    pid: sysinfo::Pid,
) -> std::collections::HashSet<sysinfo::Pid> {
    let mut descendants = std::collections::HashSet::new();
    loop {
        let mut added = false;
        for (candidate, process) in system.processes() {
            if descendants.contains(candidate) {
                continue;
            }
            let Some(parent) = process.parent() else {
                continue;
            };
            if parent == pid || descendants.contains(&parent) {
                descendants.insert(*candidate);
                added = true;
            }
        }
        if !added {
            return descendants;
        }
    }
}

#[cfg(unix)]
fn process_group_of(pid: sysinfo::Pid) -> Option<u32> {
    // SAFETY: `getpgid` only reads scheduling metadata for `pid`, and reports
    // failure through its return value for pids that no longer exist.
    let pgid = unsafe { libc::getpgid(pid.as_u32() as libc::pid_t) };
    (pgid > 0).then_some(pgid as u32)
}

#[cfg(unix)]
fn kill_process_group(pgid: u32) {
    use nix::sys::signal::{Signal, kill};
    use nix::unistd::Pid;

    // A pgid of 0 targets the caller's own process group, and 1 negates to
    // -1, which SIGKILLs every process this user is allowed to signal.
    if pgid < 2 {
        log::warn!("Refusing to force-kill process group {pgid}: pid is below 2");
        return;
    }

    match kill(Pid::from_raw(-(pgid as i32)), Signal::SIGKILL) {
        Ok(()) => log::info!("Force-killed harness process group {pgid}"),
        Err(nix::errno::Errno::ESRCH) => {
            log::info!("Harness process group {pgid} had already exited");
        }
        Err(error) => {
            log::warn!("Failed to force-kill harness process group {pgid}: {error}");
        }
    }
}

#[cfg(test)]
#[path = "process_control_tests.rs"]
mod tests;
