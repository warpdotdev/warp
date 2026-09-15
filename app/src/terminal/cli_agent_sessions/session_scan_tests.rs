use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use tempfile::TempDir;

use super::*;

const SESSION_A: &str = "61f785ca-1c31-4671-a420-f89c47875750";
const SESSION_B: &str = "0c553412-72fa-4c15-889f-9c380392eb89";

/// A fixture Claude config root plus a project directory to scan.
///
/// The config root is injected rather than discovered, so these tests never
/// read the developer's real `~/.claude` and never mutate the environment.
struct Fixture {
    /// Shared so a [`sibling`](Fixture::sibling) project keeps the same temp
    /// root alive rather than owning a second one.
    _root: std::sync::Arc<TempDir>,
    config_root: PathBuf,
    cwd: PathBuf,
    project_dir: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = TempDir::new().unwrap();
        let config_root = root.path().join(".claude");
        let cwd = root.path().join("poa-agent");
        fs::create_dir_all(&cwd).unwrap();
        // `project_dir_in` canonicalizes before encoding — on macOS the temp
        // dir is under `/var`, a symlink to `/private/var`, which is exactly
        // the realpath case that guards.
        let project_dir = project_dir_in(&config_root, &cwd).0;
        fs::create_dir_all(&project_dir).unwrap();
        Self {
            _root: std::sync::Arc::new(root),
            config_root,
            cwd,
            project_dir,
        }
    }

    /// A second project directory under the same fixture config root.
    ///
    /// The uniqueness rule exists because a broadcast `/rename` crosses
    /// directories — measured, one rename reached 13 transcripts in 6 projects
    /// — so the interesting cases cannot be built inside a single directory.
    fn sibling(&self, name: &str) -> Self {
        let cwd = self.cwd.parent().expect("temp root").join(name);
        fs::create_dir_all(&cwd).unwrap();
        let project_dir = project_dir_in(&self.config_root, &cwd).0;
        fs::create_dir_all(&project_dir).unwrap();
        Self {
            _root: self._root.clone(),
            config_root: self.config_root.clone(),
            cwd,
            project_dir,
        }
    }

    /// Writes a transcript shaped like the real thing: a working title, the
    /// first prompt, then the newest title appended at the end.
    fn write_session(&self, session_id: &str, title: &str, prompt: &str) -> PathBuf {
        self.write_titled_session(session_id, "Working title", title, prompt)
    }

    /// The same, with the first title chosen too — the field a broadcast
    /// leaves alone, and so the one the fallback lands on.
    fn write_titled_session(
        &self,
        session_id: &str,
        first_title: &str,
        last_title: &str,
        prompt: &str,
    ) -> PathBuf {
        self.write_transcript(
            session_id,
            &format!(
                concat!(
                    r#"{{"type":"ai-title","aiTitle":"{first_title}"}}"#,
                    "\n",
                    r#"{{"type":"user","message":{{"role":"user","content":"{prompt}"}}}}"#,
                    "\n",
                    r#"{{"type":"ai-title","aiTitle":"{last_title}"}}"#,
                    "\n",
                ),
                first_title = first_title,
                last_title = last_title,
                prompt = prompt,
            ),
        )
    }

    fn write_transcript(&self, session_id: &str, body: &str) -> PathBuf {
        let path = self.project_dir.join(format!("{session_id}.jsonl"));
        fs::write(&path, body).unwrap();
        path
    }

    /// Scans this directory and applies the cross-directory resolution the
    /// model applies, so a test sees the labels the rail would show.
    fn scan(&self) -> DirectoryScan {
        let (mut sessions, memo) = self.scan_raw();
        resolve_labels(&mut sessions);
        (sessions, memo)
    }

    fn scan_raw(&self) -> DirectoryScan {
        scan_directory(&self.config_root, &self.cwd, &NameMemo::new())
    }
}

fn set_mtime(path: &PathBuf, seconds_since_epoch: u64) {
    let when = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(seconds_since_epoch);
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(when)
        .unwrap();
}

