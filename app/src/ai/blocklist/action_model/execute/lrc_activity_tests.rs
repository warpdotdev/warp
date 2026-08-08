use std::cell::Cell;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use instant::Instant;
use sysinfo::Pid;

use super::{
    BlockActivity, BlockSample, LrcProcessState, MAX_TAIL_LINES, PidSample, ProcessSample,
    aggregate_state, file_size, read_tail,
};

const OUTPUT_A: u64 = 1;
const OUTPUT_B: u64 = 2;

/// A monitored command with no redirect targets, on a session where all tiers
/// are collectable.
fn local_activity(now: Instant) -> BlockActivity {
    BlockActivity::from_parts(OUTPUT_A, Vec::new(), true, now, |_| None)
}

fn activity_with_files(
    paths: &[&str],
    initial_sizes: &[Option<u64>],
    now: Instant,
) -> BlockActivity {
    let targets: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
    let sizes: Vec<(PathBuf, Option<u64>)> = targets
        .iter()
        .cloned()
        .zip(initial_sizes.iter().copied())
        .collect();
    BlockActivity::from_parts(OUTPUT_A, targets, true, now, move |path| {
        sizes
            .iter()
            .find(|(candidate, _)| candidate == path)
            .and_then(|(_, size)| *size)
    })
}

/// A process tree sample from `(pid, cpu_ms)` pairs.
fn process_sample(pids: &[(u32, u64)]) -> ProcessSample {
    ProcessSample {
        per_pid: pids
            .iter()
            .map(|(pid, cpu_ms)| PidSample {
                pid: Pid::from_u32(*pid),
                cpu_ms: *cpu_ms,
                io_write_bytes: 0,
            })
            .collect(),
        state: LrcProcessState::Running,
    }
}

fn sample(output_hash: u64, process: Option<ProcessSample>) -> BlockSample {
    BlockSample {
        output_hash,
        process,
        file_sizes: Vec::new(),
    }
}

fn file_sample(output_hash: u64, file_sizes: Vec<Option<u64>>) -> BlockSample {
    BlockSample {
        output_hash,
        process: None,
        file_sizes,
    }
}

fn no_tail(_path: &Path) -> String {
    String::new()
}

#[test]
fn silent_command_accumulates_time_since_last_activity() {
    let start = Instant::now();
    let mut activity = local_activity(start);

    for second in 1..=5 {
        activity.apply_sample(sample(OUTPUT_A, None), start + Duration::from_secs(second));
    }

    let report = activity.take_report(start + Duration::from_secs(5), no_tail);
    assert_eq!(report.since_last_activity, Some(Duration::from_secs(5)));
    assert!(!report.output_changed_since_last_read);
}

#[test]
fn output_change_resets_the_activity_clock() {
    let start = Instant::now();
    let mut activity = local_activity(start);

    activity.apply_sample(sample(OUTPUT_A, None), start + Duration::from_secs(5));
    activity.apply_sample(sample(OUTPUT_B, None), start + Duration::from_secs(6));

    let report = activity.take_report(start + Duration::from_secs(6), no_tail);
    assert_eq!(report.since_last_activity, Some(Duration::ZERO));
    assert_eq!(report.since_output_change, Some(Duration::ZERO));
    assert!(report.output_changed_since_last_read);
}

#[test]
fn output_change_flag_clears_after_being_reported() {
    let start = Instant::now();
    let mut activity = local_activity(start);

    activity.apply_sample(sample(OUTPUT_B, None), start + Duration::from_secs(1));
    let first = activity.take_report(start + Duration::from_secs(1), no_tail);
    assert!(first.output_changed_since_last_read);

    activity.apply_sample(sample(OUTPUT_B, None), start + Duration::from_secs(2));
    let second = activity.take_report(start + Duration::from_secs(2), no_tail);
    assert!(!second.output_changed_since_last_read);
    // The output has not changed since the previous sample, but the time since
    // it last changed keeps growing.
    assert_eq!(second.since_output_change, Some(Duration::from_secs(1)));
}

