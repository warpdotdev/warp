//! End-to-end hook tests with real parts: the checked-in Cursor fixtures copied to a temporary
//! directory, real `sh` scripts, the registry and its loaders, the dispatcher, and the process
//! runner. Each test is named after the Phase 1 test contract it proves (architecture.md §11).
//!
//! Every runner here gets an explicit `HOME` and `PATH`, so no test depends on the developer's
//! own environment, and nothing changes the test process's environment.

// Most of these tests run `sh` scripts and exist only on Unix; the shared helpers stay compiled
// everywhere so the Windows test can use them.
#![cfg_attr(not(unix), allow(dead_code, unused_imports))]

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use ai_types::{AIAgentActionId, AIConversationId};
use futures::executor::block_on;
use instant::Instant;
use serde_json::json;
use tempfile::TempDir;

use crate::agent::action::AIAgentActionType;
use crate::hooks::adapters::adapter_for;
use crate::hooks::dispatch::{DispatchOutcome, DispatchPolicy, HookDispatcher};
use crate::hooks::model::{
    DiagnosticSeverity, HookBoundary, HookEnvelope, HookFormat, HookPayload, HookSourceId,
    HookSourceKind, HookSourceLocation, HookVerdict, PromptProvenance,
};
use crate::hooks::registry::HookRegistry;
use crate::hooks::runner::ProcessExit;
use crate::hooks::runner::command::CommandHookRunner;
use crate::hooks::sources::{ExplicitHookSource, ExplicitSourceScope, load_explicit_sources};
use crate::hooks::test_util::{RecordingMessenger, test_envelope};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src/hooks/fixtures");
const CURSOR_HOOKS_PATH: &str = ".cursor/hooks.json";

/// The `PATH` every hook in these tests runs with. The fixture scripts need only POSIX tools.
const HOOK_PATH: &str = "/usr/bin:/bin";

/// A copy of one checked-in fixture in a temporary directory, so nothing a hook writes lands in
/// the source tree.
struct Fixture {
    _dir: TempDir,
    /// The copied fixture, canonical.
    root: PathBuf,
    /// The `HOME` hooks run with: an empty directory beside the copy.
    home: PathBuf,
}

impl Fixture {
    fn copy(name: &str) -> Self {
        let dir = TempDir::new().expect("tempdir");
        let base = dunce::canonicalize(dir.path()).expect("canonical tempdir");
        let root = base.join(name);
        copy_tree(&Path::new(FIXTURES).join(name), &root);
        make_scripts_executable(&root);
        let home = base.join("home");
        fs::create_dir_all(&home).expect("mkdir home");
        Self {
            _dir: dir,
            root,
            home,
        }
    }

    /// An empty directory next to the copy, for a source delivered with the run.
    fn sibling(&self, name: &str) -> PathBuf {
        let path = self.root.parent().expect("parent").join(name);
        fs::create_dir_all(&path).expect("mkdir");
        path
    }

    fn cursor_hooks_file(&self) -> PathBuf {
        self.root.join(CURSOR_HOOKS_PATH)
    }

    fn runnable_hooks_file(&self) -> PathBuf {
        self.root.join("run_hooks/hooks.json")
    }

    fn read(&self, relative: &str) -> String {
        fs::read_to_string(self.root.join(relative))
            .unwrap_or_else(|error| panic!("{relative} was not written: {error}"))
    }
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("mkdir");
    for entry in fs::read_dir(from).expect("read fixture dir") {
        let entry = entry.expect("dir entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("file type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).expect("copy fixture file");
        }
    }
}

/// Sets the executable bit on every `.sh` file, whatever mode the checkout gave it.
#[cfg(unix)]
fn make_scripts_executable(root: &Path) {
    use std::os::unix::fs::PermissionsExt;
    for entry in fs::read_dir(root).expect("read dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            make_scripts_executable(&path);
        } else if path.extension().is_some_and(|extension| extension == "sh") {
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod");
        }
    }
}

#[cfg(not(unix))]
fn make_scripts_executable(_root: &Path) {}

fn runner(home: &Path) -> CommandHookRunner {
    CommandHookRunner::new(
        Arc::new(|| Some(OsString::from(HOOK_PATH))),
        std::env::vars_os().collect(),
    )
    .with_home(Some(home.to_path_buf()))
}

fn dispatcher(home: &Path) -> (HookDispatcher, Arc<RecordingMessenger>) {
    let messenger = Arc::new(RecordingMessenger::new());
    (
        HookDispatcher::new(Arc::new(runner(home)), messenger.clone()),
        messenger,
    )
}

