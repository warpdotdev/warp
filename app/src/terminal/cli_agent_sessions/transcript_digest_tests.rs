use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use tempfile::TempDir;

use super::digest_impl::*;
use super::*;
use crate::terminal::CLIAgent;

const SESSION_A: &str = "61f785ca-1c31-4671-a420-f89c47875750";
const SESSION_B: &str = "0c553412-72fa-4c15-889f-9c380392eb89";

/// A directory of transcripts, shaped like Claude's own but addressed
/// directly.
///
/// The digest's core is a pure function of `(located targets, query, cache,
/// budget)` — path resolution is the only part that needs a config root — so
/// these tests hand it fixture paths and never read the developer's real
/// `~/.claude`.
struct Fixture {
    _root: TempDir,
    dir: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = TempDir::new().unwrap();
        let dir = root.path().join("projects");
        fs::create_dir_all(&dir).unwrap();
        Self { _root: root, dir }
    }

    fn path(&self, session_id: &str) -> PathBuf {
        self.dir.join(format!("{session_id}.jsonl"))
    }

    fn write(&self, session_id: &str, body: &str) -> PathBuf {
        let path = self.path(session_id);
        fs::write(&path, body).unwrap();
        path
    }

    fn append(&self, session_id: &str, body: &str) -> u64 {
        let path = self.path(session_id);
        let before = fs::metadata(&path).unwrap().len();
        let mut existing = fs::read_to_string(&path).unwrap();
        existing.push_str(body);
        fs::write(&path, existing).unwrap();
        fs::metadata(&path).unwrap().len() - before
    }
}

fn user_line(text: &str) -> String {
    format!(
        r#"{{"type":"user","isSidechain":false,"message":{{"role":"user","content":"{text}"}}}}"#
    )
}

fn tool_result_line(text: &str) -> String {
    format!(
        r#"{{"type":"user","message":{{"role":"user","content":[{{"tool_use_id":"toolu_01","type":"tool_result","content":"{text}"}}]}},"toolUseResult":{{"stdout":"{text}"}}}}"#
    )
}

fn assistant_line(text: &str) -> String {
    format!(
        r#"{{"type":"assistant","message":{{"role":"assistant","content":[{{"type":"text","text":"{text}"}}]}}}}"#
    )
}

fn tool_use_line(text: &str) -> String {
    format!(
        r#"{{"type":"assistant","message":{{"role":"assistant","content":[{{"type":"tool_use","name":"Write","input":{{"content":"{text}"}}}}]}}}}"#
    )
}

fn target(session_id: &str, task_name: &str) -> DigestTarget {
    DigestTarget {
        agent: CLIAgent::Claude,
        session_id: session_id.to_owned(),
        project_name: "warp".to_owned(),
        task_name: task_name.to_owned(),
        cwd: "/repos/warp".to_owned(),
    }
}

fn located(fixture: &Fixture, session_id: &str, task_name: &str) -> LocatedTarget {
    LocatedTarget {
        target: target(session_id, task_name),
        path: fixture.path(session_id),
    }
}

fn fresh_budget() -> ReadBudget {
    ReadBudget::new(MAX_REFRESH_READ_BYTES)
}

fn cached_entry(path: &Path, key: DigestKey, text: &str) -> (PathBuf, CachedDigest) {
    (
        path.to_path_buf(),
        CachedDigest {
            key,
            text: text.into(),
            partial: false,
            last_used: 0,
        },
    )
}

fn key_for(path: &Path) -> DigestKey {
    let metadata = fs::metadata(path).unwrap();
    DigestKey {
        path: path.to_path_buf(),
        len: metadata.len(),
        modified: metadata.modified().unwrap(),
    }
}

#[test]
fn a_user_turn_is_content_but_a_tool_result_is_not() {
    // The whole role rule in one assertion: the two records are both
    // `"type":"user"`, and the only structural difference is the `tool_use_id`
    // block. Matching on text instead would keep the tool output.
    let content = extract_content(&format!(
        "{}\n{}\n",
        user_line("find the POA-2236 regression"),
        tool_result_line("POA-2236 appears in 41 files")
    ));

    assert!(
        content.contains("find the POA-2236 regression"),
        "a user envelope without a tool_use_id is what the user typed: {content:?}"
    );
    assert!(
        !content.contains("41 files"),
        "a user envelope WITH a tool_use_id is a tool result and must be excluded: {content:?}"
    );
}

