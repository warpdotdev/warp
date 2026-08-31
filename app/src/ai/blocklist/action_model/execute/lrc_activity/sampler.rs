//! Reads the OS process table to observe a command's process tree.
//!
//! All `sysinfo` use is confined here. The crate is not built for wasm, which
//! has no local process table to read, so the accounting in the parent module
//! stays target-agnostic and wasm gets a sampler that observes nothing.

use crate::terminal::model::terminal_model::ShellProcessInfo;

cfg_if::cfg_if! {
    if #[cfg(target_family = "wasm")] {
        use super::ProcessSample;

        #[derive(Default)]
        pub(super) struct Sampler;

        impl Sampler {
            pub(super) fn collect(
                &self,
                _shell: Option<&ShellProcessInfo>,
            ) -> Option<ProcessSample> {
                None
            }

            pub(super) fn reset(&self) {}
        }
    } else {
        use std::collections::HashSet;

        use parking_lot::Mutex;
        use sysinfo::{Pid, ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System};

        use super::{PidSample, ProcessSample};
        use crate::ai::agent::LrcProcessState;

        /// Owns the process table used to observe monitored commands.
        ///
        /// Its lock is held only by the sampler task, and never while the
        /// monitor's own state lock is held, so the syscall-heavy refresh
        /// cannot block a report taken with the terminal model lock held.
        #[derive(Default)]
        pub(super) struct Sampler {
            system: Mutex<System>,
        }

        impl Sampler {
            /// Refreshes process information and summarizes the command's process tree.
            ///
            /// Discovery and measurement are split so that CPU and disk are only ever
            /// sampled for the command's tree, never across the full process table.
            pub(super) fn collect(
                &self,
                shell: Option<&ShellProcessInfo>,
            ) -> Option<ProcessSample> {
                let shell = shell?;
                let shell_pid = Pid::from_u32(shell.pid);

                let mut system = self.system.lock();
                system.refresh_processes_specifics(
                    ProcessesToUpdate::All,
                    true, /* remove_dead_processes */
                    ProcessRefreshKind::nothing(),
                );

                let pids = command_process_tree(&system, shell_pid, foreground_pgid(shell));

                system.refresh_processes_specifics(
                    ProcessesToUpdate::Some(&pids),
                    true, /* remove_dead_processes */
                    ProcessRefreshKind::nothing().with_cpu().with_disk_usage(),
                );

                let mut per_pid = Vec::with_capacity(pids.len());
                let mut states = Vec::with_capacity(pids.len());
                for pid in pids {
                    let Some(process) = system.process(pid) else {
                        continue;
                    };
                    per_pid.push(PidSample {
                        pid: pid.as_u32(),
                        cpu_ms: process.accumulated_cpu_time(),
                        io_write_bytes: process.disk_usage().total_written_bytes,
                    });
                    states.push(process_state(process.status()));
                }

                Some(ProcessSample {
                    state: aggregate_state(&states),
                    per_pid,
                })
            }

            /// Releases the process table once nothing is being monitored.
            pub(super) fn reset(&self) {
                *self.system.lock() = System::new();
            }
        }

        /// Reduces per-process states to one state for the tree, preferring whichever
        /// is the strongest evidence of progress.
        pub(super) fn aggregate_state(states: &[LrcProcessState]) -> LrcProcessState {
            for candidate in [
                LrcProcessState::Running,
                LrcProcessState::DiskWait,
                LrcProcessState::Sleeping,
                LrcProcessState::Stopped,
                LrcProcessState::Zombie,
            ] {
                if states.contains(&candidate) {
                    return candidate;
                }
            }
            LrcProcessState::Unknown
        }

        fn process_state(status: ProcessStatus) -> LrcProcessState {
            match status {
                ProcessStatus::Run | ProcessStatus::Waking => LrcProcessState::Running,
                ProcessStatus::UninterruptibleDiskSleep => LrcProcessState::DiskWait,
                ProcessStatus::Sleep | ProcessStatus::Idle | ProcessStatus::Parked => {
                    LrcProcessState::Sleeping
                }
                ProcessStatus::Stop | ProcessStatus::Tracing | ProcessStatus::LockBlocked => {
                    LrcProcessState::Stopped
                }
                ProcessStatus::Zombie | ProcessStatus::Dead | ProcessStatus::Wakekill => {
                    LrcProcessState::Zombie
                }
                ProcessStatus::Unknown(_) => LrcProcessState::Unknown,
            }
        }

        /// The processes belonging to the command the shell is currently running.
        ///
        /// Every process in the job descends from the shell, so the descendant set is
        /// the outer bound. When the pty's foreground process group is known it narrows
        /// that set to just the job in the foreground, which matters when a shell has
        /// other children (background jobs, its own helpers). Pipeline members are
        /// siblings rather than descendants of the group leader, so the group is matched
        /// per process rather than by walking down from the leader.
        ///
        /// The shell itself joins the tree when it holds the terminal, since builtins
        /// and shell functions run in that process and would otherwise leave the tree
        /// looking empty while the command is busy.
        pub(super) fn command_process_tree(
            system: &System,
            shell_pid: Pid,
            foreground_pgid: Option<u32>,
        ) -> Vec<Pid> {
            let descendants = descendants_of(system, shell_pid);

            let Some(pgid) = foreground_pgid else {
                return descendants.into_iter().collect();
            };

            let mut in_foreground_group: Vec<Pid> = descendants
                .iter()
                .filter(|pid| process_group_of(**pid) == Some(pgid))
                .copied()
                .collect();

            if process_group_of(shell_pid) == Some(pgid) {
                in_foreground_group.push(shell_pid);
            }

            // An empty result means the group is stale or unreadable rather than that
            // the command is gone, so fall back rather than under-report.
            if in_foreground_group.is_empty() {
                return descendants.into_iter().collect();
            }
            in_foreground_group
        }

        /// Every process descended from `pid`, excluding `pid` itself.
        fn descendants_of(system: &System, pid: Pid) -> HashSet<Pid> {
            let mut descendants = HashSet::new();
            // Repeatedly sweep the process table, adding processes whose parent is
            // already known to be in the tree, until nothing new appears. The tree is
            // shallow in practice, so this converges in a couple of passes.
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
        pub(super) fn process_group_of(pid: Pid) -> Option<u32> {
            // SAFETY: `getpgid` only reads scheduling metadata for `pid`, and reports
            // failure through its return value for pids that no longer exist.
            let pgid = unsafe { libc::getpgid(pid.as_u32() as libc::pid_t) };
            (pgid > 0).then_some(pgid as u32)
        }

        #[cfg(not(unix))]
        pub(super) fn process_group_of(_pid: Pid) -> Option<u32> {
            None
        }

        /// The pty's foreground process group.
        ///
        /// Returns `None` on any error rather than a guess: the stored descriptor can
        /// outlive the pty, and reporting a process group that belongs to something
        /// else would claim a dead command is still burning CPU. Callers additionally
        /// validate that the returned group is descended from the shell.
        #[cfg(unix)]
        fn foreground_pgid(shell: &ShellProcessInfo) -> Option<u32> {
            let fd = shell.pty_leader_fd?;
            // SAFETY: `tcgetpgrp` only reads terminal state for `fd`. A stale or reused
            // descriptor makes it fail or answer about an unrelated terminal; both are
            // handled by returning `None` or by the caller's ancestry check.
            let pgid = unsafe { libc::tcgetpgrp(fd) };
            (pgid > 0).then_some(pgid as u32)
        }

        #[cfg(not(unix))]
        fn foreground_pgid(_shell: &ShellProcessInfo) -> Option<u32> {
            // Windows has no controlling-terminal process group; the descendant scan is
            // the only discovery mechanism there.
            None
        }
    }
}
