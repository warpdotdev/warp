use serde_json::{json, Value};
use tempfile::TempDir;

use super::{read_journal_entries, EventJournal, JournalDiskWriter, JournalEntry};

fn payload(label: &str) -> Value {
    json!({ "type": "test", "label": label })
}

// ── sequence assignment ─────────────────────────────────────────────

#[test]
fn append_assigns_monotonic_seqs_starting_at_one() {
    let mut journal = EventJournal::default();
    assert_eq!(journal.append(payload("a")), 1);
    assert_eq!(journal.append(payload("b")), 2);
    assert_eq!(journal.append(payload("c")), 3);
}

#[test]
fn head_seq_is_zero_before_any_append() {
    let journal: EventJournal<Value> = EventJournal::default();
    assert_eq!(journal.head_seq(), 0);
}

#[test]
fn head_seq_advances_with_each_append() {
    let mut journal = EventJournal::default();
    journal.append(payload("a"));
    assert_eq!(journal.head_seq(), 1);
    journal.append(payload("b"));
    assert_eq!(journal.head_seq(), 2);
}

#[test]
fn empty_journal_reports_is_empty() {
    let journal: EventJournal<Value> = EventJournal::default();
    assert!(journal.is_empty());
    assert_eq!(journal.len(), 0);
}

// ── ring capacity / eviction ────────────────────────────────────────

#[test]
fn zero_capacity_clamps_to_one_entry() {
    let mut journal = EventJournal::with_capacity(0);
    let seq = journal.append(payload("a"));
    assert_eq!(seq, 1);
    assert_eq!(journal.len(), 1);
}

#[test]
fn eviction_drops_oldest_when_capacity_exceeded() {
    let mut journal = EventJournal::with_capacity(2);
    journal.append(payload("a")); // seq 1, evicted by overflow
    journal.append(payload("b")); // seq 2
    journal.append(payload("c")); // seq 3 — pushes seq 1 out

    let snapshot = journal.replay_since(None);
    let seqs: Vec<u64> = snapshot.entries.iter().map(|e| e.seq).collect();
    assert_eq!(seqs, vec![2, 3]);
    assert_eq!(journal.len(), 2);
}

#[test]
fn eviction_does_not_reset_seq_counter() {
    // Eviction must not let `next_seq` walk backwards — that would
    // hand out a duplicate seq and confuse readers reattaching with
    // a cursor in the middle of the new range.
    let mut journal = EventJournal::with_capacity(2);
    journal.append(payload("a")); // seq 1, evicted
    journal.append(payload("b")); // seq 2, evicted
    journal.append(payload("c")); // seq 3
    journal.append(payload("d")); // seq 4
    assert_eq!(journal.head_seq(), 4);
    let snapshot = journal.replay_since(None);
    assert_eq!(snapshot.entries.first().map(|e| e.seq), Some(3));
    assert_eq!(snapshot.entries.last().map(|e| e.seq), Some(4));
}

// ── replay_since ────────────────────────────────────────────────────

#[test]
fn replay_since_none_returns_full_ring() {
    let mut journal = EventJournal::default();
    journal.append(payload("a"));
    journal.append(payload("b"));
    journal.append(payload("c"));

    let snapshot = journal.replay_since(None);
    assert_eq!(snapshot.entries.len(), 3);
    assert_eq!(snapshot.replay_gap, None);
    assert_eq!(snapshot.head_seq, 3);
}

#[test]
fn replay_since_skips_already_seen() {
    let mut journal = EventJournal::default();
    journal.append(payload("a"));
    journal.append(payload("b"));
    journal.append(payload("c"));

    let snapshot = journal.replay_since(Some(1));
    let seqs: Vec<u64> = snapshot.entries.iter().map(|e| e.seq).collect();
    assert_eq!(seqs, vec![2, 3]);
    assert_eq!(snapshot.replay_gap, None);
    assert_eq!(snapshot.head_seq, 3);
}

#[test]
fn replay_since_at_head_returns_empty_no_gap() {
    let mut journal = EventJournal::default();
    journal.append(payload("a"));
    journal.append(payload("b"));

    let snapshot = journal.replay_since(Some(2));
    assert!(snapshot.entries.is_empty());
    assert_eq!(snapshot.replay_gap, None);
    assert_eq!(snapshot.head_seq, 2);
}

#[test]
fn replay_gap_reports_evicted_floor() {
    // Caller asks for "since seq 1", but seq 2 has been evicted.
    // Snapshot should report `replay_gap = Some(3)` so the caller
    // knows the oldest available seq is 3, not the requested 2.
    let mut journal = EventJournal::with_capacity(2);
    journal.append(payload("a")); // seq 1
    journal.append(payload("b")); // seq 2
    journal.append(payload("c")); // seq 3 — evicts seq 1
    journal.append(payload("d")); // seq 4 — evicts seq 2

    let snapshot = journal.replay_since(Some(1));
    assert_eq!(snapshot.replay_gap, Some(3));
    let seqs: Vec<u64> = snapshot.entries.iter().map(|e| e.seq).collect();
    assert_eq!(seqs, vec![3, 4]);
    assert_eq!(snapshot.head_seq, 4);
}

