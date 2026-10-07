use command::blocking::Command;

/// Verifies a worker invocation exits through worker dispatch without starting the TUI frontend.
#[test]
fn dispatches_worker_invocation_instead_of_tui_frontend() {
    let empty_file =
        std::env::temp_dir().join(format!("warp-tui-worker-dispatch-{}", std::process::id()));
    std::fs::write(&empty_file, []).expect("failed to create empty worker-dispatch fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_warp-tui-oss"))
        .arg("ripgrep-search")
        .arg("__warp_tui_worker_dispatch_probe__")
        .arg(&empty_file)
        .output()
        .expect("failed to invoke warp-tui worker");
    let _ = std::fs::remove_file(empty_file);

    assert!(
        output.status.success(),
        "worker invocation failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains("Welcome to Warp TUI"),
        "worker invocation started the TUI frontend"
    );
}
