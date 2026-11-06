use std::time::Duration;

use instant::Instant;

use super::sampler::aggregate_state;
use super::{BlockActivity, LrcActivityMonitor, LrcProcessState, PidSample, ProcessSample};
use crate::terminal::model::block::BlockId;

/// A process tree sample from `(pid, cpu_ms)` pairs.
fn process_sample(pids: &[(u32, u64)]) -> ProcessSample {
    ProcessSample {
        per_pid: pids
            .iter()
            .map(|(pid, cpu_ms)| PidSample {
                pid: *pid,
                cpu_ms: *cpu_ms,
                io_write_bytes: 0,
            })
            .collect(),
        state: LrcProcessState::Running,
    }
}

/// A process tree sample from `(pid, io_write_bytes)` pairs, with no CPU use.
fn io_sample(pids: &[(u32, u64)]) -> ProcessSample {
    ProcessSample {
        per_pid: pids
            .iter()
            .map(|(pid, io_write_bytes)| PidSample {
                pid: *pid,
                cpu_ms: 0,
                io_write_bytes: *io_write_bytes,
            })
            .collect(),
        state: LrcProcessState::Running,
    }
}

#[test]
fn a_quiet_process_tree_accumulates_time_since_last_activity() {
    let start = Instant::now();
    let mut activity = BlockActivity::new(start);

    // The first sample's pid churn (nothing -> one pid) counts as activity;
    // every later sample is identical, so the clock runs from there.
    for second in 1..=5 {
        activity.apply_sample(
            process_sample(&[(100, 1_000)]),
            start + Duration::from_secs(second),
        );
    }

    let report = activity
        .take_report(start + Duration::from_secs(5))
        .expect("report");
    assert_eq!(report.since_last_activity, Some(Duration::from_secs(4)));
}

#[test]
fn first_sighting_of_a_process_contributes_no_cpu_delta() {
    let start = Instant::now();
    let mut activity = BlockActivity::new(start);

    // A process that has been running long before monitoring began must not
    // have its lifetime CPU counted as activity.
    activity.apply_sample(
        process_sample(&[(100, 900_000)]),
        start + Duration::from_secs(1),
    );

    let report = activity
        .take_report(start + Duration::from_secs(1))
        .expect("report");
    let process = report.process.expect("process tier should be reported");
    assert_eq!(process.cpu_time_delta, Duration::ZERO);
    assert_eq!(process.live_process_count, 1);
}

#[test]
fn cpu_time_counts_as_activity() {
    let start = Instant::now();
    let mut activity = BlockActivity::new(start);

    activity.apply_sample(
        process_sample(&[(100, 1_000)]),
        start + Duration::from_secs(1),
    );
    activity.apply_sample(
        process_sample(&[(100, 1_750)]),
        start + Duration::from_secs(2),
    );

    let report = activity
        .take_report(start + Duration::from_secs(3))
        .expect("report");
    let process = report.process.expect("process tier should be reported");
    assert_eq!(process.cpu_time_delta, Duration::from_millis(750));
    // The last activity was the CPU accrual at t+2, not the report at t+3.
    assert_eq!(report.since_last_activity, Some(Duration::from_secs(1)));
}

#[test]
fn io_writes_count_as_activity() {
    let start = Instant::now();
    let mut activity = BlockActivity::new(start);

    activity.apply_sample(io_sample(&[(100, 0)]), start + Duration::from_secs(1));
    activity.apply_sample(io_sample(&[(100, 4_096)]), start + Duration::from_secs(5));

    let report = activity
        .take_report(start + Duration::from_secs(5))
        .expect("report");
    let process = report.process.expect("process tier should be reported");
    assert_eq!(process.io_write_bytes_delta, 4_096);
    assert_eq!(report.since_last_activity, Some(Duration::ZERO));
}

#[test]
fn cpu_delta_is_summed_across_the_process_tree() {
    let start = Instant::now();
    let mut activity = BlockActivity::new(start);

    activity.apply_sample(
        process_sample(&[(100, 0), (101, 0)]),
        start + Duration::from_secs(1),
    );
    activity.apply_sample(
        process_sample(&[(100, 200), (101, 300)]),
        start + Duration::from_secs(2),
    );

    let report = activity
        .take_report(start + Duration::from_secs(2))
        .expect("report");
    let process = report.process.expect("process tier should be reported");
    assert_eq!(process.cpu_time_delta, Duration::from_millis(500));
    assert_eq!(process.live_process_count, 2);
}