#[test]
fn first_sighting_of_a_process_contributes_no_cpu_delta() {
    let start = Instant::now();
    let mut activity = local_activity(start);

    // A process that has been running long before monitoring began must not
    // have its lifetime CPU counted as activity.
    activity.apply_sample(
        sample(OUTPUT_A, Some(process_sample(&[(100, 900_000)]))),
        start + Duration::from_secs(1),
    );

    let report = activity.take_report(start + Duration::from_secs(1), no_tail);
    let process = report.process.expect("process tier should be reported");
    assert_eq!(process.cpu_time_delta, Duration::ZERO);
    assert_eq!(process.live_process_count, 1);
}

#[test]
fn cpu_time_counts_as_activity() {
    let start = Instant::now();
    let mut activity = local_activity(start);

    activity.apply_sample(
        sample(OUTPUT_A, Some(process_sample(&[(100, 1_000)]))),
        start + Duration::from_secs(1),
    );
    activity.apply_sample(
        sample(OUTPUT_A, Some(process_sample(&[(100, 1_750)]))),
        start + Duration::from_secs(2),
    );

    let report = activity.take_report(start + Duration::from_secs(3), no_tail);
    let process = report.process.expect("process tier should be reported");
    assert_eq!(process.cpu_time_delta, Duration::from_millis(750));
    // The last activity was the CPU accrual at t+2, not the report at t+3.
    assert_eq!(report.since_last_activity, Some(Duration::from_secs(1)));
}

#[test]
fn cpu_delta_is_summed_across_the_process_tree() {
    let start = Instant::now();
    let mut activity = local_activity(start);

    activity.apply_sample(
        sample(OUTPUT_A, Some(process_sample(&[(100, 0), (101, 0)]))),
        start + Duration::from_secs(1),
    );
    activity.apply_sample(
        sample(OUTPUT_A, Some(process_sample(&[(100, 200), (101, 300)]))),
        start + Duration::from_secs(2),
    );

    let report = activity.take_report(start + Duration::from_secs(2), no_tail);
    let process = report.process.expect("process tier should be reported");
    assert_eq!(process.cpu_time_delta, Duration::from_millis(500));
    assert_eq!(process.live_process_count, 2);
}

#[test]
fn cpu_delta_accumulates_across_samples_within_one_report() {
    let start = Instant::now();
    let mut activity = local_activity(start);

    for second in 1..=4 {
        activity.apply_sample(
            sample(OUTPUT_A, Some(process_sample(&[(100, second * 100)]))),
            start + Duration::from_secs(second),
        );
    }

    let report = activity.take_report(start + Duration::from_secs(4), no_tail);
    let process = report.process.expect("process tier should be reported");
    // 3 deltas of 100ms; the first sample established the baseline.
    assert_eq!(process.cpu_time_delta, Duration::from_millis(300));
}

#[test]
fn cpu_delta_resets_between_reports() {
    let start = Instant::now();
    let mut activity = local_activity(start);

    activity.apply_sample(
        sample(OUTPUT_A, Some(process_sample(&[(100, 0)]))),
        start + Duration::from_secs(1),
    );
    activity.apply_sample(
        sample(OUTPUT_A, Some(process_sample(&[(100, 500)]))),
        start + Duration::from_secs(2),
    );
    activity.take_report(start + Duration::from_secs(2), no_tail);

    let second = activity.take_report(start + Duration::from_secs(3), no_tail);
    let process = second.process.expect("process tier should be reported");
    assert_eq!(process.cpu_time_delta, Duration::ZERO);
}