#[test]
fn replay_gap_not_reported_when_cursor_is_at_or_above_evicted_floor() {
    let mut journal = EventJournal::with_capacity(2);
    journal.append(payload("a")); // seq 1, evicted
    journal.append(payload("b")); // seq 2
    journal.append(payload("c")); // seq 3 — evicts seq 1

    // since_seq = 2 → we want seq > 2, ring has [2, 3], earliest = 2,
    // 2 > 2+1 is false → no gap.
    let snapshot = journal.replay_since(Some(2));
    assert_eq!(snapshot.replay_gap, None);
    assert_eq!(snapshot.entries.len(), 1);
    assert_eq!(snapshot.entries[0].seq, 3);
}

#[test]
fn replay_since_with_empty_ring_returns_no_entries_no_gap() {
    let journal: EventJournal<Value> = EventJournal::default();
    let snapshot = journal.replay_since(Some(5));
    assert!(snapshot.entries.is_empty());
    assert_eq!(snapshot.replay_gap, None);
    assert_eq!(snapshot.head_seq, 0);
}

// ── disk mirror ─────────────────────────────────────────────────────

#[test]
fn disk_writer_persists_entries_as_jsonl() {
    let tmp = TempDir::new().expect("tempdir");
    let path = tmp.path().join("journal.jsonl");
    let writer = JournalDiskWriter::<Value>::create(&path).expect("create writer");
    let mut journal = EventJournal::default().with_disk_writer(writer);

    journal.append(payload("a"));
    journal.append(payload("b"));
    journal.append(payload("c"));

    // Read the file back as raw lines and check the count + first
    // field shape. A round-trip through `read_journal_entries`
    // covers the full decode path in a separate test.
    let body = std::fs::read_to_string(&path).expect("read");
    let lines: Vec<&str> = body.lines().collect();
    assert_eq!(lines.len(), 3);
    assert!(lines[0].contains("\"seq\":1"));
    assert!(lines[1].contains("\"seq\":2"));
    assert!(lines[2].contains("\"seq\":3"));
}

#[test]
fn disk_replay_recovers_appended_entries() {
    let tmp = TempDir::new().expect("tempdir");
    let path = tmp.path().join("journal.jsonl");
    {
        let writer = JournalDiskWriter::<Value>::create(&path).expect("create writer");
        let mut journal = EventJournal::default().with_disk_writer(writer);
        journal.append(payload("a"));
        journal.append(payload("b"));
        journal.append(payload("c"));
    }

    let recovered: Vec<JournalEntry<Value>> =
        read_journal_entries(&path).expect("read journal");
    assert_eq!(recovered.len(), 3);
    let seqs: Vec<u64> = recovered.iter().map(|e| e.seq).collect();
    assert_eq!(seqs, vec![1, 2, 3]);
}

#[test]
fn disk_replay_skips_malformed_lines_silently() {
    let tmp = TempDir::new().expect("tempdir");
    let path = tmp.path().join("journal.jsonl");
    let well_formed = serde_json::to_string(&JournalEntry {
        seq: 1u64,
        ts_ms: 100,
        payload: payload("a"),
    })
    .expect("serialize");
    std::fs::write(
        &path,
        format!("{well_formed}\nthis is not valid json\n{well_formed}\n"),
    )
    .expect("write");

    let recovered: Vec<JournalEntry<Value>> =
        read_journal_entries(&path).expect("read");
    assert_eq!(recovered.len(), 2);
}

#[test]
fn disk_replay_treats_empty_lines_as_skipped() {
    let tmp = TempDir::new().expect("tempdir");
    let path = tmp.path().join("journal.jsonl");
    std::fs::write(&path, "\n\n\n").expect("write");

    let recovered: Vec<JournalEntry<Value>> =
        read_journal_entries(&path).expect("read");
    assert!(recovered.is_empty());
}

#[test]
fn append_to_preserves_existing_file_contents() {
    // The disk mirror's append-to variant must not truncate — that's
    // the only useful difference from `create`. If a daemon restarts
    // and resumes journaling, prior entries stay.
    let tmp = TempDir::new().expect("tempdir");
    let path = tmp.path().join("journal.jsonl");
    {
        let writer = JournalDiskWriter::<Value>::create(&path).expect("create");
        let mut journal = EventJournal::default().with_disk_writer(writer);
        journal.append(payload("a"));
    }
    {
        let writer = JournalDiskWriter::<Value>::append_to(&path).expect("append_to");
        let mut journal = EventJournal::default().with_disk_writer(writer);
        journal.append(payload("b"));
    }

    let recovered: Vec<JournalEntry<Value>> =
        read_journal_entries(&path).expect("read");
    assert_eq!(recovered.len(), 2);
    let labels: Vec<String> = recovered
        .iter()
        .map(|e| {
            e.payload
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        })
        .collect();
    assert_eq!(labels, vec!["a", "b"]);
}
