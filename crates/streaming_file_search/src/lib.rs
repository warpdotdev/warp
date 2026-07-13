//! # Streaming file search
//!
//! An on-the-fly fuzzy file search engine intended to replace eager repo
//! indexing for interactive file pickers (cmd-O and the `@` context menu).
//!
//! Architecture (modeled after fzf/Helix):
//! * **Producer** ([`scan`]): for git repositories, `git ls-files -z --cached
//!   --others --exclude-standard` streams tracked and untracked files while a
//!   parallel [`ignore`] walk streams directories; for non-git directories a
//!   single parallel walk streams both files and directories. Candidates are
//!   pushed into the matcher as they are discovered.
//! * **Matcher**: [`nucleo`], a lock-free streaming fuzzy matcher. Candidates
//!   are injected concurrently while queries run; queries are re-scored
//!   incrementally when the user appends to the pattern.
//! * **Lifetime**: an engine is constructed per picker open, holds its
//!   candidates in RAM while the picker is open, and is dropped on close.
//!   [`StreamingFileSearchEngine::new`] performs a short synchronous burst so
//!   small repositories are fully loaded before first paint.
//! * **Invalidation**: [`FileSearchEngine::refresh_if_stale`] rescans when the
//!   repository's `.git/index` mtime changes.
//!
//! The engine intentionally performs *recall* only: it returns candidates
//! ranked by nucleo's score. Consumers apply their own precision re-ranking
//! (e.g. Warp's `fuzzy_match_path` scoring plus git-changed / recently-opened
//! bonuses) on the returned superset, which keeps result ordering and match
//! highlighting consistent with the non-streaming implementation.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use nucleo::pattern::{CaseMatching, Normalization};
use nucleo::{Config, Nucleo};

mod scan;

use scan::{git_index_mtime, start_scan, ScanHandle};

/// How long [`StreamingFileSearchEngine::new`] synchronously waits for the
/// initial scan so that small repositories are fully loaded before the first
/// query runs (Helix uses the same pattern/duration).
const SYNC_BURST: Duration = Duration::from_millis(30);

/// A single search candidate produced by the filesystem scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileCandidate {
    /// Path relative to the searched root. Directories end with the platform
    /// path separator, matching the format produced by the eager repo index.
    pub relative_path: String,
    pub is_directory: bool,
}

/// The result of driving the matcher via [`FileSearchEngine::poll`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PollStatus {
    /// The matcher is still processing the current pattern/candidates; matched
    /// results are incomplete until this is `false`.
    pub running: bool,
    /// The set of matched results changed since the last poll.
    pub changed: bool,
}

/// A cancellable, streaming fuzzy file search engine: candidates are streamed
/// in from a filesystem scan and ranked matches are read out incrementally.
///
/// Kept as a trait so the nucleo-backed implementation can be swapped (e.g.
/// for a remote-RPC-backed engine, or a fallback matcher) without touching
/// consumers.
pub trait FileSearchEngine: Send {
    /// Updates the active fuzzy query. An empty query matches all candidates
    /// (in injection order), which callers use for zero-state listings and
    /// wildcard queries that are matched downstream.
    fn update_query(&mut self, query: &str);

    /// Drives the matcher, waiting up to `timeout_ms` for progress.
    fn poll(&mut self, timeout_ms: u64) -> PollStatus;

    /// Number of candidates currently matching the active query.
    fn matched_count(&self) -> usize;

    /// Returns up to `max_results` matching candidates, best match first.
    fn matched(&self, max_results: usize) -> Vec<FileCandidate>;

    /// Whether the filesystem scan is still streaming candidates in.
    fn is_scanning(&self) -> bool;

    /// Restarts the scan if the underlying repository changed since the scan
    /// started (detected via `.git/index` mtime). Returns whether a rescan was
    /// triggered. No-ops while a scan is already in flight.
    fn refresh_if_stale(&mut self) -> bool;
}

/// The nucleo-backed [`FileSearchEngine`] implementation.
pub struct StreamingFileSearchEngine {
    nucleo: Nucleo<FileCandidate>,
    root: PathBuf,
    scan: ScanHandle,
    /// `.git/index` mtime captured when the current scan started. `None` for
    /// non-git roots.
    scanned_git_index_mtime: Option<std::time::SystemTime>,
    /// The pattern text passed to the previous `reparse` call, used to detect
    /// appends (which nucleo re-scores incrementally instead of rematching).
    last_pattern_text: String,
}

