//! Collects evidence that an agent-monitored long-running command is still
//! doing work.
//!
//! The agent decides whether to cancel a long-running command from the snapshot
//! it is given. When that snapshot is only a terminal grid, a command that
//! redirects its output to a file, suppresses output, or computes silently is
//! indistinguishable from a hung one. This module adds three tiers of evidence —
//! terminal output changes, process-tree CPU and I/O, and growth of redirect
//! target files — so the agent can tell "silent" from "stuck".
//!
//! Sampling runs at a fixed 1 Hz for as long as an agent-monitored command is
//! active, which is what makes the reported "seconds since last activity"
//! wall-clock accurate regardless of how far apart the agent's polls are. The
//! sampler is started when an agent action that can produce a snapshot begins
//! and stops as soon as no monitored command remains, so ordinary terminal use
//! pays nothing for it.

use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use instant::Instant;
use parking_lot::{FairMutex, Mutex};
use sysinfo::{Pid, ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System};

use super::lrc_redirect::parse_redirect_targets;
use crate::ai::agent::redaction::redact_secrets;
use crate::ai::agent::{LrcActivity, LrcFileActivity, LrcProcessActivity, LrcProcessState};
use crate::terminal::TerminalModel;
use crate::terminal::model::block::{
    Block, BlockId, CURSOR_MARKER, formatted_terminal_contents_for_input,
};
use crate::terminal::model::terminal_model::ShellProcessInfo;

/// How often liveness signals are sampled while a monitored command is active.
pub const SAMPLE_INTERVAL: Duration = Duration::from_secs(1);

/// Rows of terminal output hashed to detect on-screen changes. Enough to catch
/// in-place progress bars and spinners without formatting the whole scrollback
/// on every sample.
const GRID_HASH_MAX_ROWS: usize = 200;

/// Maximum bytes of a tracked file's tail included in a report.
const MAX_TAIL_BYTES: u64 = 2048;

/// Maximum lines of a tracked file's tail included in a report.
const MAX_TAIL_LINES: usize = 20;

/// Liveness signals for the commands an agent is currently monitoring.
///
/// Cloned as an [`Arc`] into both the sampler task and the futures that build
/// snapshots, neither of which can reach a `ModelContext`.
#[derive(Default)]
pub struct LrcActivityMonitor {
    state: Mutex<MonitorState>,
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
    /// Whether the process and file tiers can be collected at all. False for
    /// remote sessions, whose processes and files live on another host.
    signals_available: bool,
    system: System,
}

/// Per-command state, accumulated across samples and reset on each report.
struct BlockActivity {
    output: OutputTier,
    process: ProcessTier,
    files: Vec<FileTier>,
    /// When any tier last showed activity.
    last_activity: Instant,
    /// Whether the process and file tiers are collectable for this command.
    signals_available: bool,
}

struct OutputTier {
    hash: u64,
    last_change: Instant,
    changed_since_report: bool,
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

struct FileTier {
    path: PathBuf,
    /// Current size, or `None` while the file does not exist.
    size: Option<u64>,
    /// Size at the previous report, used to derive the reported delta.
    size_at_last_report: u64,
}

/// One sample's raw observations for a single command.
struct BlockSample {
    output_hash: u64,
    /// `None` when the process tier could not be collected.
    process: Option<ProcessSample>,
    file_sizes: Vec<Option<u64>>,
}

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

    /// Records whether the process and file tiers can be collected for this
    /// terminal. Remote sessions report only the terminal-output tier.
    pub fn set_signals_available(&self, available: bool) {
        self.state.lock().signals_available = available;
    }

    /// Registers an in-flight agent action and reports whether a sampler task
    /// must be started. The caller must pair this with [`Self::disarm`].
    pub fn arm(&self) -> bool {
        let mut state = self.state.lock();
        state.armed_actions += 1;
        if state.sampler_running {
            return false;
        }
        state.sampler_running = true;
        true
    }

    pub fn disarm(&self) {
        let mut state = self.state.lock();
        state.armed_actions = state.armed_actions.saturating_sub(1);
    }

