use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use futures::executor::block_on;
use futures::{FutureExt, join, pin_mut, select};
use serde_json::{Value, json};
use tempfile::TempDir;
use uuid::Uuid;

use super::super::super::claude_transcript::{
    ClaudeTranscriptEnvelope, claude_config_dir, session_transcript_path,
};
use super::super::connection::InboundNotification;
use super::super::connection::tests::{FakeAgent, connect, rejecting_handler};
use super::super::protocol::InitializeResponse;
use super::*;
use crate::ai::agent_sdk::test_support::EnvVarGuard;
use crate::server::server_api::harness_support::MockHarnessSupportClient;

/// The capabilities of an agent whose `initialize` answer carried `agent_capabilities`.
fn announced(agent_capabilities: Value) -> AgentCapabilities {
    let response: InitializeResponse = serde_json::from_value(json!({
        "protocolVersion": 1,
        "agentCapabilities": agent_capabilities,
    }))
    .unwrap();
    response.agent_capabilities
}

fn capabilities(resume: bool, load: bool) -> AgentCapabilities {
    let session_capabilities = if resume {
        json!({ "resume": {} })
    } else {
        json!({})
    };
    announced(json!({ "loadSession": load, "sessionCapabilities": session_capabilities }))
}

const RESUME_ONLY: (bool, bool) = (true, false);
const LOAD_ONLY: (bool, bool) = (false, true);
const RESUME_AND_LOAD: (bool, bool) = (true, true);
const NEITHER: (bool, bool) = (false, false);

fn reopen(method: ReopenMethod) -> SessionPlan {
    SessionPlan::Reopen {
        method,
        session_id: "prior-session".to_owned(),
    }
}

fn session_update(text: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "session/update",
        "params": {
            "sessionId": "prior-session",
            "update": {
                "sessionUpdate": "agent_message_chunk",
                "content": { "type": "text", "text": text },
            },
        },
    })
}

fn update_text(notification: &InboundNotification) -> &str {
    assert_eq!(notification.method, "session/update");
    notification.params["update"]["content"]["text"]
        .as_str()
        .unwrap()
}