#[test]
fn cpu_delta_accumulates_across_samples_within_one_report() {
    let start = Instant::now();
    let mut activity = BlockActivity::new(start);

    for second in 1..=4 {
        activity.apply_sample(
            process_sample(&[(100, second * 100)]),
            start + Duration::from_secs(second),
        );
    }

    let report = activity
        .take_report(start + Duration::from_secs(4))
        .expect("report");
    let process = report.process.expect("process tier should be reported");
    // 3 deltas of 100ms; the first sample established the baseline.
    assert_eq!(process.cpu_time_delta, Duration::from_millis(300));
}

#[test]
fn cpu_delta_resets_between_reports() {
    let start = Instant::now();
    let mut activity = BlockActivity::new(start);

    activity.apply_sample(process_sample(&[(100, 0)]), start + Duration::from_secs(1));
    activity.apply_sample(
        process_sample(&[(100, 500)]),
        start + Duration::from_secs(2),
    );
    let _ = activity.take_report(start + Duration::from_secs(2));

    let second = activity
        .take_report(start + Duration::from_secs(3))
        .expect("report");
    let process = second.process.expect("process tier should be reported");
    assert_eq!(process.cpu_time_delta, Duration::ZERO);
}

#[test]
fn exited_process_tree_reports_no_live_processes_and_no_stale_cpu() {
    let start = Instant::now();
    let mut activity = BlockActivity::new(start);

    activity.apply_sample(
        process_sample(&[(100, 1_000)]),
        start + Duration::from_secs(1),
    );
    activity.apply_sample(
        process_sample(&[(100, 2_000)]),
        start + Duration::from_secs(2),
    );
    let _ = activity.take_report(start + Duration::from_secs(2));

    // The command's processes are gone: an empty tree, not a missing sample.
    activity.apply_sample(process_sample(&[]), start + Duration::from_secs(3));

    let report = activity
        .take_report(start + Duration::from_secs(4))
        .expect("report");
    let process = report.process.expect("process tier should be reported");
    assert_eq!(process.live_process_count, 0);
    assert_eq!(process.cpu_time_delta, Duration::ZERO);
}

#[test]
fn a_pid_that_exits_stops_contributing_to_later_deltas() {
    let start = Instant::now();
    let mut activity = BlockActivity::new(start);

    activity.apply_sample(
        process_sample(&[(100, 1_000)]),
        start + Duration::from_secs(1),
    );
    activity.apply_sample(process_sample(&[]), start + Duration::from_secs(2));
    let _ = activity.take_report(start + Duration::from_secs(2));

    // The same pid number reappears, now belonging to an unrelated process with
    // a large lifetime CPU total. It must be treated as newly seen.
    activity.apply_sample(
        process_sample(&[(100, 900_000)]),
        start + Duration::from_secs(3),
    );

    let report = activity
        .take_report(start + Duration::from_secs(3))
        .expect("report");
    let process = report.process.expect("process tier should be reported");
    assert_eq!(process.cpu_time_delta, Duration::ZERO);
}

#[test]
fn process_churn_counts_as_activity() {
    let start = Instant::now();
    let mut activity = BlockActivity::new(start);

    activity.apply_sample(
        process_sample(&[(100, 500)]),
        start + Duration::from_secs(1),
    );
    let _ = activity.take_report(start + Duration::from_secs(1));

    // A build that spawns and reaps compilers may show no CPU delta on any
    // single pid, but the changing set of processes is real progress.
    activity.apply_sample(
        process_sample(&[(100, 500), (101, 0)]),
        start + Duration::from_secs(5),
    );

    let report = activity
        .take_report(start + Duration::from_secs(5))
        .expect("report");
    assert_eq!(report.since_last_activity, Some(Duration::ZERO));
}

#[test]
fn same_count_pid_replacement_counts_as_activity() {
    let start = Instant::now();
    let mut activity = BlockActivity::new(start);

    activity.apply_sample(
        process_sample(&[(100, 500)]),
        start + Duration::from_secs(1),
    );
    let _ = activity.take_report(start + Duration::from_secs(1));

    activity.apply_sample(
        process_sample(&[(101, 500)]),
        start + Duration::from_secs(5),
    );

    let report = activity
        .take_report(start + Duration::from_secs(5))
        .expect("report");
    let process = report.process.expect("process tier should be reported");
    assert_eq!(process.cpu_time_delta, Duration::ZERO);
    assert_eq!(process.live_process_count, 1);
    assert_eq!(report.since_last_activity, Some(Duration::ZERO));
}

