//! Measures first-open and per-keystroke latency of the streaming engine.
//!
//! Usage: `cargo run --release -p streaming_file_search --example measure -- <repo-path>`

use std::time::Duration;

use instant::Instant;
use streaming_file_search::{FileSearchEngine, StreamingFileSearchEngine};

fn drive_until_idle(engine: &mut StreamingFileSearchEngine, wait_for_scan: bool) {
    loop {
        let status = engine.poll(10);
        if !status.running && (!wait_for_scan || !engine.is_scanning()) {
            if !wait_for_scan {
                break;
            }
            // Scan complete; one more poll to flush the tail of candidates.
            let status = engine.poll(10);
            if !status.running {
                break;
            }
        }
    }
}

fn main() {
    let root = std::env::args().nth(1).expect("usage: measure <repo-path>");
    let root = std::path::PathBuf::from(root);

    // First-open latency: engine construction (includes the 30ms sync burst).
    let start = Instant::now();
    let mut engine = StreamingFileSearchEngine::new(root.clone());
    let construct = start.elapsed();
    let after_burst = engine.matched_count();
    engine.update_query("");
    engine.poll(10);
    let burst_matched = engine.matched_count();

    // Time until the full scan completes.
    let start = Instant::now();
    while engine.is_scanning() {
        engine.poll(10);
        std::thread::sleep(Duration::from_millis(1));
    }
    drive_until_idle(&mut engine, true);
    let scan_complete = start.elapsed();
    let total = engine.matched_count();

    println!("root: {}", root.display());
    println!("construct (incl. 30ms burst): {construct:?}");
    println!("candidates injected at burst end: {after_burst} (matched after first poll: {burst_matched})");
    println!("full scan completed after further: {scan_complete:?}; total candidates: {total}");

    // Per-keystroke latency: simulate typing a query one char at a time
    // (append path), then deleting back down (re-match path).
    for query_sequence in [
        vec!["m", "mo", "mod", "mode", "model"],
        vec!["model", "mode", "mod"],
        vec!["data", "data ", "data s", "data so", "data sour"],
        vec!["zzzz"],
    ] {
        for query in query_sequence {
            let start = Instant::now();
            engine.update_query(query);
            drive_until_idle(&mut engine, false);
            let elapsed = start.elapsed();
            let top = engine.matched(1000).len();
            println!(
                "query {query:?}: {elapsed:?} ({} matched, {top} collected)",
                engine.matched_count()
            );
        }
    }
}
