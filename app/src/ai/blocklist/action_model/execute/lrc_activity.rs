//! Samples the shell's process tree to describe whether an agent-monitored
//! long-running command is still doing work.
//!
//! [`LrcActivityMonitor`] holds one entry per monitored command. While an agent
//! action that can produce a snapshot is in flight, a sampler runs every
//! [`SAMPLE_INTERVAL`] and records CPU time, bytes written, the number of live
//! processes, and the foreground process state.
//! [`LrcActivityMonitor::report`] folds everything accumulated since the
//! previous report into the [`LrcActivity`] attached to a snapshot, including
//! how long the tree has been idle.
//!
//! Only local sessions are sampled; remote sessions report no activity.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use instant::Instant;
use parking_lot::{FairMutex, Mutex};
use sysinfo::{Pid, ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System};

use crate::ai::agent::{LrcActivity, LrcProcessActivity, LrcProcessState};
use crate::terminal::TerminalModel;
use crate::terminal::model::block::BlockId;
use crate::terminal::model::terminal_model::ShellProcessInfo;

/// How often liveness signals are sampled while a monitored command is active.
pub const SAMPLE_INTERVAL: Duration = Duration::from_secs(5);

/// Liveness signals for the commands an agent is currently monitoring.
///
/// Cloned as an [`Arc`] into both the sampler task and the futures that build
/// snapshots, neither of which can reach a `ModelContext`.
#[derive(Default)]
pub struct LrcActivityMonitor {
    state: Mutex<MonitorState>,
    /// Touched only by the sampler task, of which at most one runs at a time.
    /// Kept apart from `state` so the syscall-heavy process refresh never
    /// blocks [`Self::report`], which is called with the terminal model lock
    /// held.
    system: Mutex<System>,
}

#[derive(Default)]
struct MonitorState {
    blocks: HashMap<BlockId, BlockActivity>,
    /// Agent actions in flight that could produce a snapshot. Keeps the sampler
    /// alive over the gap between an action starting and its block being
    /// registered on the first poll.
    armed_actions: usize,
    /// Whether a sampler task is currently running, so at most one exists.
    sampler_running: bool,
    /// Whether commands on this terminal can be monitored at all. False for
    /// remote sessions, whose processes live on another host; they report no
    /// activity rather than an ever-growing idle clock that falsely suggests
    /// a hang.
    monitoring_enabled: bool,
}

/// Per-command state, accumulated across samples and reset on each report.
struct BlockActivity {
    process: ProcessTier,
    /// When the process tree last showed activity.
    last_activity: Instant,
}

#[derive(Default)]
struct ProcessTier {
    /// Cumulative CPU milliseconds per pid, used to derive per-sample deltas.
    /// Rebuilt every sample so pids that exit stop contributing.
    cpu_ms_by_pid: HashMap<Pid, u64>,
    /// Cumulative bytes written per pid, used the same way.
    io_write_bytes_by_pid: HashMap<Pid, u64>,
    cpu_ms_since_report: u64,
    io_write_bytes_since_report: u64,
    state: LrcProcessState,
    live_process_count: u32,
    /// Whether the sampler has ever actually observed the process tree.
    ///
    /// A command is registered on its first snapshot, which is built before the
    /// sampler has had a chance to look at it. Until it has, every counter here
    /// is still zero, and zero is indistinguishable from a process tree with
    /// nothing left running.
    sampled: bool,
}

/// One sample's raw observations of the command's process tree.
#[derive(Clone)]
struct ProcessSample {
    per_pid: Vec<PidSample>,
    state: LrcProcessState,
}

#[derive(Clone)]
struct PidSample {
    pid: Pid,
    cpu_ms: u64,
    io_write_bytes: u64,
}

impl LrcActivityMonitor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records whether commands on this terminal can be monitored. Remote
    /// sessions cannot: they report no activity at all.
    pub fn set_monitoring_enabled(&self, enabled: bool) {
        self.state.lock().monitoring_enabled = enabled;
    }

    /// Registers an in-flight agent action and reports whether a sampler task
    /// must be started. The caller must pair this with [`Self::disarm`].
    /// Returns `false` when monitoring is disabled: there is nothing for a
    /// sampler to observe.
    pub fn arm(&self) -> bool {
        let mut state = self.state.lock();
        state.armed_actions += 1;
        if !state.monitoring_enabled || state.sampler_running {
            return false;
        }
        state.sampler_running = true;
        true
    }

    pub fn disarm(&self) {
        let mut state = self.state.lock();
        state.armed_actions = state.armed_actions.saturating_sub(1);
    }

    /// Builds the activity report for `block_id`, registering it on first
    /// sight. Returns `None` when monitoring is disabled for this terminal.
    ///
    /// Called while the terminal model lock is held, so it must not try to
    /// acquire it. The sampler never holds the monitor lock while taking the
    /// terminal lock, so this ordering cannot deadlock.
    pub fn report(&self, block_id: &BlockId) -> Option<LrcActivity> {
        let now = Instant::now();
        let mut state = self.state.lock();
        if !state.monitoring_enabled {
            return None;
        }

        let block_activity = state
            .blocks
            .entry(block_id.clone())
            .or_insert_with(|| BlockActivity::new(now));
        Some(block_activity.take_report(now))
    }

    /// Removes state for a command that is no longer being monitored.
    pub fn forget(&self, block_id: &BlockId) {
        self.state.lock().blocks.remove(block_id);
    }

    /// Takes one sample of every monitored command, returning whether sampling
    /// should continue.
    ///
    /// Locks are acquired one at a time and released before the next is taken,
    /// so this never holds the monitor lock while touching the terminal model.
    pub fn sample(&self, terminal_model: &Arc<FairMutex<TerminalModel>>) -> bool {
        let tracked: Vec<BlockId> = self.state.lock().blocks.keys().cloned().collect();

        // Read everything needed from the terminal in one pass, then release it
        // before doing any syscalls.
        let (live, finished, shell_process) = {
            let model = terminal_model.lock();
            let mut live = Vec::new();
            let mut finished = Vec::new();
            for block_id in tracked {
                match model.block_list().block_with_id(&block_id) {
                    Some(block) if !block.finished() => live.push(block_id),
                    // Gone or completed: nothing left to monitor.
                    Some(_) | None => finished.push(block_id),
                }
            }
            (live, finished, model.shell_process_info().cloned())
        };

        let process_sample = self.collect_process_sample(shell_process.as_ref());

        let now = Instant::now();
        let mut state = self.state.lock();
        for block_id in finished {
            state.blocks.remove(&block_id);
        }
        if let Some(sample) = process_sample {
            for block_id in live {
                if let Some(activity) = state.blocks.get_mut(&block_id) {
                    activity.apply_sample(sample.clone(), now);
                }
            }
        }

        let keep_sampling = !state.blocks.is_empty() || state.armed_actions > 0;
        state.sampler_running = keep_sampling;
        drop(state);
        if !keep_sampling {
            *self.system.lock() = System::new();
        }
        keep_sampling
    }

    /// Refreshes process information and summarizes the command's process tree.
    ///
    /// Discovery and measurement are split so that CPU and disk are only ever
    /// sampled for the command's tree, never across the full process table.
    fn collect_process_sample(&self, shell: Option<&ShellProcessInfo>) -> Option<ProcessSample> {
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
                pid,
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
}

