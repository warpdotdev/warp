//! Literal substring search **inside** Claude Code transcripts, for the
//! session-search popup.
//!
//! The popup's name search answers "what was that conversation called"; this
//! answers "where is that thing I pasted". Both halves are needed because a
//! session's name is one line and its transcript is megabytes.
//!
//! Every mechanic here is the Tier 2 design from `specs/samithaj/rail-search/plan.md`
//! §6–§7, unchanged — that spec designed a different (abandoned) surface, but
//! the digest itself survives verbatim:
//!
//! - **Content is what was said**: the user's own turns plus the assistant's
//!   prose. Tool-result bodies and pasted file contents are excluded, which is
//!   the whole 30× win — and is why the popup says so in its footer, because a
//!   search that silently half-answers is worse than one that admits its scope.
//! - **Role attribution is structural**, via serde, never regex: a
//!   `"type":"user"` envelope *without* a `tool_use_id` block is what the user
//!   typed; with one, it is a tool result Claude fed back in.
//! - **Cache key `(path, len, mtime)`.** The mtime half makes staleness
//!   structurally impossible; the `len` half makes growth append-only, so a
//!   transcript that grew re-reads only `[old_len, EOF)`.
//! - **Errors are never memoized.** A transient I/O failure must retry on the
//!   next search, not be cached as an empty digest forever.
//! - **Per-line `serde_json` in try/continue, lossy UTF-8 decode.** A live
//!   transcript's final line is routinely torn mid-record, and a bounded read
//!   can start inside a multi-byte character.
//! - **Literal, never fuzzy**, so `POA-2236` stays one token instead of
//!   matching every line with a `P`, an `O` and an `A` in it.
//! - **Not persisted.** Rebuilding is sub-second per project; persisting it
//!   would buy that back at the price of a staleness mode.
//!
//! # Cost
//!
//! Nothing here may run on the render path. [`TranscriptDigestModel::set_query`]
//! does every `std::fs` call in a spawned task and publishes once, at
//! completion; the palette's content data source only ever serves what was
//! already published. Reads are bounded three ways — per file, per search pass,
//! and by an LRU bound on the store — because the corpus is multi-gigabyte and
//! a single transcript here reaches 92 MB.
//!
//! # Claude only
//!
//! Codex and the other CLI agents have no per-cwd transcript store to read, so
//! content search cannot cover them. That limit is not silent: the popup's
//! footer states it.

use warpui::{Entity, ModelContext, SingletonEntity};

use crate::terminal::CLIAgent;

/// Shortest query worth a corpus scan.
///
/// A one- or two-character literal substring matches nearly every transcript,
/// so it would cost a full pass to produce fifty rows of noise. Three is short
/// enough for the ids and error fragments this search exists for.
const MIN_QUERY_CHARS: usize = 3;

/// Maximum content rows published for one query, per `rail-search/plan.md` §7.
const MAX_HITS: usize = 50;

/// One session the content search may look inside.
///
/// Flat and owned like [`AgentSessionCandidate`](crate::search::command_palette::agent_sessions::AgentSessionCandidate),
/// and for the same reason: the corpus is assembled once when the popup opens
/// and then read from a spawned task, so nothing here may need a view, a
/// workspace or the disk to be understood.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigestTarget {
    pub agent: CLIAgent,
    pub session_id: String,
    pub project_name: String,
    pub task_name: String,
    pub cwd: String,
}

/// One session whose transcript contains the query.
///
/// Carries everything its row needs, so the data source that renders it can
/// stay a pure function of what was published.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentHit {
    pub agent: CLIAgent,
    pub session_id: String,
    pub project_name: String,
    pub task_name: String,
    pub cwd: String,
    /// One line of context around the first match, already ellipsized.
    pub snippet: String,
    /// **Char** indices of the matched term within `snippet` — what the text
    /// elements' highlight API expects.
    pub snippet_match_indices: Vec<usize>,
    /// The transcript was too large to read whole, so this hit came from a
    /// bounded window of it and the absence of a hit elsewhere proves nothing.
    pub partial: bool,
}

