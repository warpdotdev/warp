use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use futures::executor::block_on;
use futures::join;
use serde_json::{Value, json};
use tempfile::TempDir;
use uuid::Uuid;

use super::super::super::claude_transcript::{
    ClaudeTranscriptEnvelope, claude_config_dir, session_transcript_path,
};
use super::super::connection::tests::{FakeAgent, connect, rejecting_handler};
use super::super::protocol::{InitializeResponse, SessionCapabilities};
use super::*;
use crate::server::server_api::harness_support::MockHarnessSupportClient;

fn capabilities(resume: bool) -> SessionCapabilities {
    let agent_capabilities = if resume {
        json!({ "sessionCapabilities": { "resume": {} } })
    } else {
        json!({})
    };
    let response: InitializeResponse = serde_json::from_value(json!({
        "protocolVersion": 1,
        "agentCapabilities": agent_capabilities,
    }))
    .unwrap();
    response.agent_capabilities.session_capabilities
}

fn conversation_id() -> ServerConversationToken {
    ServerConversationToken::new("conversation-id".to_owned())
}

/// Points `CLAUDE_CONFIG_DIR` at `dir` for the test's duration.
struct ClaudeConfigDir(Option<std::ffi::OsString>);

impl ClaudeConfigDir {
    fn set(dir: &Path) -> Self {
        let previous = std::env::var_os("CLAUDE_CONFIG_DIR");
        // SAFETY: callers are `#[serial_test::serial]`, so no other thread touches the
        // process environment concurrently.
        unsafe { std::env::set_var("CLAUDE_CONFIG_DIR", dir) };
        Self(previous)
    }
}

impl Drop for ClaudeConfigDir {
    fn drop(&mut self) {
        match self.0.take() {
            // SAFETY: see `ClaudeConfigDir::set`.
            Some(dir) => unsafe { std::env::set_var("CLAUDE_CONFIG_DIR", dir) },
            // SAFETY: see `ClaudeConfigDir::set`.
            None => unsafe { std::env::remove_var("CLAUDE_CONFIG_DIR") },
        }
    }
}

fn claude_envelope(cwd: &Path, uuid: Uuid) -> bytes::Bytes {
    let envelope = ClaudeTranscriptEnvelope {
        cwd: cwd.to_path_buf(),
        uuid,
        claude_version: None,
        entries: vec![json!({"type": "user", "message": {"content": "first prompt"}})],
        subagents: HashMap::new(),
        todos: HashMap::new(),
    };
    serde_json::to_vec(&envelope).unwrap().into()
}

#[test]
fn a_new_conversation_has_nothing_to_resume() {
    assert_eq!(session_to_resume(None, &capabilities(true)), None);
}

#[test]
fn a_rehydrated_session_is_resumed_only_when_the_agent_supports_it() {
    assert_eq!(
        session_to_resume(Some(Ok("session".to_owned())), &capabilities(true)),
        Some(Ok("session".to_owned()))
    );
    assert_eq!(
        session_to_resume(Some(Ok("session".to_owned())), &capabilities(false)),
        Some(Err(ResumeMiss::AgentCannotResume))
    );
}

#[test]
fn a_rehydration_miss_is_kept_whatever_the_agent_supports() {
    assert_eq!(
        session_to_resume(Some(Err(ResumeMiss::NoTranscript)), &capabilities(true)),
        Some(Err(ResumeMiss::NoTranscript))
    );
}

#[tokio::test]
async fn harnesses_without_a_restorable_transcript_skip_the_download() {
    // No expectations: fetching the transcript would panic.
    let client = Arc::new(MockHarnessSupportClient::new());
    for kind in [Some(CliTranscriptKind::Pi), None] {
        let result =
            rehydrate_cli_session(kind, &conversation_id(), client.clone(), Path::new("/w")).await;
        assert_eq!(result, Err(ResumeMiss::Unsupported));
    }
}

#[tokio::test]
async fn a_missing_transcript_is_reported_as_such() {
    let mut client = MockHarnessSupportClient::new();
    client
        .expect_fetch_transcript()
        .returning(|| Err(anyhow::anyhow!("upstream returned status 404")));

    let result = rehydrate_cli_session(
        Some(CliTranscriptKind::Codex),
        &conversation_id(),
        Arc::new(client),
        Path::new("/w"),
    )
    .await;

    assert_eq!(result, Err(ResumeMiss::NoTranscript));
}