fn source_id(hooks_file: &Path, scope: ExplicitSourceScope) -> HookSourceId {
    HookSourceId {
        kind: scope.source_kind(),
        location: HookSourceLocation::CanonicalPath(
            dunce::canonicalize(hooks_file).expect("canonical hook source"),
        ),
    }
}

fn source(hooks_file: &Path, scope: ExplicitSourceScope) -> ExplicitHookSource {
    ExplicitHookSource {
        path: hooks_file.to_path_buf(),
        format: HookFormat::Cursor,
        scope,
    }
}

fn run_source(hooks_file: &Path) -> ExplicitHookSource {
    source(hooks_file, ExplicitSourceScope::Run)
}

fn load_sources(registry: &mut HookRegistry, sources: &[ExplicitHookSource]) {
    let failures = load_explicit_sources(sources, registry);
    assert!(failures.is_empty(), "the files load: {failures:?}");
}

fn load_run_file(registry: &mut HookRegistry, hooks_file: &Path) {
    load_sources(registry, &[run_source(hooks_file)]);
}

/// Writes `body` as an executable `sh` script.
#[cfg(unix)]
fn write_script(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    fs::write(path, format!("#!/bin/sh\n{body}\n")).expect("write script");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("chmod");
}

/// A hook directory delivered with the run: `hooks.json` with one `definition` for `event`, whose
/// command runs `./hook.sh` with `body`.
#[cfg(unix)]
fn write_run_hooks(
    dir: &Path,
    event: &str,
    body: &str,
    mut definition: serde_json::Value,
) -> PathBuf {
    write_script(&dir.join("hook.sh"), body);
    definition["command"] = json!("./hook.sh");
    let hooks_file = dir.join("hooks.json");
    fs::write(
        &hooks_file,
        json!({ "version": 1, "hooks": { event: [definition] } }).to_string(),
    )
    .expect("write hooks.json");
    hooks_file
}

fn shell_action(command: &str) -> AIAgentActionType {
    AIAgentActionType::RequestCommandOutput {
        command: command.to_owned(),
        is_read_only: None,
        is_risky: None,
        wait_until_completion: true,
        uses_pager: None,
        rationale: None,
        citations: Vec::new(),
    }
}

/// A `tool.pre` envelope for a shell command run at `cwd` inside the project at `root`.
fn shell_envelope(command: &str, cwd: &Path, root: Option<&Path>) -> HookEnvelope {
    let action = shell_action(command);
    let mut envelope = test_envelope(AIConversationId::new(), HookPayload::ToolPre { action });
    envelope.cwd = Some(cwd.to_path_buf());
    envelope.roots = root.map(Path::to_path_buf).into_iter().collect();
    envelope.action_id = Some(AIAgentActionId::from("call_1".to_owned()));
    envelope
}

fn prompt_envelope(text: &str, cwd: &Path) -> HookEnvelope {
    let mut envelope = test_envelope(
        AIConversationId::new(),
        HookPayload::PromptSubmit {
            text: text.to_owned(),
            attachments_summary: Vec::new(),
            provenance: PromptProvenance::User,
        },
    );
    envelope.cwd = Some(cwd.to_path_buf());
    envelope
}

fn dispatch(
    dispatcher: &HookDispatcher,
    registry: &HookRegistry,
    envelope: HookEnvelope,
) -> DispatchOutcome {
    block_on(dispatcher.dispatch(
        envelope,
        registry.snapshot(),
        DispatchPolicy::default(),
        Arc::new(AtomicBool::new(false)),
    ))
}

fn reasons(outcome: &DispatchOutcome) -> Vec<&str> {
    outcome
        .effects
        .reasons
        .iter()
        .map(|reason| reason.text.as_str())
        .collect()
}