    /// Builds the activity report for `block`, registering it on first sight.
    ///
    /// Called while the terminal model lock is held, so it must not try to
    /// acquire it. The sampler never holds the monitor lock while taking the
    /// terminal lock, so this ordering cannot deadlock.
    pub fn report(&self, block: &Block, model: &TerminalModel) -> Option<LrcActivity> {
        let now = Instant::now();
        let mut state = self.state.lock();
        let signals_available = state.signals_available;

        if !state.blocks.contains_key(block.id()) {
            let activity = BlockActivity::new(block, model, signals_available, now);
            state.blocks.insert(block.id().clone(), activity);
        }

        let block_activity = state.blocks.get_mut(block.id())?;
        Some(block_activity.take_report(now, read_and_redact_tail))
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
        let tracked: Vec<(BlockId, Vec<PathBuf>)> = {
            let state = self.state.lock();
            state
                .blocks
                .iter()
                .map(|(block_id, activity)| {
                    (
                        block_id.clone(),
                        activity
                            .files
                            .iter()
                            .map(|file| file.path.clone())
                            .collect(),
                    )
                })
                .collect()
        };

        // Read everything needed from the terminal in one pass, then release it
        // before doing any syscalls.
        let (grid_hashes, finished, shell_process) = {
            let model = terminal_model.lock();
            let mut grid_hashes = HashMap::new();
            let mut finished = Vec::new();
            for (block_id, _) in &tracked {
                match model.block_list().block_with_id(block_id) {
                    Some(block) if !block.finished() => {
                        grid_hashes.insert(block_id.clone(), grid_hash(block, &model));
                    }
                    // Gone or completed: nothing left to monitor.
                    Some(_) | None => finished.push(block_id.clone()),
                }
            }
            (grid_hashes, finished, model.shell_process_info().cloned())
        };

        let signals_available = self.state.lock().signals_available;
        let process_sample = if signals_available {
            self.collect_process_sample(shell_process.as_ref())
        } else {
            None
        };

        let mut samples = HashMap::new();
        for (block_id, paths) in tracked {
            let Some(output_hash) = grid_hashes.get(&block_id).copied() else {
                continue;
            };
            let file_sizes = if signals_available {
                paths.iter().map(|path| file_size(path)).collect()
            } else {
                vec![None; paths.len()]
            };
            samples.insert(
                block_id,
                BlockSample {
                    output_hash,
                    process: process_sample.clone(),
                    file_sizes,
                },
            );
        }

        let now = Instant::now();
        let mut state = self.state.lock();
        for block_id in finished {
            state.blocks.remove(&block_id);
        }
        for (block_id, sample) in samples {
            if let Some(activity) = state.blocks.get_mut(&block_id) {
                activity.apply_sample(sample, now);
            }
        }

        let keep_sampling = !state.blocks.is_empty() || state.armed_actions > 0;
        state.sampler_running = keep_sampling;
        keep_sampling
    }

    /// Refreshes process information and summarizes the command's process tree.
    fn collect_process_sample(&self, shell: Option<&ShellProcessInfo>) -> Option<ProcessSample> {
        let shell = shell?;
        let shell_pid = Pid::from_u32(shell.pid);

        let mut state = self.state.lock();
        state.system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true, /* remove_dead_processes */
            ProcessRefreshKind::nothing().with_cpu().with_disk_usage(),
        );

        let pids = command_process_tree(&state.system, shell_pid, foreground_pgid(shell));