#[test]
fn uuid_filename_filter_accepts_only_transcripts() {
    assert_eq!(
        session_id_from_transcript_file_name(&format!("{SESSION_A}.jsonl")),
        Some(SESSION_A)
    );
    // The `<uuid>/` subdirectories (session memory, subagents) sit right next
    // to the transcripts; without the `.jsonl` suffix they would become rows
    // for sessions that cannot be resumed.
    assert_eq!(session_id_from_transcript_file_name(SESSION_A), None);
    assert_eq!(session_id_from_transcript_file_name("memory"), None);
    // Anchored: no prefix, no suffix, no partial ids.
    assert_eq!(
        session_id_from_transcript_file_name(&format!("prefix-{SESSION_A}.jsonl")),
        None
    );
    assert_eq!(
        session_id_from_transcript_file_name(&format!("{SESSION_A}-extra.jsonl")),
        None
    );
    assert_eq!(
        session_id_from_transcript_file_name("61f785ca-1c31-4671-a420.jsonl"),
        None
    );
    assert_eq!(session_id_from_transcript_file_name("notes.jsonl"), None);
    assert_eq!(session_id_from_transcript_file_name("summary.json"), None);
    // Uppercase is not the form Claude writes, so it is not accepted either.
    assert_eq!(
        session_id_from_transcript_file_name("61F785CA-1C31-4671-A420-F89C47875750.jsonl"),
        None
    );
}

#[test]
fn scan_names_every_session_in_a_project_directory() {
    let fixture = Fixture::new();
    fixture.write_session(SESSION_A, "Add retries to the ingest DAG", "add retries");
    fixture.write_session(SESSION_B, "Rerank eval harness", "rerank");

    let (sessions, memo) = fixture.scan();

    assert_eq!(sessions.len(), 2, "both transcripts should be listed");
    let labels: HashMap<&str, Option<&str>> = sessions
        .iter()
        .map(|session| (session.session_id.as_str(), session.label.as_deref()))
        .collect();
    assert_eq!(
        labels.get(SESSION_A),
        Some(&Some("Add retries to the ingest DAG")),
        "the newest title wins, via the tail read"
    );
    assert_eq!(labels.get(SESSION_B), Some(&Some("Rerank eval harness")));
    // The cwd is echoed back as asked, not canonicalized, so the rail buckets
    // scanned rows by exactly the path it buckets tabs by.
    assert!(
        sessions
            .iter()
            .all(|session| session.cwd == fixture.cwd.to_string_lossy())
    );
    assert_eq!(memo.len(), 2, "successful reads are memoised");
}

#[test]
fn scan_ignores_subdirectories_and_foreign_files() {
    let fixture = Fixture::new();
    fixture.write_session(SESSION_A, "The only real session", "go");
    // Exactly what a live project directory looks like around a transcript.
    fs::create_dir_all(fixture.project_dir.join(SESSION_B).join("subagents")).unwrap();
    fs::create_dir_all(fixture.project_dir.join("memory")).unwrap();
    fs::write(fixture.project_dir.join("notes.jsonl"), "{}\n").unwrap();

    let (sessions, _) = fixture.scan();

    assert_eq!(
        sessions
            .iter()
            .map(|session| session.session_id.as_str())
            .collect::<Vec<_>>(),
        vec![SESSION_A]
    );
}

#[test]
fn scan_orders_newest_first() {
    let fixture = Fixture::new();
    let older = fixture.write_session(SESSION_A, "Older work", "a");
    let newer = fixture.write_session(SESSION_B, "Newer work", "b");
    set_mtime(&older, 1_700_000_000);
    set_mtime(&newer, 1_700_000_060);

    let (sessions, _) = fixture.scan();

    assert_eq!(
        sessions
            .iter()
            .map(|session| session.session_id.as_str())
            .collect::<Vec<_>>(),
        vec![SESSION_B, SESSION_A]
    );
}

