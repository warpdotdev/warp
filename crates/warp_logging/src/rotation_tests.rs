use std::io::Read;

use super::*;

fn read(path: &Path) -> String {
    let mut s = String::new();
    File::open(path).unwrap().read_to_string(&mut s).unwrap();
    s
}

#[test]
fn writes_below_threshold_do_not_rotate() {
    let tmp = tempfile::tempdir().unwrap();
    let mut w = RotatingFileWriter::open(tmp.path(), "warp.log", 1024, 3).unwrap();
    w.write_all(b"hello world\n").unwrap();
    w.flush().unwrap();
    assert_eq!(read(&tmp.path().join("warp.log")), "hello world\n");
    assert!(!tmp.path().join("warp.log.in_session.0").exists());
}

#[test]
fn crossing_threshold_rotates_to_in_session_zero() {
    let tmp = tempfile::tempdir().unwrap();
    let mut w = RotatingFileWriter::open(tmp.path(), "warp.log", 16, 3).unwrap();
    w.write_all(b"first batch ").unwrap(); // 12 bytes
    w.write_all(b"more content").unwrap(); // crosses 16 → rotate before write
    w.flush().unwrap();
    assert_eq!(
        read(&tmp.path().join("warp.log.in_session.0")),
        "first batch "
    );
    assert_eq!(read(&tmp.path().join("warp.log")), "more content");
}

#[test]
fn repeated_rotations_shift_slots_up() {
    let tmp = tempfile::tempdir().unwrap();
    let mut w = RotatingFileWriter::open(tmp.path(), "warp.log", 8, 3).unwrap();
    // Each write of ~10 bytes crosses the 8-byte threshold and triggers
    // a rotation before the write lands. So the *previous* batch becomes
    // .in_session.0 each time, shifting older slots up.
    w.write_all(b"aaaaaaaaa\n").unwrap(); // first write — no prior content, becomes active
    w.write_all(b"bbbbbbbbb\n").unwrap(); // rotates "aaa..." -> .0
    w.write_all(b"ccccccccc\n").unwrap(); // rotates "bbb..." -> .0, "aaa..." -> .1
    w.write_all(b"ddddddddd\n").unwrap(); // rotates "ccc..." -> .0, "bbb..." -> .1, "aaa..." -> .2
    w.flush().unwrap();
    assert_eq!(read(&tmp.path().join("warp.log")), "ddddddddd\n");
    assert_eq!(
        read(&tmp.path().join("warp.log.in_session.0")),
        "ccccccccc\n"
    );
    assert_eq!(
        read(&tmp.path().join("warp.log.in_session.1")),
        "bbbbbbbbb\n"
    );
    assert_eq!(
        read(&tmp.path().join("warp.log.in_session.2")),
        "aaaaaaaaa\n"
    );
}

#[test]
fn overflow_drops_the_oldest_slot() {
    let tmp = tempfile::tempdir().unwrap();
    let mut w = RotatingFileWriter::open(tmp.path(), "warp.log", 8, 2).unwrap();
    w.write_all(b"aaaaaaaaa\n").unwrap();
    w.write_all(b"bbbbbbbbb\n").unwrap(); // rotates -> .0 = aaa
    w.write_all(b"ccccccccc\n").unwrap(); // rotates -> .0 = bbb, .1 = aaa
    w.write_all(b"ddddddddd\n").unwrap(); // rotates -> .0 = ccc, .1 = bbb, aaa dropped
    w.flush().unwrap();
    assert_eq!(read(&tmp.path().join("warp.log")), "ddddddddd\n");
    assert_eq!(
        read(&tmp.path().join("warp.log.in_session.0")),
        "ccccccccc\n"
    );
    assert_eq!(
        read(&tmp.path().join("warp.log.in_session.1")),
        "bbbbbbbbb\n"
    );
    assert!(!tmp.path().join("warp.log.in_session.2").exists());
}

#[test]
fn oversized_first_write_does_not_promote_empty_file_to_in_session_zero() {
    // Regression for the Oz nit on #11000: when the very first write
    // exceeds `max_bytes`, the rotator must NOT rename an empty active
    // file into `.in_session.0` — that would burn a retention slot
    // before any useful data exists. The oversized payload stays in the
    // active file and is promoted on the next real rotation.
    let tmp = tempfile::tempdir().unwrap();
    let mut w = RotatingFileWriter::open(tmp.path(), "warp.log", 8, 3).unwrap();
    w.write_all(b"oversized first payload\n").unwrap(); // 24 bytes, > 8
    w.flush().unwrap();
    assert!(!tmp.path().join("warp.log.in_session.0").exists());
    assert_eq!(
        read(&tmp.path().join("warp.log")),
        "oversized first payload\n"
    );

    // On the next write the (now-populated) active file rotates
    // normally and the oversized payload becomes `.in_session.0`.
    w.write_all(b"next\n").unwrap();
    w.flush().unwrap();
    assert_eq!(
        read(&tmp.path().join("warp.log.in_session.0")),
        "oversized first payload\n"
    );
    assert_eq!(read(&tmp.path().join("warp.log")), "next\n");
}

#[test]
fn zero_max_rotation_truncates_in_place_without_sidecar() {
    let tmp = tempfile::tempdir().unwrap();
    let mut w = RotatingFileWriter::open(tmp.path(), "warp.log", 8, 0).unwrap();
    // First write skips rotation since the active file is still empty;
    // both batches land sequentially, and the second crosses the
    // threshold so the truncate-in-place branch fires.
    w.write_all(b"first batch\n").unwrap();
    w.write_all(b"second batch\n").unwrap();
    w.flush().unwrap();
    // With max_rotation=0, no .in_session.N file should ever exist.
    assert!(!tmp.path().join("warp.log.in_session.0").exists());
    // The active file holds only the most recent batch (older content
    // truncated since slot 0 is not retained).
    assert_eq!(read(&tmp.path().join("warp.log")), "second batch\n");
}

#[test]
fn zero_max_bytes_disables_rotation_entirely() {
    let tmp = tempfile::tempdir().unwrap();
    let mut w = RotatingFileWriter::open(tmp.path(), "warp.log", 0, 3).unwrap();
    for _ in 0..100 {
        w.write_all(b"line\n").unwrap();
    }
    w.flush().unwrap();
    assert!(!tmp.path().join("warp.log.in_session.0").exists());
    assert_eq!(read(&tmp.path().join("warp.log")).len(), 500);
}