#[test]
fn assistant_prose_is_content_but_its_tool_arguments_are_not() {
    let content = extract_content(&format!(
        "{}\n{}\n",
        assistant_line("The deadlock is in the terminal model lock"),
        tool_use_line("every byte of a file being written")
    ));

    assert!(content.contains("deadlock is in the terminal model lock"));
    assert!(
        !content.contains("every byte of a file"),
        "tool_use blocks carry pasted file bodies, which are the 30x cost this \
         digest exists to avoid: {content:?}"
    );
}

#[test]
fn a_torn_final_line_does_not_abort_the_file() {
    // Exactly what a live transcript looks like mid-write, and what a bounded
    // window's first line looks like always: the good records around the torn
    // one must still be digested.
    let torn = format!(
        "{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":\"cut off her\n{}\n{}",
        user_line("the second prompt survives"),
        r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","tex"#
    );
    let content = extract_content(&torn);

    assert!(
        content.contains("the second prompt survives"),
        "one unparseable line must skip that line only: {content:?}"
    );
}

#[test]
fn a_grown_transcript_re_reads_only_its_new_bytes() {
    let fixture = Fixture::new();
    let path = fixture.write(SESSION_A, &format!("{}\n", user_line("the first prompt")));
    let first_len = fs::metadata(&path).unwrap().len();

    let mut cache = DigestCache::new();
    let mut budget = fresh_budget();
    let (first, _) = refresh_digest(&path, &mut cache, &mut budget, 1).unwrap();
    assert_eq!(
        budget.read, first_len,
        "the first pass reads the whole transcript"
    );
    assert!(first.contains("the first prompt"));

    let appended = fixture.append(SESSION_A, &format!("{}\n", user_line("the second prompt")));
    let read_before_growth = budget.read;
    let (grown, _) = refresh_digest(&path, &mut cache, &mut budget, 2).unwrap();

    assert_eq!(
        budget.read - read_before_growth,
        appended,
        "a transcript is append-only, so growth must re-read `[old_len, EOF)` \
         and nothing else"
    );
    assert!(
        grown.contains("the first prompt") && grown.contains("the second prompt"),
        "the incremental read must extend the digest, not replace it: {grown:?}"
    );
}

#[test]
fn an_unreadable_transcript_is_never_memoized() {
    let fixture = Fixture::new();
    let path = fixture.path(SESSION_B);
    let mut cache = DigestCache::new();
    let mut budget = fresh_budget();

    assert!(
        refresh_digest(&path, &mut cache, &mut budget, 1).is_none(),
        "a transcript that cannot be opened yields no digest"
    );
    assert!(
        cache.is_empty(),
        "caching the failure would make a transient I/O error permanent for the \
         rest of the session"
    );

    fixture.write(SESSION_B, &format!("{}\n", user_line("it came back")));
    let (text, _) = refresh_digest(&path, &mut cache, &mut budget, 2)
        .expect("the next search must retry, not serve a memoized empty digest");
    assert!(text.contains("it came back"));
}