#[test]
fn memoised_names_are_not_re_read() {
    let fixture = Fixture::new();
    let path = fixture.write_session(SESSION_A, "On disk", "a");
    set_mtime(&path, 1_700_000_000);
    let modified = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);

    // Seeded with a value the transcript does not contain: if it comes back,
    // the file was not read.
    let memoised = TranscriptNames {
        last_title: Some("Name from the memo".to_owned()),
        ..TranscriptNames::default()
    };
    let mut memo = NameMemo::new();
    memo.insert((path.clone(), modified), memoised.clone());
    let (mut sessions, returned) = scan_directory(&fixture.config_root, &fixture.cwd, &memo);
    resolve_labels(&mut sessions);

    assert_eq!(sessions[0].label.as_deref(), Some("Name from the memo"));
    // The returned memo is complete, not incremental — that is what lets the
    // model replace the directory's bucket instead of growing it forever.
    assert_eq!(returned.len(), 1);
    assert_eq!(returned.get(&(path.clone(), modified)), Some(&memoised));

    // A changed mtime invalidates the entry, because the key includes it.
    set_mtime(&path, 1_700_000_999);
    let (mut sessions, returned) = scan_directory(&fixture.config_root, &fixture.cwd, &memo);
    resolve_labels(&mut sessions);
    assert_eq!(sessions[0].label.as_deref(), Some("On disk"));
    assert_eq!(returned.len(), 1, "the stale key is not carried forward");
    assert!(
        !returned.contains_key(&(path, modified)),
        "the superseded entry is dropped, so the memo cannot grow without bound"
    );
}

#[test]
fn scan_of_an_unknown_directory_is_empty_not_an_error() {
    let fixture = Fixture::new();
    let never_used = fixture.cwd.parent().unwrap().join("never-used");
    fs::create_dir_all(&never_used).unwrap();

    let (sessions, memo) = scan_directory(&fixture.config_root, &never_used, &NameMemo::new());

    assert!(sessions.is_empty());
    assert!(memo.is_empty());
}

#[test]
fn scan_is_bounded_per_directory() {
    let fixture = Fixture::new();
    // More transcripts than the cap, each with a distinct mtime.
    for index in 0..MAX_SCANNED_SESSIONS_PER_DIR + 4 {
        let session_id = format!("61f785ca-1c31-4671-a420-f89c478{index:05}");
        let path = fixture.write_session(&session_id, &format!("Session {index}"), "go");
        set_mtime(&path, 1_700_000_000 + index as u64);
    }

    let (sessions, _) = fixture.scan();

    assert_eq!(sessions.len(), MAX_SCANNED_SESSIONS_PER_DIR);
    assert_eq!(
        sessions[0].label.as_deref(),
        Some(format!("Session {}", MAX_SCANNED_SESSIONS_PER_DIR + 3).as_str()),
        "the cap keeps the newest, not an arbitrary slice"
    );
}

#[test]
fn agent_name_records_never_name_a_scanned_session() {
    // `agentName` was contaminated in 13 of 13 measured transcripts and is not
    // a mirror of `aiTitle`, so it is not a source even when it is the only
    // name-like record in the file.
    let fixture = Fixture::new();
    fixture.write_transcript(
        SESSION_A,
        concat!(
            r#"{"type":"user","message":{"role":"user","content":"build the video pipeline"}}"#,
            "\n",
            r#"{"type":"agent-name","agentName":"unified-trading-handoff-setup"}"#,
            "\n",
        ),
    );

    let (sessions, _) = fixture.scan();

    assert_eq!(
        sessions[0].label.as_deref(),
        Some("build the video pipeline"),
        "the prompt names the session; the broadcast agentName is ignored"
    );
}

#[test]
fn a_last_title_two_sessions_claim_falls_back_to_each_ones_first_title() {
    let fixture = Fixture::new();
    fixture.write_titled_session(
        SESSION_A,
        "Import client repayment data into MIFOS",
        "unified-trading-handoff-setup",
        "import the repayments",
    );
    fixture.write_titled_session(
        SESSION_B,
        "Set up UAT test data for customer grade scenarios",
        "unified-trading-handoff-setup",
        "set up the UAT data",
    );

    let (sessions, _) = fixture.scan();

    let labels: HashMap<&str, Option<&str>> = sessions
        .iter()
        .map(|session| (session.session_id.as_str(), session.label.as_deref()))
        .collect();
    assert_eq!(
        labels.get(SESSION_A),
        Some(&Some("Import client repayment data into MIFOS")),
        "a title two sessions claim names neither of them"
    );
    assert_eq!(
        labels.get(SESSION_B),
        Some(&Some("Set up UAT test data for customer grade scenarios"))
    );
}

