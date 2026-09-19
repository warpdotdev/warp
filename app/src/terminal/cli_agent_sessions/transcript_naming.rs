//! Conversation-name candidates from a Claude Code transcript.
//!
//! The project rail names a task by its conversation, not its directory. For a
//! dormant task (agent exited, or the app restarted) the only surviving source
//! is the transcript on disk, so this module derives display candidates from it.
//!
//! # Why the newest name in a transcript cannot be trusted on its own
//!
//! Claude Code's `/rename` **broadcasts**: it writes the new name into the
//! transcripts of other sessions that happen to be live at the same time.
//! Measured on this machine, the name from a single `/rename`
//! ("unified-trading-handoff-setup") is the last `aiTitle` record in 13
//! transcripts spread across 6 project directories, which is exactly the "many
//! rail rows, one name" bug. The contaminated records carry the *file's own*
//! `sessionId`, so no per-record validation can spot them.
//!
//! What does separate a broadcast from a real rename is **uniqueness, not
//! position**: across 159 titled transcripts, 44 changed title at least once —
//! 29 to a value no other session claims (a genuine `/rename`, e.g.
//! "project-rail-task-status") and 15 to a value shared with other sessions
//! (contamination). Always taking the first title would therefore destroy 29
//! correct names to fix 15. So this module no longer picks the name: it reports
//! every candidate the transcript offers and the caller — the only thing with a
//! view over *all* sessions — applies the rule ([`TranscriptNames::resolve`]).
//!
//! Candidates, best first:
//!
//! 1. [`last_title`](TranscriptNames::last_title) — the newest
//!    `{"type":"ai-title","aiTitle":…}`, found by a **tail** read: Claude
//!    appends a fresh record every turn and `/rename` appends one at the very
//!    end, so only reading from the end can see a rename. Correct for a genuine
//!    rename; another session's name after a broadcast.
//! 2. [`first_title`](TranscriptNames::first_title) — the oldest `ai-title`,
//!    from a **head** read. In all 13 contaminated files measured, the first
//!    title was still correct and session-specific ("Import client repayment
//!    data into MIFOS", …).
//! 3. [`prompt`](TranscriptNames::prompt) — the first real user prompt. Written
//!    once at session start and never rewritten, so it cannot be cross-written
//!    by another session: the trustworthy floor.
//! 4. [`slug`](TranscriptNames::slug) — the transcript's own slug, de-kebabed.
//!    Last resort before no name at all.
//!
//! `agent-name` / `agentName` is deliberately **not** a source. It is not a
//! mirror of `aiTitle`: one measured file carried `aiTitle` "Set up UAT test
//! data for customer grade scenarios" while its `agentName` simultaneously held
//! a completely different session's name. It was contaminated in 13 of 13 files
//! and never contributed a name `aiTitle` did not already have, so reading it
//! can only ever produce a wrong name.
//!
//! Junk names are rejected rather than displayed: Claude's auto-generated
//! `<dir>-<2hex>` display name (its own docs say it is not a resume handle),
//! bare hex blobs, whitespace, and the truncated cwd that produced the
//! original "six rows all reading `..uellig/repos/poa-agent`" bug.
//!
//! Reads are **bounded** (64 KiB tail + 256 KiB head, and a single read when
//! the whole file fits in the tail window) and belong off the render path:
//! callers resolve in a spawned task and cache the result
//! (`AgentSessionHandleOp::SetTitle`, or `session_scan`'s per-mtime memo);
//! nothing here may run inside element layout. The corpus is multi-gigabyte —
//! no code path may ever read a whole transcript.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::ai::agent_sdk::driver::harness::claude_transcript::{claude_config_dir, encode_cwd};

/// Head-read budget. Large enough that the first prompt and the first
/// `ai-title` are always inside it; small enough to be harmless on a slow
/// disk. (Matches the bound Orbit settled on for the same job.)
const HEAD_READ_BYTES: usize = 256 * 1024;

/// Tail-read budget. A `/rename` lands as the *last* `ai-title` record, and
/// Claude re-emits one every turn, so the newest name is always within a few
/// kilobytes of EOF; 64 KiB is slack for a turn carrying large tool output.
const TAIL_READ_BYTES: u64 = 64 * 1024;

/// Maximum length of a derived label; longer candidates are ellipsized.
const MAX_LABEL_LEN: usize = 80;

/// Prefix Claude injects ahead of replayed context. Never a name.
const CAVEAT_PREFIX: &str = "Caveat:";

