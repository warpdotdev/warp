use std::collections::HashMap;
use std::fs;
use std::path::Path;

use tempfile::TempDir;
use uuid::Uuid;

use super::*;

fn write_file(dir: &Path, name: &str, content: &str) {
    fs::write(dir.join(name), content).unwrap();
}

#[test]
fn encode_cwd_replaces_slashes_and_dots() {
    assert_eq!(
        encode_cwd(Path::new("/Users/ben/src/foo")),
        "-Users-ben-src-foo"
    );
    assert_eq!(encode_cwd(Path::new("/")), "-");
    assert_eq!(encode_cwd(Path::new("/a/b")), "-a-b");
    assert_eq!(
        encode_cwd(Path::new("/Users/ben/.config/foo")),
        "-Users-ben--config-foo"
    );
    assert_eq!(encode_cwd(Path::new("/a.b/c.d")), "-a-b-c-d");
}

#[test]
fn read_envelope_main_only() {
    let tmp = TempDir::new().unwrap();
    let cwd = Path::new("/my/project");
    let uuid = Uuid::new_v4();

    let encoded = encode_cwd(cwd);
    let projects_dir = tmp.path().join("projects").join(&encoded);
    fs::create_dir_all(&projects_dir).unwrap();
    write_file(
        &projects_dir,
        &format!("{uuid}.jsonl"),
        "{\"type\":\"user\"}\n{\"type\":\"assistant\"}\n",
    );

    let envelope = read_envelope(uuid, cwd, tmp.path()).unwrap();
    assert_eq!(
        envelope.entries,
        vec![
            serde_json::json!({"type": "user"}),
            serde_json::json!({"type": "assistant"}),
        ]
    );
    assert_eq!(envelope.uuid, uuid);
    assert_eq!(envelope.cwd, cwd);
    assert!(envelope.subagents.is_empty());
    assert!(envelope.todos.is_empty());
}

#[test]
fn read_envelope_with_subagents() {
    let tmp = TempDir::new().unwrap();
    let cwd = Path::new("/my/project");
    let uuid = Uuid::new_v4();

    let encoded = encode_cwd(cwd);
    let projects_dir = tmp.path().join("projects").join(&encoded);
    fs::create_dir_all(&projects_dir).unwrap();
    write_file(&projects_dir, &format!("{uuid}.jsonl"), "");

    let subagents_dir = projects_dir.join(uuid.to_string()).join("subagents");
    fs::create_dir_all(&subagents_dir).unwrap();
    write_file(
        &subagents_dir,
        "agent-abc123def456.jsonl",
        "{\"type\":\"user\"}\n",
    );

    let envelope = read_envelope(uuid, cwd, tmp.path()).unwrap();
    assert_eq!(
        envelope.subagents["agent-abc123def456"],
        vec![serde_json::json!({"type": "user"})]
    );
}

#[test]
fn read_envelope_missing_session_file() {
    let tmp = TempDir::new().unwrap();
    let cwd = Path::new("/my/project");
    let uuid = Uuid::new_v4();

    // No files created - should return Ok with empty entries rather than an error.
    let envelope = read_envelope(uuid, cwd, tmp.path()).unwrap();
    assert!(envelope.entries.is_empty());
    assert!(envelope.subagents.is_empty());
    assert!(envelope.todos.is_empty());
}

#[test]
fn write_envelope_creates_files() {
    let tmp = TempDir::new().unwrap();
    let cwd = Path::new("/my/project");
    let uuid = Uuid::new_v4();
    let todo_stem = format!("{uuid}-agent-{uuid}");

    let envelope = ClaudeTranscriptEnvelope {
        cwd: cwd.to_path_buf(),
        uuid,
        claude_version: None,
        entries: vec![serde_json::json!({"type": "user"})],
        subagents: HashMap::from([(
            "agent-abc".to_string(),
            vec![serde_json::json!({"type": "assistant"})],
        )]),
        todos: HashMap::from([(
            todo_stem.clone(),
            serde_json::json!([{"id": "1", "title": "Do it"}]),
        )]),
    };

    write_envelope(&envelope, tmp.path()).unwrap();

    let encoded = encode_cwd(cwd);
    let projects_dir = tmp.path().join("projects").join(&encoded);

    // Main session JSONL.
    let session_file = projects_dir.join(format!("{uuid}.jsonl"));
    assert!(session_file.exists(), "session JSONL missing");
    assert_eq!(read_jsonl(&session_file).unwrap(), envelope.entries);

    // Subagent JSONL.
    let subagent_file = projects_dir
        .join(uuid.to_string())
        .join("subagents")
        .join("agent-abc.jsonl");
    assert!(subagent_file.exists(), "subagent JSONL missing");
    assert_eq!(
        read_jsonl(&subagent_file).unwrap(),
        envelope.subagents["agent-abc"]
    );

    // Todo JSON.
    let todo_file = tmp.path().join("todos").join(format!("{todo_stem}.json"));
    assert!(todo_file.exists(), "todo JSON missing");
    let todo_on_disk: serde_json::Value =
        serde_json::from_slice(&fs::read(&todo_file).unwrap()).unwrap();
    assert_eq!(todo_on_disk, envelope.todos[&todo_stem]);
}