/// Waits for `path` to be written, for scripts that record a pid once they are running.
fn wait_for_file(path: &Path, within: Duration) -> bool {
    let deadline = Instant::now() + within;
    while Instant::now() < deadline {
        if path.exists() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

#[cfg(unix)]
fn recorded_pid(path: &Path) -> i32 {
    fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("{} was not written: {error}", path.display()))
        .trim()
        .parse()
        .expect("a pid")
}

/// Whether process `pid` is gone within `within`.
#[cfg(unix)]
fn gone_within(pid: i32, within: Duration) -> bool {
    use nix::sys::signal::kill;
    use nix::unistd::Pid;
    let deadline = Instant::now() + within;
    while Instant::now() < deadline {
        // Signal 0 only checks that the pid exists.
        if kill(Pid::from_raw(pid), None).is_err() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

const SAMPLE_RM_REASON: &str = "This project's hook does not allow rm -rf. Remove files one at a time, or ask the user to do it.";
const SAMPLE_PUSH_REASON: &str = "This project's hook does not allow force pushes. Push without --force, or ask the user to do it.";

/// §11 "adapter and reducer fixtures": the sample parses into the bindings it declares, and a
/// denial from two sources reduces in precedence order with both reasons kept.
#[cfg(unix)]
#[test]
fn adapter_and_reducer_fixtures() {
    let fixture = Fixture::copy("sample_project");
    let adapter = adapter_for(HookFormat::Cursor).expect("the Cursor adapter is registered");
    let cursor_hooks_file = fixture.cursor_hooks_file();
    let parsed_source = source_id(&cursor_hooks_file, ExplicitSourceScope::FactoryAgent);
    let config = adapter
        .parse(&fs::read(cursor_hooks_file).expect("read"), &parsed_source)
        .expect("the sample parses");
    assert!(config.warnings.is_empty(), "{:?}", config.warnings);
    let mut declared = adapter
        .bindings(&config)
        .into_iter()
        .map(|binding| {
            assert_eq!(binding.matcher, None);
            assert_eq!(binding.timeout, binding.boundary.default_timeout());
            (binding.vendor_event, binding.boundary, binding.command)
        })
        .collect::<Vec<_>>();
    declared.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        declared,
        vec![
            (
                "afterShellExecution".to_owned(),
                HookBoundary::ToolResult,
                ".cursor/scripts/audit.sh".to_owned()
            ),
            (
                "beforeShellExecution".to_owned(),
                HookBoundary::ToolPre,
                ".cursor/scripts/deny_dangerous.sh".to_owned()
            ),
            (
                "beforeSubmitPrompt".to_owned(),
                HookBoundary::PromptSubmit,
                ".cursor/scripts/log_prompt.sh".to_owned()
            ),
            (
                "sessionStart".to_owned(),
                HookBoundary::SessionStart,
                ".cursor/scripts/session_env.sh".to_owned()
            ),
            ("stop".to_owned(), HookBoundary::TurnStop, "true".to_owned()),
        ]
    );

    // A run-delivered policy that also denies, beside the factory agent's explicit source.
    let run_dir = fixture.sibling("run-policy");
    let run_hooks = write_run_hooks(
        &run_dir,
        "beforeShellExecution",
        r#"cat > /dev/null
printf '{"permission":"deny","agent_message":"The run policy forbids deleting directories."}'"#,
        json!({ "matcher": "rm " }),
    );
    let mut registry = HookRegistry::new();
    load_sources(
        &mut registry,
        &[
            source(
                &fixture.runnable_hooks_file(),
                ExplicitSourceScope::FactoryAgent,
            ),
            run_source(&run_hooks),
        ],
    );

    let (dispatcher, _) = dispatcher(&fixture.home);
    let outcome = dispatch(
        &dispatcher,
        &registry,
        shell_envelope("rm -rf build", &fixture.root, Some(&fixture.root)),
    );
    assert_eq!(outcome.effects.verdict, HookVerdict::Deny);
    assert_eq!(
        reasons(&outcome),
        vec![
            SAMPLE_RM_REASON,
            "The run policy forbids deleting directories."
        ],
        "the factory-agent source ranks above the run source, and both reasons are kept"
    );
    assert_eq!(
        outcome
            .effects
            .verdict_binding
            .as_ref()
            .map(|binding| binding.source.kind),
        Some(HookSourceKind::FactoryAgent)
    );
    let ran = outcome
        .invocations
        .iter()
        .map(|invocation| invocation.binding.source.kind)
        .collect::<Vec<_>>();
    assert_eq!(
        ran,
        vec![HookSourceKind::FactoryAgent, HookSourceKind::RunDelivered]
    );
}

/// §11 "real-subprocess stdin": the sample's deny script receives the documented
/// `beforeShellExecution` JSON on stdin.
#[cfg(unix)]
#[test]
fn real_subprocess_stdin() {
    let fixture = Fixture::copy("sample_project");
    let cwd = fixture.root.join("src");
    fs::create_dir_all(&cwd).expect("mkdir");
    let mut registry = HookRegistry::new();
    load_run_file(&mut registry, &fixture.runnable_hooks_file());
    let (dispatcher, _) = dispatcher(&fixture.home);

    let outcome = dispatch(
        &dispatcher,
        &registry,
        shell_envelope("git push --force origin main", &cwd, Some(&fixture.root)),
    );
    assert_eq!(outcome.effects.verdict, HookVerdict::Deny);
    assert_eq!(reasons(&outcome), vec![SAMPLE_PUSH_REASON]);

    let received: serde_json::Value =
        serde_json::from_str(&fixture.read(".hook-last-shell-input.json"))
            .expect("the script saw one JSON object");
    assert_eq!(received["hook_event_name"], "beforeShellExecution");
    assert_eq!(received["command"], "git push --force origin main");
    assert_eq!(received["cwd"], cwd.to_string_lossy().as_ref());
    assert_eq!(
        received["workspace_roots"],
        json!([fixture.root.to_string_lossy()])
    );
    assert_eq!(received["tool_use_id"], "call_1");
    assert_eq!(received["cursor_version"], "test");
    assert_eq!(received["warp_version"], "test");
    assert_eq!(received["transcript_path"], serde_json::Value::Null);
    assert_eq!(received["user_email"], serde_json::Value::Null);
    assert!(received["conversation_id"].is_string());
    assert!(received["generation_id"].is_string());
}

/// §11 "malformed and empty output": garbage and silence, under both failure policies, on a
/// permission hook and on the prompt hook, through the real runner.
#[cfg(unix)]
#[test]
fn malformed_and_empty_output() {
    let fixture = Fixture::copy("sample_project");
    let (dispatcher, _) = dispatcher(&fixture.home);
    let garbage = "cat > /dev/null; echo 'this is not json'";
    let silence = "cat > /dev/null";

    for (body, fail_closed, denies) in [
        (garbage, false, true),
        (garbage, true, true),
        (silence, false, false),
        (silence, true, true),
    ] {
        let dir = TempDir::new().expect("tempdir");
        let hooks = write_run_hooks(
            dir.path(),
            "beforeShellExecution",
            body,
            json!({ "failClosed": fail_closed }),
        );
        let mut registry = HookRegistry::new();
        load_run_file(&mut registry, &hooks);
        let outcome = dispatch(
            &dispatcher,
            &registry,
            shell_envelope("ls", &fixture.root, Some(&fixture.root)),
        );
        let expected = if denies {
            HookVerdict::Deny
        } else {
            HookVerdict::NoOpinion
        };
        assert_eq!(
            outcome.effects.verdict, expected,
            "beforeShellExecution, body {body:?}, failClosed {fail_closed}"
        );
        assert_eq!(outcome.invocations[0].exit, ProcessExit::Code(0));
    }

    // On `beforeSubmitPrompt`, malformed output follows the failure policy too.
    for (body, fail_closed) in [
        (garbage, false),
        (garbage, true),
        (silence, false),
        (silence, true),
    ] {
        let dir = TempDir::new().expect("tempdir");
        let hooks = write_run_hooks(
            dir.path(),
            "beforeSubmitPrompt",
            body,
            json!({ "failClosed": fail_closed }),
        );
        let mut registry = HookRegistry::new();
        load_run_file(&mut registry, &hooks);
        let outcome = dispatch(
            &dispatcher,
            &registry,
            prompt_envelope("hello", &fixture.root),
        );
        assert_eq!(
            outcome.effects.halt_turn, fail_closed,
            "beforeSubmitPrompt, body {body:?}, failClosed {fail_closed}"
        );
    }
}

/// §11 "timeout, cancellation, output limit": a sleeping hook times out and leaves nothing
/// running, a cancel flag set while it runs stops it, and 2 MiB of stdout is cut off and read as
/// malformed even though the whole of it would have parsed.
#[cfg(unix)]
#[test]
fn timeout_cancellation_output_limit() {
    let fixture = Fixture::copy("sample_project");
    let (dispatcher, _) = dispatcher(&fixture.home);
    let sleeper = r#"cat > /dev/null
echo $$ > "$PWD/pid"
sleep 30
printf '{"permission":"deny"}'"#;

    // Timeout: fails open, as Cursor documents, and the process is gone.
    let dir = TempDir::new().expect("tempdir");
    let hooks = write_run_hooks(
        dir.path(),
        "beforeShellExecution",
        sleeper,
        json!({ "timeout": 5 }),
    );
    let mut registry = HookRegistry::new();
    load_run_file(&mut registry, &hooks);
    let started = Instant::now();
    let outcome = dispatch(
        &dispatcher,
        &registry,
        shell_envelope("ls", &fixture.root, Some(&fixture.root)),
    );
    assert!(started.elapsed() < Duration::from_secs(25));
    assert_eq!(outcome.invocations[0].exit, ProcessExit::TimedOut);
    assert_eq!(outcome.effects.verdict, HookVerdict::NoOpinion);
    let pid = recorded_pid(&dir.path().join("pid"));
    assert!(
        gone_within(pid, Duration::from_secs(5)),
        "process {pid} survived the timeout"
    );

    // Cancellation: the flag is set once the script is running.
    let dir = TempDir::new().expect("tempdir");
    let hooks = write_run_hooks(dir.path(), "beforeShellExecution", sleeper, json!({}));
    let mut registry = HookRegistry::new();
    load_run_file(&mut registry, &hooks);
    let cancel = Arc::new(AtomicBool::new(false));
    let pid_file = dir.path().join("pid");
    let canceller = {
        let cancel = cancel.clone();
        let pid_file = pid_file.clone();
        std::thread::spawn(move || {
            let started = wait_for_file(&pid_file, Duration::from_secs(30));
            cancel.store(true, Ordering::Release);
            started
        })
    };
    let started = Instant::now();
    let outcome = block_on(dispatcher.dispatch(
        shell_envelope("ls", &fixture.root, Some(&fixture.root)),
        registry.snapshot(),
        DispatchPolicy::default(),
        cancel,
    ));
    assert!(
        canceller.join().expect("canceller"),
        "the script never started"
    );
    assert!(outcome.cancelled, "a cancelled firing is not a verdict");
    assert_eq!(outcome.invocations[0].exit, ProcessExit::Cancelled);
    assert!(started.elapsed() < Duration::from_secs(25));
    let pid = recorded_pid(&pid_file);
    assert!(
        gone_within(pid, Duration::from_secs(5)),
        "process {pid} survived cancellation"
    );

    // Output limit: valid JSON padded with 2 MiB of spaces is cut at 1 MiB and denied.
    let dir = TempDir::new().expect("tempdir");
    let hooks = write_run_hooks(
        dir.path(),
        "beforeShellExecution",
        r#"cat > /dev/null
printf '{"permission":"allow"}'
dd if=/dev/zero bs=1024 count=2048 2>/dev/null | tr '\000' ' '"#,
        json!({}),
    );
    let mut registry = HookRegistry::new();
    load_run_file(&mut registry, &hooks);
    let outcome = dispatch(
        &dispatcher,
        &registry,
        shell_envelope("ls", &fixture.root, Some(&fixture.root)),
    );
    assert_eq!(outcome.invocations[0].exit, ProcessExit::Code(0));
    assert_eq!(
        outcome.effects.verdict,
        HookVerdict::Deny,
        "a cut-off response is malformed output on a permission hook"
    );
}

/// The resolved source list is loaded once. Later file edits and load attempts cannot change the
/// parsed bindings used by this worker.
#[test]
fn resolved_sources_finalize_once() {
    let fixture = Fixture::copy("sample_project");
    let hooks_file = fixture.runnable_hooks_file();
    let mut registry = HookRegistry::new();
    load_run_file(&mut registry, &hooks_file);
    assert!(registry.is_finalized());
    let snapshot = registry.snapshot();
    let revision = snapshot.revision;
    assert_eq!(
        snapshot
            .bindings
            .iter()
            .filter(|active| active.binding.boundary == HookBoundary::ToolPre)
            .count(),
        1
    );

    fs::write(
        &hooks_file,
        r#"{ "version": 1, "hooks": { "beforeShellExecution": [
            { "command": "true" }, { "command": "true" }
        ] } }"#,
    )
    .expect("rewrite hooks.json");
    let failures = load_explicit_sources(&[run_source(&hooks_file)], &mut registry);
    assert_eq!(failures.len(), 1);
    assert!(failures[0].message.contains("already finalized"));
    let snapshot = registry.snapshot();
    assert_eq!(snapshot.revision, revision);
    assert_eq!(
        snapshot
            .bindings
            .iter()
            .filter(|active| active.binding.boundary == HookBoundary::ToolPre)
            .count(),
        1,
        "the parsed hook set stays fixed after finalization"
    );
}

