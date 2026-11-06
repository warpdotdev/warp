use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::json;
use tempfile::TempDir;
use uuid::Uuid;

use super::super::super::claude_transcript::encode_cwd;
use super::CliTranscriptKind;

#[test]
fn pi_transcript_root_uses_the_run_environment() {
    let first = HashMap::from([(
        OsString::from("PI_CODING_AGENT_DIR"),
        OsString::from("/tmp/pi-first-run"),
    )]);
    let second = HashMap::from([(
        OsString::from("PI_CODING_AGENT_DIR"),
        OsString::from("/tmp/pi-second-run"),
    )]);

    assert_eq!(
        CliTranscriptKind::Pi.root(&first).unwrap(),
        PathBuf::from("/tmp/pi-first-run")
    );
    assert_eq!(
        CliTranscriptKind::Pi.root(&second).unwrap(),
        PathBuf::from("/tmp/pi-second-run")
    );
    assert!(CliTranscriptKind::Pi.root(&HashMap::new()).is_err());
}

fn write(path: &Path) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, "{\"type\":\"session_meta\"}\n").unwrap();
}

#[test]
fn codex_transcript_is_the_rollout_named_after_the_acp_session_id() {
    let sessions_root = TempDir::new().unwrap();
    let cwd = Path::new("/workspace");
    let session_id = Uuid::new_v4();
    let kind = CliTranscriptKind::Codex;
    assert_eq!(kind.locate(sessions_root.path(), cwd, session_id), None);

    let rollout = sessions_root
        .path()
        .join("2026/10/09")
        .join(format!("rollout-2026-10-09T17-00-00-{session_id}.jsonl"));
    write(&rollout);
    write(&sessions_root.path().join("2026/10/09").join(format!(
        "rollout-2026-10-09T16-00-00-{}.jsonl",
        Uuid::new_v4()
    )));

    assert_eq!(
        kind.locate(sessions_root.path(), cwd, session_id),
        Some(rollout)
    );
}

#[test]
fn claude_transcript_is_the_project_session_file_named_after_the_acp_session_id() {
    let config_dir = TempDir::new().unwrap();
    let cwd = Path::new("/workspace/repo");
    let session_id = Uuid::new_v4();
    let kind = CliTranscriptKind::Claude;
    assert_eq!(kind.locate(config_dir.path(), cwd, session_id), None);

    let transcript = config_dir
        .path()
        .join("projects")
        .join(encode_cwd(cwd))
        .join(format!("{session_id}.jsonl"));
    write(&transcript);

    assert_eq!(
        kind.locate(config_dir.path(), cwd, session_id),
        Some(transcript)
    );
    assert_eq!(
        kind.locate(config_dir.path(), Path::new("/elsewhere"), session_id),
        None,
        "the transcript is keyed by the session's working directory"
    );
}

#[test]
fn pi_fallback_prefers_session_identity_over_newer_same_cwd_files() {
    let dir = TempDir::new().unwrap();
    let sessions = dir.path().join("sessions");
    fs::create_dir_all(&sessions).unwrap();
    let cwd = Path::new("/workspace/repo");
    let session_id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
    let transcript =
        sessions.join("2026-10-12T10-00-00-000Z_550e8400-e29b-41d4-a716-446655440000.jsonl");
    let body = json!({"type": "session", "id": session_id, "cwd": "/workspace/repo"}).to_string();
    fs::write(&transcript, &body).unwrap();
    fs::write(
        sessions.join("newer.jsonl"),
        json!({
            "type": "session",
            "id": "550e8400-e29b-41d4-a716-446655440001",
            "cwd": "/workspace/repo"
        })
        .to_string(),
    )
    .unwrap();

    let kind = CliTranscriptKind::Pi;
    assert_eq!(
        kind.locate(dir.path(), cwd, session_id),
        Some(transcript.clone())
    );
    assert_eq!(
        kind.locate(dir.path(), Path::new("/elsewhere"), session_id),
        None
    );
    assert!(kind.locate(dir.path(), cwd, Uuid::new_v4()).is_some());
    let capture = kind
        .capture(session_id, &transcript, dir.path(), cwd, true, None)
        .unwrap();
    assert_eq!(capture.transcript_body, body.as_bytes());
    assert!(capture.usage_request.is_none());
}
