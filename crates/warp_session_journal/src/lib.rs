//! Per-session ring buffer of resequencable events with an optional
//! on-disk JSONL mirror.
//!
//! Foundation for SSH session resilience: when a connection drops
//! between a long-running task and the desktop client, the client
//! reconnects and asks "give me events since seq N" instead of
//! starting mid-stream. The journal keeps a bounded window of recent
//! events so a fresh client can catch up to the live tail, and
//! reports a gap when the requested cursor is older than the
//! oldest surviving entry.
//!
//! Designed against the [#11925](https://github.com/warpdotdev/warp/issues/11925)
//! and [#12329](https://github.com/warpdotdev/warp/issues/12329)
//! reattach + agent-notification asks. R3.1 of the SSH connection
//! management roadmap.
//!
//! ## Sequence semantics
//!
//! Sequence numbers are per-`EventJournal`, start at 1, monotonically
//! increase, and never reset for the lifetime of the journal.
//! [`EventJournal::head_seq`] is 0 before the first append. Overflow
//! at `u64::MAX` panics — a single session producing 2^64 events is
//! treated as a logic error, not a recoverable runtime condition.
//!
//! ## Eviction
//!
//! Bounded by entry count (default [`DEFAULT_CAPACITY`] = 1024). When
//! the ring is full the oldest entry is dropped, so the journal can
//! outrun a slow reattach indefinitely without unbounded memory.
//! Eviction means [`EventJournal::replay_since`] can no longer satisfy
//! a `since_seq` older than the oldest seq still in the ring — the
//! returned [`ReplaySnapshot::replay_gap`] reports the earliest
//! surviving seq so the caller can fall back to a full reload.
//!
//! ## Disk mirror
//!
//! When [`EventJournal::with_disk_writer`] attaches a
//! [`JournalDiskWriter`], every successful in-memory append also
//! appends one JSON-encoded line to the disk file. A disk write
//! failure detaches the writer (logged once) but does not abort the
//! in-memory append — losing durability for one event is acceptable
//! whereas dropping a live event the consumer is waiting on is not.

use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// Default ring capacity. Sized to hold roughly ten typical agent
/// turns of streaming output (each turn ≈ 50-100 events). Override
/// via [`EventJournal::with_capacity`] if profiling shows the right
/// number is different for a given workload.
pub const DEFAULT_CAPACITY: usize = 1024;

/// A single recorded event in the journal: a monotonically-assigned
/// sequence number, the wall-clock timestamp at which it was
/// appended, and the caller's payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalEntry<P> {
    pub seq: u64,
    pub ts_ms: i64,
    pub payload: P,
}

/// Result of an [`EventJournal::replay_since`] call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplaySnapshot<P> {
    /// Entries the caller asked for, in seq order.
    pub entries: Vec<JournalEntry<P>>,
    /// `Some(earliest_available)` when the caller's `since_seq`
    /// would have required entries the ring has evicted. Callers
    /// interpret this as "the local cursor is behind the journal's
    /// oldest surviving entry — fall back to a full reload or accept
    /// the gap".
    pub replay_gap: Option<u64>,
    /// Highest seq currently observed (0 when the journal is empty).
    /// Callers store this so a follow-up reattach knows where to
    /// resume.
    pub head_seq: u64,
}

/// Bounded ring buffer of events, monotonically sequenced.
pub struct EventJournal<P> {
    ring: VecDeque<JournalEntry<P>>,
    /// Seq number for the next append (always `head_seq + 1`). Held
    /// outside the ring so eviction doesn't reset the counter.
    next_seq: u64,
    capacity: usize,
    /// Optional disk-backed mirror. When set, every successful
    /// in-memory append is also written to the on-disk JSONL file
    /// so the journal can be reconstructed across restarts.
    disk_writer: Option<JournalDiskWriter<P>>,
}

impl<P> std::fmt::Debug for EventJournal<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventJournal")
            .field("ring_len", &self.ring.len())
            .field("next_seq", &self.next_seq)
            .field("capacity", &self.capacity)
            .field("disk_backed", &self.disk_writer.is_some())
            .finish()
    }
}

impl<P> Default for EventJournal<P> {
    fn default() -> Self {
        Self::with_capacity(DEFAULT_CAPACITY)
    }
}

impl<P> EventJournal<P> {
    /// Build a journal with the given ring capacity.
    ///
    /// A zero-capacity ring would refuse every append, so the input
    /// is clamped to 1. Tests exercising eviction behaviour use
    /// small but non-zero capacities; production callers should
    /// stick with [`DEFAULT_CAPACITY`] or a measured override.
    pub fn with_capacity(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            ring: VecDeque::with_capacity(capacity),
            next_seq: 1,
            capacity,
            disk_writer: None,
        }
    }

    /// Attach a disk-backed JSONL mirror. Idempotent — a second call
    /// overwrites the prior writer. Production wires the writer once
    /// at journal creation; this builder shape exists so tests can
    /// drive the disk-backed path without a second factory.
    pub fn with_disk_writer(mut self, writer: JournalDiskWriter<P>) -> Self {
        self.disk_writer = Some(writer);
        self
    }

    /// Highest seq currently observed. 0 before the first append.
    pub fn head_seq(&self) -> u64 {
        self.next_seq.saturating_sub(1)
    }

    /// Number of entries currently held in the ring (after any
    /// evictions). Useful for tests + diagnostics.
    pub fn len(&self) -> usize {
        self.ring.len()
    }

    /// `true` iff no entries are held. Mirrors [`Self::len`] for
    /// callers preferring the boolean shape.
    pub fn is_empty(&self) -> bool {
        self.ring.is_empty()
    }
}

