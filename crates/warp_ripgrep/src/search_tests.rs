use std::io::{self, Write};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::time::Duration;
use std::{fs, thread};

use tempfile::TempDir;

use super::{
    MAX_JSON_RECORD_BYTES, SEARCHER_MULTILINE_HEAP_LIMIT, search_to_writer, search_to_writer_inner,
};
use crate::types::RipgrepMessage;
#[derive(Default)]
struct WriteStats {
    total_bytes: usize,
    largest_write: usize,
    write_count: usize,
}

impl Write for WriteStats {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.total_bytes += buf.len();
        self.largest_write = self.largest_write.max(buf.len());
        self.write_count += 1;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn parallel_workers_enter_multiple_file_searches_before_either_finishes() {
    let temp_dir = TempDir::new().unwrap();
    for index in 0..8 {
        fs::write(
            temp_dir.path().join(format!("file-{index}.txt")),
            "alpha\nbeta\n",
        )
        .unwrap();
    }

    let (entered_tx, entered_rx) = mpsc::channel();
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let search_gate = Arc::clone(&gate);
    let root = temp_dir.path().to_path_buf();
    let search = thread::spawn(move || {
        search_to_writer_inner(
            &["alpha\nbeta".to_string()],
            vec![root],
            false,
            true,
            Vec::new(),
            Some(2),
            move || {
                entered_tx.send(()).unwrap();
                let (released, condvar) = &*search_gate;
                let mut released = released.lock().unwrap();
                while !*released {
                    released = condvar.wait(released).unwrap();
                }
            },
        )
    });

    let first_worker = entered_rx.recv_timeout(Duration::from_secs(2));
    let second_worker = entered_rx.recv_timeout(Duration::from_secs(2));
    let (released, condvar) = &*gate;
    *released.lock().unwrap() = true;
    condvar.notify_all();
    let output = search.join().unwrap().unwrap();

    assert!(first_worker.is_ok());
    assert!(
        second_worker.is_ok(),
        "a whole-search output lock prevented the second worker from scanning"
    );
    let match_count = String::from_utf8(output)
        .unwrap()
        .lines()
        .filter(|line| {
            matches!(
                serde_json::from_str(line).unwrap(),
                RipgrepMessage::Match { .. }
            )
        })
        .count();
    assert_eq!(match_count, 8);
}

#[test]
fn multiline_search_streams_many_matches_without_buffering_the_file_output() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("many-matches.txt");
    fs::write(&path, "alpha\nbeta\n".repeat(20_000)).unwrap();

    let stats = search_to_writer(
        &["alpha\nbeta".to_string()],
        vec![temp_dir.path().to_path_buf()],
        false,
        true,
        WriteStats::default(),
    )
    .unwrap();

    assert!(stats.total_bytes > 1024 * 1024);
    assert!(stats.write_count > 1);
    assert!(stats.largest_write <= MAX_JSON_RECORD_BYTES);
    assert!(stats.largest_write < stats.total_bytes);
}

#[test]
fn multiline_search_skips_files_above_the_heap_limit() {
    let temp_dir = TempDir::new().unwrap();
    let small_path = temp_dir.path().join("small.txt");
    fs::write(&small_path, "alpha\nbeta\n").unwrap();

    let large_path = temp_dir.path().join("large.txt");
    let mut large_contents = b"alpha\nbeta\n".to_vec();
    large_contents.resize(SEARCHER_MULTILINE_HEAP_LIMIT + 1, b'x');
    fs::write(&large_path, large_contents).unwrap();

    let output = search_to_writer(
        &["alpha\nbeta".to_string()],
        vec![temp_dir.path().to_path_buf()],
        false,
        true,
        Vec::new(),
    )
    .unwrap();

    let mut matched_paths = Vec::new();
    let mut limit_reached_count = 0;
    for line in String::from_utf8(output).unwrap().lines() {
        match serde_json::from_str(line).unwrap() {
            RipgrepMessage::Match { data } => matched_paths.push(data.path.text),
            RipgrepMessage::LimitReached => limit_reached_count += 1,
            RipgrepMessage::Begin | RipgrepMessage::End => {}
        }
    }
    assert_eq!(matched_paths, vec![small_path.to_string_lossy()]);
    assert_eq!(limit_reached_count, 1);
}
