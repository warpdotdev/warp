use std::path::Path;

use command::blocking::Command;

use super::*;

/// Bytes that are intentionally not valid UTF-8 (PNG magic followed by
/// continuation bytes), so a lossy string decode would corrupt them.
const BINARY_BYTES: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0xFF, 0xFE, 0x00, 0x80, 0xC3, 0x28,
];

fn git(repo: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(repo)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .status()
        .expect("run git");
    assert!(status.success(), "git {args:?} failed");
}

#[test]
fn run_git_command_bytes_round_trips_binary_blobs() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let repo = dir.path();

    git(repo, &["init", "--quiet"]);
    std::fs::write(repo.join("img.png"), BINARY_BYTES).expect("write binary file");
    git(repo, &["add", "img.png"]);
    git(
        repo,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--quiet",
            "-m",
            "add binary",
        ],
    );

    let bytes =
        futures_lite::future::block_on(run_git_command_bytes(repo, &["show", "HEAD:img.png"]))
            .expect("git show should succeed");
    assert_eq!(bytes, BINARY_BYTES);

    // The string variant lossily decodes the same blob, corrupting it — the
    // reason image bytes must go through `run_git_command_bytes`.
    let lossy = futures_lite::future::block_on(run_git_command(repo, &["show", "HEAD:img.png"]))
        .expect("git show should succeed");
    assert_ne!(lossy.as_bytes(), BINARY_BYTES);
}

#[test]
fn run_git_command_bytes_errors_on_missing_blob() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let repo = dir.path();
    git(repo, &["init", "--quiet"]);

    let result =
        futures_lite::future::block_on(run_git_command_bytes(repo, &["show", "HEAD:missing.png"]));
    assert!(result.is_err());
}