/// Every naming candidate one transcript offers, already tidied and
/// junk-filtered — a `Some` here is always a string worth displaying.
///
/// Deliberately *not* collapsed to a single name: choosing between
/// [`last_title`](Self::last_title) and [`first_title`](Self::first_title)
/// requires knowing what other sessions are called, which is knowledge this
/// module (one file, one read) does not and should not have.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TranscriptNames {
    /// Newest `ai-title` in the file. The genuine name after a `/rename` —
    /// or another session's name after a broadcast.
    pub last_title: Option<String>,
    /// Oldest `ai-title` in the file. Immune to the broadcast in every
    /// contaminated file measured.
    pub first_title: Option<String>,
    /// First real user prompt: written once, at session start.
    pub prompt: Option<String>,
    /// The transcript's `slug`, de-kebabed.
    pub slug: Option<String>,
}

impl TranscriptNames {
    /// The display label for this session, applying the uniqueness rule.
    ///
    /// `is_unique` answers "does no *other* session claim this exact title" —
    /// the caller owns that view. A title only one session claims is that
    /// session's own name (the 29 genuine renames); a title several claim is a
    /// broadcast and is dropped in favour of the next candidate, ending at the
    /// prompt, which cannot be cross-written.
    pub fn resolve(&self, is_unique: impl Fn(&str) -> bool) -> Option<String> {
        if let Some(last) = &self.last_title
            && is_unique(last)
        {
            return Some(last.clone());
        }
        if let Some(first) = &self.first_title
            && is_unique(first)
        {
            return Some(first.clone());
        }
        self.prompt.clone().or_else(|| self.slug.clone())
    }

    /// The label to show when uniqueness is unknowable — no scan has run, or it
    /// has not reached this session's directory, so the sibling transcripts a
    /// broadcast would have contaminated have not been read.
    ///
    /// Skips [`last_title`](Self::last_title) rather than trusting it: it was
    /// the corrupt field in 13 of 13 measured files, while `first_title` was
    /// correct in all 13, so the first title is the safer of the two whenever
    /// we cannot tell them apart.
    pub fn resolve_without_uniqueness(&self) -> Option<String> {
        self.first_title
            .clone()
            .or_else(|| self.prompt.clone())
            .or_else(|| self.slug.clone())
    }

    /// The titles this transcript asserts, for a caller building the
    /// title -> claiming sessions map.
    ///
    /// Only the two `ai-title` candidates: those are the records a broadcast
    /// writes. The prompt is excluded on purpose — it is the floor precisely
    /// because it cannot be cross-written, and two sessions legitimately
    /// starting from the same short instruction must not disqualify each other.
    pub fn claimed_titles(&self) -> impl Iterator<Item = &str> {
        [self.last_title.as_deref(), self.first_title.as_deref()]
            .into_iter()
            .flatten()
    }

    /// Whether the transcript yielded nothing at all — a normal outcome for a
    /// tiny or truncated session, and the signal callers use to avoid
    /// memoising a read that could succeed later.
    pub fn is_empty(&self) -> bool {
        self.last_title.is_none()
            && self.first_title.is_none()
            && self.prompt.is_none()
            && self.slug.is_none()
    }
}

/// Where Claude Code stores the transcript for `session_id` started in `cwd`:
/// `<config>/projects/<encoded-cwd>/<session_id>.jsonl`. Derived, not awaited:
/// the hook only reports `transcript_path` on `stop`, but `cwd` + session id
/// arrive on every event.
pub fn claude_transcript_path(cwd: &Path, session_id: &str) -> Option<PathBuf> {
    Some(claude_project_dir(cwd)?.join(format!("{session_id}.jsonl")))
}

/// The directory holding every transcript Claude Code recorded for `cwd`.
///
/// `cwd` is used as given; callers that have a real directory should
/// canonicalize first (Claude files a session under the resolved realpath, so
/// a symlinked checkout would otherwise miss). Canonicalization is I/O and is
/// deliberately left to the caller's background scan rather than baked in
/// here, where this is also called from paths that only have a remembered
/// string.
pub fn claude_project_dir(cwd: &Path) -> Option<PathBuf> {
    Some(
        claude_config_dir()
            .ok()?
            .join("projects")
            .join(encode_cwd(cwd)),
    )
}