/// The server reads a missing submessage and an all-zero one differently, so a
/// quiet-but-inspected process tree must still produce a present submessage.
#[test]
fn a_fully_quiet_process_tree_is_still_reported() {
    let start = Instant::now();
    let mut activity = BlockActivity::new(start);

    // Inspected repeatedly, and every reading is zero: no CPU, no I/O, and a
    // single sleeping process.
    for second in 1..=5 {
        activity.apply_sample(
            ProcessSample {
                per_pid: vec![PidSample {
                    pid: 100,
                    cpu_ms: 0,
                    io_write_bytes: 0,
                }],
                state: LrcProcessState::Sleeping,
            },
            start + Duration::from_secs(second),
        );
    }

    let report = activity
        .take_report(start + Duration::from_secs(5))
        .expect("report");
    let process = report
        .process
        .expect("an all-zero reading is still a reading");
    assert_eq!(process.cpu_time_delta, Duration::ZERO);
    assert_eq!(process.io_write_bytes_delta, 0);
    assert_eq!(process.live_process_count, 1);
    assert_eq!(process.state, LrcProcessState::Sleeping);
}

/// A command is registered when its first snapshot is built, before the sampler
/// has run for it. Reporting the still-zero counters then would describe a
/// healthy command as a process tree with nothing running.
#[test]
fn no_report_until_the_process_tree_has_actually_been_sampled() {
    let start = Instant::now();
    let mut activity = BlockActivity::new(start);

    assert!(activity.take_report(start).is_none());

    activity.apply_sample(
        process_sample(&[(100, 1_000)]),
        start + Duration::from_secs(1),
    );

    let report = activity
        .take_report(start + Duration::from_secs(1))
        .expect("a sampled tree produces a report");
    assert!(report.process.is_some());
}

#[test]
fn a_remote_terminal_reports_no_activity() {
    let monitor = LrcActivityMonitor::new();
    monitor.set_monitoring_enabled(false);

    assert!(monitor.report(&BlockId::new()).is_none());
}

#[test]
fn a_remote_terminal_does_not_start_the_sampler() {
    let monitor = LrcActivityMonitor::new();
    monitor.set_monitoring_enabled(false);

    assert!(!monitor.arm());
}

#[test]
fn a_local_terminal_starts_the_sampler_but_reports_nothing_before_sampling() {
    let monitor = LrcActivityMonitor::new();
    monitor.set_monitoring_enabled(true);

    assert!(monitor.arm());
    // The sampler has not observed this block yet: no report is better than a
    // fabricated all-zeros reading.
    assert!(monitor.report(&BlockId::new()).is_none());
}

/// A builtin or shell function runs in the shell process itself, which then
/// holds the terminal. Sampling only descendants would show an empty tree while
/// the command is busy.
#[cfg(unix)]
#[test]
fn the_shell_joins_the_tree_when_it_holds_the_terminal() {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

    use super::sampler::{command_process_tree, process_group_of};

    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true, /* remove_dead_processes */
        ProcessRefreshKind::nothing(),
    );

    // This test's own process stands in for the shell, so the process group it
    // reports is a real one rather than a fabricated number.
    let shell_pid = Pid::from_u32(std::process::id());
    let shell_pgid = process_group_of(shell_pid).expect("the test process has a process group");

    let tree = command_process_tree(&system, shell_pid, Some(shell_pgid));
    assert!(tree.contains(&shell_pid));
}

#[cfg(unix)]
#[test]
fn the_shell_stays_out_of_the_tree_when_another_job_holds_the_terminal() {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

    use super::sampler::{command_process_tree, process_group_of};

    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true, /* remove_dead_processes */
        ProcessRefreshKind::nothing(),
    );

    let shell_pid = Pid::from_u32(std::process::id());
    let shell_pgid = process_group_of(shell_pid).expect("the test process has a process group");

    // Any group other than the shell's stands for an external job holding the
    // terminal; the shell is then only waiting on it.
    let tree = command_process_tree(&system, shell_pid, Some(shell_pgid + 1));
    assert!(!tree.contains(&shell_pid));
}

#[test]
fn aggregate_state_prefers_the_strongest_evidence_of_progress() {
    assert_eq!(
        aggregate_state(&[LrcProcessState::Sleeping, LrcProcessState::Running]),
        LrcProcessState::Running
    );
    assert_eq!(
        aggregate_state(&[LrcProcessState::Sleeping, LrcProcessState::DiskWait]),
        LrcProcessState::DiskWait
    );
    assert_eq!(
        aggregate_state(&[LrcProcessState::Zombie, LrcProcessState::Sleeping]),
        LrcProcessState::Sleeping
    );
    assert_eq!(aggregate_state(&[]), LrcProcessState::Unknown);
}
