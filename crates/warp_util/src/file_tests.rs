use tempfile::TempDir;
use warpui_core::r#async::block_on;

use super::{FileLoadError, read_capped, read_to_string_capped};

fn write_file(dir: &TempDir, name: &str, contents: &[u8]) -> std::path::PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, contents).expect("write temp file");
    path
}

#[test]
fn read_to_string_capped_reads_file_under_limit() {
    let dir = TempDir::new().expect("create tempdir");
    let path = write_file(&dir, "small.txt", b"hello world");

    let contents = block_on(read_to_string_capped(&path, 1024)).expect("should read file");
    assert_eq!(contents, "hello world");
}

#[test]
fn read_to_string_capped_rejects_file_over_limit() {
    let dir = TempDir::new().expect("create tempdir");
    let path = write_file(&dir, "big.txt", &vec![b'a'; 2048]);

    let error = block_on(read_to_string_capped(&path, 1024)).expect_err("should reject");
    assert!(
        matches!(
            error,
            FileLoadError::TooLarge {
                size_estimate: Some(2048),
                limit_bytes: 1024
            }
        ),
        "expected a TooLarge estimate of 2048 over a 1024 limit, got {error:?}"
    );
}

#[test]
fn read_to_string_capped_accepts_a_file_exactly_at_the_limit() {
    // The cap rejects content *exceeding* `max_bytes`; a file of exactly `max_bytes` is the
    // boundary and must be accepted, not off-by-one rejected.
    let dir = TempDir::new().expect("create tempdir");
    let path = write_file(&dir, "exact.txt", &vec![b'a'; 1024]);

    let contents =
        block_on(read_to_string_capped(&path, 1024)).expect("exact-limit file should be accepted");
    assert_eq!(contents.len(), 1024);
}

#[test]
fn read_to_string_capped_missing_file_reports_io_error() {
    let dir = TempDir::new().expect("create tempdir");
    let missing = dir.path().join("does-not-exist.txt");

    let error = block_on(read_to_string_capped(&missing, 1024)).expect_err("should fail");
    assert!(
        matches!(error, FileLoadError::IOError(ref err) if err.kind() == std::io::ErrorKind::NotFound),
        "expected a NotFound IOError, got {error:?}"
    );
}

#[cfg(unix)]
#[test]
fn read_capped_rejects_unbounded_data_from_a_device_reporting_zero_length() {
    // `/dev/zero` is a character device that yields unbounded data while `stat` reports a
    // length of 0. A stat-then-reread-by-path implementation would see `0 <= max_bytes`, pass
    // the check, and then hand an effectively unbounded stream to an unbounded read. The cap
    // must be enforced by the read itself, independent of what `stat` reports.
    let metadata_len = std::fs::metadata("/dev/zero")
        .expect("stat /dev/zero")
        .len();
    assert_eq!(metadata_len, 0, "/dev/zero should report a length of 0");

    let error = block_on(read_capped(std::path::Path::new("/dev/zero"), 1024))
        .expect_err("should reject unbounded device data");
    // `/dev/zero` isn't a regular file, so its (lying) `stat` length must not be surfaced as a
    // size estimate at all.
    assert!(
        matches!(
            error,
            FileLoadError::TooLarge {
                size_estimate: None,
                limit_bytes: 1024
            }
        ),
        "expected no size estimate over a 1024 limit, got {error:?}"
    );
}

#[cfg(unix)]
#[test]
fn read_capped_rejects_content_that_grows_past_the_cap_while_being_read() {
    // A FIFO's content isn't fixed at open time the way a regular file's is -- it arrives (and
    // can keep growing past the cap) while the read is in progress, the same class of hazard as
    // a file being mutated or atomically replaced between a `stat` and a later, separate read.
    // `stat` also reports a fixed length of 0 for a FIFO, doubling as another
    // zero-length-metadata case.
    let dir = TempDir::new().expect("create tempdir");
    let fifo_path = dir.path().join("fifo");
    let status = command::blocking::Command::new("mkfifo")
        .arg(&fifo_path)
        .status()
        .expect("mkfifo should run");
    assert!(status.success(), "mkfifo should succeed");

    let metadata_len = std::fs::metadata(&fifo_path).expect("stat fifo").len();
    assert_eq!(metadata_len, 0, "a FIFO should report a length of 0");

    let writer_path = fifo_path.clone();
    let writer = std::thread::spawn(move || {
        use std::io::Write as _;
        // Opens block until a reader connects; `read_capped` provides the reader below.
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .open(&writer_path)
            .expect("open fifo for writing");
        let chunk = vec![b'a'; 4096];
        // Keep writing well past the cap; a correct reader stops well before this loop ends,
        // and the loop exits once the reader disconnects and the write returns an error.
        for _ in 0..256 {
            if file.write_all(&chunk).is_err() {
                break;
            }
        }
    });

    let error = block_on(read_capped(&fifo_path, 1024)).expect_err("should reject");
    // As above: a FIFO isn't a regular file, so its (lying) `stat` length must not be surfaced
    // as a size estimate.
    assert!(
        matches!(
            error,
            FileLoadError::TooLarge {
                size_estimate: None,
                limit_bytes: 1024
            }
        ),
        "expected no size estimate over a 1024 limit, got {error:?}"
    );

    writer.join().expect("writer thread should not panic");
}
