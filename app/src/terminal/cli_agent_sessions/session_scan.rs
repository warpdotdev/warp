//! Discovery of Claude Code sessions from Claude's **own** on-disk state.
//!
//! The durable handle store only knows sessions Warp itself spawned and
//! witnessed a plugin event for. A session started from another terminal, or
//! imported with a profile, has no row there at all — so the rail used to show
//! a path-derived label for work it could not even see. This module scans
//! `~/.claude/projects/<encoded-cwd>/*.jsonl` so every session recorded in a
//! project's directory can be listed and named, witnessed or not.
//!
//! # Authority split (invariant)
//!
//! The two sources are authoritative for different things, and neither may
//! encroach on the other:
//!
//! - **The handle store owns pane binding.** It is the only thing that knows
//!   which `terminal_panes.uuid` ran a session, so it is the only thing that
//!   may resume one *in place*. A scanned session must **never** fabricate a
//!   pane or tab association: Warp never saw it run, so it has no pane. Scanned
//!   sessions therefore render as project-level resumable rows only
//!   ([`DormantTaskOrigin::Scanned`](crate::workspace::project_layout::DormantTaskOrigin)),
//!   and resume by opening a fresh tab at the scanned directory. This is
//!   enforced structurally rather than by a check: the resume path derives its
//!   "resume in place" pane from a handle lookup, and an unwitnessed session
//!   has no handle to look up.
//! - **The disk scan owns names and existence.** The transcript is where
//!   `/rename` lands, and it lands *after* spawn, so for names the scan is
//!   fresher than any cached handle title — a handle-bound row prefers the
//!   scanned name when the scan has one. It is also the only evidence that an
//!   unwitnessed session exists at all.
//!
//! The join key between them is the session UUID.
//!
//! # Cost
//!
//! Discovery is stat-only (one `read_dir` plus one `metadata` per entry);
//! names come from [`transcript_naming`](super::transcript_naming)'s bounded
//! tail+head reads, memoised per `(path, mtime)` so an unchanged transcript is
//! read exactly once per run. Every filesystem call is individually
//! error-tolerant: transcripts are pruned concurrently by Claude itself, and a
//! vanished file is a normal outcome, never an error.
//!
//! Nothing here may run on the render path — [`ClaudeSessionScanModel::refresh`]
//! does the work in a spawned task and the rail reads only the cached result.
//!
//! # Known gaps
//!
//! - **Sessions that `cd`-ed away are missed.** A session started in `P` that
//!   moved elsewhere is filed under a *sibling* directory whose encoded name
//!   begins with `<encoded P>-`. Measured on this machine: 1 of 47 populated
//!   directories, a session that entered a `.claude/worktrees/…` checkout.
//!   Recovering it means confirming each sibling's own `cwd` head field, which
//!   is an unbounded head-read fan-out across siblings for one rail label —
//!   not worth it until a project is observed losing rows to it. Note the
//!   fallback is never a *wrong* row, only a missing one: a directory name is
//!   deliberately never reverse-mapped back to a cwd, because the encoding is
//!   lossy and would guess wrong on any path whose separators encode like `-`.
//! - **A session that is live but not yet identified can appear as a row.**
//!   Between Warp spawning `claude` and the plugin reporting the session id,
//!   Claude has already written the transcript, but the id is in neither
//!   `live_session_ids` (which keys on the reported id) nor the handle mirror
//!   (which does not mirror in-flight rows). The row is harmless — resume is
//!   prefill-never-executed, so the user sees the command before anything runs
//!   — and it disappears on the next scan once the handle is identified.
//! - **`~/.claude/sessions/<pid>.json` is not consulted.** That file names
//!   *running* sessions, and every row here is by construction one Warp is not
//!   running; there is also no pid to join on for a session Warp never spawned.

// Only the scanning half of this module needs a map; the browser build keeps
// the plain data types and a no-op refresh.
#[cfg(not(target_family = "wasm"))]
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use warpui::{Entity, ModelContext, SingletonEntity};

#[cfg(not(target_family = "wasm"))]
use super::transcript_naming::TranscriptNames;

/// How many sessions a single directory contributes, newest first.
///
/// Bounds the naming I/O of the first scan of a long-lived project (some
/// directories here hold hundreds of transcripts) while leaving generous
/// headroom over the rail's own row cap, so live and already-witnessed
/// sessions filtered out downstream do not starve the list.
#[cfg(not(target_family = "wasm"))]
const MAX_SCANNED_SESSIONS_PER_DIR: usize = 16;

