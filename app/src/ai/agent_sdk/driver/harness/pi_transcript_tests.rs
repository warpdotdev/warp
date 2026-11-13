use std::fs::OpenOptions;

use serde_json::json;
use tempfile::TempDir;
use warp_harness_usage::api::HarnessUsageSnapshot;
use warp_harness_usage::{CaptureDiagnostics, ExtractionOutcome, extract_pi, parse_jsonl};

use super::*;
use crate::ai::agent_sdk::driver::harness::pi_usage::capture_transcript_with_usage;
use crate::ai::agent_sdk::driver::harness::transcript_persistence::JSONL_LIMITS;
use crate::ai::agent_sdk::test_support::EnvVarGuard;

#[test]
#[serial_test::serial]
fn restored_pi_history_survives_the_next_capture() {
    let home = TempDir::new().unwrap();
    let _home = EnvVarGuard::set(
        if cfg!(windows) { "USERPROFILE" } else { "HOME" },
        home.path(),
    );
    let root = TempDir::new().unwrap();
    let cwd = TempDir::new().unwrap();
    let native_id = Uuid::new_v4();
    let header = json!({"type": "session", "version": 3, "id": native_id, "cwd": cwd.path()});
    let prior = r#"{"type":"message","id":"old","message":{"role":"assistant","content":[],"provider":"test","model":"model","usage":{"input":10,"output":2,"cacheRead":0,"cacheWrite":0}}}"#;
    let body = format!("{header}\n{prior}");
    let map_path = session_map_path().unwrap();
    fs::create_dir_all(map_path.parent().unwrap()).unwrap();
    let existing =
        json!({"sessionId": "other", "cwd": "/other", "sessionFile": "/other/file.jsonl"});
    fs::write(
        &map_path,
        json!({"version": 1, "sessions": {"other": existing}}).to_string(),
    )
    .unwrap();

    let id = rehydrate_pi_transcript(body.as_bytes(), root.path(), cwd.path()).unwrap();

    let map: Value = serde_json::from_slice(&fs::read(map_path).unwrap()).unwrap();
    assert_eq!(map["sessions"]["other"], existing);
    assert_eq!(map["sessions"][id.to_string()]["cwd"], json!(cwd.path()));
    let path = PathBuf::from(
        map["sessions"][id.to_string()]["sessionFile"]
            .as_str()
            .unwrap(),
    );
    assert!(path.starts_with(root.path().canonicalize().unwrap()));
    assert_eq!(fs::read_to_string(&path).unwrap(), format!("{body}\n"));
    assert_eq!(
        find_session_file(root.path(), cwd.path(), id),
        Some(path.clone())
    );
    let mut file = OpenOptions::new().append(true).open(&path).unwrap();
    let next = r#"{"type":"message","id":"new","message":{"role":"assistant","content":[],"provider":"test","model":"model","usage":{"input":20,"output":3,"cacheRead":0,"cacheWrite":0}}}"#;
    writeln!(file, "{next}").unwrap();

    let capture = capture_transcript_with_usage(id, &path, true, None).unwrap();
    let parsed = parse_jsonl(capture.transcript_body.as_slice(), JSONL_LIMITS);
    let diagnostics = CaptureDiagnostics {
        root: parsed.diagnostics,
        ..Default::default()
    };
    let ExtractionOutcome::Usable(usage) =
        extract_pi(&id.to_string(), &parsed.entries, &diagnostics)
    else {
        panic!("restored usage was not captured");
    };
    let HarnessUsageSnapshot::Pi(snapshot) = usage.snapshot else {
        panic!("wrong usage harness");
    };
    assert_eq!(snapshot.payload.attribution.len(), 1);
    assert_eq!(snapshot.payload.attribution[0].usage.input, Some(30));
    assert_eq!(snapshot.payload.attribution[0].usage.output, Some(5));
}

#[test]
fn an_invalid_pi_header_cannot_write_outside_the_run_directory() {
    let root = TempDir::new().unwrap();
    let cwd = Path::new("/workspace");
    for header in [
        json!({"type": "message", "id": Uuid::new_v4(), "cwd": cwd}),
        json!({"type": "session", "id": "../../elsewhere", "cwd": cwd}),
        json!({"type": "session", "id": Uuid::new_v4(), "cwd": "/different"}),
    ] {
        assert!(rehydrate_pi_transcript(header.to_string().as_bytes(), root.path(), cwd).is_err());
    }
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
#[serial_test::serial]
fn an_unreadable_pi_session_map_is_not_replaced() {
    let home = TempDir::new().unwrap();
    let _home = EnvVarGuard::set(
        if cfg!(windows) { "USERPROFILE" } else { "HOME" },
        home.path(),
    );
    let root = TempDir::new().unwrap();
    let cwd = Path::new("/workspace");
    let header = json!({"type": "session", "id": Uuid::new_v4(), "cwd": cwd});
    let map_path = session_map_path().unwrap();
    fs::create_dir_all(map_path.parent().unwrap()).unwrap();
    fs::write(&map_path, b"invalid json").unwrap();

    assert!(rehydrate_pi_transcript(header.to_string().as_bytes(), root.path(), cwd).is_err());
    assert_eq!(fs::read(map_path).unwrap(), b"invalid json");
}