/// Reads every naming candidate for the session behind `transcript_path`.
///
/// An unreadable file yields an empty [`TranscriptNames`] — a normal outcome
/// (deleted transcript, tiny session), never an error.
pub fn read_transcript_names(transcript_path: &Path, cwd: &Path) -> TranscriptNames {
    // Tail first: it is the only read that can see a `/rename`, and it is the
    // smaller of the two.
    let Some(tail) = read_tail(transcript_path) else {
        return TranscriptNames::default();
    };
    if tail.covers_whole_file {
        // That one seek already read the entire transcript, so a head read
        // would be duplicated I/O for bytes we are holding.
        return names_from_transcript_text(&tail.text, cwd);
    }

    let mut names = read_head(transcript_path)
        .map(|head| names_from_transcript_text(&head, cwd))
        .unwrap_or_default();
    // The tail's own newest title wins when it has one: it is the only tier
    // that can see a `/rename`. When the tail window holds no title at all — a
    // session named before a very long tool loop — the head read's last title
    // is left standing, which is closer to the newest name than nothing.
    if let Some(newest) = last_title_from_tail(&tail.text, cwd) {
        names.last_title = Some(newest);
    }
    names
}

/// A tail read plus whether it happened to cover the file from byte 0, which
/// is what lets a short transcript be parsed from a single read.
struct TailRead {
    text: String,
    covers_whole_file: bool,
}

/// Reads at most [`TAIL_READ_BYTES`] ending at EOF.
///
/// The *first* line of the slice is normally cut mid-record; the per-line
/// parse below skips it. No locking: Claude appends without one, so a torn
/// final line is expected and handled the same way.
fn read_tail(transcript_path: &Path) -> Option<TailRead> {
    let mut file = File::open(transcript_path).ok()?;
    let len = file.metadata().ok()?.len();
    file.seek(SeekFrom::Start(len.saturating_sub(TAIL_READ_BYTES)))
        .ok()?;
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).ok()?;
    Some(TailRead {
        // Lossy, not strict: the seek can land inside a multi-byte character.
        text: String::from_utf8_lossy(&buffer).into_owned(),
        covers_whole_file: len <= TAIL_READ_BYTES,
    })
}

/// Reads at most [`HEAD_READ_BYTES`] from offset 0.
fn read_head(transcript_path: &Path) -> Option<String> {
    let mut buffer = vec![0_u8; HEAD_READ_BYTES];
    let read = File::open(transcript_path)
        .and_then(|mut file| file.read(&mut buffer))
        .ok()?;
    buffer.truncate(read);
    Some(String::from_utf8_lossy(&buffer).into_owned())
}

/// Pure core of the tail tier: the newest acceptable title in `tail`.
///
/// Walks backwards so a `/rename` — appended last — wins over the titles
/// Claude emits every turn, and so a long tail costs one reversed scan rather
/// than a full parse.
fn last_title_from_tail(tail: &str, cwd: &Path) -> Option<String> {
    tail.lines()
        .rev()
        .filter_map(title_record_text)
        .map(|candidate| tidy(&candidate))
        .find(|candidate| is_acceptable_label(candidate, cwd))
}

/// Pure core of the bounded read: every candidate carried by `text`, in one
/// pass.
///
/// Both the first and the last title are collected. (This used to keep only the
/// last, on the reasoning that it was "strictly closer to the newest name" —
/// the measurement above overturns that: the last title is the field a
/// `/rename` broadcast corrupts, in 13 of 13 files, and the first is the field
/// that survived in all 13.)
///
/// The final line of a bounded read is usually truncated mid-record; malformed
/// lines are skipped, never treated as errors.
fn names_from_transcript_text(text: &str, cwd: &Path) -> TranscriptNames {
    let mut names = TranscriptNames::default();

    for line in text.lines() {
        // Cheap substring pre-filters keep serde parsing off most lines.
        let looks_like_title = line.contains("\"ai-title\"");
        let looks_like_user = names.prompt.is_none() && line.contains("\"user\"");
        let looks_like_slug = names.slug.is_none() && line.contains("\"slug\"");
        if !looks_like_title && !looks_like_user && !looks_like_slug {
            continue;
        }
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if names.slug.is_none()
            && let Some(text) = record.get("slug").and_then(Value::as_str)
        {
            // De-kebab: the slug is a filename-safe rendering of a phrase.
            names.slug = acceptable(&text.replace('-', " "), cwd);
        }
        match record.get("type").and_then(Value::as_str) {
            Some("ai-title") => {
                if let Some(title) = title_text(&record).and_then(|title| acceptable(&title, cwd)) {
                    // Junk titles never occupy either slot, so a rejected first
                    // record does not cost the session its real first title.
                    names.first_title.get_or_insert_with(|| title.clone());
                    names.last_title = Some(title);
                }
            }
            Some("user") => {
                if names.prompt.is_none()
                    && let Some(text) = real_user_prompt_text(&record)
                {
                    names.prompt = acceptable(&text, cwd);
                }
            }
            // A leading `summary` record describes the *pre-compaction parent*
            // conversation, not this one, so it is never a name source.
            // `agent-name` is listed here to be explicit that it is ignored:
            // see the module header — it is contaminated, not a mirror of
            // `aiTitle`, and reading it is what made unrelated sessions share a
            // name. Listed rather than matched by wildcard so a new record type
            // that does carry a name forces a decision here.
            Some("agent-name") | Some(_) | None => {}
        }
    }

    names
}

