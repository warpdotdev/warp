use std::time::Duration;

use super::{OutputError, collect_output_with_timeout};

#[cfg(windows)]
#[test]
fn timeout_terminates_child_process() {
    futures_lite::future::block_on(async {
        let mut command = async_process::Command::new("powershell.exe");
        command.args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Start-Sleep -Seconds 300",
        ]);
        command
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .reap_on_drop(true);
        let child = command.spawn().expect("spawn timeout child");

        let error = collect_output_with_timeout(child, Duration::from_millis(500))
            .await
            .expect_err("command should time out");
        let OutputError::TimedOut { process_id, .. } = error else {
            panic!("expected timeout, got {error:?}");
        };

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let output = std::process::Command::new("tasklist.exe")
                .args(["/FI", &format!("PID eq {process_id}"), "/NH", "/FO", "CSV"])
                .output()
                .expect("inspect timed-out process");
            let stdout = String::from_utf8_lossy(&output.stdout);
            if !stdout.contains(&process_id.to_string()) {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "timed-out process {process_id} was not terminated"
            );
            async_io::Timer::after(Duration::from_millis(25)).await;
        }
    });
}
