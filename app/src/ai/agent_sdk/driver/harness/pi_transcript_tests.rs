use std::fs::{self, OpenOptions};
use std::io::Write as _;

use serde_json::{Value, json};
use tempfile::TempDir;
use warp_harness_usage::api::HarnessUsageSnapshot;
use warp_harness_usage::{CaptureDiagnostics, ExtractionOutcome, extract_pi, parse_jsonl};

use super::super::json_utils::entries_to_jsonl;
use super::super::pi_usage::capture_transcript_with_usage;
use super::super::transcript_persistence::JSONL_LIMITS;
use super::*;

#[test]
fn restored_pi_usage_includes_earlier_executions_per_model() {
    let root = TempDir::new().unwrap();
    let cwd = TempDir::new().unwrap();
    let map_dir = TempDir::new().unwrap();
    let map_path = map_dir.path().join("session-map.json");
    let previous = json!({"sessionFile": "unrelated.jsonl", "cwd": "/unrelated"});
    fs::write(
        &map_path,
        json!({"version": 1, "sessions": {"other": previous}}).to_string(),
    )
    .unwrap();
    let native_id = Uuid::new_v4();
    let mut transcript = entries_to_jsonl(&[
        json!({"type": "session", "version": 3, "id": native_id, "cwd": cwd.path()}),
        json!({
            "type": "message", "id": "first", "parentId": null,
            "message": {
                "role": "assistant", "provider": "anthropic", "model": "model-a", "content": [],
                "usage": {"input": 100, "output": 10, "cacheRead": 20, "cacheWrite": 30}
            }
        }),
    ])
    .unwrap();
    transcript.pop();

    let acp_id = rehydrate_pi_transcript(&transcript, root.path(), cwd.path(), &map_path).unwrap();

    assert_ne!(acp_id, native_id);
    let map: Value = serde_json::from_slice(&fs::read(&map_path).unwrap()).unwrap();
    assert_eq!(map["sessions"]["other"], previous);
    let entry = &map["sessions"][acp_id.to_string()];
    assert_eq!(entry["sessionId"], acp_id.to_string());
    assert_eq!(entry["cwd"], json!(cwd.path()));
    let path = Path::new(entry["sessionFile"].as_str().unwrap());
    assert!(path.starts_with(root.path().canonicalize().unwrap()));
    let restored = fs::read(path).unwrap();
    assert_eq!(&restored[..restored.len() - 1], transcript);
    assert!(restored.ends_with(b"\n"));
    assert_eq!(session_header(path, cwd.path()), Some(native_id));

    let mut file = OpenOptions::new().append(true).open(path).unwrap();
    file.write_all(
        &entries_to_jsonl(&[
            json!({
                "type": "message", "id": "second", "parentId": "first",
                "message": {
                    "role": "assistant", "provider": "anthropic", "model": "model-a", "content": [],
                    "usage": {"input": 40, "output": 4, "cacheRead": 5, "cacheWrite": 6}
                }
            }),
            json!({
                "type": "message", "id": "third", "parentId": "second",
                "message": {
                    "role": "assistant", "provider": "openai", "model": "model-b", "content": [],
                    "usage": {"input": 7, "output": 8, "cacheRead": 0, "cacheWrite": 0}
                }
            }),
        ])
        .unwrap(),
    )
    .unwrap();

    let captured = capture_transcript_with_usage(acp_id, path, true, None).unwrap();
    let parsed = parse_jsonl(captured.transcript_body.as_slice(), JSONL_LIMITS);
    let outcome = extract_pi(
        &acp_id.to_string(),
        &parsed.entries,
        &CaptureDiagnostics {
            root: parsed.diagnostics,
            ..Default::default()
        },
    );
    let ExtractionOutcome::Usable(usage) = outcome else {
        panic!("expected cumulative Pi usage: {outcome:?}");
    };
    let HarnessUsageSnapshot::Pi(snapshot) = usage.snapshot else {
        panic!("expected Pi snapshot");
    };
    assert_eq!(
        serde_json::to_value(snapshot.payload.attribution).unwrap(),
        json!([
            {
                "provider": "anthropic", "model": "model-a", "maxInputTokens": 150,
                "usage": {"input": 140, "output": 14, "cacheRead": 25, "cacheWrite": 36}
            },
            {
                "provider": "openai", "model": "model-b", "maxInputTokens": 7,
                "usage": {"input": 7, "output": 8, "cacheRead": 0, "cacheWrite": 0}
            }
        ])
    );
}

#[test]
fn invalid_pi_headers_do_not_write_session_state() {
    let root = TempDir::new().unwrap();
    let map_path = root.path().join("map.json");
    let cwd = Path::new("/workspace");
    for header in [
        json!({"type": "session", "id": "../escape", "cwd": cwd}),
        json!({"type": "session", "id": Uuid::new_v4(), "cwd": "/elsewhere"}),
        json!({"type": "message", "id": Uuid::new_v4(), "cwd": cwd}),
        Value::Null,
    ] {
        assert!(
            rehydrate_pi_transcript(header.to_string().as_bytes(), root.path(), cwd, &map_path)
                .is_err()
        );
    }
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn invalid_pi_session_maps_are_not_overwritten() {
    let root = TempDir::new().unwrap();
    let map_path = root.path().join("map.json");
    let cwd = Path::new("/workspace");
    let transcript = json!({"type": "session", "id": Uuid::new_v4(), "cwd": cwd}).to_string();
    for map in [
        "not json",
        r#"{"version":2,"sessions":{}}"#,
        r#"{"version":1,"sessions":[]}"#,
    ] {
        fs::write(&map_path, map).unwrap();

        assert!(
            rehydrate_pi_transcript(transcript.as_bytes(), root.path(), cwd, &map_path).is_err()
        );

        assert_eq!(fs::read_to_string(&map_path).unwrap(), map);
        assert!(!root.path().join("sessions").exists());
    }
}

#[test]
fn an_unwritable_pi_transcript_does_not_register_a_session() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("sessions"), "not a directory").unwrap();
    let map_path = root.path().join("map.json");
    let cwd = Path::new("/workspace");
    let transcript = json!({"type": "session", "id": Uuid::new_v4(), "cwd": cwd}).to_string();

    assert!(rehydrate_pi_transcript(transcript.as_bytes(), root.path(), cwd, &map_path).is_err());

    assert!(!map_path.exists());
}