#[test]
fn exited_process_tree_reports_no_live_processes_and_no_stale_cpu() {
    let start = Instant::now();
    let mut activity = local_activity(start);

    activity.apply_sample(
        sample(OUTPUT_A, Some(process_sample(&[(100, 1_000)]))),
        start + Duration::from_secs(1),
    );
    activity.apply_sample(
        sample(OUTPUT_A, Some(process_sample(&[(100, 2_000)]))),
        start + Duration::from_secs(2),
    );
    activity.take_report(start + Duration::from_secs(2), no_tail);

    // The command's processes are gone: an empty tree, not a missing sample.
    activity.apply_sample(
        sample(OUTPUT_A, Some(process_sample(&[]))),
        start + Duration::from_secs(3),
    );

    let report = activity.take_report(start + Duration::from_secs(4), no_tail);
    let process = report.process.expect("process tier should be reported");
    assert_eq!(process.live_process_count, 0);
    assert_eq!(process.cpu_time_delta, Duration::ZERO);
}

#[test]
fn a_pid_that_exits_stops_contributing_to_later_deltas() {
    let start = Instant::now();
    let mut activity = local_activity(start);

    activity.apply_sample(
        sample(OUTPUT_A, Some(process_sample(&[(100, 1_000)]))),
        start + Duration::from_secs(1),
    );
    activity.apply_sample(
        sample(OUTPUT_A, Some(process_sample(&[]))),
        start + Duration::from_secs(2),
    );
    activity.take_report(start + Duration::from_secs(2), no_tail);

    // The same pid number reappears, now belonging to an unrelated process with
    // a large lifetime CPU total. It must be treated as newly seen.
    activity.apply_sample(
        sample(OUTPUT_A, Some(process_sample(&[(100, 900_000)]))),
        start + Duration::from_secs(3),
    );

    let report = activity.take_report(start + Duration::from_secs(3), no_tail);
    let process = report.process.expect("process tier should be reported");
    assert_eq!(process.cpu_time_delta, Duration::ZERO);
}

#[test]
fn process_churn_counts_as_activity() {
    let start = Instant::now();
    let mut activity = local_activity(start);

    activity.apply_sample(
        sample(OUTPUT_A, Some(process_sample(&[(100, 500)]))),
        start + Duration::from_secs(1),
    );
    activity.take_report(start + Duration::from_secs(1), no_tail);

    // A build that spawns and reaps compilers may show no CPU delta on any
    // single pid, but the changing set of processes is real progress.
    activity.apply_sample(
        sample(OUTPUT_A, Some(process_sample(&[(100, 500), (101, 0)]))),
        start + Duration::from_secs(5),
    );

    let report = activity.take_report(start + Duration::from_secs(5), no_tail);
    assert_eq!(report.since_last_activity, Some(Duration::ZERO));
}

#[test]
fn file_growth_counts_as_activity() {
    let start = Instant::now();
    let mut activity = activity_with_files(&["/tmp/build.log"], &[Some(0)], start);

    activity.apply_sample(
        file_sample(OUTPUT_A, vec![Some(0)]),
        start + Duration::from_secs(1),
    );
    activity.apply_sample(
        file_sample(OUTPUT_A, vec![Some(4_096)]),
        start + Duration::from_secs(2),
    );

    let report = activity.take_report(start + Duration::from_secs(3), no_tail);
    assert_eq!(report.since_last_activity, Some(Duration::from_secs(1)));
    assert_eq!(report.files.len(), 1);
    assert_eq!(report.files[0].size_bytes, 4_096);
    assert_eq!(report.files[0].size_delta_bytes, 4_096);
}

#[test]
fn file_delta_is_measured_between_reports() {
    let start = Instant::now();
    let mut activity = activity_with_files(&["/tmp/build.log"], &[Some(100)], start);

    activity.apply_sample(
        file_sample(OUTPUT_A, vec![Some(300)]),
        start + Duration::from_secs(1),
    );
    let first = activity.take_report(start + Duration::from_secs(1), no_tail);
    assert_eq!(first.files[0].size_delta_bytes, 200);

    activity.apply_sample(
        file_sample(OUTPUT_A, vec![Some(500)]),
        start + Duration::from_secs(2),
    );
    let second = activity.take_report(start + Duration::from_secs(2), no_tail);
    assert_eq!(second.files[0].size_delta_bytes, 200);
    assert_eq!(second.files[0].size_bytes, 500);
}