impl BlockActivity {
    fn new(now: Instant) -> Self {
        Self {
            process: ProcessTier::default(),
            // A command that has only just come under monitoring has no history
            // of inactivity, so its clock starts now rather than at zero.
            last_activity: now,
        }
    }

    /// Folds one sample into the accumulated state.
    fn apply_sample(&mut self, sample: ProcessSample, now: Instant) {
        let mut cpu_ms_by_pid = HashMap::with_capacity(sample.per_pid.len());
        let mut io_write_bytes_by_pid = HashMap::with_capacity(sample.per_pid.len());
        let mut cpu_delta = 0u64;
        let mut io_delta = 0u64;

        for pid_sample in &sample.per_pid {
            // A pid seen for the first time contributes no delta: its
            // accumulated total predates monitoring.
            if let Some(previous) = self.process.cpu_ms_by_pid.get(&pid_sample.pid) {
                cpu_delta += pid_sample.cpu_ms.saturating_sub(*previous);
            }
            if let Some(previous) = self.process.io_write_bytes_by_pid.get(&pid_sample.pid) {
                io_delta += pid_sample.io_write_bytes.saturating_sub(*previous);
            }
            cpu_ms_by_pid.insert(pid_sample.pid, pid_sample.cpu_ms);
            io_write_bytes_by_pid.insert(pid_sample.pid, pid_sample.io_write_bytes);
        }
        let pid_set_changed = cpu_ms_by_pid.len() != self.process.cpu_ms_by_pid.len()
            || cpu_ms_by_pid
                .keys()
                .any(|pid| !self.process.cpu_ms_by_pid.contains_key(pid));

        self.process.cpu_ms_by_pid = cpu_ms_by_pid;
        self.process.io_write_bytes_by_pid = io_write_bytes_by_pid;
        self.process.cpu_ms_since_report += cpu_delta;
        self.process.io_write_bytes_since_report += io_delta;
        self.process.state = sample.state;
        self.process.live_process_count = sample.per_pid.len() as u32;
        self.process.sampled = true;

        // Process churn is itself progress: a build spawning and reaping
        // compilers may never accumulate much CPU in any single process.
        if cpu_delta > 0 || io_delta > 0 || pid_set_changed {
            self.last_activity = now;
        }
    }

    /// Produces the report for a snapshot and resets the per-report accumulators.
    fn take_report(&mut self, now: Instant) -> LrcActivity {
        // An all-zero process tier is a meaningful reading — an exited tree —
        // so it is reported rather than suppressed. It is only withheld when no
        // reading was taken at all, which must not be mistaken for one.
        let process_collected = self.process.sampled;
        let process = process_collected.then(|| LrcProcessActivity {
            cpu_time_delta: Duration::from_millis(self.process.cpu_ms_since_report),
            state: self.process.state,
            live_process_count: self.process.live_process_count,
            io_write_bytes_delta: self.process.io_write_bytes_since_report,
        });

        let report = LrcActivity {
            since_last_activity: Some(now.saturating_duration_since(self.last_activity)),
            process,
            signals_unavailable: !process_collected,
        };

        self.process.cpu_ms_since_report = 0;
        self.process.io_write_bytes_since_report = 0;

        report
    }
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

/// Reduces per-process states to one state for the tree, preferring whichever
/// is the strongest evidence of progress.
fn aggregate_state(states: &[LrcProcessState]) -> LrcProcessState {
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
fn command_process_tree(system: &System, shell_pid: Pid, foreground_pgid: Option<u32>) -> Vec<Pid> {
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
fn process_group_of(pid: Pid) -> Option<u32> {
    // SAFETY: `getpgid` only reads scheduling metadata for `pid`, and reports
    // failure through its return value for pids that no longer exist.
    let pgid = unsafe { libc::getpgid(pid.as_u32() as libc::pid_t) };
    (pgid > 0).then_some(pgid as u32)
}

#[cfg(not(unix))]
fn process_group_of(_pid: Pid) -> Option<u32> {
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

#[cfg(test)]
#[path = "lrc_activity_tests.rs"]
mod tests;
