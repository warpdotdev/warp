use std::ffi::OsStr;
use std::path::Path;

use command::r#async::Command;

use super::{
    MountResponse, detect_command, failed_planning_mode, modes_command, mount_command,
    mount_error_diagnostic, run_spacectl_modes,
};
use crate::{CacheScope, CacheSetupError};

fn args(command: &Command) -> Vec<&OsStr> {
    command.get_args().collect()
}

fn assert_disables_toolchain_downloads(command: &Command) {
    let debug = format!("{command:?}");
    assert!(debug.contains(r#"GOTOOLCHAIN="local""#), "{debug}");
    assert!(debug.contains(r#"RUSTUP_AUTO_INSTALL="0""#), "{debug}");
}

#[test]
fn detect_command_has_exact_argv_cache_root_and_repo_cwd() {
    let command = detect_command(Path::new("/cache/repos/key"), Path::new("/work/repo"));
    assert_eq!(command.get_program(), "spacectl");
    assert_eq!(
        args(&command),
        [
            "cache",
            "mount",
            "--detect=*",
            "--dry_run=true",
            "--cache_root",
            "/cache/repos/key",
            "-o",
            "json",
        ]
    );
    assert_eq!(command.get_current_dir(), Some(Path::new("/work/repo")));
    assert_disables_toolchain_downloads(&command);
}

#[test]
fn mount_command_has_exact_explicit_modes_dry_run_false_root_and_cwd() {
    let command = mount_command(
        Path::new("/cache/shared"),
        Path::new("/tmp/scratch"),
        &["cargo".to_owned(), "go".to_owned()],
    );
    assert_eq!(command.get_program(), "spacectl");
    assert_eq!(
        args(&command),
        [
            "cache",
            "mount",
            "--mode=cargo,go",
            "--dry_run=false",
            "--cache_root",
            "/cache/shared",
            "-o",
            "json",
        ]
    );
    assert_eq!(command.get_current_dir(), Some(Path::new("/tmp/scratch")));
    assert_disables_toolchain_downloads(&command);
}

#[test]
fn modes_command_has_exact_argv_and_repo_cwd() {
    let command = modes_command(Path::new("/work/repo/sub"));
    assert_eq!(command.get_program(), "spacectl");
    assert_eq!(args(&command), ["cache", "modes", "-o", "json"]);
    assert_eq!(command.get_current_dir(), Some(Path::new("/work/repo/sub")));
    assert_disables_toolchain_downloads(&command);
}

#[test]
fn mode_query_reports_only_detected_modes() {
    let report = futures::executor::block_on(run_spacectl_modes(
        CacheScope::Global,
        "repos/key/nested/id".into(),
        Path::new("/work/repo/sub"),
        "id",
        &|_| {
            futures::future::ready(Ok(br#"{
                "modes": {
                    "rust": {"detected": true},
                    "apt": {"detected": true},
                    "go": {"detected": false},
                    "future": {}
                }
            }"#
            .to_vec()))
        },
    ));
    assert!(report.succeeded());
    assert_eq!(report.modes, ["apt", "rust"]);
    let response = report.response.unwrap();
    assert_eq!(response.input.modes, ["apt", "rust"]);
    assert!(response.output.mounts.is_empty());

    let failed = futures::executor::block_on(run_spacectl_modes(
        CacheScope::Global,
        "repos/key/nested/id".into(),
        Path::new("/work/repo/sub"),
        "id",
        &|_| futures::future::ready(Ok(b"not-json".to_vec())),
    ));
    assert_eq!(failed.error, Some(CacheSetupError::JsonParseFailed));
}

#[test]
fn mount_response_deserializes_spacectl_output() {
    let response = serde_json::from_slice::<MountResponse>(
        br#"{
            "input": {"modes": ["cargo", "go"], "future": true},
            "output": {
                "add_envs": {"GOCACHE": "/cache/go"},
                "mounts": [
                    {"mode": "cargo", "cache_hit": true, "cache_path": "/cache/repos/key/target", "mount_path": "/work/repo/target"},
                    {"mode": "go", "cache_hit": false}
                ],
                "disk_usage": {"total": "20G", "used": "4G"},
                "unknown": "ignored"
            },
            "unknown": []
        }"#,
    )
    .unwrap();
    assert_eq!(response.input.modes, ["cargo", "go"]);
    assert_eq!(response.output.add_envs["GOCACHE"], "/cache/go");
    assert!(response.output.mounts[0].cache_hit);
    assert_eq!(
        response.output.mounts[0].cache_path,
        Path::new("/cache/repos/key/target")
    );
    assert_eq!(
        response.output.mounts[0].mount_path,
        Path::new("/work/repo/target")
    );
    assert!(!response.output.mounts[1].cache_hit);
    assert_eq!(response.output.mounts[1].cache_path, Path::new(""));
    let disk_usage = response.output.disk_usage.unwrap();
    assert_eq!(disk_usage.total, "20G");
    assert_eq!(disk_usage.used, "4G");

    let response = serde_json::from_slice::<MountResponse>(br#"{"input":{},"output":{}}"#).unwrap();
    assert_eq!(response.output.disk_usage, None);
}

fn nonzero_exit(stdout: &str, stderr: &str) -> CacheSetupError {
    CacheSetupError::NonzeroExit {
        exit_code: Some(1),
        stdout: stdout.to_owned(),
        stderr: stderr.to_owned(),
    }
}

#[test]
fn failed_planning_mode_reads_spacectl_json_and_log_errors() {
    let modes = [
        "apt".to_owned(),
        "go".to_owned(),
        "golangci-lint".to_owned(),
    ];
    assert_eq!(
        failed_planning_mode(
            &nonzero_exit(
                r#"{"error": true, "message": "planning golangci-lint: golangci-lint cache status: exit status 3"}"#,
                "",
            ),
            &modes,
        ),
        Some("golangci-lint".to_owned())
    );
    assert_eq!(
        failed_planning_mode(
            &nonzero_exit("", "ERROR planning go: go env: exit status 1"),
            &modes
        ),
        Some("go".to_owned())
    );
}

#[test]
fn failed_planning_mode_ignores_unattributable_errors() {
    let modes = ["apt".to_owned(), "go".to_owned()];
    for error in [
        nonzero_exit(
            r#"{"error": true, "message": "mounting mode path \"/root/go\": permission denied"}"#,
            "",
        ),
        nonzero_exit(
            r#"{"error": true, "message": "planning npm: npm missing"}"#,
            "",
        ),
        nonzero_exit("planning go", ""),
        CacheSetupError::Timeout,
    ] {
        assert_eq!(failed_planning_mode(&error, &modes), None, "{error}");
    }
}

#[test]
fn mount_error_diagnostic_prefers_spacectl_stderr() {
    let error = CacheSetupError::NonzeroExit {
        exit_code: Some(17),
        stdout: String::new(),
        stderr: "mount failed: permission denied".to_owned(),
    };
    assert_eq!(
        mount_error_diagnostic(&error),
        "mount failed: permission denied"
    );
}

#[test]
fn mount_error_diagnostic_includes_exit_code_without_stderr() {
    let error = CacheSetupError::NonzeroExit {
        exit_code: Some(17),
        stdout: String::new(),
        stderr: String::new(),
    };

    assert_eq!(
        mount_error_diagnostic(&error),
        "spacectl exited unsuccessfully with exit code 17"
    );
}