#[test]
fn when_both_titles_are_shared_the_first_prompt_names_the_session() {
    // 29 files have a *first* title that is itself shared, so the chain has to
    // go one step further — to the prompt, which is written once at session
    // start and cannot be cross-written.
    let fixture = Fixture::new();
    fixture.write_titled_session(
        SESSION_A,
        "Shared kickoff title",
        "unified-trading-handoff-setup",
        "import the repayments",
    );
    fixture.write_titled_session(
        SESSION_B,
        "Shared kickoff title",
        "unified-trading-handoff-setup",
        "set up the UAT data",
    );

    let (sessions, _) = fixture.scan();

    let labels: HashMap<&str, Option<&str>> = sessions
        .iter()
        .map(|session| (session.session_id.as_str(), session.label.as_deref()))
        .collect();
    assert_eq!(labels.get(SESSION_A), Some(&Some("import the repayments")));
    assert_eq!(labels.get(SESSION_B), Some(&Some("set up the UAT data")));
}

#[test]
fn uniqueness_is_computed_across_directories() {
    // The contamination crosses directories: one `/rename` put its name in the
    // last `aiTitle` of 13 transcripts spread over 6 project directories.
    let first = Fixture::new();
    let second = first.sibling("mifos-import");
    first.write_titled_session(
        SESSION_A,
        "Build llm_explainer episode 1 video pipeline",
        "unified-trading-handoff-setup",
        "build the pipeline",
    );
    second.write_titled_session(
        SESSION_B,
        "Import client repayment data into MIFOS",
        "unified-trading-handoff-setup",
        "import the repayments",
    );

    // One directory at a time, the broadcast is invisible — which is why the
    // model resolves over its whole view instead of per scan.
    let (mut alone, _) = first.scan_raw();
    resolve_labels(&mut alone);
    assert_eq!(
        alone[0].label.as_deref(),
        Some("unified-trading-handoff-setup"),
        "a single directory cannot see the collision"
    );

    let mut both = first.scan_raw().0;
    both.extend(second.scan_raw().0);
    let claims = resolve_labels(&mut both);

    let labels: HashMap<&str, Option<&str>> = both
        .iter()
        .map(|session| (session.session_id.as_str(), session.label.as_deref()))
        .collect();
    assert_eq!(
        labels.get(SESSION_A),
        Some(&Some("Build llm_explainer episode 1 video pipeline"))
    );
    assert_eq!(
        labels.get(SESSION_B),
        Some(&Some("Import client repayment data into MIFOS"))
    );
    assert!(!claims.is_only_claimant("unified-trading-handoff-setup", SESSION_A));
    assert!(claims.is_only_claimant("Import client repayment data into MIFOS", SESSION_B));
}

#[test]
fn claims_report_which_sessions_the_scan_has_actually_seen() {
    // The live-session path keys off this: a session the scan has not seen
    // means its directory was never scanned, so uniqueness is unknowable and
    // an empty claims map must not certify a contaminated title as unique.
    let fixture = Fixture::new();
    fixture.write_session(SESSION_A, "Rerank eval harness", "rerank");

    let (mut sessions, _) = fixture.scan_raw();
    let claims = resolve_labels(&mut sessions);

    assert!(claims.knows_session(SESSION_A));
    assert!(!claims.knows_session(SESSION_B));
    assert!(!TitleClaims::default().knows_session(SESSION_A));
}

#[test]
fn transcript_existence_is_checked_against_the_project_directory() {
    let fixture = Fixture::new();
    fixture.write_session(SESSION_A, "Present", "go");

    assert!(
        transcript_exists_in(&fixture.config_root, &fixture.cwd, SESSION_A),
        "the scanned session's transcript is still there"
    );
    assert!(
        !transcript_exists_in(&fixture.config_root, &fixture.cwd, SESSION_B),
        "a pruned session must not be offered for resume"
    );
    // A non-UUID id can never name a transcript, so it never passes.
    assert!(!transcript_exists_in(
        &fixture.config_root,
        &fixture.cwd,
        "../../etc/passwd"
    ));
}
