use super::{detect_oom_shutdown, oom_kill_line_matches_pid, oom_shutdown_message};

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
fn classifies_oom_from_exit_status() {
    assert_eq!(
        oom_shutdown_message(true, false).as_deref(),
        Some("agent process was OOM-killed (exit status 137)")
    );
}

#[test]
fn classifies_oom_from_kernel_evidence() {
    assert_eq!(
        oom_shutdown_message(false, true).as_deref(),
        Some("agent process was OOM-killed (kernel evidence)")
    );
}

#[test]
fn classifies_oom_from_both_sources() {
    assert_eq!(
        oom_shutdown_message(true, true).as_deref(),
        Some("agent process was OOM-killed (exit status 137 and kernel evidence)")
    );
}

#[test]
fn does_not_classify_oom_without_a_signal() {
    assert!(oom_shutdown_message(false, false).is_none());
}

#[test]
fn skips_oom_detection_for_clean_exit() {
    assert!(detect_oom_shutdown(Some(0), Some(4242)).is_none());
}

#[test]
fn skips_oom_detection_without_exit_status() {
    assert!(detect_oom_shutdown(None, Some(4242)).is_none());
}