impl<P: Clone + Serialize> EventJournal<P> {
    /// Append `payload` with a fresh seq. Evicts the oldest entry
    /// when the ring is full. Returns the appended seq so the caller
    /// can include it on the wire alongside the event.
    pub fn append(&mut self, payload: P) -> u64 {
        let seq = self.next_seq;
        self.next_seq = seq
            .checked_add(1)
            .expect("event journal seq overflow (2^64 events in one session)");
        if self.ring.len() >= self.capacity {
            self.ring.pop_front();
        }
        let entry = JournalEntry {
            seq,
            ts_ms: now_ms(),
            payload,
        };
        // Mirror to disk before pushing into the ring so the
        // on-disk order matches the in-memory order. A write
        // failure detaches the writer (defensive — repeated failures
        // would spam logs) but lets the in-memory append succeed.
        if let Some(writer) = self.disk_writer.as_mut() {
            if let Err(err) = writer.append(&entry) {
                log::warn!(
                    "journal: disk append failed at seq {}: {err}; detaching disk mirror",
                    entry.seq,
                );
                self.disk_writer = None;
            }
        }
        self.ring.push_back(entry);
        seq
    }
}

impl<P: Clone> EventJournal<P> {
    /// Snapshot entries the caller asked for.
    ///
    /// `since_seq=None` means "give me everything currently in the
    /// ring". `since_seq=Some(n)` means "give me entries with
    /// `seq > n`"; if the ring no longer holds seq `n+1` (the caller
    /// missed events that have been evicted) the snapshot reports
    /// the gap via [`ReplaySnapshot::replay_gap`] so the caller can
    /// fall back to a full reload.
    pub fn replay_since(&self, since_seq: Option<u64>) -> ReplaySnapshot<P> {
        let head_seq = self.head_seq();
        let replay_gap = match (since_seq, self.ring.front()) {
            // Caller asked for entries newer than `n`. The smallest
            // seq still in the ring is `earliest.seq`. If
            // `earliest.seq > n + 1`, the entry at `n + 1` was
            // evicted — gap.
            (Some(n), Some(earliest)) if earliest.seq > n.saturating_add(1) => Some(earliest.seq),
            _ => None,
        };
        let cutoff = since_seq.unwrap_or(0);
        let entries: Vec<JournalEntry<P>> = self
            .ring
            .iter()
            .filter(|e| e.seq > cutoff)
            .cloned()
            .collect();
        ReplaySnapshot {
            entries,
            replay_gap,
            head_seq,
        }
    }
}

/// Disk-backed JSONL mirror of an [`EventJournal`]. Each call to
/// [`Self::append`] writes one line of the form `<json>\n` where the
/// JSON is the serde-serialized [`JournalEntry`].
pub struct JournalDiskWriter<P> {
    writer: BufWriter<File>,
    path: PathBuf,
    _marker: std::marker::PhantomData<fn() -> P>,
}

impl<P> std::fmt::Debug for JournalDiskWriter<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JournalDiskWriter")
            .field("path", &self.path)
            .finish()
    }
}

impl<P: Serialize> JournalDiskWriter<P> {
    /// Create (or truncate) the JSONL file at `path`. Parent
    /// directory is created if it doesn't exist.
    pub fn create(path: impl Into<PathBuf>) -> std::io::Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&path)?;
        Ok(Self {
            writer: BufWriter::new(file),
            path,
            _marker: std::marker::PhantomData,
        })
    }

    /// Open the file at `path` for appending. Does NOT truncate, so
    /// existing entries are preserved.
    pub fn append_to(path: impl Into<PathBuf>) -> std::io::Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        Ok(Self {
            writer: BufWriter::new(file),
            path,
            _marker: std::marker::PhantomData,
        })
    }

    /// Append one entry as a JSONL line. Flushes before returning so
    /// a process crash between calls can lose at most the
    /// most-recent entry.
    pub fn append(&mut self, entry: &JournalEntry<P>) -> std::io::Result<()> {
        let line = serde_json::to_string(entry)?;
        self.writer.write_all(line.as_bytes())?;
        self.writer.write_all(b"\n")?;
        self.writer.flush()?;
        Ok(())
    }

    /// Path the writer is mirroring to.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Consume the writer and return the path (e.g. so a later
    /// replay reader can be opened).
    pub fn into_path(self) -> PathBuf {
        self.path
    }
}

/// Read a JSONL journal file produced by [`JournalDiskWriter`] back
/// into a vector of entries. Malformed lines are logged and skipped;
/// the rest are returned in file order.
pub fn read_journal_entries<P: DeserializeOwned>(
    path: &Path,
) -> std::io::Result<Vec<JournalEntry<P>>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut entries = Vec::new();
    for (line_no, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<JournalEntry<P>>(&line) {
            Ok(entry) => entries.push(entry),
            Err(err) => {
                log::warn!(
                    "journal: skipping malformed JSONL line {} in {}: {err}",
                    line_no + 1,
                    path.display(),
                );
            }
        }
    }
    Ok(entries)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