/// What the content search is doing right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DigestStatus {
    /// Nothing to search: no corpus, or a query below [`MIN_QUERY_CHARS`].
    #[default]
    Idle,
    /// A pass is running. The palette must not claim "no results" while this
    /// is the status.
    Searching,
    /// The published hits are the complete answer for the published query.
    Finished,
}

/// Emitted when a search pass completes and [`TranscriptDigestModel::hits`]
/// have been replaced. The palette re-runs its query once on this.
pub struct TranscriptDigestSearchFinished;

/// Singleton store of transcript digests, plus the results of the latest
/// content search over them.
///
/// Same shape as [`ClaudeSessionScanModel`](super::session_scan::ClaudeSessionScanModel):
/// the main thread takes an owned snapshot, the spawned task does every
/// filesystem call, and only finished results come back to be published with
/// an `emit` and a `notify`.
#[derive(Default)]
pub struct TranscriptDigestModel {
    /// The sessions this search may look inside, set when the popup opens.
    corpus: Vec<DigestTarget>,
    /// The query [`Self::hits`] belong to. The data source compares this
    /// against the palette's current query text and serves nothing when they
    /// differ, which is what keeps a stale result set off the screen.
    query: String,
    hits: Vec<ContentHit>,
    status: DigestStatus,
    /// Transcripts finished in the published pass, and how many there were.
    scanned: usize,
    total: usize,
    /// Monotonic, so a superseded pass's results are dropped rather than
    /// overwriting a newer query's. Doubles as the LRU stamp on cache entries.
    search_id: u64,
    /// Digested transcript text, keyed by path. Never persisted.
    #[cfg(not(target_family = "wasm"))]
    digests: DigestCache,
}

impl Entity for TranscriptDigestModel {
    type Event = TranscriptDigestSearchFinished;
}

impl SingletonEntity for TranscriptDigestModel {}

impl TranscriptDigestModel {
    /// The query the published hits answer. Empty when nothing has been
    /// searched for yet.
    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn hits(&self) -> &[ContentHit] {
        &self.hits
    }

    pub fn status(&self) -> DigestStatus {
        self.status
    }

    /// Transcripts finished so far, out of the total in the corpus.
    ///
    /// Phase 2 publishes exactly once, at completion, so `scanned` steps
    /// straight from `0` to `total`; throttled progress milestones are a
    /// deliberate later phase, not an oversight.
    pub fn progress(&self) -> (usize, usize) {
        (self.scanned, self.total)
    }

    /// Replaces the set of sessions the search may look inside.
    ///
    /// Called when the popup opens, from the same candidate assembly the name
    /// search uses. Published results are dropped with the corpus they came
    /// from: they answered a question about sessions that may no longer be the
    /// ones on offer.
    pub fn set_corpus(&mut self, corpus: Vec<DigestTarget>, ctx: &mut ModelContext<Self>) {
        self.corpus = corpus;
        self.query.clear();
        self.hits.clear();
        self.status = DigestStatus::Idle;
        self.scanned = 0;
        self.total = 0;
        ctx.notify();
    }

