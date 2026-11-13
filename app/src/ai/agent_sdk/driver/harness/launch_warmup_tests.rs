use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use tempfile::tempdir;
use warpui::r#async::executor::Background;

use super::{LaunchFileWarmup, WarmupStats, read_files};
use crate::ai::agent_sdk::setup_observability::SetupClientEventReporter;
use crate::server::server_api::ai::MockAIClient;

fn write_file(path: &Path, len: usize) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, vec![b'x'; len]).unwrap();
}

#[test]
fn reads_every_regular_file_under_each_root() {
    let dir = tempdir().unwrap();
    let bin = dir.path().join("bin");
    let lib = dir.path().join("lib");
    write_file(&bin.join("node"), 3);
    write_file(&lib.join("node_modules/adapter/index.js"), 5);
    write_file(&lib.join("node_modules/adapter/vendor/cli"), 7);

    let stats = read_files(
        &[bin, lib, dir.path().join("missing")],
        &AtomicBool::new(false),
        u64::MAX,
    );

    assert_eq!(
        stats,
        WarmupStats {
            files: 3,
            bytes: 15
        }
    );
}

#[cfg(unix)]
#[test]
fn does_not_follow_symlinks() {
    let dir = tempdir().unwrap();
    let outside = dir.path().join("outside");
    write_file(&outside.join("large"), 100);
    let root = dir.path().join("root");
    write_file(&root.join("real"), 4);
    std::os::unix::fs::symlink(outside.join("large"), root.join("file-link")).unwrap();
    std::os::unix::fs::symlink(&outside, root.join("dir-link")).unwrap();

    let stats = read_files(&[root], &AtomicBool::new(false), u64::MAX);

    assert_eq!(stats, WarmupStats { files: 1, bytes: 4 });
}

#[test]
fn a_stopped_warmup_reads_nothing() {
    let dir = tempdir().unwrap();
    write_file(&dir.path().join("file"), 10);

    let stats = read_files(&[dir.path().to_owned()], &AtomicBool::new(true), u64::MAX);

    assert_eq!(stats, WarmupStats::default());
}

#[test]
fn stops_at_the_byte_budget() {
    let dir = tempdir().unwrap();
    for name in ["a", "b", "c"] {
        write_file(&dir.path().join(name), 10);
    }

    let stats = read_files(&[dir.path().to_owned()], &AtomicBool::new(false), 15);

    assert_eq!(
        stats,
        WarmupStats {
            files: 2,
            bytes: 15
        }
    );
}

#[test]
fn starts_only_when_a_root_exists() {
    let dir = tempdir().unwrap();
    write_file(&dir.path().join("file"), 10);
    let existing = vec![dir.path().to_owned()];
    let missing = vec![dir.path().join("missing")];
    let mut client = MockAIClient::new();
    client.expect_post_agent_run_client_event().times(0);
    let background = Arc::new(Background::new(1, |_| "launch-warmup-test".to_string()));
    let setup_events = SetupClientEventReporter::standby(Arc::new(client), background);

    assert!(LaunchFileWarmup::start(missing, &setup_events).is_none());
    assert!(LaunchFileWarmup::start(Vec::new(), &setup_events).is_none());
    let warmup = LaunchFileWarmup::start(existing, &setup_events).expect("a warm-up starts");
    warmup.stop();
}
