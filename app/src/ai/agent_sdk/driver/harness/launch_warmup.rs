//! Reads a harness's launch files ahead of its launch. A sandbox starts with an empty file
//! cache, so the first process to load the harness's runtime, adapter and CLI waits on storage.
//! Reading them while the workspace is prepared moves that wait off the launch.

use std::fs::File;
use std::io::{self, Read as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use instant::Instant;
use walkdir::WalkDir;
use warp_core::features::FeatureFlag;

use crate::ai::agent_sdk::setup_observability::{SetupClientEventReporter, SetupStep};

/// Bounds one warm-up, so an unexpectedly large directory cannot keep it reading for a whole run.
const MAX_WARMUP_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// How much is read between checks of the stop flag.
const READ_CHUNK_BYTES: usize = 1024 * 1024;

/// A warm-up reading in the background. It runs to completion unless [`Self::stop`] is called.
pub(crate) struct LaunchFileWarmup {
    stop: Arc<AtomicBool>,
}

impl LaunchFileWarmup {
    /// Starts reading every file under the directories `roots` returns that exist, recorded as a
    /// setup step. Returns `None` when there is nothing to read.
    pub(crate) fn start(
        setup_events: &SetupClientEventReporter,
        roots: impl FnOnce() -> Vec<PathBuf>,
    ) -> Option<Self> {
        if !FeatureFlag::HarnessLaunchFileWarmup.is_enabled() {
            return None;
        }
        let roots: Vec<_> = roots().into_iter().filter(|root| root.is_dir()).collect();
        if roots.is_empty() {
            return None;
        }

        let stop = Arc::new(AtomicBool::new(false));
        let reader_stop = stop.clone();
        setup_events.record_value_detached(SetupStep::HarnessLaunchFileWarmup, async move {
            let started = Instant::now();
            let read = tokio::task::spawn_blocking(move || {
                read_files(&roots, &reader_stop, MAX_WARMUP_BYTES)
            })
            .await;
            match read {
                Ok(stats) => log::info!(
                    "Harness launch file warm-up read {} files ({} bytes) in {:?}",
                    stats.files,
                    stats.bytes,
                    started.elapsed()
                ),
                Err(error) => log::warn!("Harness launch file warm-up did not finish: {error}"),
            }
        });
        Some(Self { stop })
    }

    /// Stops the warm-up within one read, leaving storage to whatever runs next.
    pub(crate) fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
struct WarmupStats {
    files: u64,
    bytes: u64,
}

/// Reads the regular files under `roots` until all are read, `stop` is set, or `max_bytes` have
/// been read. Symlinks are not followed and unreadable entries are skipped: the read is only an
/// optimization, so nothing here is worth failing over.
fn read_files(roots: &[PathBuf], stop: &AtomicBool, max_bytes: u64) -> WarmupStats {
    let mut stats = WarmupStats::default();
    let mut chunk = vec![0; READ_CHUNK_BYTES];
    let files = roots
        .iter()
        .flat_map(|root| WalkDir::new(root).follow_links(false))
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file());
    for entry in files {
        if stop.load(Ordering::Relaxed) || stats.bytes >= max_bytes {
            break;
        }
        if let Ok(read) = read_file(entry.path(), &mut chunk, stop, max_bytes - stats.bytes) {
            stats.files += 1;
            stats.bytes += read;
        }
    }
    stats
}

fn read_file(path: &Path, chunk: &mut [u8], stop: &AtomicBool, budget: u64) -> io::Result<u64> {
    let mut file = File::open(path)?.take(budget);
    let mut total = 0;
    while !stop.load(Ordering::Relaxed) {
        match file.read(chunk)? {
            0 => break,
            read => total += read as u64,
        }
    }
    Ok(total)
}

#[cfg(test)]
#[path = "launch_warmup_tests.rs"]
mod tests;
