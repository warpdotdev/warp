//! The vertical slice, end to end with real parts: a Cursor `hooks.json` on disk, a real script,
//! the real registry loader, the real dispatcher, and the real process runner.

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use ai_types::{AIAgentActionId, AIConversationId};
use futures::executor::block_on;
use instant::Instant;
use nix::sys::signal::kill;
use nix::unistd::Pid;
use serde_json::json;
use tempfile::TempDir;

use crate::agent::action::AIAgentActionType;
use crate::hooks::adapters::HookConfigError;
use crate::hooks::dispatch::{DispatchOutcome, HookDispatcher};
use crate::hooks::model::{
    CanonicalToolName, HookBoundary, HookEnvelope, HookFormat, HookPayload, HookSourceKind,
    HookVerdict,
};
use crate::hooks::registry::HookRegistry;
use crate::hooks::runner::ProcessExit;
use crate::hooks::runner::command::CommandHookRunner;
use crate::hooks::test_util::{RecordingMessenger, test_envelope};

struct Harness {
    dir: TempDir,
    registry: HookRegistry,
    dispatcher: HookDispatcher,
    messenger: Arc<RecordingMessenger>,
}

impl Harness {
    /// Writes the script body to `./hook.sh`, writes a `hooks.json` whose one
    /// `beforeShellExecution` definition is `definition` with `command` set to that script, and
    /// loads the file into the registry.
    fn new(script_body: &str, mut definition: serde_json::Value) -> Self {
        let dir = TempDir::new().expect("tempdir");
        let script = dir.path().join("hook.sh");
        fs::write(&script, format!("#!/bin/sh\n{script_body}\n")).expect("write script");
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("chmod");
        definition["command"] = json!("./hook.sh");
        let hooks_json = dir.path().join("hooks.json");
        fs::write(
            &hooks_json,
            json!({ "version": 1, "hooks": { "beforeShellExecution": [definition] } }).to_string(),
        )
        .expect("write hooks.json");

        let mut registry = HookRegistry::new();
        registry
            .install_explicit_source(
                &hooks_json,
                HookFormat::Cursor,
                HookSourceKind::RunDelivered,
            )
            .expect("the file loads");

        let messenger = Arc::new(RecordingMessenger::new());
        let dispatcher = HookDispatcher::new(
            Arc::new(CommandHookRunner::new(Arc::new(|| None))),
            messenger.clone(),
        );
        Self {
            dir,
            registry,
            dispatcher,
            messenger,
        }
    }

    fn dispatch(&self, command: &str) -> DispatchOutcome {
        block_on(self.dispatcher.dispatch(
            shell_envelope(command, self.dir.path()),
            self.registry.snapshot(),
            Arc::new(AtomicBool::new(false)),
        ))
    }
}

fn shell_envelope(command: &str, cwd: &Path) -> HookEnvelope {
    let action = AIAgentActionType::RequestCommandOutput {
        command: command.to_owned(),
        is_read_only: None,
        is_risky: None,
        wait_until_completion: true,
        uses_pager: None,
        rationale: None,
        citations: Vec::new(),
    };
    let tool_name = CanonicalToolName::for_action(&action);
    let mut envelope = test_envelope(
        AIConversationId::new(),
        HookPayload::ToolPre { action, tool_name },
    );
    envelope.cwd = Some(cwd.to_path_buf());
    envelope.action_id = Some(AIAgentActionId::from("call_1".to_owned()));
    envelope
}

