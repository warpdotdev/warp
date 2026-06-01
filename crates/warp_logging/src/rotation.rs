//! In-session size-based rotation for `warp.log`.
//!
//! The existing startup rotation (`rotate_log_files`) handles the
//! per-execution boundary: each launch's `warp.log` becomes `warp.log.old.N`
//! at the next launch, with older files shifting up and the oldest dropping
//! off. That model bounds disk usage *per restart* but the active session's
//! log itself grows unboundedly.
//!
//! This module adds the orthogonal in-session bound: a `Write` wrapper that
//! rotates the active file once its byte count crosses a configured
//! threshold. Rotated copies land at `warp.log.in_session.N` (distinct from
//! the startup `.old.N` slots, which log-bundle uploads and other UX depend
//! on). When the configured number of `.in_session.N` slots is full, the
//! oldest is discarded — matching `rotate_log_files`'s overflow semantics.
//!
//! See warpdotdev/warp#10879.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// `Write` implementation that rotates its backing file once `max_bytes` of
/// writes have accumulated. On rotation, the active path is renamed to
/// `<base>.in_session.0`, existing `<base>.in_session.N` files shift up,
/// and the oldest beyond `max_rotation` is deleted. A fresh empty active
/// file is then opened.
pub(crate) struct RotatingFileWriter {
    log_directory: PathBuf,
    base_file_name: String,
    max_bytes: u64,
    max_rotation: usize,
    bytes_written: u64,
    file: File,
}

impl RotatingFileWriter {
    /// Opens (or truncates) `<log_directory>/<base_file_name>` and starts
    /// tracking byte counts toward `max_bytes`. `max_rotation` is the number
    /// of `.in_session.N` slots to retain.
    pub(crate) fn open(
        log_directory: impl Into<PathBuf>,
        base_file_name: impl Into<String>,
        max_bytes: u64,
        max_rotation: usize,
    ) -> io::Result<Self> {
        let log_directory = log_directory.into();
        let base_file_name = base_file_name.into();
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(log_directory.join(&base_file_name))?;
        Ok(Self {
            log_directory,
            base_file_name,
            max_bytes,
            max_rotation,
            bytes_written: 0,
            file,
        })
    }

    fn in_session_path(&self, index: usize) -> PathBuf {
        self.log_directory
            .join(format!("{}.in_session.{index}", self.base_file_name))
    }

    fn active_path(&self) -> PathBuf {
        self.log_directory.join(&self.base_file_name)
    }

    /// Rotates the active file. Drops the oldest `.in_session.N`, shifts
    /// the remaining slots up by one, renames the active file into slot 0,
    /// and opens a fresh active file.
    fn rotate(&mut self) -> io::Result<()> {
        if self.max_rotation == 0 {
            // Caller asked for zero retained rotations — just truncate and
            // continue without producing a sidecar file.
            self.file.flush()?;
            self.file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(self.active_path())?;
            self.bytes_written = 0;
            return Ok(());
        }

        self.file.flush()?;

        // Delete the oldest slot.
        let oldest = self.in_session_path(self.max_rotation - 1);
        if oldest.exists() {
            fs::remove_file(&oldest)?;
        }

        // Shift remaining slots up: N-2 -> N-1, ..., 0 -> 1.
        for n in (0..self.max_rotation - 1).rev() {
            let src = self.in_session_path(n);
            if src.exists() {
                fs::rename(src, self.in_session_path(n + 1))?;
            }
        }

        // Move the active file to slot 0 and open a fresh active file.
        fs::rename(self.active_path(), self.in_session_path(0))?;
        self.file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(self.active_path())?;
        self.bytes_written = 0;
        Ok(())
    }
}

impl Write for RotatingFileWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // Rotate only when the *current* active file already holds content;
        // otherwise an oversized first write would promote an empty file to
        // `.in_session.0` and burn a retention slot before any useful log
        // data exists. The oversized payload still lands in the active file,
        // and is preserved on the next real rotation.
        if !buf.is_empty()
            && self.max_bytes > 0
            && self.bytes_written > 0
            && self.bytes_written.saturating_add(buf.len() as u64) > self.max_bytes
        {
            self.rotate()?;
        }
        let n = self.file.write(buf)?;
        self.bytes_written = self.bytes_written.saturating_add(n as u64);
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

/// Wraps `file` in a [`RotatingFileWriter`] when `max_file_size_bytes` is
/// `Some(_)` and non-zero. Otherwise returns the raw file boxed as a
/// `Write` so callers can use a uniform target type.
///
/// The `file` argument is the already-opened active file at
/// `<log_directory>/<base_file_name>`. When rotation is enabled we discard
/// it and reopen via `RotatingFileWriter::open` so the rotation state
/// owns the file descriptor.
pub(crate) fn wrap_for_rotation(
    file: File,
    log_directory: &Path,
    base_file_name: &str,
    max_file_size_bytes: Option<u64>,
    max_rotation: usize,
) -> io::Result<Box<dyn Write + Send + 'static>> {
    match max_file_size_bytes {
        Some(max_bytes) if max_bytes > 0 => {
            // The file passed in was opened with truncate=true by the caller;
            // we'll reopen via RotatingFileWriter::open which has the same
            // semantics. Drop the existing handle first to keep file
            // descriptors symmetric.
            drop(file);
            let writer = RotatingFileWriter::open(
                log_directory.to_path_buf(),
                base_file_name.to_string(),
                max_bytes,
                max_rotation,
            )?;
            Ok(Box::new(writer))
        }
        _ => Ok(Box::new(file)),
    }
}

#[cfg(test)]
mod tests {
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
}