    /// Searches every transcript in the corpus for `query`, off-thread.
    ///
    /// The caller is expected to debounce: each call re-reads whatever grew and
    /// scans every digest.
    #[cfg(not(target_family = "wasm"))]
    pub fn set_query(&mut self, query: String, ctx: &mut ModelContext<Self>) {
        let query = query.trim().to_owned();
        // An unchanged query is not merely redundant work, it is a loop:
        // finishing a pass makes the palette re-run its query, which lands back
        // here through the same debounce, which would start another pass. The
        // `Idle` exception lets a query that was too short when it was first
        // seen start a pass once the corpus arrives.
        if query == self.query && !matches!(self.status, DigestStatus::Idle) {
            return;
        }

        self.query = query.clone();
        self.hits.clear();
        self.scanned = 0;
        self.search_id += 1;
        let search_id = self.search_id;

        // Claude-only, and stated in the UI: no other agent keeps a per-cwd
        // transcript store to read.
        let corpus: Vec<DigestTarget> = self
            .corpus
            .iter()
            .filter(|target| target.agent == CLIAgent::Claude)
            .cloned()
            .collect();

        if query.chars().count() < MIN_QUERY_CHARS || corpus.is_empty() {
            self.status = DigestStatus::Idle;
            self.total = 0;
            ctx.notify();
            return;
        }

        self.status = DigestStatus::Searching;
        self.total = corpus.len();
        // Cheap despite its size: digest bodies are `Arc<str>`, so this clones
        // pointers, not text. The refreshed entries come back and are merged.
        let cache = self.digests.clone();
        ctx.notify();

        let _ = ctx.spawn(
            async move {
                let mut cache = cache;
                let mut budget = ReadBudget::new(MAX_REFRESH_READ_BYTES);
                let located = locate_targets(&corpus);
                let scanned = located.len();
                let hits = search_corpus(&located, &query, &mut cache, &mut budget, search_id);
                (hits, cache, scanned)
            },
            move |me, (hits, cache, scanned), ctx| {
                // The cache is merged even when the pass was superseded: its
                // entries are keyed by `(path, len, mtime)`, so they are true
                // regardless of which query produced them, and discarding them
                // would make every keystroke re-read the whole corpus.
                me.digests.extend(cache);
                evict_cold_digests(&mut me.digests);

                if search_id != me.search_id {
                    return;
                }
                me.hits = hits;
                me.scanned = scanned;
                me.total = scanned;
                me.status = DigestStatus::Finished;
                ctx.emit(TranscriptDigestSearchFinished);
                ctx.notify();
            },
        );
    }

    /// No Claude transcripts on disk in the browser, so there is nothing to
    /// search; the popup shows its name results only.
    #[cfg(target_family = "wasm")]
    pub fn set_query(&mut self, query: String, ctx: &mut ModelContext<Self>) {
        self.query = query.trim().to_owned();
        self.hits.clear();
        self.status = DigestStatus::Idle;
        self.scanned = 0;
        self.total = 0;
        ctx.notify();
    }
}

// Everything below is the off-thread half: pure functions over a config root, a
// path and a cache, so they can be tested against a fixture directory instead
// of the developer's real `~/.claude`.
#[cfg(not(target_family = "wasm"))]
mod digest_impl {
    use std::collections::HashMap;
    use std::fs::File;
    use std::io::{Read, Seek, SeekFrom};
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::SystemTime;

    use memchr::memmem;
    use serde_json::Value;

    use super::{ContentHit, DigestTarget, MAX_HITS};
    use crate::terminal::CLIAgent;
    use crate::terminal::cli_agent_sessions::transcript_naming::real_user_prompt_text;

    /// Most raw transcript bytes read from one file in one pass.
    ///
    /// The window is the file's **tail**, not its head: a long session's newest
    /// turns are what a "find that conversation" search is reaching for, and the
    /// tail is also what append-only growth extends, so the same bound composes
    /// with the incremental re-read instead of fighting it.
    pub(super) const MAX_FILE_READ_BYTES: u64 = 8 * 1024 * 1024;

    /// Most digested text kept for one transcript. Over this, the **oldest**
    /// text is dropped at a line boundary and the digest is flagged partial.
    pub(super) const MAX_DIGEST_BYTES: usize = 64 * 1024;

    /// Most transcripts kept in the store, evicted least-recently-searched
    /// first. The corpus is bounded per project but the store outlives any one
    /// popup, so without this it would only ever grow.
    pub(super) const MAX_CACHED_DIGESTS: usize = 512;

