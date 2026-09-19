use std::path::Path;

use super::*;

const CWD: &str = "/Users/example/dev/poa-agent";

fn names(text: &str) -> TranscriptNames {
    names_from_transcript_text(text, Path::new(CWD))
}

/// The label for a session whose titles collide with nothing — the common case,
/// and the one that exercises the cascade in its preferred order.
fn label(text: &str) -> Option<String> {
    names(text).resolve(|_| true)
}

fn tail_title(tail: &str) -> Option<String> {
    last_title_from_tail(tail, Path::new(CWD))
}

/// A golden transcript shaped like the real thing (Claude Code 2.1.221, the
/// version these record shapes were read off on this machine):
/// `ai-title` + its `agent-name` sibling up front, the head `cwd`/`slug`
/// carrier, the first prompt behind an injected `<command-name>` wrapper, and
/// a renamed `ai-title` appended at the very end the way `/rename` writes one.
const GOLDEN_TRANSCRIPT: &str = concat!(
    r#"{"type":"ai-title","aiTitle":"Initial working title","sessionId":"61f785ca-1c31-4671-a420-f89c47875750"}"#,
    "\n",
    r#"{"type":"agent-name","agentName":"Initial working title"}"#,
    "\n",
    r#"{"type":"mode","mode":"default","cwd":"/Users/example/dev/poa-agent","slug":"quietly-humming-otter"}"#,
    "\n",
    r#"{"type":"user","message":{"role":"user","content":"<command-name>/context</command-name>"}}"#,
    "\n",
    r#"{"type":"user","message":{"role":"user","content":"add retries to the ingest DAG"}}"#,
    "\n",
    r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"..."}]}}"#,
    "\n",
    r#"{"type":"ai-title","aiTitle":"Add retries to the ingest DAG"}"#,
    "\n",
    r#"{"type":"ai-title","aiTitle":"Ingest reliability work"}"#,
    "\n",
);

#[test]
fn every_candidate_is_reported_from_one_pass() {
    let names = names(GOLDEN_TRANSCRIPT);
    assert_eq!(names.last_title.as_deref(), Some("Ingest reliability work"));
    assert_eq!(names.first_title.as_deref(), Some("Initial working title"));
    assert_eq!(
        names.prompt.as_deref(),
        Some("add retries to the ingest DAG")
    );
    assert_eq!(names.slug.as_deref(), Some("quietly humming otter"));
}

#[test]
fn a_unique_last_title_wins_so_a_genuine_rename_survives() {
    // 29 of the 44 measured title changes were to a value no other session
    // claims — real `/rename`s, which must keep showing the new name.
    assert_eq!(
        label(GOLDEN_TRANSCRIPT).as_deref(),
        Some("Ingest reliability work")
    );
}

#[test]
fn a_shared_last_title_falls_back_to_the_first_title() {
    // The broadcast case: `/rename` wrote this name into 13 transcripts across
    // 6 projects, each carrying its own sessionId. The first title survived in
    // all 13, so it is the next candidate.
    let resolved = names(GOLDEN_TRANSCRIPT)
        .resolve(|title| title != "Ingest reliability work")
        .expect("the first title should stand in");
    assert_eq!(resolved, "Initial working title");
}

#[test]
fn a_shared_first_title_too_falls_back_to_the_prompt() {
    // 29 files have a *first* title that is itself shared, so the chain has to
    // go one step further. A prompt is written once at session start and cannot
    // be cross-written, which is what makes it the floor.
    let resolved = names(GOLDEN_TRANSCRIPT)
        .resolve(|_| false)
        .expect("the prompt should stand in");
    assert_eq!(resolved, "add retries to the ingest DAG");
}

#[test]
fn without_uniqueness_the_first_title_is_preferred_over_the_last() {
    // No scan has run, so a collision cannot be detected. The last title was
    // the corrupt field in 13 of 13 measured files; the first was correct in
    // all 13, so it is the safer of the two.
    assert_eq!(
        names(GOLDEN_TRANSCRIPT)
            .resolve_without_uniqueness()
            .as_deref(),
        Some("Initial working title")
    );
}