#[test]
fn write_envelope_round_trip() {
    let tmp = TempDir::new().unwrap();
    let cwd = Path::new("/test/dir");
    let uuid = Uuid::new_v4();

    let original = ClaudeTranscriptEnvelope {
        cwd: cwd.to_path_buf(),
        uuid,
        claude_version: None,
        entries: vec![serde_json::json!({"type": "user"})],
        subagents: HashMap::new(),
        todos: HashMap::new(),
    };

    write_envelope(&original, tmp.path()).unwrap();

    let decoded = read_envelope(uuid, cwd, tmp.path()).unwrap();
    assert_eq!(decoded, original);
}

#[test]
fn write_session_index_entry_creates_missing_file() {
    let tmp = TempDir::new().unwrap();
    let uuid = Uuid::new_v4();
    let cwd = Path::new("/my/project");

    write_session_index_entry(uuid, cwd, tmp.path()).unwrap();

    let index: serde_json::Value =
        serde_json::from_slice(&fs::read(tmp.path().join("sessions-index.json")).unwrap()).unwrap();
    let entry = &index[uuid.to_string()];
    assert_eq!(entry["sessionId"], uuid.to_string());
    assert_eq!(entry["cwd"], "/my/project");
    assert_eq!(entry["projectPath"], "-my-project");
    assert_eq!(
        entry["transcriptPath"],
        format!("projects/-my-project/{uuid}.jsonl")
    );
}

#[test]
fn write_session_index_entry_preserves_other_entries() {
    let tmp = TempDir::new().unwrap();
    let other_uuid = Uuid::new_v4();
    fs::write(
        tmp.path().join("sessions-index.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            other_uuid.to_string(): {"sessionId": other_uuid.to_string(), "custom_field": 42},
        }))
        .unwrap(),
    )
    .unwrap();

    let new_uuid = Uuid::new_v4();
    write_session_index_entry(new_uuid, Path::new("/my/project"), tmp.path()).unwrap();

    let index: serde_json::Value =
        serde_json::from_slice(&fs::read(tmp.path().join("sessions-index.json")).unwrap()).unwrap();
    // New entry landed.
    assert_eq!(
        index[new_uuid.to_string()]["sessionId"],
        new_uuid.to_string()
    );
    // Old entry preserved verbatim, including unknown fields.
    assert_eq!(
        index[other_uuid.to_string()]["sessionId"],
        other_uuid.to_string()
    );
    assert_eq!(index[other_uuid.to_string()]["custom_field"], 42);
}

#[test]
fn write_session_index_entry_overwrites_same_session() {
    let tmp = TempDir::new().unwrap();
    let uuid = Uuid::new_v4();

    write_session_index_entry(uuid, Path::new("/old/cwd"), tmp.path()).unwrap();
    write_session_index_entry(uuid, Path::new("/new/cwd"), tmp.path()).unwrap();

    let index: serde_json::Value =
        serde_json::from_slice(&fs::read(tmp.path().join("sessions-index.json")).unwrap()).unwrap();
    // Only one entry for this session id, with the newer cwd.
    assert_eq!(index.as_object().unwrap().len(), 1);
    assert_eq!(index[uuid.to_string()]["cwd"], "/new/cwd");
}

#[test]
fn write_session_index_entry_overwrites_malformed_file() {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("sessions-index.json"), b"not json").unwrap();

    let uuid = Uuid::new_v4();
    write_session_index_entry(uuid, Path::new("/my/project"), tmp.path()).unwrap();

    let index: serde_json::Value =
        serde_json::from_slice(&fs::read(tmp.path().join("sessions-index.json")).unwrap()).unwrap();
    assert_eq!(index[uuid.to_string()]["sessionId"], uuid.to_string());
}