/// The title carried by a single transcript line, if it is a title record.
/// Used by the reversed tail walk, which has no state to accumulate.
fn title_record_text(line: &str) -> Option<String> {
    if !line.contains("\"ai-title\"") {
        return None;
    }
    let record = serde_json::from_str::<Value>(line).ok()?;
    match record.get("type").and_then(Value::as_str) {
        Some("ai-title") => title_text(&record),
        Some(_) | None => None,
    }
}

/// The name field of a title record.
fn title_text(record: &Value) -> Option<String> {
    record
        .get("aiTitle")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// Prompt text from a `user` record, or `None` when this record is not the
/// user's own words.
///
/// Rejected, so the scan moves on to the *next* user record rather than
/// giving up on the tier: subagent sidechain replays, Claude's injected
/// wrappers (`<command-name>…`, `<local-command-stdout>…`), and the
/// `Caveat:` preamble it prepends to replayed context. All three are
/// interstitial — there is always a real prompt after them.
fn real_user_prompt_text(record: &Value) -> Option<String> {
    if record.get("isSidechain").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let text = user_prompt_text(record)?;
    let trimmed = text.trim();
    if trimmed.starts_with('<') || trimmed.starts_with(CAVEAT_PREFIX) {
        return None;
    }
    Some(text)
}

/// Extracts prompt text from a `user` record. `message.content` is either a
/// plain string or an array of content blocks with `text` fields.
fn user_prompt_text(record: &Value) -> Option<String> {
    let content = record.get("message")?.get("content")?;
    match content {
        Value::String(text) => Some(text.clone()),
        Value::Array(blocks) => blocks
            .iter()
            .find_map(|block| block.get("text").and_then(Value::as_str))
            .map(str::to_owned),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::Object(_) => None,
    }
}

/// A candidate tidied for display, or `None` when it is not worth showing.
/// Applied at capture so every slot of [`TranscriptNames`] holds a usable
/// string and no tier has to re-check.
fn acceptable(candidate: &str, cwd: &Path) -> Option<String> {
    let tidied = tidy(candidate);
    is_acceptable_label(&tidied, cwd).then_some(tidied)
}

/// Collapses whitespace and ellipsizes to [`MAX_LABEL_LEN`].
fn tidy(candidate: &str) -> String {
    let collapsed = candidate.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= MAX_LABEL_LEN {
        return collapsed;
    }
    let truncated: String = collapsed.chars().take(MAX_LABEL_LEN - 1).collect();
    format!("{}…", truncated.trim_end())
}

/// Whether a candidate is worth showing over the caller's floor.
///
/// Rejects empty/whitespace, Claude's auto-generated `<cwd-basename>-<2hex>`
/// display name, anything that is just the directory name again — the rail
/// exists to replace path-derived labels, not to relay them — and bare hex
/// blobs, which are ids leaking into a name slot.
fn is_acceptable_label(candidate: &str, cwd: &Path) -> bool {
    if candidate.is_empty() {
        return false;
    }
    if is_hex_blob(candidate) {
        return false;
    }
    let Some(basename) = cwd.file_name().and_then(|name| name.to_str()) else {
        return true;
    };
    if candidate.eq_ignore_ascii_case(basename) {
        return false;
    }
    // Claude's junk default: "<basename>-<2 hex chars>", e.g. "poa-agent-0f".
    if let Some(suffix) = candidate
        .strip_prefix(basename)
        .and_then(|rest| rest.strip_prefix('-'))
        && suffix.len() == 2
        && suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return false;
    }
    true
}

/// Whether a candidate is nothing but hex (ignoring hyphens) — a session id,
/// a commit sha or a uuid fragment that reached a name slot. Bounded at eight
/// digits so real words that happen to be hex ("added", "beef") still pass.
fn is_hex_blob(candidate: &str) -> bool {
    let digits = candidate.replace('-', "");
    digits.len() >= 8 && digits.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
#[path = "transcript_naming_tests.rs"]
mod tests;