impl StreamingFileSearchEngine {
    /// Creates an engine rooted at `root` and starts streaming candidates.
    /// Blocks for at most [`SYNC_BURST`] so small repositories are fully
    /// loaded before the caller runs its first query.
    pub fn new(root: PathBuf) -> Self {
        let nucleo = Nucleo::new(
            Config::DEFAULT.match_paths(),
            // Consumers poll the engine per query rather than reacting to
            // matcher wakeups, so no notification callback is needed.
            Arc::new(|| {}),
            None,
            1,
        );
        let scanned_git_index_mtime = git_index_mtime(&root);
        let scan = start_scan(root.clone(), nucleo.injector());
        scan.wait_for_completion(SYNC_BURST);
        Self {
            nucleo,
            root,
            scan,
            scanned_git_index_mtime,
            last_pattern_text: String::new(),
        }
    }

    fn apply_pattern(&mut self, pattern_text: &str) {
        let append = !pattern_text.is_empty()
            && !self.last_pattern_text.is_empty()
            && pattern_text.starts_with(&self.last_pattern_text);
        self.nucleo.pattern.reparse(
            0,
            pattern_text,
            CaseMatching::Ignore,
            Normalization::Smart,
            append,
        );
        self.last_pattern_text = pattern_text.to_string();
    }
}

impl FileSearchEngine for StreamingFileSearchEngine {
    fn update_query(&mut self, query: &str) {
        let pattern_text = build_pattern_text(query);
        if pattern_text == self.last_pattern_text {
            return;
        }
        self.apply_pattern(&pattern_text);
    }

    fn poll(&mut self, timeout_ms: u64) -> PollStatus {
        let status = self.nucleo.tick(timeout_ms);
        PollStatus {
            running: status.running,
            changed: status.changed,
        }
    }

    fn matched_count(&self) -> usize {
        self.nucleo.snapshot().matched_item_count() as usize
    }

    fn matched(&self, max_results: usize) -> Vec<FileCandidate> {
        let snapshot = self.nucleo.snapshot();
        let count = snapshot
            .matched_item_count()
            .min(u32::try_from(max_results).unwrap_or(u32::MAX));
        snapshot
            .matched_items(0..count)
            .map(|item| item.data.clone())
            .collect()
    }

    fn is_scanning(&self) -> bool {
        !self.scan.is_complete()
    }

    fn refresh_if_stale(&mut self) -> bool {
        if self.is_scanning() {
            return false;
        }
        let current = git_index_mtime(&self.root);
        if current == self.scanned_git_index_mtime {
            return false;
        }
        self.scan.cancel();
        self.nucleo.restart(true);
        self.scanned_git_index_mtime = current;
        self.scan = start_scan(self.root.clone(), self.nucleo.injector());
        // `restart` drops matcher state; re-apply the active pattern from
        // scratch (never as an append).
        let pattern_text = std::mem::take(&mut self.last_pattern_text);
        self.apply_pattern(&pattern_text);
        true
    }
}

impl Drop for StreamingFileSearchEngine {
    fn drop(&mut self) {
        self.scan.cancel();
    }
}

/// Converts a user query into nucleo pattern text.
///
/// nucleo parses fzf-style syntax (leading `!` negation, leading `'`
/// substring, leading `^` prefix, trailing `$` postfix), but Warp's existing
/// file search treats these characters literally, so operator positions are
/// escaped here. nucleo only honors `\` escapes at those operator positions;
/// all other characters are already literal. Whitespace is preserved: nucleo
/// treats whitespace-separated words as AND-ed atoms, which mirrors the
/// multi-term semantics of Warp's `fuzzy_match_path`.
fn build_pattern_text(query: &str) -> String {
    let mut pattern = String::with_capacity(query.len());
    for atom in query.split_whitespace() {
        if !pattern.is_empty() {
            pattern.push(' ');
        }
        // A leading `!` inverts the atom; a leading `'`/`^` (only checked when
        // there is no leading `!`) selects substring/prefix matching.
        if atom.starts_with('!') || atom.starts_with('\'') || atom.starts_with('^') {
            pattern.push('\\');
        }
        // A trailing `$` selects postfix matching (checked even after a
        // leading operator escape).
        match atom.strip_suffix('$') {
            Some(rest) => {
                pattern.push_str(rest);
                pattern.push_str("\\$");
            }
            None => pattern.push_str(atom),
        }
    }
    pattern
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
