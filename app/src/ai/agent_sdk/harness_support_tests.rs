use warp_cli::harness_support::ReportShutdownArgs;

#[cfg(target_os = "linux")]
use super::kernel_log_commands;
use super::{
    OomDetectionResult, apply_oom_classification, detect_oom_shutdown, oom_kill_line_matches_pid,
};

#[test]
fn matches_out_of_memory_killed_process_line_for_pid() {
    assert!(oom_kill_line_matches_pid(
        "[123.456] Out of memory: Killed process 4242 (warp) total-vm:1000kB",
        4242
    ));
}

#[test]
fn matches_memory_cgroup_killed_process_line_for_pid() {
    assert!(oom_kill_line_matches_pid(
        "[123.456] Memory cgroup out of memory: Killed process 4242 (oz) total-vm:1000kB",
        4242
    ));
}

#[test]
fn matches_compact_oom_kill_pid_field() {
    assert!(oom_kill_line_matches_pid(
        "[123.456] oom-kill:constraint=CONSTRAINT_MEMCG,task=oz,pid=4242,uid=1000",
        4242
    ));
}

#[test]
fn matches_spaced_oom_kill_pid_field() {
    assert!(oom_kill_line_matches_pid(
        "[123.456] oom-kill: task=oz-dev, pid=4242, uid=1000",
        4242
    ));
}

#[test]
fn rejects_killed_process_line_without_oom_marker() {
    assert!(!oom_kill_line_matches_pid(
        "[123.456] Killed process 4242 (warp) with signal SIGKILL",
        4242
    ));
}

#[test]
fn rejects_generic_pid_field() {
    assert!(!oom_kill_line_matches_pid(
        "[123.456] agent pid=4242 exited",
        4242
    ));
}

#[test]
fn rejects_oom_killer_invocation_without_victim() {
    assert!(!oom_kill_line_matches_pid(
        "[123.456] invoked oom-killer: gfp_mask=0x0",
        4242
    ));
}

#[test]
fn rejects_killed_process_line_for_longer_pid() {
    assert!(!oom_kill_line_matches_pid(
        "[123.456] Out of memory: Killed process 42420 (warp) total-vm:1000kB",
        4242
    ));
}

#[test]
fn rejects_oom_kill_field_for_longer_pid() {
    assert!(!oom_kill_line_matches_pid(
        "[123.456] oom-kill:constraint=CONSTRAINT_MEMCG,task=oz,pid=42420,uid=1000",
        4242
    ));
}

#[test]
fn rejects_oom_kill_pid_with_non_delimiter_suffix() {
    assert!(!oom_kill_line_matches_pid(
        "[123.456] oom-kill:constraint=CONSTRAINT_MEMCG,task=oz,pid=4242foo,uid=1000",
        4242
    ));
}

#[test]
fn rejects_similarly_named_oom_kill_field() {
    assert!(!oom_kill_line_matches_pid(
        "[123.456] oom-kill:constraint=CONSTRAINT_MEMCG,task=oz,cpid=4242,uid=1000",
        4242
    ));
}

#[test]
fn oom_classification_populates_an_absent_error_pair() {
    let mut args = ReportShutdownArgs {
        error_category: None,
        error_message: None,
        pid: Some(4242),
        exit_code: Some(137),
    };

    apply_oom_classification(
        &mut args,
        OomDetectionResult {
            exit_status: true,
            kernel_logs: false,
        },
    );

    assert_eq!(args.error_category.as_deref(), Some("oom"));
    assert_eq!(
        args.error_message.as_deref(),
        Some("The agent sandbox ran out of memory.")
    );
}

#[test]
fn oom_classification_replaces_a_malformed_error_pair() {
    let mut args = ReportShutdownArgs {
        error_category: Some("process_exit".to_string()),
        error_message: None,
        pid: Some(4242),
        exit_code: Some(137),
    };

    apply_oom_classification(
        &mut args,
        OomDetectionResult {
            exit_status: true,
            kernel_logs: false,
        },
    );

    assert_eq!(args.error_category.as_deref(), Some("oom"));
    assert_eq!(
        args.error_message.as_deref(),
        Some("The agent sandbox ran out of memory.")
    );
}

#[test]
fn oom_classification_replaces_a_different_error_pair() {
    let mut args = ReportShutdownArgs {
        error_category: Some("process_exit".to_string()),
        error_message: Some("agent exited".to_string()),
        pid: Some(4242),
        exit_code: Some(137),
    };

    apply_oom_classification(
        &mut args,
        OomDetectionResult {
            exit_status: false,
            kernel_logs: true,
        },
    );

    assert_eq!(args.error_category.as_deref(), Some("oom"));
    assert_eq!(
        args.error_message.as_deref(),
        Some("The agent sandbox ran out of memory.")
    );
}

#[cfg(target_os = "linux")]
#[test]
fn kernel_log_commands_filter_to_oom_messages() {
    assert_eq!(
        kernel_log_commands(),
        [
            (
                "dmesg",
                &["--level=info,warn,err,crit,alert,emerg", "--color=never"][..]
            ),
            (
                "journalctl",
                &[
                    "-k",
                    "--priority=0..6",
                    "--no-pager",
                    "--grep=(?i)(oom-kill:|out of memory: killed process)"
                ][..]
            )
        ]
    );
}

#[test]
fn skips_oom_detection_for_clean_exit() {
    assert!(detect_oom_shutdown(Some(0), Some(4242)).is_none());
}

#[test]
fn skips_oom_detection_without_exit_status() {
    assert!(detect_oom_shutdown(None, Some(4242)).is_none());
}
