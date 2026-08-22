use std::io::ErrorKind;

use futures_lite::future::block_on;
use tempfile::TempDir;

use super::{read_capped, read_to_string_capped};

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
    // Regression for APP-4801: reading a file whose on-disk size exceeds the cap must not
    // attempt to reserve a String of that size; it should be rejected up front.
    let dir = TempDir::new().expect("create tempdir");
    let path = write_file(&dir, "big.txt", &vec![b'a'; 2048]);

    let error = block_on(read_to_string_capped(&path, 1024)).expect_err("should reject");
    assert_ne!(error.kind(), ErrorKind::NotFound);
    assert!(error.to_string().contains("too large"), "got: {error}");
}

#[test]
fn read_to_string_capped_missing_file_reports_not_found() {
    let dir = TempDir::new().expect("create tempdir");
    let missing = dir.path().join("does-not-exist.txt");

    let error = block_on(read_to_string_capped(&missing, 1024)).expect_err("should fail");
    assert_eq!(error.kind(), ErrorKind::NotFound);
}

#[test]
fn read_capped_reads_file_under_limit() {
    let dir = TempDir::new().expect("create tempdir");
    let path = write_file(&dir, "small.bin", &[1, 2, 3, 4]);

    let contents = block_on(read_capped(&path, 1024)).expect("should read file");
    assert_eq!(contents, vec![1, 2, 3, 4]);
}

#[test]
fn read_capped_rejects_file_over_limit() {
    let dir = TempDir::new().expect("create tempdir");
    let path = write_file(&dir, "big.bin", &vec![0u8; 2048]);

    let error = block_on(read_capped(&path, 1024)).expect_err("should reject");
    assert_ne!(error.kind(), ErrorKind::NotFound);
    assert!(error.to_string().contains("too large"), "got: {error}");
}

#[cfg(unix)]
#[test]
fn read_capped_rejects_unbounded_data_from_a_device_reporting_zero_length() {
    // Regression for APP-4801 review: `/dev/zero` is a character device that yields
    // unbounded data while `stat` reports a length of 0. A stat-then-reread-by-path
    // implementation would see `0 <= max_bytes`, pass the check, and then hand an effectively
    // unbounded stream to `async_fs::read`/`read_to_string`'s unbounded reservation. The cap
    // must be enforced by the read itself (via `AsyncReadExt::take`), independent of what
    // `stat` reports.
    let metadata_len = std::fs::metadata("/dev/zero")
        .expect("stat /dev/zero")
        .len();
    assert_eq!(metadata_len, 0, "/dev/zero should report a length of 0");

    let error = block_on(read_capped(std::path::Path::new("/dev/zero"), 1024))
        .expect_err("should reject unbounded device data");
    assert!(error.to_string().contains("too large"), "got: {error}");
}

#[cfg(unix)]
#[test]
fn read_capped_rejects_content_that_grows_past_the_cap_while_being_read() {
    // Regression for APP-4801 review: a FIFO's content isn't fixed at open time the way a
    // regular file's is -- it arrives (and can keep growing past the cap) while the read is
    // in progress, the same class of hazard as a file being mutated or atomically replaced
    // between a `stat` and a later, separate read. `stat` also reports a fixed length of 0
    // for a FIFO, so this doubles as another zero-length-metadata case.
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
    assert!(error.to_string().contains("too large"), "got: {error}");

    writer.join().expect("writer thread should not panic");
}
