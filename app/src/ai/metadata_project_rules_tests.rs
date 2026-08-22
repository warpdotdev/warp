use async_io::block_on;
use tempfile::TempDir;
use warp_util::local_or_remote_path::LocalOrRemotePath;

use super::{MAX_PROJECT_RULE_FILE_BYTES, read_local_rule_contents};

fn write_file(dir: &TempDir, name: &str, contents: &[u8]) -> std::path::PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, contents).expect("write temp file");
    path
}

#[test]
fn reads_local_rule_files() {
    let dir = TempDir::new().expect("create tempdir");
    let path = write_file(&dir, "WARP.md", b"# Rule\nBe concise.");

    let contents = block_on(read_local_rule_contents(vec![LocalOrRemotePath::Local(
        path.clone(),
    )]))
    .expect("should succeed");

    assert_eq!(
        contents,
        vec![(
            LocalOrRemotePath::Local(path),
            "# Rule\nBe concise.".to_string()
        )]
    );
}

#[test]
fn skips_oversized_rule_file_without_failing_the_batch() {
    // Regression for APP-4801: an oversized rule file must be skipped (matching the existing
    // "log and skip" behavior for any unreadable rule file), not read wholesale into memory.
    let dir = TempDir::new().expect("create tempdir");
    let oversized_path = write_file(
        &dir,
        "oversized.md",
        &vec![b'a'; (MAX_PROJECT_RULE_FILE_BYTES + 1) as usize],
    );
    let ok_path = write_file(&dir, "ok.md", b"fine");

    let contents = block_on(read_local_rule_contents(vec![
        LocalOrRemotePath::Local(oversized_path),
        LocalOrRemotePath::Local(ok_path.clone()),
    ]))
    .expect("should succeed overall");

    assert_eq!(
        contents,
        vec![(LocalOrRemotePath::Local(ok_path), "fine".to_string())]
    );
}
