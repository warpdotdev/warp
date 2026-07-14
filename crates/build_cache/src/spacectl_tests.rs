use std::ffi::OsStr;
use std::path::Path;

use super::{detect_command, mount_command, parse_mount_response};

fn args(command: &std::process::Command) -> Vec<&OsStr> {
    command.get_args().collect()
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
}

#[test]
fn parse_mount_response_reads_modes_mounts_add_envs_and_optional_disk_usage() {
    let response = parse_mount_response(
        br#"{
            "input": {"modes": ["cargo", "go"], "future": true},
            "output": {
                "add_envs": {"GOCACHE": "/cache/go"},
                "mounts": [
                    {"mode": "cargo", "cache_hit": true, "cache_path": "ignored", "mount_path": "ignored"},
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
    assert!(!response.output.mounts[1].cache_hit);
    let disk_usage = response.output.disk_usage.unwrap();
    assert_eq!(disk_usage.total, "20G");
    assert_eq!(disk_usage.used, "4G");

    let response = parse_mount_response(br#"{"input":{},"output":{}}"#).unwrap();
    assert_eq!(response.output.disk_usage, None);
}