#[test]
fn a_file_that_does_not_exist_yet_is_omitted_from_the_report() {
    let start = Instant::now();
    let mut activity = activity_with_files(&["/tmp/build.log"], &[None], start);

    activity.apply_sample(
        file_sample(OUTPUT_A, vec![None]),
        start + Duration::from_secs(1),
    );

    let report = activity.take_report(start + Duration::from_secs(1), no_tail);
    assert!(report.files.is_empty());
    assert_eq!(report.since_last_activity, Some(Duration::from_secs(1)));
}

#[test]
fn a_file_appearing_counts_as_activity() {
    let start = Instant::now();
    let mut activity = activity_with_files(&["/tmp/build.log"], &[None], start);

    activity.apply_sample(
        file_sample(OUTPUT_A, vec![Some(64)]),
        start + Duration::from_secs(3),
    );

    let report = activity.take_report(start + Duration::from_secs(3), no_tail);
    assert_eq!(report.since_last_activity, Some(Duration::ZERO));
    assert_eq!(report.files[0].size_delta_bytes, 64);
}

#[test]
fn a_truncated_file_is_not_treated_as_activity() {
    let start = Instant::now();
    let mut activity = activity_with_files(&["/tmp/build.log"], &[Some(1_000)], start);

    activity.apply_sample(
        file_sample(OUTPUT_A, vec![Some(0)]),
        start + Duration::from_secs(2),
    );

    let report = activity.take_report(start + Duration::from_secs(2), no_tail);
    assert_eq!(report.since_last_activity, Some(Duration::from_secs(2)));
    assert_eq!(report.files[0].size_delta_bytes, -1_000);
}

#[test]
fn the_file_tail_is_read_only_when_a_report_is_built() {
    let start = Instant::now();
    let mut activity = activity_with_files(&["/tmp/build.log"], &[Some(10)], start);

    let reads = Cell::new(0);
    let counting_tail = |_: &Path| {
        reads.set(reads.get() + 1);
        String::new()
    };

    for second in 1..=10 {
        activity.apply_sample(
            file_sample(OUTPUT_A, vec![Some(10 * second)]),
            start + Duration::from_secs(second),
        );
    }
    assert_eq!(reads.get(), 0, "sampling must not read file contents");

    activity.take_report(start + Duration::from_secs(10), counting_tail);
    assert_eq!(reads.get(), 1);
}

#[test]
fn remote_sessions_report_only_the_terminal_tier() {
    let start = Instant::now();
    let mut activity = BlockActivity::from_parts(
        OUTPUT_A,
        vec![PathBuf::from("/tmp/build.log")],
        false, /* signals_available */
        start,
        |_| panic!("file sizes must not be probed when signals are unavailable"),
    );

    activity.apply_sample(
        file_sample(OUTPUT_B, vec![None]),
        start + Duration::from_secs(1),
    );

    let report = activity.take_report(start + Duration::from_secs(1), no_tail);
    assert!(report.signals_unavailable);
    assert!(report.process.is_none());
    assert!(report.files.is_empty());
    // The terminal tier still works, so output changes are still reported.
    assert!(report.output_changed_since_last_read);
}