#[test]
fn agent_name_records_are_never_a_name_source() {
    // `agentName` is not a mirror of `aiTitle`: one measured file held
    // `aiTitle` "Set up UAT test data for customer grade scenarios" while its
    // `agentName` was a different session's name at the same moment. It was
    // contaminated in 13 of 13 files, so it contributes nothing at any tier.
    let only_agent_name = concat!(
        r#"{"type":"agent-name","agentName":"unified-trading-handoff-setup"}"#,
        "\n",
    );
    assert_eq!(names(only_agent_name), TranscriptNames::default());
    assert_eq!(tail_title(only_agent_name), None);
    assert_eq!(label(only_agent_name), None);

    // Even as the newest record of all, it must not displace the real title.
    let after_a_title = concat!(
        r#"{"type":"ai-title","aiTitle":"Import client repayment data into MIFOS"}"#,
        "\n",
        r#"{"type":"agent-name","agentName":"unified-trading-handoff-setup"}"#,
        "\n",
    );
    assert_eq!(
        tail_title(after_a_title).as_deref(),
        Some("Import client repayment data into MIFOS")
    );
    assert_eq!(
        label(after_a_title).as_deref(),
        Some("Import client repayment data into MIFOS")
    );
}

#[test]
fn claimed_titles_are_the_two_ai_titles_and_never_the_prompt() {
    // The prompt is the floor precisely because it cannot be cross-written;
    // two sessions starting from the same short instruction must not
    // disqualify each other's name.
    let names = names(GOLDEN_TRANSCRIPT);
    assert_eq!(
        names.claimed_titles().collect::<Vec<_>>(),
        vec!["Ingest reliability work", "Initial working title"]
    );
}

#[test]
fn falls_back_to_first_user_prompt_without_ai_title() {
    let head = concat!(
        r#"{"type":"summary","summary":"unrelated"}"#,
        "\n",
        r#"{"type":"user","message":{"role":"user","content":"fix the retry backoff in the DAG"}}"#,
        "\n",
        r#"{"type":"user","message":{"role":"user","content":"second prompt ignored"}}"#,
        "\n",
    );
    assert_eq!(
        label(head).as_deref(),
        Some("fix the retry backoff in the DAG")
    );
}

#[test]
fn array_content_blocks_are_supported() {
    let head = concat!(
        r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"prompt from blocks"}]}}"#,
        "\n",
    );
    assert_eq!(label(head).as_deref(), Some("prompt from blocks"));
}

#[test]
fn sidechain_user_records_are_not_prompts() {
    let head = concat!(
        r#"{"type":"user","isSidechain":true,"message":{"role":"user","content":"subagent replay"}}"#,
        "\n",
        r#"{"type":"user","message":{"role":"user","content":"the real prompt"}}"#,
        "\n",
    );
    assert_eq!(label(head).as_deref(), Some("the real prompt"));
}

#[test]
fn junk_auto_name_is_rejected_and_falls_through() {
    // "poa-agent-0f" is Claude's own `<dir>-<2hex>` display default.
    let head = concat!(
        r#"{"type":"ai-title","aiTitle":"poa-agent-0f"}"#,
        "\n",
        r#"{"type":"user","message":{"role":"user","content":"real work description"}}"#,
        "\n",
    );
    let names = names(head);
    assert_eq!(names.first_title, None, "junk never occupies a title slot");
    assert_eq!(names.last_title, None);
    assert_eq!(label(head).as_deref(), Some("real work description"));
}

#[test]
fn a_junk_first_record_does_not_cost_the_session_its_real_first_title() {
    let head = concat!(
        r#"{"type":"ai-title","aiTitle":"poa-agent-0f"}"#,
        "\n",
        r#"{"type":"ai-title","aiTitle":"Build llm_explainer episode 1 video pipeline"}"#,
        "\n",
        r#"{"type":"ai-title","aiTitle":"unified-trading-handoff-setup"}"#,
        "\n",
    );
    assert_eq!(
        names(head).first_title.as_deref(),
        Some("Build llm_explainer episode 1 video pipeline")
    );
}