/// Minimum gap between two scans of the same directory.
///
/// The rail recomputes on its existing cadence and the kick sites are user
/// actions, so this exists only to stop a burst of actions from re-reading the
/// same directories. Deliberately no watcher and no timer: a session started
/// elsewhere appears on the next project interaction, not instantly.
#[cfg(not(target_family = "wasm"))]
const RESCAN_INTERVAL: std::time::Duration = std::time::Duration::from_secs(20);

/// Name *candidates* for one directory, keyed by the transcript they came from
/// and that file's mtime. Including the mtime is what makes a stale entry
/// impossible: a rewritten transcript simply misses the memo and is re-read.
///
/// Candidates rather than resolved labels: which candidate wins depends on what
/// *other* sessions are called ([`resolve_labels`]), so a memoised label would
/// pin a decision that a newly scanned sibling can legitimately overturn
/// without this file changing at all.
#[cfg(not(target_family = "wasm"))]
pub type NameMemo = HashMap<(PathBuf, SystemTime), TranscriptNames>;

/// What one directory scan produces: the rows, and the **complete** memo for
/// the sessions it listed.
///
/// Complete rather than incremental so the caller can replace that directory's
/// entries wholesale. An append-only memo would accumulate one dead entry per
/// transcript write — an active session rewrites its transcript every turn —
/// and nothing would ever evict them.
#[cfg(not(target_family = "wasm"))]
pub type DirectoryScan = (Vec<ScannedSession>, NameMemo);

/// One Claude Code session found on disk.
///
/// Carries no pane, tab or handle: by construction this is what Claude
/// recorded, not what Warp ran. See the module's authority split.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannedSession {
    /// The transcript's filename stem, already validated as a UUID — which is
    /// what makes it safe to put in a `claude --resume` command line.
    pub session_id: String,
    /// The project directory that was scanned, as the caller supplied it (not
    /// the canonicalized form used to locate the transcript), so it can be
    /// bucketed by exactly the same path the rest of the rail buckets by.
    pub cwd: String,
    /// Everything the transcript offers as a name. Kept alongside the resolved
    /// label because the resolution depends on the other rows and is redone
    /// whenever the set of scanned sessions changes.
    #[cfg(not(target_family = "wasm"))]
    pub names: TranscriptNames,
    /// Resolved conversation name, or `None` when the transcript yielded no
    /// acceptable one.
    ///
    /// Assigned by [`resolve_labels`] over *all* scanned directories, never by
    /// the directory scan alone: a broadcast `/rename` corrupts transcripts in
    /// sibling projects, so the evidence that a title is not this session's own
    /// is usually in a different directory.
    pub label: Option<String>,
    /// Transcript mtime; the rail orders scanned rows newest-first by it.
    pub modified: SystemTime,
}

/// The session id encoded in a transcript filename, if it is one.
///
/// The filter is an **anchored** UUID plus the `.jsonl` extension because a
/// project directory also holds `<uuid>/` *subdirectories* (session memory and
/// subagent transcripts) and a `memory/` directory; without the anchor those
/// would become rows for sessions that cannot be resumed. Returning the id
/// rather than a bool is what lets callers take the id **from the validated
/// filename**, so "the id is a real UUID and `<id>.jsonl` exists" is a
/// property of how a row was built rather than a check someone can forget.
pub fn session_id_from_transcript_file_name(file_name: &str) -> Option<&str> {
    let stem = file_name.strip_suffix(".jsonl")?;
    is_session_uuid(stem).then_some(stem)
}

/// Whether `id` is a canonical lowercase 8-4-4-4-12 hex UUID.
///
/// Stricter than [`is_valid_session_id`](crate::terminal::cli_agent_resume::is_valid_session_id),
/// which accepts any `[A-Za-z0-9_-]{1,64}` token because other agents use other
/// id shapes. Filenames get the strict form: it is the anchor that keeps
/// directories and stray files out of the list.
fn is_session_uuid(id: &str) -> bool {
    const GROUPS: [usize; 5] = [8, 4, 4, 4, 12];
    let mut groups = id.split('-');
    for expected in GROUPS {
        let Some(group) = groups.next() else {
            return false;
        };
        if group.len() != expected
            || !group
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        {
            return false;
        }
    }
    groups.next().is_none()
}