/// The server reads a missing submessage and an all-zero one differently, so a
/// quiet-but-inspected process tree must still produce a present submessage.
#[test]
fn a_fully_quiet_process_tree_is_still_reported() {
    let start = Instant::now();
    let mut activity = local_activity(start);

    // Inspected repeatedly, and every reading is zero: no CPU, no I/O, and a
    // single sleeping process.
    for second in 1..=5 {
        activity.apply_sample(
            sample(
                OUTPUT_A,
                Some(ProcessSample {
                    per_pid: vec![PidSample {
                        pid: Pid::from_u32(100),
                        cpu_ms: 0,
                        io_write_bytes: 0,
                    }],
                    state: LrcProcessState::Sleeping,
                }),
            ),
            start + Duration::from_secs(second),
        );
    }

    let report = activity.take_report(start + Duration::from_secs(5), no_tail);
    let process = report
        .process
        .expect("an all-zero reading is still a reading");
    assert_eq!(process.cpu_time_delta, Duration::ZERO);
    assert_eq!(process.io_write_bytes_delta, 0);
    assert_eq!(process.live_process_count, 1);
    assert_eq!(process.state, LrcProcessState::Sleeping);
    assert!(!report.signals_unavailable);
}

/// A command is registered when its first snapshot is built, before the sampler
/// has run for it. Reporting the still-zero counters then would describe a
/// healthy command as a process tree with nothing running.
#[test]
fn the_process_tier_is_withheld_until_it_has_actually_been_sampled() {
    let start = Instant::now();
    let mut activity = local_activity(start);

    let first = activity.take_report(start, no_tail);
    assert!(first.process.is_none());
    assert!(first.signals_unavailable);

    activity.apply_sample(
        sample(OUTPUT_A, Some(process_sample(&[(100, 1_000)]))),
        start + Duration::from_secs(1),
    );

    let second = activity.take_report(start + Duration::from_secs(1), no_tail);
    assert!(second.process.is_some());
    assert!(!second.signals_unavailable);
}

/// Zero growth is a real observation about a tracked file, not an absence of
/// one, so it must survive to the wire rather than being filtered out.
#[test]
fn a_tracked_file_that_has_not_grown_is_still_reported() {
    let start = Instant::now();
    let mut activity = activity_with_files(&["/tmp/build.log"], &[Some(4_096)], start);

    activity.apply_sample(
        file_sample(OUTPUT_A, vec![Some(4_096)]),
        start + Duration::from_secs(1),
    );

    let report = activity.take_report(start + Duration::from_secs(1), no_tail);
    assert_eq!(report.files.len(), 1);
    assert_eq!(report.files[0].size_bytes, 4_096);
    assert_eq!(report.files[0].size_delta_bytes, 0);
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

#[test]
fn file_size_reports_regular_files_only() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("build.log");
    std::fs::write(&path, b"hello").expect("write");

    assert_eq!(file_size(&path), Some(5));
    assert_eq!(file_size(&dir.path().join("missing.log")), None);
    assert_eq!(file_size(dir.path()), None);
}

#[cfg(unix)]
#[test]
fn file_size_does_not_follow_symlinks() {
    let dir = tempfile::tempdir().expect("temp dir");
    let target = dir.path().join("secret.txt");
    std::fs::write(&target, b"sensitive").expect("write");
    let link = dir.path().join("build.log");
    std::os::unix::fs::symlink(&target, &link).expect("symlink");

    assert_eq!(file_size(&link), None);
}

#[test]
fn read_tail_returns_the_last_lines_of_a_file() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("build.log");
    let mut file = std::fs::File::create(&path).expect("create");
    for line in 0..100 {
        writeln!(file, "line {line}").expect("write");
    }

    let tail = read_tail(&path).expect("tail");
    let lines: Vec<&str> = tail.lines().collect();
    assert_eq!(lines.len(), MAX_TAIL_LINES);
    assert_eq!(lines.last(), Some(&"line 99"));
}

#[test]
fn read_tail_caps_the_bytes_it_reads() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("build.log");
    // One very long line, so the line cap cannot be what bounds the result.
    std::fs::write(&path, "x".repeat(100_000)).expect("write");

    let tail = read_tail(&path).expect("tail");
    assert!(tail.len() <= super::MAX_TAIL_BYTES as usize);
}

#[test]
fn read_tail_skips_binary_files() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("archive.tar");
    std::fs::write(&path, [0x1f, 0x8b, 0x00, 0x00, 0x42]).expect("write");

    assert_eq!(read_tail(&path), None);
}