    /// Most raw bytes read across one whole search pass. Beyond it the
    /// remaining transcripts are skipped rather than read; a corpus that big
    /// warms over successive searches instead of stalling one.
    pub(super) const MAX_REFRESH_READ_BYTES: u64 = 96 * 1024 * 1024;

    /// Characters of context kept before the match in a snippet.
    const SNIPPET_CONTEXT_CHARS: usize = 40;

    /// Longest snippet, in characters. One line, ellipsized both ends.
    const SNIPPET_MAX_CHARS: usize = 160;

    /// The identity of a digested transcript: the file, how long it was, and
    /// when it was last written.
    ///
    /// All three components matter. `path` is which file; `modified` makes a
    /// rewritten transcript miss rather than serve stale text; `len` is what
    /// makes growth append-only, because a longer file with the same path is a
    /// file whose new bytes are exactly `[old_len, EOF)`.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(super) struct DigestKey {
        pub(super) path: PathBuf,
        pub(super) len: u64,
        pub(super) modified: SystemTime,
    }

    /// One transcript's digested conversation text.
    #[derive(Clone)]
    pub(super) struct CachedDigest {
        pub(super) key: DigestKey,
        /// Shared so snapshotting the whole store into a spawned task copies
        /// pointers rather than megabytes.
        pub(super) text: Arc<str>,
        pub(super) partial: bool,
        /// The search that last used this entry, for eviction.
        pub(super) last_used: u64,
    }

    /// Digested transcripts, keyed by path so a grown file can find its own
    /// previous digest before its key is compared.
    pub(super) type DigestCache = HashMap<PathBuf, CachedDigest>;

    /// One target with its transcript located on disk.
    pub(super) struct LocatedTarget {
        pub(super) target: DigestTarget,
        pub(super) path: PathBuf,
    }

    /// Bytes still readable in this pass, and how many have been read.
    ///
    /// The counter is what the "a grown file re-reads only the tail" test
    /// asserts on: the property is invisible in the results, since a full
    /// re-read would produce the same hits.
    pub(super) struct ReadBudget {
        pub(super) remaining: u64,
        pub(super) read: u64,
    }

    impl ReadBudget {
        pub(super) fn new(remaining: u64) -> Self {
            Self { remaining, read: 0 }
        }
    }

    /// Resolves each target's transcript path.
    ///
    /// Claude files a session under the **realpath**, so the cwd is
    /// canonicalized before it is encoded — the same rule `session_scan`
    /// follows, and the reason a symlinked checkout is not missed. Resolved
    /// once per distinct directory, because canonicalization is I/O and a
    /// corpus has far more sessions than directories.
    pub(super) fn locate_targets(targets: &[DigestTarget]) -> Vec<LocatedTarget> {
        use crate::ai::agent_sdk::driver::harness::claude_transcript::{
            claude_config_dir, encode_cwd,
        };

        let Ok(config_root) = claude_config_dir() else {
            return Vec::new();
        };
        let mut project_dir_by_cwd: HashMap<String, PathBuf> = HashMap::new();
        let mut located = Vec::with_capacity(targets.len());
        for target in targets {
            if target.agent != CLIAgent::Claude {
                continue;
            }
            let project_dir = project_dir_by_cwd
                .entry(target.cwd.clone())
                .or_insert_with(|| {
                    let cwd = PathBuf::from(&target.cwd);
                    let canonical = std::fs::canonicalize(&cwd).unwrap_or(cwd);
                    config_root.join("projects").join(encode_cwd(&canonical))
                });
            located.push(LocatedTarget {
                target: target.clone(),
                path: project_dir.join(format!("{}.jsonl", target.session_id)),
            });
        }
        located
    }

    /// Refreshes every target's digest and returns the ones containing `query`.
    ///
    /// A pure function of `(targets, query, cache, budget)`, which is what lets
    /// the tests point it at a fixture directory.
    pub(super) fn search_corpus(
        targets: &[LocatedTarget],
        query: &str,
        cache: &mut DigestCache,
        budget: &mut ReadBudget,
        search_id: u64,
    ) -> Vec<ContentHit> {
        // ASCII-lowercased on both sides so the byte offset a match lands on
        // maps straight back into the original text for the snippet.
        // Case-folding beyond ASCII is a deliberate non-goal: it changes byte
        // lengths, and this search exists for ids, paths and error fragments.
        let needle = query.to_ascii_lowercase();
        let mut hits = Vec::new();
        for target in targets {
            if hits.len() >= MAX_HITS {
                break;
            }
            let Some((text, partial)) = refresh_digest(&target.path, cache, budget, search_id)
            else {
                continue;
            };
            if let Some(hit) = find_hit(&target.target, &text, partial, &needle) {
                hits.push(hit);
            }
        }
        hits
    }

    /// The digest for `path`, reading only what has changed since last time.
    ///
    /// Returns `None` — **without caching anything** — when the file cannot be
    /// read or the pass has no budget left for it. That is the "errors are
    /// never memoized" rule: a transcript that was mid-rename, on a stalled
    /// network mount, or momentarily gone must be retried by the next search,
    /// not remembered as empty for the rest of the session.
    pub(super) fn refresh_digest(
        path: &Path,
        cache: &mut DigestCache,
        budget: &mut ReadBudget,
        search_id: u64,
    ) -> Option<(Arc<str>, bool)> {
        let mut file = File::open(path).ok()?;
        let metadata = file.metadata().ok()?;
        let len = metadata.len();
        let modified = metadata.modified().ok()?;
        let key = DigestKey {
            path: path.to_path_buf(),
            len,
            modified,
        };

        if let Some(cached) = cache.get_mut(path)
            && cached.key == key
        {
            cached.last_used = search_id;
            return Some((cached.text.clone(), cached.partial));
        }

        // Append-only growth: same file, longer than what was digested, so the
        // only new bytes are `[old_len, EOF)`. A file that shrank was rewritten
        // and a file that grew by more than one window's worth is cheaper to
        // re-window than to stitch, so both fall through to a fresh tail read.
        let grown_from = cache
            .get(path)
            .filter(|cached| {
                cached.key.path == key.path
                    && cached.key.len < len
                    && len - cached.key.len <= MAX_FILE_READ_BYTES
            })
            .map(|cached| (cached.key.len, cached.text.to_string(), cached.partial));

        let (start, mut text, mut partial) = match grown_from {
            Some((old_len, text, partial)) => (old_len, text, partial),
            None => {
                let start = len.saturating_sub(MAX_FILE_READ_BYTES);
                (start, String::new(), start > 0)
            }
        };

        let chunk = read_range(&mut file, start, len - start, budget)?;
        let extracted = extract_content(&chunk);
        if !extracted.is_empty() {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&extracted);
        }
        let (text, trimmed) = trim_digest_front(text);
        partial |= trimmed;

        let text: Arc<str> = Arc::from(text);
        cache.insert(
            path.to_path_buf(),
            CachedDigest {
                key,
                text: text.clone(),
                partial,
                last_used: search_id,
            },
        );
        Some((text, partial))
    }

    /// Reads `wanted` bytes from `start`, charging the pass budget.
    ///
    /// Returns `None` rather than a short read when the budget cannot cover the
    /// whole range: a truncated body cached under a complete key would be
    /// indistinguishable from a real digest forever after.
    fn read_range(
        file: &mut File,
        start: u64,
        wanted: u64,
        budget: &mut ReadBudget,
    ) -> Option<String> {
        if wanted > budget.remaining {
            return None;
        }
        file.seek(SeekFrom::Start(start)).ok()?;
        let mut buffer = Vec::new();
        // `take` consumes the reader, so borrow it rather than the file itself.
        let read = file.take(wanted).read_to_end(&mut buffer).ok()?;
        budget.remaining = budget.remaining.saturating_sub(read as u64);
        budget.read += read as u64;
        // Lossy, not strict: a window can begin inside a multi-byte character,
        // and a live transcript's last line can be torn mid-write.
        Some(String::from_utf8_lossy(&buffer).into_owned())
    }

    /// The conversation text inside a slice of transcript JSONL.
    ///
    /// One line at a time, `serde_json` in try/continue: the first line of a
    /// tail window is normally cut mid-record and the last line of a live
    /// transcript routinely is too. Neither may abort the file.
    pub(super) fn extract_content(chunk: &str) -> String {
        let mut content = String::new();
        for line in chunk.lines() {
            // Cheap pre-filter, as `transcript_naming` does: only user and
            // assistant envelopes carry anything anyone said.
            if !line.contains("\"user\"") && !line.contains("\"assistant\"") {
                continue;
            }
            let Ok(record) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            let text = match record.get("type").and_then(Value::as_str) {
                Some("user") => user_turn_text(&record),
                Some("assistant") => assistant_prose(&record),
                // Everything else is excluded on purpose: `attachment` records
                // are the pasted file contents, `system` and `file-history-*`
                // are bookkeeping, and `ai-title` is already searchable as the
                // session's name.
                Some(_) | None => None,
            };
            let Some(text) = text else {
                continue;
            };
            if !content.is_empty() {
                content.push('\n');
            }
            content.push_str(&text);
        }
        content
    }

    /// What the user typed in a `user` record, or `None` when the record is
    /// Claude feeding itself a tool result.
    fn user_turn_text(record: &Value) -> Option<String> {
        if is_tool_result(record) {
            return None;
        }
        // Reused rather than reimplemented: this is the same "what did the user
        // actually type" question the rail asks when naming a session, with
        // sidechain replays and Claude's injected wrappers already rejected.
        real_user_prompt_text(record)
    }

    /// Whether a `user` envelope is a tool result rather than a person.
    ///
    /// Structural, per Orbit's rule: the content block carries a `tool_use_id`
    /// naming the call it answers. Nothing here matches on text. (The extractor
    /// would skip these anyway — a `tool_result` block has no `text` field for
    /// `user_prompt_text` to find — but that is a coincidence of shape, and the
    /// rule this search is specified by should be visible in the code.)
    fn is_tool_result(record: &Value) -> bool {
        record
            .get("message")
            .and_then(|message| message.get("content"))
            .and_then(Value::as_array)
            .is_some_and(|blocks| {
                blocks
                    .iter()
                    .any(|block| block.get("tool_use_id").is_some())
            })
    }

    /// The assistant's prose in an `assistant` record.
    ///
    /// `text` blocks only. A `tool_use` block is the *arguments* of a call —
    /// whole files, patches, command lines — and `thinking` is not what the
    /// assistant said; including either is how a digest turns back into the
    /// transcript it exists to avoid reading.
    fn assistant_prose(record: &Value) -> Option<String> {
        let blocks = record.get("message")?.get("content")?.as_array()?;
        let mut prose = String::new();
        for block in blocks {
            if block.get("type").and_then(Value::as_str) != Some("text") {
                continue;
            }
            let Some(text) = block.get("text").and_then(Value::as_str) else {
                continue;
            };
            if !prose.is_empty() {
                prose.push('\n');
            }
            prose.push_str(text);
        }
        (!prose.is_empty()).then_some(prose)
    }

    /// Drops the oldest text once a digest outgrows [`MAX_DIGEST_BYTES`].
    ///
    /// From the front, so what is kept is the newest conversation — the same
    /// recency argument as the tail read window.
    fn trim_digest_front(text: String) -> (String, bool) {
        if text.len() <= MAX_DIGEST_BYTES {
            return (text, false);
        }
        let mut cut = text.len() - MAX_DIGEST_BYTES;
        while cut < text.len() && !text.is_char_boundary(cut) {
            cut += 1;
        }
        // Cut at a line boundary so a turn is never half-kept and a snippet can
        // never start mid-sentence for no visible reason.
        let cut = text[cut..].find('\n').map_or(text.len(), |at| cut + at + 1);
        (text[cut..].to_owned(), true)
    }

    /// The hit for one target, if its digest contains the (lowercased) needle.
    fn find_hit(
        target: &DigestTarget,
        text: &str,
        partial: bool,
        needle: &str,
    ) -> Option<ContentHit> {
        let haystack = text.to_ascii_lowercase();
        let at = memmem::find(haystack.as_bytes(), needle.as_bytes())?;
        // ASCII-lowercasing preserves byte length and UTF-8 is
        // self-synchronizing, so a match is always aligned — but a corrupt
        // digest must not be able to panic the palette.
        if !text.is_char_boundary(at) {
            return None;
        }
        let (snippet, snippet_match_indices) = snippet_around(text, at, needle.len());
        Some(ContentHit {
            agent: target.agent,
            session_id: target.session_id.clone(),
            project_name: target.project_name.clone(),
            task_name: target.task_name.clone(),
            cwd: target.cwd.clone(),
            snippet,
            snippet_match_indices,
            partial,
        })
    }

    /// One line of context around the match, ellipsized, plus the **char**
    /// indices of the match within it.
    ///
    /// One line because a palette row is one line: a digest keeps a turn's own
    /// newlines, so the enclosing line is the natural unit and a long pasted
    /// line is cut around the match rather than from its start.
    pub(super) fn snippet_around(text: &str, at: usize, len: usize) -> (String, Vec<usize>) {
        let line_start = text[..at].rfind('\n').map_or(0, |newline| newline + 1);
        let line_end = text[at..]
            .find('\n')
            .map_or(text.len(), |newline| at + newline);
        let line = &text[line_start..line_end];
        let match_start = at - line_start;
        let match_end = (match_start + len).min(line.len());

        let window_start = line[..match_start]
            .char_indices()
            .rev()
            .nth(SNIPPET_CONTEXT_CHARS - 1)
            .map_or(0, |(index, _)| index);
        let tail = &line[window_start..];
        let body: String = tail.chars().take(SNIPPET_MAX_CHARS).collect();
        let truncated = tail.chars().count() > SNIPPET_MAX_CHARS;

        let mut snippet = String::new();
        if window_start > 0 {
            snippet.push('…');
        }
        snippet.push_str(&body);
        if truncated {
            snippet.push('…');
        }

        let highlight_start =
            usize::from(window_start > 0) + line[window_start..match_start].chars().count();
        let highlight_len = line[match_start..match_end].chars().count();
        let body_chars = usize::from(window_start > 0) + body.chars().count();
        let indices = (highlight_start..highlight_start + highlight_len)
            .filter(|index| *index < body_chars)
            .collect();
        (snippet, indices)
    }

    /// Evicts the least-recently-searched digests once the store is over bound.
    pub(super) fn evict_cold_digests(cache: &mut DigestCache) {
        if cache.len() <= MAX_CACHED_DIGESTS {
            return;
        }
        let mut stamps: Vec<u64> = cache.values().map(|cached| cached.last_used).collect();
        stamps.sort_unstable();
        let cutoff = stamps[cache.len() - MAX_CACHED_DIGESTS];
        cache.retain(|_, cached| cached.last_used >= cutoff);
    }
}

#[cfg(not(target_family = "wasm"))]
use digest_impl::{
    DigestCache, MAX_REFRESH_READ_BYTES, ReadBudget, evict_cold_digests, locate_targets,
    search_corpus,
};

#[cfg(all(test, not(target_family = "wasm")))]
#[path = "transcript_digest_tests.rs"]
mod tests;