/// Which sessions claim each conversation title, across every scanned
/// directory.
///
/// This is the whole discriminator for the broadcast bug. Claude Code's
/// `/rename` writes the new name into the transcripts of other sessions that
/// are live at the same time — measured here, one rename's name is the last
/// `aiTitle` of 13 transcripts in 6 different project directories, all carrying
/// their own `sessionId`, so nothing inside a single file can tell it is not
/// that file's name. A title only one session claims is that session's own; a
/// title several claim names none of them.
#[cfg(not(target_family = "wasm"))]
#[derive(Debug, Default, Clone)]
pub struct TitleClaims {
    by_title: HashMap<String, HashSet<String>>,
    sessions: HashSet<String>,
}

#[cfg(not(target_family = "wasm"))]
impl TitleClaims {
    /// Whether the scan has seen this session at all.
    ///
    /// The question callers really need answered is "could this map know about
    /// a collision", and a session the scan has not seen means its *directory*
    /// has not been scanned either — so the siblings a broadcast would have
    /// contaminated have not been read, and uniqueness is unknowable rather
    /// than true. Without this check an empty map would silently certify every
    /// title as unique, which is exactly the contaminated last-title answer.
    pub fn knows_session(&self, session_id: &str) -> bool {
        self.sessions.contains(session_id)
    }

    /// Whether `session_id` is the only session claiming `title`.
    pub fn is_only_claimant(&self, title: &str, session_id: &str) -> bool {
        self.by_title
            .get(title)
            .is_none_or(|claimants| claimants.iter().all(|claimant| claimant == session_id))
    }

    fn claim(&mut self, title: &str, session_id: &str) {
        self.by_title
            .entry(title.to_owned())
            .or_default()
            .insert(session_id.to_owned());
    }
}

/// Assigns every session's label from its candidates, using claims built across
/// **all** the sessions given, and returns the claims for reuse.
///
/// Must be called over the whole scanned set rather than one directory at a
/// time: the contamination crosses directories (13 transcripts, 6 projects, one
/// `/rename`), so a per-directory view would certify a broadcast name as unique
/// and keep showing it.
#[cfg(not(target_family = "wasm"))]
pub fn resolve_labels(sessions: &mut [ScannedSession]) -> TitleClaims {
    let mut claims = TitleClaims::default();
    for session in sessions.iter() {
        claims.sessions.insert(session.session_id.clone());
        for title in session.names.claimed_titles() {
            claims.claim(title, &session.session_id);
        }
    }
    for session in sessions.iter_mut() {
        let label = session
            .names
            .resolve(|title| claims.is_only_claimant(title, &session.session_id));
        session.label = label;
    }
    claims
}

pub struct ClaudeSessionScanChanged;

/// Singleton cache of the most recent scan of each project directory.
///
/// Read from the render path; written only by [`Self::refresh`]'s spawned
/// task. Holding results here (rather than recomputing) is what keeps the
/// filesystem off the UI thread.
#[derive(Default)]
pub struct ClaudeSessionScanModel {
    /// Every session from the latest scan of each directory, directory-major.
    sessions: Vec<ScannedSession>,
    /// Resolved names, per scanned directory. Only successful reads are
    /// cached, so a transcript that yielded no name is retried on the next
    /// scan (a name can appear later). Bucketing by directory keeps the memo
    /// bounded: each scan replaces its directory's bucket outright.
    #[cfg(not(target_family = "wasm"))]
    names: HashMap<PathBuf, NameMemo>,
    /// Who claims each title, over every directory scanned so far. Rebuilt with
    /// the labels after each scan, and handed to the live-session path, which
    /// resolves one session at a time and has no such view of its own.
    #[cfg(not(target_family = "wasm"))]
    claims: TitleClaims,
    /// Directories with a scan in flight, so overlapping kicks do not stack.
    #[cfg(not(target_family = "wasm"))]
    in_flight: std::collections::HashSet<PathBuf>,
    #[cfg(not(target_family = "wasm"))]
    last_scanned: HashMap<PathBuf, instant::Instant>,
}

impl Entity for ClaudeSessionScanModel {
    type Event = ClaudeSessionScanChanged;
}

impl SingletonEntity for ClaudeSessionScanModel {}

impl ClaudeSessionScanModel {
    /// Every scanned session, across all scanned directories.
    pub fn sessions(&self) -> &[ScannedSession] {
        &self.sessions
    }