#[tokio::test]
#[serial_test::serial]
async fn a_claude_session_saved_under_another_directory_is_not_resumed() {
    let config_dir = TempDir::new().unwrap();
    let _env = ClaudeConfigDir::set(config_dir.path());
    let mut client = MockHarnessSupportClient::new();
    let envelope = claude_envelope(Path::new("/original/cwd"), Uuid::new_v4());
    client
        .expect_fetch_transcript()
        .returning(move || Ok(envelope.clone()));

    let result = rehydrate_cli_session(
        Some(CliTranscriptKind::Claude),
        &conversation_id(),
        Arc::new(client),
        Path::new("/different/cwd"),
    )
    .await;

    assert!(
        matches!(&result, Err(ResumeMiss::Rehydration(error)) if error.contains("working directory")),
        "unexpected result: {result:?}"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn a_claude_session_is_written_back_where_the_cli_resumes_it() {
    let config_dir = TempDir::new().unwrap();
    let _env = ClaudeConfigDir::set(config_dir.path());
    let cwd = TempDir::new().unwrap();
    let uuid = Uuid::new_v4();
    let mut client = MockHarnessSupportClient::new();
    let envelope = claude_envelope(cwd.path(), uuid);
    client
        .expect_fetch_transcript()
        .returning(move || Ok(envelope.clone()));

    let result = rehydrate_cli_session(
        Some(CliTranscriptKind::Claude),
        &conversation_id(),
        Arc::new(client),
        cwd.path(),
    )
    .await;

    assert_eq!(result, Ok(uuid.to_string()));
    let transcript = session_transcript_path(&claude_config_dir().unwrap(), cwd.path(), uuid);
    assert!(
        std::fs::read_to_string(&transcript)
            .unwrap()
            .contains("first prompt"),
        "transcript not rehydrated at {}",
        transcript.display()
    );
}

/// Answers the client's next request with `result`, or with an error when it is `None`, and
/// returns that request.
async fn answer_next(agent: &mut FakeAgent, result: Option<Value>) -> Value {
    let request = agent.next_message().await;
    let response = match result {
        Some(result) => json!({"jsonrpc": "2.0", "id": request["id"], "result": result}),
        None => json!({
            "jsonrpc": "2.0",
            "id": request["id"],
            "error": {"code": -32002, "message": "Resource not found"},
        }),
    };
    agent.send(response).await;
    request
}

#[test]
fn a_rehydrated_session_is_resumed_in_place_of_a_new_one() {
    let (connection, _notifications, mut agent) = connect(rejecting_handler());
    block_on(async {
        let open = open_session(
            &connection,
            Some(Ok("prior-session".to_owned())),
            "/workspace".to_owned(),
            Vec::new(),
        );
        let agent = async {
            let request = answer_next(&mut agent, Some(json!({"configOptions": []}))).await;
            assert_eq!(request["method"], "session/resume");
            assert_eq!(
                request["params"],
                json!({"sessionId": "prior-session", "cwd": "/workspace", "mcpServers": []})
            );
        };
        let (opened, ()) = join!(open, agent);
        let (session, origin) = opened.unwrap();
        assert_eq!(session.session_id, "prior-session");
        assert_eq!(origin, SessionOrigin::Resumed);
    });
}

#[test]
fn a_refused_resume_falls_back_to_a_new_session() {
    let (connection, _notifications, mut agent) = connect(rejecting_handler());
    block_on(async {
        let open = open_session(
            &connection,
            Some(Ok("prior-session".to_owned())),
            "/workspace".to_owned(),
            Vec::new(),
        );
        let agent = async {
            let resume = answer_next(&mut agent, None).await;
            assert_eq!(resume["method"], "session/resume");
            let new = answer_next(&mut agent, Some(json!({"sessionId": "fresh"}))).await;
            assert_eq!(new["method"], "session/new");
        };
        let (opened, ()) = join!(open, agent);
        let (session, origin) = opened.unwrap();
        assert_eq!(session.session_id, "fresh");
        assert!(
            matches!(&origin, SessionOrigin::ResumeMissed(ResumeMiss::Rejected(error)) if error.contains("Resource not found")),
            "unexpected origin: {origin:?}"
        );
    });
}

#[test]
fn without_a_session_to_resume_a_new_one_is_opened() {
    for (to_resume, expected_origin) in [
        (None, SessionOrigin::New),
        (
            Some(Err(ResumeMiss::NoTranscript)),
            SessionOrigin::ResumeMissed(ResumeMiss::NoTranscript),
        ),
    ] {
        let (connection, _notifications, mut agent) = connect(rejecting_handler());
        block_on(async {
            let open = open_session(&connection, to_resume, "/workspace".to_owned(), Vec::new());
            let agent = async {
                let request = answer_next(&mut agent, Some(json!({"sessionId": "fresh"}))).await;
                assert_eq!(request["method"], "session/new");
            };
            let (opened, ()) = join!(open, agent);
            let (session, origin) = opened.unwrap();
            assert_eq!(session.session_id, "fresh");
            assert_eq!(origin, expected_origin);
        });
    }
}