/// The malformed fixture loads its one valid definition and reports the rest.
#[test]
fn the_malformed_fixture_loads_what_it_can_and_reports_the_rest() {
    let fixture = Fixture::copy("malformed");
    let mut registry = HookRegistry::new();
    load_run_file(&mut registry, &fixture.cursor_hooks_file());
    let snapshot = registry.snapshot();
    let active = snapshot
        .bindings
        .iter()
        .map(|active| {
            (
                active.binding.vendor_event.as_str(),
                active.binding.id.index,
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(active, vec![("beforeShellExecution", 1)]);
    let warnings = snapshot
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Warning)
        .map(|diagnostic| diagnostic.message.as_str())
        .collect::<Vec<_>>();
    assert!(
        warnings.iter().any(|message| message.contains("matcher")),
        "{warnings:?}"
    );
    assert!(
        warnings
            .iter()
            .any(|message| message.contains("onTabClosed")),
        "{warnings:?}"
    );
}

/// §11 "no orphans after a timeout": a hook's background child is gone within the kill grace
/// once the hook times out.
#[cfg(unix)]
#[test]
fn no_orphans_after_timeout() {
    use crate::hooks::runner::command::HOOK_KILL_GRACE;

    let fixture = Fixture::copy("sample_project");
    let (dispatcher, _) = dispatcher(&fixture.home);
    let dir = TempDir::new().expect("tempdir");
    let hooks = write_run_hooks(
        dir.path(),
        "beforeShellExecution",
        r#"cat > /dev/null
trap '' TERM
sleep 60 &
echo $! > "$PWD/grandchild.pid"
echo $$ > "$PWD/pid"
sleep 60"#,
        json!({ "timeout": 5 }),
    );
    let mut registry = HookRegistry::new();
    load_run_file(&mut registry, &hooks);
    let outcome = dispatch(
        &dispatcher,
        &registry,
        shell_envelope("ls", &fixture.root, Some(&fixture.root)),
    );
    assert_eq!(outcome.invocations[0].exit, ProcessExit::TimedOut);
    for name in ["pid", "grandchild.pid"] {
        let pid = recorded_pid(&dir.path().join(name));
        assert!(
            gone_within(pid, HOOK_KILL_GRACE + Duration::from_secs(5)),
            "{name} {pid} outlived the hook"
        );
    }
}

/// §11 "no orphans after a timeout", on Windows: the Job Object ends a grandchild the hook
/// started.
#[cfg(windows)]
#[test]
#[ignore = "written without a Windows machine to run it on; run it on Windows to verify"]
fn no_orphans_after_timeout() {
    let fixture = Fixture::copy("sample_project");
    let (dispatcher, _) = dispatcher(&fixture.home);
    let dir = TempDir::new().expect("tempdir");
    fs::write(
        dir.path().join("hook.cmd"),
        "@echo off\r\npowershell -NoProfile -Command \"$p = Start-Process -FilePath ping \
         -ArgumentList '-n','60','127.0.0.1' -PassThru -WindowStyle Hidden; Set-Content -Path \
         'grandchild.pid' -Value $p.Id; Wait-Process -Id $p.Id\"\r\n",
    )
    .expect("write script");
    let hooks = dir.path().join("hooks.json");
    fs::write(
        &hooks,
        json!({ "version": 1, "hooks": { "beforeShellExecution": [
            { "command": "hook.cmd", "timeout": 10 }
        ] } })
        .to_string(),
    )
    .expect("write hooks.json");
    let mut registry = HookRegistry::new();
    load_run_file(&mut registry, &hooks);
    let outcome = dispatch(
        &dispatcher,
        &registry,
        shell_envelope("dir", &fixture.root, Some(&fixture.root)),
    );
    assert_eq!(outcome.invocations[0].exit, ProcessExit::TimedOut);
    let pid = fs::read_to_string(dir.path().join("grandchild.pid"))
        .expect("the grandchild was recorded")
        .trim()
        .to_owned();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut alive = true;
    while alive && Instant::now() < deadline {
        let listed = command::blocking::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
            .output()
            .expect("tasklist runs");
        alive = String::from_utf8_lossy(&listed.stdout).contains(&format!("\"{pid}\""));
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(!alive, "grandchild {pid} outlived the hook");
}

/// §11 latency: a trivial hook adds well under two seconds to the operation it guards.
#[cfg(unix)]
#[test]
fn a_trivial_hook_adds_under_two_seconds() {
    let fixture = Fixture::copy("sample_project");
    let mut registry = HookRegistry::new();
    load_run_file(&mut registry, &fixture.runnable_hooks_file());
    let (dispatcher, _) = dispatcher(&fixture.home);
    // The first run of a script written a moment ago pays for the operating system loading it
    // (and, on machines with endpoint scanning, for the scan), which is not what a hook adds to
    // each operation. It is run once and logged, and the second run is timed.
    let cold = Instant::now();
    let outcome = dispatch(
        &dispatcher,
        &registry,
        shell_envelope("ls", &fixture.root, Some(&fixture.root)),
    );
    assert_eq!(outcome.effects.verdict, HookVerdict::Allow);
    log::info!("first trivial hook run took {:?}", cold.elapsed());

    let started = Instant::now();
    let outcome = dispatch(
        &dispatcher,
        &registry,
        shell_envelope("ls", &fixture.root, Some(&fixture.root)),
    );
    let elapsed = started.elapsed();
    assert_eq!(outcome.effects.verdict, HookVerdict::Allow);
    assert!(elapsed < Duration::from_secs(2), "took {elapsed:?}");
}

/// The `beforeShellExecution` slice PR 2 proved first, kept here with the rest of the suite.
#[cfg(unix)]
mod shell_slice {
    use super::*;

    struct Harness {
        fixture: Fixture,
        dir: TempDir,
        registry: HookRegistry,
        dispatcher: HookDispatcher,
        messenger: Arc<RecordingMessenger>,
    }

    impl Harness {
        fn new(script_body: &str) -> Self {
            Self::with_definition(script_body, json!({}))
        }

        fn with_definition(script_body: &str, definition: serde_json::Value) -> Self {
            let fixture = Fixture::copy("sample_project");
            let dir = TempDir::new().expect("tempdir");
            let hooks =
                write_run_hooks(dir.path(), "beforeShellExecution", script_body, definition);
            let mut registry = HookRegistry::new();
            load_run_file(&mut registry, &hooks);
            assert!(!registry.snapshot().is_empty(), "the file loads");
            let (dispatcher, messenger) = dispatcher(&fixture.home);
            Self {
                fixture,
                dir,
                registry,
                dispatcher,
                messenger,
            }
        }

        fn dispatch(&self, command: &str) -> DispatchOutcome {
            dispatch(
                &self.dispatcher,
                &self.registry,
                shell_envelope(command, self.dir.path(), None),
            )
        }
    }

    #[test]
    fn a_script_that_exits_two_denies_with_its_agent_message() {
        let harness = Harness::new(
            r#"printf '{"agent_message":"rm is not allowed in this repo","user_message":"Blocked"}'
exit 2"#,
        );
        let outcome = harness.dispatch("rm -rf /tmp/x");
        assert_eq!(outcome.effects.verdict, HookVerdict::Deny);
        assert_eq!(reasons(&outcome), vec!["rm is not allowed in this repo"]);
        assert_eq!(outcome.effects.user_notices, vec!["Blocked"]);
        assert_eq!(outcome.invocations.len(), 1);
        assert_eq!(outcome.invocations[0].exit, ProcessExit::Code(2));
        assert_eq!(
            harness.messenger.notices().len(),
            1,
            "the user message was routed"
        );
    }

    #[test]
    fn a_script_that_exits_two_silently_denies_with_the_default_reason() {
        let harness = Harness::new("exit 2");
        let outcome = harness.dispatch("rm -rf /tmp/x");
        assert_eq!(outcome.effects.verdict, HookVerdict::Deny);
        assert_eq!(
            reasons(&outcome),
            vec!["Denied by hook ./hook.sh (exit 2)."]
        );
    }

    #[test]
    fn a_script_that_prints_allow_allows() {
        let harness = Harness::new(r#"printf '{"permission":"allow"}'"#);
        let outcome = harness.dispatch("ls");
        assert_eq!(outcome.effects.verdict, HookVerdict::Allow);
        assert!(outcome.effects.reasons.is_empty());
    }

    #[test]
    fn a_script_that_prints_nothing_has_no_opinion_when_it_fails_open() {
        let harness = Harness::new("exit 0");
        let outcome = harness.dispatch("ls");
        assert_eq!(outcome.effects.verdict, HookVerdict::NoOpinion);
        assert_eq!(outcome.invocations.len(), 1, "the hook did run");
    }

    #[test]
    fn a_script_that_prints_something_other_than_json_denies() {
        let harness = Harness::new("echo not json");
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
        let harness = Harness::with_definition("exit 1", json!({ "failClosed": true }));
        let outcome = harness.dispatch("ls");
        assert_eq!(outcome.effects.verdict, HookVerdict::Deny);
        assert_eq!(
            reasons(&outcome),
            vec!["Hook ./hook.sh exited with code 1, so the command was denied."]
        );

        let open = Harness::new("exit 1");
        assert_eq!(open.dispatch("ls").effects.verdict, HookVerdict::NoOpinion);
    }

    #[test]
    fn the_script_receives_cursor_shaped_json_on_stdin() {
        let harness = Harness::new(r#"cat > "$PWD/stdin.json"; printf '{"permission":"allow"}'"#);
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
        let harness = Harness::new(r#"pwd > "$PWD/cwd.txt"; printf '{"permission":"allow"}'"#);
        harness.dispatch("ls");
        let recorded = fs::read_to_string(harness.dir.path().join("cwd.txt")).expect("cwd file");
        assert_eq!(
            PathBuf::from(recorded.trim()).canonicalize().expect("cwd"),
            harness.dir.path().canonicalize().expect("tempdir")
        );
    }

    #[test]
    fn the_hook_gets_the_home_and_path_it_was_given() {
        let harness = Harness::new(
            r#"printf '%s\n%s\n' "$HOME" "$PATH" > "$PWD/env.txt"; printf '{"permission":"allow"}'"#,
        );
        harness.dispatch("ls");
        let recorded = fs::read_to_string(harness.dir.path().join("env.txt")).expect("env file");
        assert_eq!(
            recorded.lines().collect::<Vec<_>>(),
            vec![harness.fixture.home.to_string_lossy().as_ref(), HOOK_PATH]
        );
    }

    #[test]
    fn a_fail_closed_script_that_times_out_denies() {
        let harness =
            Harness::with_definition("sleep 30", json!({ "timeout": 1, "failClosed": true }));
        let outcome = harness.dispatch("ls");
        assert_eq!(outcome.effects.verdict, HookVerdict::Deny);
        assert_eq!(
            reasons(&outcome),
            vec!["Hook ./hook.sh timed out, so the command was denied."]
        );
    }

    #[test]
    fn a_matcher_that_does_not_match_leaves_the_command_alone() {
        let harness = Harness::with_definition("exit 2", json!({ "matcher": "^rm " }));
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
    fn loading_the_same_file_again_is_rejected_after_finalization() {
        let mut harness = Harness::new("exit 2");
        let path = harness.dir.path().join("hooks.json");
        fs::write(
            &path,
            r#"{ "version": 1, "hooks": { "beforeShellExecution": [
                { "command": "./hook.sh" }, { "command": "./hook.sh", "matcher": "x" }
            ] } }"#,
        )
        .expect("rewrite hooks.json");
        let failures = load_explicit_sources(&[run_source(&path)], &mut harness.registry);
        assert_eq!(failures.len(), 1);
        assert!(failures[0].message.contains("already finalized"));
        let snapshot = harness.registry.snapshot();
        assert_eq!(snapshot.bindings.len(), 1);
        assert!(snapshot.has_bindings_for(HookBoundary::ToolPre));
    }

    #[test]
    fn a_missing_or_invalid_file_is_reported_and_activates_nothing() {
        let dir = TempDir::new().expect("tempdir");
        let mut registry = HookRegistry::new();
        let missing = dir.path().join("hooks.json");
        let failures = load_explicit_sources(&[run_source(&missing)], &mut registry);
        assert_eq!(
            failures.len(),
            1,
            "a missing file is a failure the caller sees"
        );
        assert_eq!(failures[0].severity, DiagnosticSeverity::Error);
        assert!(registry.snapshot().is_empty());

        fs::write(&missing, "{ not json").expect("write");
        let mut registry = HookRegistry::new();
        let failures = load_explicit_sources(&[run_source(&missing)], &mut registry);
        assert_eq!(
            failures.len(),
            1,
            "a file that does not parse is a failure the caller sees: {failures:?}"
        );
        assert!(
            failures[0]
                .message
                .contains("could not load configured hook source")
        );
        assert!(registry.snapshot().is_empty());
        assert!(registry.snapshot().diagnostics.iter().any(|diagnostic| {
            diagnostic.severity == DiagnosticSeverity::Error
                && diagnostic
                    .message
                    .contains("could not load configured hook source")
        }));
    }
}