    /// Who claims each conversation title, over every directory scanned so far.
    ///
    /// Exposed for the handle-bound (live) sessions in
    /// [`CLIAgentSessionsModel`](super::CLIAgentSessionsModel), which resolve a
    /// title one session at a time: this scan is the only place with a view
    /// wide enough to spot a name shared with another session.
    #[cfg(not(target_family = "wasm"))]
    pub fn title_claims(&self) -> &TitleClaims {
        &self.claims
    }

    /// The scanned session with this id, if the last scan saw it. The join key
    /// with the handle store, and the only place a scanned session's directory
    /// comes from when resuming it.
    pub fn session(&self, session_id: &str) -> Option<&ScannedSession> {
        self.sessions
            .iter()
            .find(|session| session.session_id == session_id)
    }

    /// Rescans `dirs` off-thread, skipping any that is in flight or was
    /// scanned within [`RESCAN_INTERVAL`].
    ///
    /// Never blocks the caller: the whole filesystem walk and every transcript
    /// read happen in the spawned task, and only the finished rows come back.
    #[cfg(not(target_family = "wasm"))]
    pub fn refresh(&mut self, dirs: Vec<PathBuf>, ctx: &mut ModelContext<Self>) {
        let now = instant::Instant::now();
        let due: Vec<PathBuf> = dirs
            .into_iter()
            .filter(|dir| {
                !self.in_flight.contains(dir)
                    && self
                        .last_scanned
                        .get(dir)
                        .is_none_or(|at| now.duration_since(*at) >= RESCAN_INTERVAL)
            })
            .collect();
        if due.is_empty() {
            return;
        }
        // Resolved once, on this thread, and passed down: the scan is then a
        // pure function of (config root, directory, memo), which is what makes
        // it testable against a fixture instead of the developer's real
        // `~/.claude`.
        let Ok(config_root) =
            crate::ai::agent_sdk::driver::harness::claude_transcript::claude_config_dir()
        else {
            return;
        };
        self.in_flight.extend(due.iter().cloned());
        // Each directory's own memo travels into the task, so an unchanged
        // transcript is not re-read there; the refreshed bucket comes back and
        // replaces it.
        let known_names: Vec<NameMemo> = due
            .iter()
            .map(|dir| self.names.get(dir).cloned().unwrap_or_default())
            .collect();

        let _ = ctx.spawn(
            async move {
                let mut scanned = Vec::new();
                for (dir, known) in due.iter().zip(known_names) {
                    scanned.push((dir.clone(), scan_directory(&config_root, dir, &known)));
                }
                scanned
            },
            |me: &mut Self, scanned, ctx| {
                let finished_at = instant::Instant::now();
                for (dir, (sessions, names)) in scanned {
                    me.in_flight.remove(&dir);
                    me.last_scanned.insert(dir.clone(), finished_at);
                    // A directory's rows and names are both replaced whole, so
                    // a session Claude has since deleted disappears from the
                    // rail rather than lingering as an unresumable row.
                    let dir_key = dir.to_string_lossy().into_owned();
                    me.sessions.retain(|session| session.cwd != dir_key);
                    me.sessions.extend(sessions);
                    me.names.insert(dir, names);
                }
                // Once, after the whole merge: resolving inside the loop would
                // judge uniqueness against a half-updated set, and a name is
                // only trustworthy relative to every session currently known.
                me.claims = resolve_labels(&mut me.sessions);
                ctx.emit(ClaudeSessionScanChanged);
                ctx.notify();
            },
        );
    }

    /// No Claude state to scan in the browser; the rail simply shows the
    /// handle-store rows there.
    #[cfg(target_family = "wasm")]
    pub fn refresh(&mut self, dirs: Vec<PathBuf>, ctx: &mut ModelContext<Self>) {
        drop((dirs, ctx));
    }
}

/// The directory Claude Code records `cwd`'s transcripts in, plus the resolved
/// path it was derived from.
///
/// Claude files a session under the **realpath**: a session started in a
/// symlinked checkout is recorded under the resolved directory, so the path is
/// canonicalized before it is encoded. A path that cannot be resolved
/// (unmounted volume, deleted worktree) is still worth trying as-is.
#[cfg(not(target_family = "wasm"))]
fn project_dir_in(config_root: &Path, cwd: &Path) -> (PathBuf, PathBuf) {
    use crate::ai::agent_sdk::driver::harness::claude_transcript::encode_cwd;

    let canonical = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    let project_dir = config_root.join("projects").join(encode_cwd(&canonical));
    (project_dir, canonical)
}