fn wait_until_gone(pid: i32) -> bool {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        // Signal 0 only checks that the pid exists.
        if kill(Pid::from_raw(pid), None).is_err() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

fn reasons(outcome: &DispatchOutcome) -> Vec<&str> {
    outcome
        .effects
        .reasons
        .iter()
        .map(|reason| reason.text.as_str())
        .collect()
}

#[test]
fn a_script_that_exits_two_denies_with_its_agent_message() {
    let harness = Harness::new(
        r#"printf '{"agent_message":"rm is not allowed in this repo","user_message":"Blocked"}'
exit 2"#,
        json!({}),
    );
    let outcome = harness.dispatch("rm -rf /tmp/x");
    assert_eq!(outcome.effects.verdict, HookVerdict::Deny);
    assert_eq!(reasons(&outcome), vec!["rm is not allowed in this repo"]);
    assert_eq!(outcome.effects.user_notices, vec!["Blocked"]);
    assert_eq!(outcome.invocations.len(), 1);
    assert_eq!(outcome.invocations[0].exit, ProcessExit::Code(2));
    assert_eq!(outcome.registry_revision, 1);
    assert_eq!(
        harness.messenger.notices().len(),
        1,
        "the user message was routed"
    );
}

#[test]
fn a_script_that_exits_two_silently_denies_with_the_default_reason() {
    let harness = Harness::new("exit 2", json!({}));
    let outcome = harness.dispatch("rm -rf /tmp/x");
    assert_eq!(outcome.effects.verdict, HookVerdict::Deny);
    assert_eq!(
        reasons(&outcome),
        vec!["Denied by hook ./hook.sh (exit 2)."]
    );
}

#[test]
fn a_script_that_prints_allow_allows() {
    let harness = Harness::new(r#"printf '{"permission":"allow"}'"#, json!({}));
    let outcome = harness.dispatch("ls");
    assert_eq!(outcome.effects.verdict, HookVerdict::Allow);
    assert!(outcome.effects.reasons.is_empty());
}

#[test]
fn a_script_that_prints_nothing_has_no_opinion_when_it_fails_open() {
    let harness = Harness::new("exit 0", json!({}));
    let outcome = harness.dispatch("ls");
    assert_eq!(outcome.effects.verdict, HookVerdict::NoOpinion);
    assert_eq!(outcome.invocations.len(), 1, "the hook did run");
}

#[test]
fn a_script_that_prints_something_other_than_json_denies() {
    let harness = Harness::new("echo not json", json!({}));
    let outcome = harness.dispatch("ls");
    assert_eq!(outcome.effects.verdict, HookVerdict::Deny);
    assert_eq!(
        reasons(&outcome),
        vec![
            "Hook ./hook.sh printed output that is not a valid permission decision, so the \
             command was denied."
        ]
    );
}

#[test]
fn a_fail_closed_script_that_exits_one_denies() {
    let harness = Harness::new("exit 1", json!({ "failClosed": true }));
    let outcome = harness.dispatch("ls");
    assert_eq!(outcome.effects.verdict, HookVerdict::Deny);
    assert_eq!(
        reasons(&outcome),
        vec!["Hook ./hook.sh exited with code 1, so the command was denied."]
    );

    let open = Harness::new("exit 1", json!({}));
    assert_eq!(open.dispatch("ls").effects.verdict, HookVerdict::NoOpinion);
}

#[test]
fn the_script_receives_cursor_shaped_json_on_stdin() {
    let harness = Harness::new(
        r#"cat > "$PWD/stdin.json"; printf '{"permission":"allow"}'"#,
        json!({}),
    );
    let outcome = harness.dispatch("git push --force");
    assert_eq!(outcome.effects.verdict, HookVerdict::Allow);

    let received: serde_json::Value = serde_json::from_slice(
        &fs::read(harness.dir.path().join("stdin.json")).expect("stdin file"),
    )
    .expect("stdin is JSON");
    assert_eq!(received["hook_event_name"], "beforeShellExecution");
    assert_eq!(received["command"], "git push --force");
    assert_eq!(
        received["cwd"],
        harness.dir.path().to_string_lossy().as_ref()
    );
    assert_eq!(received["tool_use_id"], "call_1");
    assert_eq!(received["transcript_path"], serde_json::Value::Null);
    assert!(received["conversation_id"].is_string());
    assert!(received["generation_id"].is_string());
}

#[test]
fn the_hook_runs_in_the_directory_that_holds_the_file() {
    let harness = Harness::new(
        r#"pwd > "$PWD/cwd.txt"; printf '{"permission":"allow"}'"#,
        json!({}),
    );
    harness.dispatch("ls");
    let recorded = fs::read_to_string(harness.dir.path().join("cwd.txt")).expect("cwd file");
    assert_eq!(
        PathBuf::from(recorded.trim()).canonicalize().expect("cwd"),
        harness.dir.path().canonicalize().expect("tempdir")
    );
}

#[test]
fn a_script_that_outlives_its_timeout_is_killed() {
    let harness = Harness::new(
        r#"echo $$ > "$PWD/pid"
sleep 30
printf '{"permission":"allow"}'"#,
        json!({ "timeout": 1 }),
    );
    let started = Instant::now();
    let outcome = harness.dispatch("ls");
    assert!(started.elapsed() < Duration::from_secs(10));
    assert_eq!(outcome.invocations[0].exit, ProcessExit::TimedOut);
    assert_eq!(
        outcome.effects.verdict,
        HookVerdict::NoOpinion,
        "a timed-out hook without failClosed fails open, as Cursor documents"
    );

    let pid = fs::read_to_string(harness.dir.path().join("pid"))
        .expect("the script recorded its pid")
        .trim()
        .parse::<i32>()
        .expect("a pid");
    assert!(wait_until_gone(pid), "process {pid} survived the timeout");
}

#[test]
fn a_fail_closed_script_that_times_out_denies() {
    let harness = Harness::new("sleep 30", json!({ "timeout": 1, "failClosed": true }));
    let outcome = harness.dispatch("ls");
    assert_eq!(outcome.effects.verdict, HookVerdict::Deny);
    assert_eq!(
        reasons(&outcome),
        vec!["Hook ./hook.sh timed out, so the command was denied."]
    );
}

#[test]
fn a_matcher_that_does_not_match_leaves_the_command_alone() {
    let harness = Harness::new("exit 2", json!({ "matcher": "^rm " }));
    assert_eq!(
        harness.dispatch("ls").effects.verdict,
        HookVerdict::NoOpinion
    );
    assert_eq!(
        harness.dispatch("rm -rf x").effects.verdict,
        HookVerdict::Deny
    );
}

#[test]
fn loading_the_same_file_again_replaces_its_bindings() {
    let mut harness = Harness::new("exit 2", json!({}));
    let path = harness.dir.path().join("hooks.json");
    fs::write(
        &path,
        r#"{ "version": 1, "hooks": { "beforeShellExecution": [
            { "command": "./hook.sh" }, { "command": "./hook.sh", "matcher": "x" }
        ] } }"#,
    )
    .expect("rewrite hooks.json");
    harness
        .registry
        .install_explicit_source(&path, HookFormat::Cursor, HookSourceKind::RunDelivered)
        .expect("reloads");
    let snapshot = harness.registry.snapshot();
    assert_eq!(snapshot.revision, 2);
    assert_eq!(snapshot.bindings.len(), 2);
    assert!(snapshot.has_bindings_for(HookBoundary::ToolPre));
}

#[test]
fn a_missing_or_invalid_file_is_an_error_and_changes_nothing() {
    let dir = TempDir::new().expect("tempdir");
    let mut registry = HookRegistry::new();
    let missing = dir.path().join("hooks.json");
    let error = registry
        .install_explicit_source(&missing, HookFormat::Cursor, HookSourceKind::RunDelivered)
        .expect_err("missing file");
    assert!(matches!(error, HookConfigError::Unreadable(_)));

    fs::write(&missing, "{ not json").expect("write");
    let error = registry
        .install_explicit_source(&missing, HookFormat::Cursor, HookSourceKind::RunDelivered)
        .expect_err("invalid file");
    assert!(matches!(error, HookConfigError::Parse(_)));
    assert_eq!(registry.revision(), 0);
    assert!(registry.snapshot().is_empty());
}