#[test]
fn bare_directory_name_is_rejected() {
    let head = concat!(r#"{"type":"ai-title","aiTitle":"poa-agent"}"#, "\n");
    assert_eq!(label(head), None);
}

#[test]
fn whitespace_only_candidates_are_rejected() {
    let head = concat!(
        r#"{"type":"ai-title","aiTitle":"   "}"#,
        "\n",
        r#"{"type":"user","message":{"role":"user","content":"  \n\t "}}"#,
        "\n",
    );
    assert_eq!(label(head), None);
    assert!(names(head).is_empty());
}

#[test]
fn long_labels_are_collapsed_and_ellipsized() {
    let long = "word ".repeat(60);
    let head = format!(r#"{{"type":"user","message":{{"role":"user","content":"{long}"}}}}"#);
    let resolved = label(&head).expect("long prompt should still name the row");
    assert!(resolved.chars().count() <= 80);
    assert!(resolved.ends_with('…'));
    assert!(!resolved.contains("  "), "whitespace is collapsed");
}

#[test]
fn truncated_final_line_and_garbage_lines_are_skipped() {
    let head = concat!(
        r#"not json at all"#,
        "\n",
        r#"{"type":"ai-title","aiTitle":"Debug cargo install compilation error"}"#,
        "\n",
        // A record cut off mid-way by the bounded read.
        r#"{"type":"ai-title","aiTi"#,
    );
    assert_eq!(
        label(head).as_deref(),
        Some("Debug cargo install compilation error")
    );
}

#[test]
fn empty_and_missing_content_yield_no_candidates() {
    assert!(names("").is_empty());
    assert!(names("\n\n").is_empty());
    let head = concat!(r#"{"type":"user","message":{"role":"user"}}"#, "\n");
    assert!(names(head).is_empty());
    assert_eq!(label(head), None);
}

#[test]
fn transcript_path_matches_claude_layout() {
    // `encode_cwd` replaces every non-alphanumeric character with `-` per
    // Claude's convention.
    let path = claude_transcript_path(
        Path::new("/Users/example/dev/poa-agent"),
        "61f785ca-1c31-4671-a420-f89c47875750",
    )
    .expect("path should derive");
    let path = path.to_string_lossy();
    assert!(
        path.ends_with(
            "projects/-Users-example-dev-poa-agent/61f785ca-1c31-4671-a420-f89c47875750.jsonl"
        ),
        "unexpected layout: {path}"
    );
}

#[test]
fn project_dir_mangles_dots_underscores_and_spaces() {
    // The mangle is the whole discovery mechanism: get it wrong and a project
    // silently has no sessions at all. Verified against this machine's real
    // `~/.claude/projects` layout.
    let cases = [
        (
            "/Users/example/dev/poa-agent",
            "-Users-example-dev-poa-agent",
        ),
        (
            "/Users/example/dev/cse_market_analysis",
            "-Users-example-dev-cse-market-analysis",
        ),
        (
            "/Users/example/My Projects/app",
            "-Users-example-My-Projects-app",
        ),
        (
            "/Users/example/dev/app/.claude/worktrees/w1",
            "-Users-example-dev-app--claude-worktrees-w1",
        ),
    ];
    for (cwd, expected) in cases {
        let dir = claude_project_dir(Path::new(cwd)).expect("dir should derive");
        assert!(
            dir.ends_with(expected),
            "{cwd} encoded as {dir:?}, expected to end with {expected}"
        );
    }
}

#[test]
fn unreadable_file_yields_no_candidates_not_an_error() {
    assert!(
        read_transcript_names(
            Path::new("/nonexistent/definitely/missing.jsonl"),
            Path::new(CWD)
        )
        .is_empty()
    );
}

#[test]
fn a_short_transcript_is_read_end_to_end_by_the_tail_seek_alone() {
    // Exercises the real seek-based path, not just the pure core: the tail
    // read starts mid-file, so its first line is normally cut — but a
    // transcript smaller than the tail budget is covered from byte 0, and that
    // one read has to yield the *first* title as well as the last.
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir
        .path()
        .join("61f785ca-1c31-4671-a420-f89c47875750.jsonl");
    std::fs::write(&path, GOLDEN_TRANSCRIPT).unwrap();

    let tail = read_tail(&path).expect("the fixture is readable");
    assert!(tail.covers_whole_file);
    let names = read_transcript_names(&path, Path::new(CWD));
    assert_eq!(names.last_title.as_deref(), Some("Ingest reliability work"));
    assert_eq!(names.first_title.as_deref(), Some("Initial working title"));
    assert_eq!(
        names.prompt.as_deref(),
        Some("add retries to the ingest DAG")
    );
}

#[test]
fn a_rename_past_the_head_window_is_still_seen() {
    // The whole point of the tail read: `/rename` appends at EOF, so a
    // head-only reader would keep showing the session's original name.
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir
        .path()
        .join("61f785ca-1c31-4671-a420-f89c47875750.jsonl");
    let mut transcript = String::from(r#"{"type":"ai-title","aiTitle":"Initial working title"}"#);
    transcript.push('\n');
    transcript.push_str(r#"{"type":"user","message":{"role":"user","content":"start the work"}}"#);
    transcript.push('\n');
    let filler = "x".repeat(4096);
    while transcript.len() < HEAD_READ_BYTES * 2 {
        transcript.push_str(&format!(
            r#"{{"type":"assistant","message":{{"role":"assistant","content":[{{"type":"text","text":"{filler}"}}]}}}}"#
        ));
        transcript.push('\n');
    }
    transcript.push_str(r#"{"type":"ai-title","aiTitle":"Ingest reliability work"}"#);
    transcript.push('\n');
    std::fs::write(&path, &transcript).unwrap();

    let names = read_transcript_names(&path, Path::new(CWD));
    assert_eq!(
        names.last_title.as_deref(),
        Some("Ingest reliability work"),
        "the tail read is the only tier that can see a rename"
    );
    assert_eq!(
        names.first_title.as_deref(),
        Some("Initial working title"),
        "and the head read is the only tier that can see the original"
    );
    assert_eq!(names.prompt.as_deref(), Some("start the work"));
}

#[test]
fn head_title_names_a_transcript_whose_tail_window_has_none() {
    // A long tool loop after the naming turn pushes every title out of the
    // 64 KiB tail window; the 256 KiB head read is the tier that catches it.
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir
        .path()
        .join("61f785ca-1c31-4671-a420-f89c47875750.jsonl");
    let mut transcript = String::from(r#"{"type":"ai-title","aiTitle":"Rerank eval harness"}"#);
    transcript.push('\n');
    let filler = "x".repeat(4096);
    while transcript.len() < (TAIL_READ_BYTES as usize) * 2 {
        transcript.push_str(&format!(
            r#"{{"type":"assistant","message":{{"role":"assistant","content":[{{"type":"text","text":"{filler}"}}]}}}}"#
        ));
        transcript.push('\n');
    }
    assert!(
        transcript.len() < HEAD_READ_BYTES,
        "must stay inside the head window"
    );
    std::fs::write(&path, &transcript).unwrap();

    let tail = read_tail(&path).expect("the fixture is readable");
    assert!(!tail.covers_whole_file);
    assert_eq!(
        last_title_from_tail(&tail.text, Path::new(CWD)),
        None,
        "no title record inside the tail window"
    );
    assert_eq!(
        read_transcript_names(&path, Path::new(CWD))
            .last_title
            .as_deref(),
        Some("Rerank eval harness"),
        "the head read's own last title stands in"
    );
}

#[test]
fn injected_wrappers_and_caveats_are_skipped_for_the_next_real_prompt() {
    // These are interstitial, so the tier moves on rather than giving up:
    // there is always a real prompt behind them.
    let head = concat!(
        r#"{"type":"user","message":{"role":"user","content":"<command-name>/context</command-name>"}}"#,
        "\n",
        r#"{"type":"user","message":{"role":"user","content":"Caveat: The messages below were generated..."}}"#,
        "\n",
        r#"{"type":"user","message":{"role":"user","content":"port the scan mechanism"}}"#,
        "\n",
    );
    assert_eq!(
        names(head).prompt.as_deref(),
        Some("port the scan mechanism")
    );
    assert_eq!(label(head).as_deref(), Some("port the scan mechanism"));
}

#[test]
fn a_wrapper_only_transcript_yields_no_prompt_name() {
    let head = concat!(
        r#"{"type":"user","message":{"role":"user","content":"<local-command-stdout>ok</local-command-stdout>"}}"#,
        "\n",
        r#"{"type":"user","message":{"role":"user","content":"Caveat: replayed context"}}"#,
        "\n",
    );
    assert_eq!(names(head).prompt, None);
    assert_eq!(label(head), None);
}

#[test]
fn leading_summary_records_never_become_the_name() {
    // A `summary` describes the *pre-compaction parent* conversation, so using
    // it would name this session after a different one.
    let head = concat!(
        r#"{"type":"summary","summary":"Refactor the ingestion pipeline","leafUuid":"abc"}"#,
        "\n",
        r#"{"type":"user","message":{"role":"user","content":"now do the rail"}}"#,
        "\n",
    );
    assert_eq!(label(head).as_deref(), Some("now do the rail"));
}

#[test]
fn slug_is_the_last_resort_and_is_de_kebabed() {
    let head = concat!(
        r#"{"type":"mode","mode":"default","slug":"quietly-humming-otter"}"#,
        "\n",
        r#"{"type":"user","message":{"role":"user","content":"<command-name>/clear</command-name>"}}"#,
        "\n",
    );
    assert_eq!(label(head).as_deref(), Some("quietly humming otter"));
}

#[test]
fn derived_style_junk_names_are_rejected_at_every_tier() {
    // `nameSource: "derived"` names look like `<dir>-<2hex>`; Claude's own docs
    // say they are display junk, not a handle.
    let head = concat!(
        r#"{"type":"ai-title","aiTitle":"poa-agent-3f"}"#,
        "\n",
        r#"{"type":"user","message":{"role":"user","content":"the actual request"}}"#,
        "\n",
    );
    assert_eq!(tail_title(head), None, "junk is rejected in the tail too");
    assert_eq!(label(head).as_deref(), Some("the actual request"));
}

#[test]
fn bare_hex_blobs_are_rejected() {
    // A session id or sha that reached a name slot is never a name.
    for junk in [
        r#"{"type":"ai-title","aiTitle":"3f8a9c2d"}"#,
        r#"{"type":"ai-title","aiTitle":"61f785ca-1c31"}"#,
        r#"{"type":"ai-title","aiTitle":"deadbeefcafe"}"#,
    ] {
        assert_eq!(tail_title(junk), None, "should reject: {junk}");
        assert!(names(junk).is_empty(), "should reject: {junk}");
    }
    // Short hex-looking words are real words, and must survive.
    assert_eq!(
        tail_title(r#"{"type":"ai-title","aiTitle":"decaf"}"#).as_deref(),
        Some("decaf")
    );
}

#[test]
fn tail_skips_a_record_cut_by_the_seek() {
    // The first line of a tail read normally starts mid-record.
    let tail = concat!(
        r#"e":"ai-title","aiTitle":"Cut in half"}"#,
        "\n",
        r#"{"type":"ai-title","aiTitle":"Whole record"}"#,
        "\n",
    );
    assert_eq!(tail_title(tail).as_deref(), Some("Whole record"));
}