/// Scans one project directory: discovery by stat, names by bounded read.
///
/// Returns the rows plus the complete name memo for those rows, so the caller
/// can replace this directory's bucket outright.
/// Runs off the UI thread; every fallible call degrades to "skip this entry".
#[cfg(not(target_family = "wasm"))]
fn scan_directory(config_root: &Path, dir: &Path, known_names: &NameMemo) -> DirectoryScan {
    let (project_dir, canonical) = project_dir_in(config_root, dir);
    let Ok(entries) = std::fs::read_dir(&project_dir) else {
        // No directory for this project: the overwhelmingly common case for a
        // directory nobody has ever run Claude in.
        return (Vec::new(), NameMemo::new());
    };

    let mut candidates: Vec<(String, PathBuf, SystemTime)> = Vec::new();
    for entry in entries.flatten() {
        // `is_file` as well as the name filter: the `<uuid>/` subdirectories
        // sitting alongside the transcripts must never become rows.
        if !entry.file_type().is_ok_and(|kind| kind.is_file()) {
            continue;
        }
        let file_name = entry.file_name();
        let Some(session_id) = file_name
            .to_str()
            .and_then(session_id_from_transcript_file_name)
        else {
            continue;
        };
        let Ok(modified) = entry.metadata().and_then(|meta| meta.modified()) else {
            continue;
        };
        candidates.push((session_id.to_owned(), entry.path(), modified));
    }

    // Newest first, then bounded: naming is the only expensive step and it
    // must not scale with the size of a long-lived project's history.
    candidates.sort_by(|left, right| right.2.cmp(&left.2));
    candidates.truncate(MAX_SCANNED_SESSIONS_PER_DIR);

    let cwd = dir.to_string_lossy().into_owned();
    let mut sessions = Vec::with_capacity(candidates.len());
    let mut memo = NameMemo::new();
    for (session_id, path, modified) in candidates {
        let key = (path.clone(), modified);
        let names = match known_names.get(&key) {
            Some(cached) => cached.clone(),
            None => super::transcript_naming::read_transcript_names(&path, &canonical),
        };
        // Carried forward whether it was a hit or a fresh read, so the returned
        // memo describes exactly the sessions that still exist. A transcript
        // that yielded nothing is left out, so it is retried next scan.
        if !names.is_empty() {
            memo.insert(key, names.clone());
        }
        sessions.push(ScannedSession {
            session_id,
            cwd: cwd.clone(),
            names,
            // Filled in by `resolve_labels` once every directory's rows are
            // merged; this scan cannot see the sibling projects a broadcast
            // rename reached.
            label: None,
            modified,
        });
    }
    (sessions, memo)
}

/// Whether `session_id` still has a transcript under `cwd`.
///
/// Re-checked at the moment of resume: a row can outlive the file it was built
/// from (Claude prunes its own history), and resuming a session Claude has
/// forgotten fails with "No conversation found with session ID". Together with
/// the anchored-UUID filename filter this is the pair of conditions a scanned
/// row's resume command requires — and both are properties of the file the row
/// was built from, not of a string someone re-validated.
#[cfg(not(target_family = "wasm"))]
pub fn transcript_exists(cwd: &Path, session_id: &str) -> bool {
    crate::ai::agent_sdk::driver::harness::claude_transcript::claude_config_dir()
        .is_ok_and(|config_root| transcript_exists_in(&config_root, cwd, session_id))
}

#[cfg(not(target_family = "wasm"))]
fn transcript_exists_in(config_root: &Path, cwd: &Path, session_id: &str) -> bool {
    session_id_from_transcript_file_name(&format!("{session_id}.jsonl")).is_some()
        && project_dir_in(config_root, cwd)
            .0
            .join(format!("{session_id}.jsonl"))
            .is_file()
}

/// No Claude state on disk in the browser, so nothing scanned can be resumed.
#[cfg(target_family = "wasm")]
pub fn transcript_exists(cwd: &Path, session_id: &str) -> bool {
    drop((cwd, session_id));
    false
}

#[cfg(all(test, not(target_family = "wasm")))]
#[path = "session_scan_tests.rs"]
mod tests;