fn conversation_id() -> ServerConversationToken {
    ServerConversationToken::new("conversation-id".to_owned())
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

/// A client whose transcript endpoint answers every fetch with `response`.
fn client_answering(
    response: impl Fn() -> anyhow::Result<bytes::Bytes> + Send + 'static,
) -> Arc<dyn HarnessSupportClient> {
    let mut client = MockHarnessSupportClient::new();
    client.expect_fetch_transcript().returning(response);
    Arc::new(client)
}

fn error_chain(error: &AgentDriverError) -> String {
    anyhow::Chain::new(error)
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(": ")
}

#[test]
fn harness_transcripts_with_a_located_root_are_restorable() {
    assert_eq!(
        restorable_transcript(None, true),
        Err(ResumeMiss::Unsupported)
    );
    assert_eq!(
        restorable_transcript(Some(CliTranscriptKind::Pi), true),
        Ok(CliTranscriptKind::Pi)
    );
    assert_eq!(
        restorable_transcript(Some(CliTranscriptKind::Claude), false),
        Err(ResumeMiss::TranscriptRootUnresolved(
            CliTranscriptKind::Claude
        ))
    );
    assert_eq!(
        restorable_transcript(Some(CliTranscriptKind::Codex), true),
        Ok(CliTranscriptKind::Codex)
    );
}

#[test]
fn an_unresolved_transcript_root_names_the_startup_warning() {
    let miss = ResumeMiss::TranscriptRootUnresolved(CliTranscriptKind::Claude).to_string();
    assert!(
        miss.contains("Cannot locate Claude transcripts"),
        "unexpected message: {miss}"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn a_pi_transcript_is_loaded_from_the_run_directory() {
    let home = TempDir::new().unwrap();
    let _home = EnvVarGuard::set(
        if cfg!(windows) { "USERPROFILE" } else { "HOME" },
        home.path(),
    );
    let root = TempDir::new().unwrap();
    let cwd = TempDir::new().unwrap();
    let transcript = format!(
        "{}\n",
        json!({"type": "session", "version": 3, "id": Uuid::new_v4(), "cwd": cwd.path()})
    );
    let bytes = bytes::Bytes::from(transcript.clone());
    let client = client_answering(move || Ok(bytes.clone()));

    let session_id = rehydrate_cli_session(
        CliTranscriptKind::Pi,
        &conversation_id(),
        client,
        cwd.path(),
        root.path(),
    )
    .await
    .unwrap();
    let id = Uuid::parse_str(&session_id).unwrap();
    let path = CliTranscriptKind::Pi
        .locate(root.path(), cwd.path(), id)
        .unwrap();
    assert_eq!(std::fs::read_to_string(path).unwrap(), transcript);
    let (pending, result_tx, _abort) = pending_rehydration();
    result_tx.send(Ok(session_id.clone())).unwrap();
    let (resume, load) = LOAD_ONLY;
    let plan = plan_session(Some(Ok(pending)), &capabilities(resume, load))
        .await
        .unwrap();
    assert_eq!(
        plan,
        SessionPlan::Reopen {
            method: ReopenMethod::Load,
            session_id
        }
    );
}

#[tokio::test]
async fn a_missing_transcript_is_a_miss() {
    let client = client_answering(|| Err(anyhow::anyhow!("upstream returned status 404")));

    let result = rehydrate_cli_session(
        CliTranscriptKind::Codex,
        &conversation_id(),
        client,
        Path::new("/w"),
        Path::new("/unused"),
    )
    .await;

    assert!(
        matches!(
            result,
            Err(RehydrateError::Missed(ResumeMiss::NoTranscript))
        ),
        "unexpected result: {result:?}"
    );
}

#[tokio::test]
async fn a_transcript_that_cannot_be_downloaded_fails_the_run() {
    for client in [
        client_answering(|| Err(anyhow::anyhow!("upstream returned status 503"))),
        client_answering(|| Ok(bytes::Bytes::from_static(b"not json"))),
    ] {
        let result = rehydrate_cli_session(
            CliTranscriptKind::Codex,
            &conversation_id(),
            client,
            Path::new("/w"),
            Path::new("/unused"),
        )
        .await;

        assert!(
            matches!(
                result,
                Err(RehydrateError::Failed(
                    AgentDriverError::ConversationLoadFailed(_)
                ))
            ),
            "unexpected result: {result:?}"
        );
    }
}

#[tokio::test]
#[serial_test::serial]
async fn a_claude_session_saved_under_another_directory_fails_the_run() {
    let config_dir = TempDir::new().unwrap();
    let _config_dir = EnvVarGuard::set("CLAUDE_CONFIG_DIR", config_dir.path());
    let envelope = claude_envelope(Path::new("/original/cwd"), Uuid::new_v4());
    let client = client_answering(move || Ok(envelope.clone()));

    let result = rehydrate_cli_session(
        CliTranscriptKind::Claude,
        &conversation_id(),
        client,
        Path::new("/different/cwd"),
        config_dir.path(),
    )
    .await;

    let Err(RehydrateError::Failed(error @ AgentDriverError::ConfigBuildFailed(_))) = result else {
        panic!("unexpected result: {result:?}");
    };
    let chain = error_chain(&error);
    assert!(chain.contains("working directory"), "{chain}");
}

#[tokio::test]
#[serial_test::serial]
async fn a_claude_session_is_written_back_where_the_cli_resumes_it() {
    let config_dir = TempDir::new().unwrap();
    let _config_dir = EnvVarGuard::set("CLAUDE_CONFIG_DIR", config_dir.path());
    let cwd = TempDir::new().unwrap();
    let uuid = Uuid::new_v4();
    let envelope = claude_envelope(cwd.path(), uuid);
    let client = client_answering(move || Ok(envelope.clone()));

    let result = rehydrate_cli_session(
        CliTranscriptKind::Claude,
        &conversation_id(),
        client,
        cwd.path(),
        config_dir.path(),
    )
    .await;

    assert_eq!(result.unwrap(), uuid.to_string());
    let transcript = session_transcript_path(&claude_config_dir().unwrap(), cwd.path(), uuid);
    assert!(
        std::fs::read_to_string(&transcript)
            .unwrap()
            .contains("first prompt"),
        "transcript not rehydrated at {}",
        transcript.display()
    );
}

/// A rehydration whose outcome the test supplies through the returned sender.
fn pending_rehydration() -> (
    PendingRehydration,
    oneshot::Sender<Result<String, RehydrateError>>,
    AbortHandle,
) {
    let (result_tx, result_rx) = oneshot::channel();
    let (abort, _registration) = AbortHandle::new_pair();
    (
        PendingRehydration::from_parts(result_rx, abort.clone()),
        result_tx,
        abort,
    )
}

#[test]
fn a_new_conversation_has_nothing_to_resume() {
    for (resume, load) in [RESUME_ONLY, LOAD_ONLY, RESUME_AND_LOAD, NEITHER] {
        let plan = block_on(plan_session(None, &capabilities(resume, load))).unwrap();
        assert_eq!(plan, SessionPlan::New);
    }
}

#[test]
fn a_miss_decided_before_launch_is_kept_whatever_the_agent_supports() {
    for (resume, load) in [RESUME_ONLY, LOAD_ONLY, RESUME_AND_LOAD, NEITHER] {
        let miss = ResumeMiss::TranscriptRootUnresolved(CliTranscriptKind::Claude);
        let plan = block_on(plan_session(
            Some(Err(miss.clone())),
            &capabilities(resume, load),
        ))
        .unwrap();
        assert_eq!(plan, SessionPlan::Missed(miss));
    }
}

#[test]
fn a_rehydrated_session_is_reopened_through_the_method_the_agent_announces() {
    for ((resume, load), method) in [
        (RESUME_ONLY, ReopenMethod::Resume),
        (LOAD_ONLY, ReopenMethod::Load),
        (RESUME_AND_LOAD, ReopenMethod::Resume),
    ] {
        let (pending, result_tx, _abort) = pending_rehydration();
        result_tx.send(Ok("session".to_owned())).unwrap();

        let plan = block_on(plan_session(Some(Ok(pending)), &capabilities(resume, load))).unwrap();

        assert_eq!(
            plan,
            SessionPlan::Reopen {
                method,
                session_id: "session".to_owned(),
            },
            "resume={resume} load={load}"
        );
    }
}

#[test]
fn the_reopening_method_follows_what_real_adapters_announce() {
    let pi_acp = announced(json!({
        "loadSession": true,
        "sessionCapabilities": { "list": {}, "delete": {} },
    }));
    assert_eq!(ReopenMethod::for_agent(&pi_acp), Some(ReopenMethod::Load));

    let claude_agent_acp = announced(json!({
        "loadSession": true,
        "sessionCapabilities": { "resume": {}, "list": {}, "close": {} },
    }));
    assert_eq!(
        ReopenMethod::for_agent(&claude_agent_acp),
        Some(ReopenMethod::Resume)
    );

    assert_eq!(ReopenMethod::for_agent(&announced(json!({}))), None);
}

#[test]
fn the_download_is_abandoned_when_the_agent_cannot_reopen_a_session() {
    let (pending, _result_tx, abort) = pending_rehydration();
    let (resume, load) = NEITHER;

    // The sender never fires, so waiting for the download would hang.
    let plan = block_on(plan_session(Some(Ok(pending)), &capabilities(resume, load))).unwrap();

    assert_eq!(plan, SessionPlan::Missed(ResumeMiss::AgentCannotReopen));
    assert!(abort.is_aborted());
}

#[test]
fn a_transcript_found_missing_after_launch_is_a_miss() {
    for (resume, load) in [RESUME_ONLY, LOAD_ONLY] {
        let (pending, result_tx, _abort) = pending_rehydration();
        result_tx
            .send(Err(RehydrateError::Missed(ResumeMiss::NoTranscript)))
            .unwrap();

        let plan = block_on(plan_session(Some(Ok(pending)), &capabilities(resume, load))).unwrap();

        assert_eq!(plan, SessionPlan::Missed(ResumeMiss::NoTranscript));
    }
}

#[test]
fn a_rehydration_failure_fails_the_handshake_with_its_own_error() {
    let (pending, result_tx, _abort) = pending_rehydration();
    result_tx
        .send(Err(RehydrateError::Failed(
            AgentDriverError::ConversationLoadFailed("upstream returned status 503".to_owned()),
        )))
        .unwrap();

    let error = block_on(plan_session(Some(Ok(pending)), &capabilities(true, false))).unwrap_err();

    assert!(
        matches!(&error, AgentDriverError::ConversationLoadFailed(message) if message.contains("503")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn a_dropped_rehydration_fails_the_handshake() {
    let (pending, result_tx, _abort) = pending_rehydration();
    drop(result_tx);

    let error = block_on(plan_session(Some(Ok(pending)), &capabilities(false, true))).unwrap_err();

    assert!(
        matches!(&error, AgentDriverError::ConfigBuildFailed(_)),
        "unexpected error: {error:?}"
    );
}

#[test]
fn a_restarted_phase_gets_the_full_budget_again() {
    let budget = Duration::from_millis(400);
    let clock = PhaseClock::start(HarnessSetupStage::Initialize, "first");
    block_on(async {
        let expire = clock.expire(budget).fuse();
        // 500ms of work in total, which outruns one budget but not two phases of it.
        let work = async {
            Timer::after(Duration::from_millis(200)).await;
            clock.restart(HarnessSetupStage::SessionOpen, "second");
            Timer::after(Duration::from_millis(300)).await;
        }
        .fuse();
        pin_mut!(expire, work);
        select! {
            () = expire => panic!("the deadline ignored the restart"),
            () = work => {}
        }
    });
    assert_eq!(clock.phase(), "second");
    assert_eq!(clock.stage(), HarnessSetupStage::SessionOpen);
}

#[test]
fn a_phase_that_is_not_restarted_expires() {
    let clock = PhaseClock::start(HarnessSetupStage::Initialize, "only");
    block_on(async {
        let expire = clock.expire(Duration::from_millis(100)).fuse();
        let work = Timer::after(Duration::from_millis(400)).fuse();
        pin_mut!(expire, work);
        select! {
            () = expire => {}
            _ = work => panic!("the deadline never fired"),
        }
    });
    assert_eq!(clock.phase(), "only");
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
    let clock = PhaseClock::start(HarnessSetupStage::SessionOpen, "opening");
    block_on(async {
        let open = open_session(
            &connection,
            reopen(ReopenMethod::Resume),
            "/workspace".to_owned(),
            Vec::new(),
            &clock,
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
        assert_eq!(origin, SessionOrigin::Resumed(ReopenMethod::Resume));
    });
    assert_eq!(clock.phase(), "opening");
}

#[test]
fn a_resumed_session_keeps_the_notifications_around_its_answer() {
    let (connection, notifications, mut agent) = connect(rejecting_handler());
    let clock = PhaseClock::start(HarnessSetupStage::SessionOpen, "opening");
    block_on(async {
        let open = open_session(
            &connection,
            reopen(ReopenMethod::Resume),
            "/workspace".to_owned(),
            Vec::new(),
            &clock,
        );
        let agent = async {
            let request = agent.next_message().await;
            agent.send(session_update("before the answer")).await;
            agent
                .send(json!({"jsonrpc": "2.0", "id": request["id"], "result": {}}))
                .await;
            agent.send(session_update("after the answer")).await;
        };
        let (opened, ()) = join!(open, agent);
        opened.unwrap();
        assert_eq!(
            update_text(&notifications.recv().await.unwrap()),
            "before the answer"
        );
        assert_eq!(
            update_text(&notifications.recv().await.unwrap()),
            "after the answer"
        );
    });
}

#[test]
fn a_loaded_session_discards_its_replayed_history_and_keeps_what_follows() {
    let (connection, notifications, mut agent) = connect(rejecting_handler());
    let clock = PhaseClock::start(HarnessSetupStage::SessionOpen, "opening");
    block_on(async {
        let open = open_session(
            &connection,
            reopen(ReopenMethod::Load),
            "/workspace".to_owned(),
            Vec::new(),
            &clock,
        );
        let agent = async {
            let request = agent.next_message().await;
            assert_eq!(request["method"], "session/load");
            assert_eq!(
                request["params"],
                json!({"sessionId": "prior-session", "cwd": "/workspace", "mcpServers": []})
            );
            agent.send(session_update("replayed prompt")).await;
            agent.send(session_update("replayed answer")).await;
            agent
                .send(json!({"jsonrpc": "2.0", "method": "_adapter/replayed", "params": {}}))
                .await;
            agent
                .send(json!({"jsonrpc": "2.0", "id": request["id"], "result": null}))
                .await;
            // Written straight after the answer, before the client has acted on it.
            agent.send(session_update("live")).await;
        };
        let (opened, ()) = join!(open, agent);
        let (session, origin) = opened.unwrap();
        assert_eq!(session.session_id, "prior-session");
        assert!(session.config_options.is_empty());
        assert_eq!(origin, SessionOrigin::Resumed(ReopenMethod::Load));

        assert_eq!(update_text(&notifications.recv().await.unwrap()), "live");
        assert!(notifications.try_recv().is_err());
    });
    assert_eq!(clock.phase(), "opening");
}

#[test]
fn a_refused_load_discards_its_partial_replay_and_falls_back_to_a_new_session() {
    let (connection, notifications, mut agent) = connect(rejecting_handler());
    let clock = PhaseClock::start(HarnessSetupStage::SessionOpen, "opening");
    block_on(async {
        let open = open_session(
            &connection,
            reopen(ReopenMethod::Load),
            "/workspace".to_owned(),
            Vec::new(),
            &clock,
        );
        let agent = async {
            let load = agent.next_message().await;
            assert_eq!(load["method"], "session/load");
            agent.send(session_update("replayed prompt")).await;
            agent
                .send(json!({
                    "jsonrpc": "2.0",
                    "id": load["id"],
                    "error": {"code": -32603, "message": "Session file is corrupt"},
                }))
                .await;
            let new = answer_next(&mut agent, Some(json!({"sessionId": "fresh"}))).await;
            assert_eq!(new["method"], "session/new");
            assert_eq!(
                new["params"],
                json!({"cwd": "/workspace", "mcpServers": []})
            );
            agent.send(session_update("live")).await;
        };
        let (opened, ()) = join!(open, agent);
        let (session, origin) = opened.unwrap();
        assert_eq!(session.session_id, "fresh");
        assert!(
            matches!(
                &origin,
                SessionOrigin::ResumeMissed(ResumeMiss::Rejected { method: ReopenMethod::Load, error })
                    if error.contains("Session file is corrupt")
            ),
            "unexpected origin: {origin:?}"
        );

        assert_eq!(update_text(&notifications.recv().await.unwrap()), "live");
        assert!(notifications.try_recv().is_err());
    });
    assert_eq!(clock.phase(), FALLBACK_SESSION_PHASE);
    assert_eq!(clock.stage(), HarnessSetupStage::SessionOpen);
}

#[test]
fn a_load_the_agent_does_not_answer_fails_the_handshake() {
    let (connection, _notifications, mut agent) = connect(rejecting_handler());
    let clock = PhaseClock::start(HarnessSetupStage::SessionOpen, "opening");
    block_on(async {
        let open = open_session(
            &connection,
            reopen(ReopenMethod::Load),
            "/workspace".to_owned(),
            Vec::new(),
            &clock,
        );
        let agent = async {
            let request = agent.next_message().await;
            assert_eq!(request["method"], "session/load");
            agent.send(session_update("replayed prompt")).await;
            agent.hang_up()
        };
        let (opened, _read_loop) = join!(open, agent);
        let error = opened.unwrap_err();
        assert!(error.downcast_ref::<RpcError>().is_none(), "{error:#}");
        let message = format!("{error:#}");
        assert!(message.contains("session/load failed"), "{message}");
        assert!(message.contains("closed"), "{message}");
    });
    // No fallback was attempted.
    assert_eq!(clock.phase(), "opening");
}

#[test]
fn a_load_answer_that_cannot_be_read_fails_the_handshake() {
    let (connection, _notifications, mut agent) = connect(rejecting_handler());
    let clock = PhaseClock::start(HarnessSetupStage::SessionOpen, "opening");
    block_on(async {
        let open = open_session(
            &connection,
            reopen(ReopenMethod::Load),
            "/workspace".to_owned(),
            Vec::new(),
            &clock,
        );
        let agent = async {
            let request = answer_next(&mut agent, Some(json!("not a session"))).await;
            assert_eq!(request["method"], "session/load");
        };
        let (opened, ()) = join!(open, agent);
        let error = opened.unwrap_err();
        assert!(error.downcast_ref::<RpcError>().is_none(), "{error:#}");
        let message = format!("{error:#}");
        assert!(message.contains("session/load failed"), "{message}");
    });
    assert_eq!(clock.phase(), "opening");
}

#[test]
fn each_session_origin_has_its_own_metric_label() {
    let rejected = |method| {
        SessionOrigin::ResumeMissed(ResumeMiss::Rejected {
            method,
            error: "refused".to_owned(),
        })
    };
    let labelled = [
        (SessionOrigin::New, "new"),
        (SessionOrigin::Resumed(ReopenMethod::Resume), "resumed"),
        (SessionOrigin::Resumed(ReopenMethod::Load), "loaded"),
        (
            SessionOrigin::ResumeMissed(ResumeMiss::Unsupported),
            "resume_missed_unsupported",
        ),
        (
            SessionOrigin::ResumeMissed(ResumeMiss::TranscriptRootUnresolved(
                CliTranscriptKind::Claude,
            )),
            "resume_missed_transcript_root_unresolved",
        ),
        (
            SessionOrigin::ResumeMissed(ResumeMiss::NoTranscript),
            "resume_missed_no_transcript",
        ),
        (
            SessionOrigin::ResumeMissed(ResumeMiss::AgentCannotReopen),
            "resume_missed_agent_cannot_reopen",
        ),
        (rejected(ReopenMethod::Resume), "resume_missed_rejected"),
        (rejected(ReopenMethod::Load), "resume_missed_load_rejected"),
    ];
    for (origin, label) in &labelled {
        assert_eq!(origin.metric_label(), *label, "{origin:?}");
        // The server drops a metric whose outcome label is longer than this.
        assert!(label.len() <= 64, "{label}");
    }
    let mut labels = labelled.iter().map(|(_, label)| *label).collect::<Vec<_>>();
    labels.sort_unstable();
    labels.dedup();
    assert_eq!(labels.len(), labelled.len());
}

#[test]
fn a_miss_names_the_method_the_agent_would_not_take() {
    assert_eq!(
        ResumeMiss::AgentCannotReopen.to_string(),
        "the agent supports neither session/resume nor session/load"
    );
    for (method, name) in [
        (ReopenMethod::Resume, "session/resume"),
        (ReopenMethod::Load, "session/load"),
    ] {
        let miss = ResumeMiss::Rejected {
            method,
            error: "ACP error -32002: Resource not found".to_owned(),
        };
        assert_eq!(
            miss.to_string(),
            format!("the agent refused {name}: ACP error -32002: Resource not found")
        );
    }
}

#[test]
fn a_refused_resume_falls_back_to_a_new_session_with_a_fresh_phase() {
    let (connection, _notifications, mut agent) = connect(rejecting_handler());
    let clock = PhaseClock::start(HarnessSetupStage::SessionOpen, "opening");
    block_on(async {
        let open = open_session(
            &connection,
            reopen(ReopenMethod::Resume),
            "/workspace".to_owned(),
            Vec::new(),
            &clock,
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
            matches!(
                &origin,
                SessionOrigin::ResumeMissed(ResumeMiss::Rejected { method: ReopenMethod::Resume, error })
                    if error.contains("Resource not found")
            ),
            "unexpected origin: {origin:?}"
        );
    });
    assert_eq!(clock.phase(), FALLBACK_SESSION_PHASE);
    assert_eq!(clock.stage(), HarnessSetupStage::SessionOpen);
}

#[test]
fn a_resume_the_agent_does_not_answer_fails_the_handshake() {
    let (connection, _notifications, mut agent) = connect(rejecting_handler());
    let clock = PhaseClock::start(HarnessSetupStage::SessionOpen, "opening");
    block_on(async {
        let open = open_session(
            &connection,
            reopen(ReopenMethod::Resume),
            "/workspace".to_owned(),
            Vec::new(),
            &clock,
        );
        let agent = async {
            let request = agent.next_message().await;
            assert_eq!(request["method"], "session/resume");
            agent.hang_up()
        };
        let (opened, _read_loop) = join!(open, agent);
        let error = opened.unwrap_err();
        assert!(error.downcast_ref::<RpcError>().is_none(), "{error:#}");
        let message = format!("{error:#}");
        assert!(message.contains("session/resume failed"), "{message}");
        assert!(message.contains("closed"), "{message}");
    });
    // No fallback was attempted.
    assert_eq!(clock.phase(), "opening");
}

#[test]
fn without_a_session_to_resume_a_new_one_is_opened() {
    for (plan, expected_origin) in [
        (SessionPlan::New, SessionOrigin::New),
        (
            SessionPlan::Missed(ResumeMiss::NoTranscript),
            SessionOrigin::ResumeMissed(ResumeMiss::NoTranscript),
        ),
    ] {
        let (connection, _notifications, mut agent) = connect(rejecting_handler());
        let clock = PhaseClock::start(HarnessSetupStage::SessionOpen, "opening");
        block_on(async {
            let open = open_session(
                &connection,
                plan,
                "/workspace".to_owned(),
                Vec::new(),
                &clock,
            );
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