#[test]
fn the_cache_hits_only_on_an_exact_path_len_and_mtime() {
    let fixture = Fixture::new();
    let path = fixture.write(SESSION_A, &format!("{}\n", user_line("on disk")));
    let key = key_for(&path);

    // Exact key: served without touching the disk at all.
    let mut cache = DigestCache::from([cached_entry(&path, key.clone(), "from the cache")]);
    let mut budget = fresh_budget();
    let (text, _) = refresh_digest(&path, &mut cache, &mut budget, 1).unwrap();
    assert_eq!(&*text, "from the cache");
    assert_eq!(budget.read, 0, "an unchanged transcript is not re-read");

    // Same file, stale mtime (a rewrite that happened to keep the length).
    let stale_mtime = DigestKey {
        modified: key.modified - Duration::from_secs(1),
        ..key.clone()
    };
    let mut cache = DigestCache::from([cached_entry(&path, stale_mtime, "from the cache")]);
    let mut budget = fresh_budget();
    let (text, _) = refresh_digest(&path, &mut cache, &mut budget, 1).unwrap();
    assert!(text.contains("on disk"), "a stale mtime must miss");
    assert!(budget.read > 0);

    // Same file, stale length: the append-only path, which still reads.
    let stale_len = DigestKey {
        len: key.len - 1,
        ..key.clone()
    };
    let mut cache = DigestCache::from([cached_entry(&path, stale_len, "from the cache")]);
    let mut budget = fresh_budget();
    refresh_digest(&path, &mut cache, &mut budget, 1).unwrap();
    assert!(budget.read > 0, "a stale length must miss");

    // A key for another file never answers for this one.
    let other_path = fixture.write(SESSION_B, &format!("{}\n", user_line("another session")));
    let mut cache = DigestCache::from([cached_entry(&path, key_for(&other_path), "wrong file")]);
    let mut budget = fresh_budget();
    let (text, _) = refresh_digest(&path, &mut cache, &mut budget, 1).unwrap();
    assert!(text.contains("on disk"), "a key for another file must miss");
}

#[test]
fn search_is_literal_and_case_insensitive_across_the_corpus() {
    let fixture = Fixture::new();
    fixture.write(
        SESSION_A,
        &format!("{}\n", user_line("chase the POA-2236 regression")),
    );
    fixture.write(SESSION_B, &format!("{}\n", user_line("unrelated work")));
    let targets = [
        located(&fixture, SESSION_A, "Rail search"),
        located(&fixture, SESSION_B, "Something else"),
    ];

    let mut cache = DigestCache::new();
    let mut budget = fresh_budget();
    let hits = search_corpus(&targets, "poa-2236", &mut cache, &mut budget, 1);

    assert_eq!(hits.len(), 1, "only the transcript containing it matches");
    assert_eq!(hits[0].session_id, SESSION_A);
    assert_eq!(hits[0].task_name, "Rail search");

    // Literal, not fuzzy: the characters of `POA-2236` scattered through a
    // sentence must not match, or every ticket id would hit every session.
    let mut cache = DigestCache::new();
    let mut budget = fresh_budget();
    assert!(
        search_corpus(&targets, "prga", &mut cache, &mut budget, 1).is_empty(),
        "a subsequence is not a substring"
    );
}

#[test]
fn a_snippet_is_one_ellipsized_line_around_the_match() {
    let long = format!(
        "an earlier line\n{}NEEDLE{}\na later line",
        "before ".repeat(30),
        " after".repeat(60)
    );
    let at = long.find("NEEDLE").unwrap();
    let (snippet, indices) = snippet_around(&long, at, "NEEDLE".len());

    assert!(
        !snippet.contains('\n'),
        "a palette row is one line: {snippet:?}"
    );
    assert!(!snippet.contains("an earlier line") && !snippet.contains("a later line"));
    assert!(snippet.starts_with('…') && snippet.ends_with('…'));
    let chars: String = snippet
        .chars()
        .skip(*indices.first().unwrap())
        .take(indices.len())
        .collect();
    assert_eq!(
        chars, "NEEDLE",
        "the highlight indices are char indices into the snippet"
    );
}

#[test]
fn the_least_recently_searched_digests_are_evicted_first() {
    let mut cache = DigestCache::new();
    for index in 0..MAX_CACHED_DIGESTS + 10 {
        let path = PathBuf::from(format!("/transcripts/{index}.jsonl"));
        let key = DigestKey {
            path: path.clone(),
            len: 1,
            modified: SystemTime::UNIX_EPOCH,
        };
        // The first ten were last touched by an older search than the rest.
        let (path, mut entry) = cached_entry(&path, key, "digest");
        entry.last_used = if index < 10 { 1 } else { 2 };
        cache.insert(path, entry);
    }

    evict_cold_digests(&mut cache);

    assert!(cache.len() <= MAX_CACHED_DIGESTS);
    assert!(
        cache
            .values()
            .all(|entry| entry.last_used == 2 || cache.len() == MAX_CACHED_DIGESTS),
        "eviction drops the coldest entries, never the ones a search just used"
    );
}