        let mut per_pid = Vec::with_capacity(pids.len());
        let mut states = Vec::with_capacity(pids.len());
        for pid in pids {
            let Some(process) = state.system.process(pid) else {
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
    fn new(block: &Block, model: &TerminalModel, signals_available: bool, now: Instant) -> Self {
        let cwd = block.pwd().map(PathBuf::from);
        let targets = parse_redirect_targets(
            &block.command_with_secrets_unobfuscated(false),
            cwd.as_deref(),
        );
        Self::from_parts(
            grid_hash(block, model),
            targets,
            signals_available,
            now,
            file_size,
        )
    }

    /// `probe_size` is injected so tests can drive the file tier without
    /// touching the filesystem.
    fn from_parts(
        output_hash: u64,
        targets: Vec<PathBuf>,
        signals_available: bool,
        now: Instant,
        probe_size: impl Fn(&Path) -> Option<u64>,
    ) -> Self {
        let files = targets
            .into_iter()
            .map(|path| {
                let size = signals_available.then(|| probe_size(&path)).flatten();
                FileTier {
                    path,
                    size,
                    size_at_last_report: size.unwrap_or(0),
                }
            })
            .collect();

        Self {
            output: OutputTier {
                hash: output_hash,
                last_change: now,
                changed_since_report: false,
            },
            process: ProcessTier::default(),
            files,
            // A command that has only just come under monitoring has no history
            // of inactivity, so its clock starts now rather than at zero.
            last_activity: now,
            signals_available,
        }
    }

    /// Folds one sample into the accumulated state.
    fn apply_sample(&mut self, sample: BlockSample, now: Instant) {
        let mut saw_activity = false;

        if sample.output_hash != self.output.hash {
            self.output.hash = sample.output_hash;
            self.output.last_change = now;
            self.output.changed_since_report = true;
            saw_activity = true;
        }

        if let Some(process) = sample.process {
            let previous_count = self.process.live_process_count;
            let mut cpu_ms_by_pid = HashMap::with_capacity(process.per_pid.len());
            let mut io_write_bytes_by_pid = HashMap::with_capacity(process.per_pid.len());
            let mut cpu_delta = 0u64;
            let mut io_delta = 0u64;

            for pid_sample in &process.per_pid {
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

            self.process.cpu_ms_by_pid = cpu_ms_by_pid;
            self.process.io_write_bytes_by_pid = io_write_bytes_by_pid;
            self.process.cpu_ms_since_report += cpu_delta;
            self.process.io_write_bytes_since_report += io_delta;
            self.process.state = process.state;
            self.process.live_process_count = process.per_pid.len() as u32;
            self.process.sampled = true;

            // Process churn is itself progress: a build spawning and reaping
            // compilers may never accumulate much CPU in any single process.
            let count_changed = self.process.live_process_count != previous_count;
            saw_activity |= cpu_delta > 0 || io_delta > 0 || count_changed;
        }

        for (file, size) in self.files.iter_mut().zip(sample.file_sizes) {
            if size.unwrap_or(0) > file.size.unwrap_or(0) {
                saw_activity = true;
            }
            file.size = size;
        }

        if saw_activity {
            self.last_activity = now;
        }
    }

    /// Produces the report for a snapshot and resets the per-report accumulators.
    ///
    /// `read_tail` is injected so the (comparatively expensive) file read and
    /// secret-redaction pass happens only here, never on the sampling path.
    fn take_report(&mut self, now: Instant, read_tail: impl Fn(&Path) -> String) -> LrcActivity {
        // An all-zero process tier is a meaningful reading — an exited tree —
        // so it is reported rather than suppressed. It is only withheld when no
        // reading was taken at all, which must not be mistaken for one.
        let process_collected = self.signals_available && self.process.sampled;
        let process = process_collected.then(|| LrcProcessActivity {
            cpu_time_delta: Duration::from_millis(self.process.cpu_ms_since_report),
            state: self.process.state,
            live_process_count: self.process.live_process_count,
            io_write_bytes_delta: self.process.io_write_bytes_since_report,
        });

        let files = self
            .files
            .iter_mut()
            .filter_map(|file| {
                let size = file.size?;
                let activity = LrcFileActivity {
                    path: file.path.to_string_lossy().into_owned(),
                    size_bytes: size,
                    size_delta_bytes: size as i64 - file.size_at_last_report as i64,
                    tail: read_tail(&file.path),
                };
                file.size_at_last_report = size;
                Some(activity)
            })
            .collect();

        let report = LrcActivity {
            since_last_activity: Some(now.saturating_duration_since(self.last_activity)),
            output_changed_since_last_read: self.output.changed_since_report,
            since_output_change: Some(now.saturating_duration_since(self.output.last_change)),
            process,
            files,
            signals_unavailable: !process_collected,
        };

        self.output.changed_since_report = false;
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

fn grid_hash(block: &Block, model: &TerminalModel) -> u64 {
    let contents = if model.is_alt_screen_active() {
        formatted_terminal_contents_for_input(
            model.alt_screen().grid_handler(),
            Some(GRID_HASH_MAX_ROWS),
            CURSOR_MARKER,
        )
    } else {
        formatted_terminal_contents_for_input(
            block.output_grid().grid_handler(),
            Some(GRID_HASH_MAX_ROWS),
            CURSOR_MARKER,
        )
    };

    let mut hasher = DefaultHasher::new();
    contents.hash(&mut hasher);
    hasher.finish()
}

/// Size of `path`, or `None` when it does not exist or is not a regular file.
/// Symlinks are not followed, so a command cannot be made to report on a file
/// somewhere else entirely.
fn file_size(path: &Path) -> Option<u64> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    metadata.is_file().then_some(metadata.len())
}

/// Reads the tail of `path` and redacts any secrets in it.
fn read_and_redact_tail(path: &Path) -> String {
    let Some(mut tail) = read_tail(path) else {
        return String::new();
    };
    redact_secrets(&mut tail);
    tail
}

fn read_tail(path: &Path) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};

    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let offset = len.saturating_sub(MAX_TAIL_BYTES);
    file.seek(SeekFrom::Start(offset)).ok()?;

    let mut buffer = Vec::with_capacity(MAX_TAIL_BYTES as usize);
    file.take(MAX_TAIL_BYTES).read_to_end(&mut buffer).ok()?;

    // Binary output is noise to the agent, and a partial read can split a
    // multi-byte character at the start of the window.
    let text = String::from_utf8_lossy(&buffer);
    if text.contains('\u{0}') {
        return None;
    }

    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(MAX_TAIL_LINES);
    Some(lines[start..].join("\n"))
}

/// The processes belonging to the command the shell is currently running.
///
/// Every process in the job descends from the shell, so the descendant set is
/// the outer bound. When the pty's foreground process group is known it narrows
/// that set to just the job in the foreground, which matters when a shell has
/// other children (background jobs, its own helpers). Pipeline members are
/// siblings rather than descendants of the group leader, so the group is matched
/// per process rather than by walking down from the leader.
fn command_process_tree(system: &System, shell_pid: Pid, foreground_pgid: Option<u32>) -> Vec<Pid> {
    let descendants = descendants_of(system, shell_pid);

    let Some(pgid) = foreground_pgid else {
        return descendants.into_iter().collect();
    };

    let in_foreground_group: Vec<Pid> = descendants
        .iter()
        .filter(|pid| process_group_of(**pid) == Some(pgid))
        .copied()
        .collect();

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
