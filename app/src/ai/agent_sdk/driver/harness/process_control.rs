//! Best-effort SIGKILL of a third-party harness process group when graceful
//! `/exit` does not complete within the bounded shutdown ladder.

use crate::terminal::model::terminal_model::ShellProcessInfo;

/// SIGKILL the harness only if its process group can be proved to belong to
/// `shell`'s live descendant tree and is not this process's own group.
///
/// If that cannot be proved, skips the signal. The caller still returns on
/// the bounded path; the sandbox is torn down afterward.
pub(crate) fn force_kill_harness_if_safe(shell: &ShellProcessInfo) {
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
#[cfg(any(unix, test))]
pub(super) fn proven_kill_pgid(
    candidate_foreground_pgid: Option<u32>,
    shell_pid: u32,
    self_pgid: u32,
    tree_pgids: &[u32],
) -> Option<u32> {
    let is_safe = |pgid: u32| pgid != self_pgid && tree_pgids.contains(&pgid);
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
    // A closed or reused descriptor can fail or name an unrelated terminal's
    // group; that value is not signaled unless `proven_kill_pgid` accepts it.
    nix::unistd::tcgetpgrp(fd)
        .ok()
        .map(|pid| pid.as_raw() as u32)
        .filter(|&pgid| pgid > 0)
}

#[cfg(unix)]
fn current_pgid() -> u32 {
    nix::unistd::getpgrp().as_raw() as u32
}

#[cfg(unix)]
fn live_tree_pgids(shell_pid: u32) -> Vec<u32> {
    use std::collections::{HashMap, HashSet};

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

    let mut children_of: HashMap<u32, Vec<u32>> = HashMap::new();
    for (pid, process) in system.processes() {
        if let Some(parent) = process.parent() {
            children_of
                .entry(parent.as_u32())
                .or_default()
                .push(pid.as_u32());
        }
    }

    let mut pgids = HashSet::new();
    if let Some(pgid) = process_group_of(shell) {
        pgids.insert(pgid);
    }
    for pid in descendants_from_parent_map(&children_of, shell_pid) {
        if let Some(pgid) = process_group_of(Pid::from_u32(pid)) {
            pgids.insert(pgid);
        }
    }
    pgids.into_iter().collect()
}

/// Direct descendants of `root`, walking a parent→children map once.
/// `root` itself is not included. Duplicate edges and cycles are skipped.
#[cfg(any(unix, test))]
fn descendants_from_parent_map(
    children_of: &std::collections::HashMap<u32, Vec<u32>>,
    root: u32,
) -> std::collections::HashSet<u32> {
    use std::collections::HashSet;

    let mut descendants = HashSet::new();
    let mut stack = vec![root];
    while let Some(pid) = stack.pop() {
        let Some(children) = children_of.get(&pid) else {
            continue;
        };
        for &child in children {
            if descendants.insert(child) {
                stack.push(child);
            }
        }
    }
    descendants
}

#[cfg(unix)]
fn process_group_of(pid: sysinfo::Pid) -> Option<u32> {
    let pid = nix::unistd::Pid::from_raw(pid.as_u32() as i32);
    nix::unistd::getpgid(Some(pid))
        .ok()
        .map(|pgid| pgid.as_raw() as u32)
        .filter(|&pgid| pgid > 0)
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
