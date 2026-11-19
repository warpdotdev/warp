//! End-to-end hook tests through the agent loop: the action model, the controller, and the
//! agent driver, with the real `HookRuntime` wired to the real process runner and the checked-in
//! Cursor fixtures. The tests named after Phase 1 test contracts (architecture.md §11) prove those
//! contracts; the factory-layout tests at the end run a factory's declared hook list through the
//! driver.
//!
//! Every runner here gets an explicit `HOME` and `PATH`, hooks write only inside temporary
//! directories, and nothing changes the test process's environment.

#![cfg(unix)]

use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use ai::agent::action_result::RequestCommandOutputResult;
use ai::hooks::runner::command::CommandHookRunner;
use ai::hooks::{
    APPROVAL_UNAVAILABLE_PREFIX, ExplicitHookSource, HookDiagnostic, HookDirectory, HookFormat,
    HookMessenger, HookNotice, HookNoticeKind, HookOnAsk, HookRegistry, HookSourceKind,
    HookSourcePolicy, RunMetadata, load_explicit_sources,
};
use cloud_object_models::CodeForge;
use instant::Instant;
use serde_json::{Value, json};
use warp_cli::agent::OutputFormat;
use warp_core::execution_mode::{AppExecutionMode, ExecutionMode};
use warp_multi_agent_api::message::{AgentReasoning, Message as MessageKind};
use warp_multi_agent_api::request::input::tool_call_result::Result as WireResult;
use warp_multi_agent_api::request::input::user_inputs::user_input::Input as WireInput;
use warp_multi_agent_api::{self as api, Message};
use warpui::r#async::Timer;
use warpui::elements::Empty;
use warpui::platform::WindowStyle;
use warpui::{
    App, AppContext, Element, Entity, EntityId, ModelHandle, SingletonEntity, TypedActionView, View,
};

use super::test_utils::{
    ControllerHarness, HookTestApp, action_model, add_messages_event, controller_harness,
    conversation_status, create_task_with_messages, done_event, exchange_count,
    finish_turn_with_text, hook_test_app, init_project_action, let_background_work_settle,
    record_statuses, root_task_id, shell_action, shell_tool_call_message, stream_init_event,
    text_message,
};
use super::{HookRuntime, RunIdentity, RunSurface};
use crate::ai::agent::conversation::{AIConversation, AIConversationId, ConversationStatus};
use crate::ai::agent::{
    AIAgentAction, AIAgentActionId, AIAgentActionResult, AIAgentActionResultType,
    CancellationReason,
};
use crate::ai::agent_sdk::driver::hooks::{
    ConfiguredHookLocation, ConfiguredHookSource, RunHooks, write_diagnostic, write_notice,
};
use crate::ai::agent_sdk::driver::terminal::TerminalDriver;
use crate::ai::agent_sdk::driver::{AgentDriver, AgentDriverError, SDKConversationOutputStatus};
use crate::ai::blocklist::{BlocklistAIActionModel, BlocklistAIHistoryModel};
use crate::ai::cloud_environments::SourceRepo;
use crate::ai::execution_profiles::ActionPermission;
use crate::ai::execution_profiles::profiles::AIExecutionProfilesModel;
use crate::settings::AgentModeCommandExecutionPredicate;
use crate::terminal::ShellLaunchData;
use crate::terminal::event::BlockMetadataReceivedEvent;
use crate::terminal::model::block::BlockMetadata;
use crate::terminal::model::session::{SessionId, SessionInfo};
use crate::terminal::model::terminal_model::BlockIndex;
use crate::terminal::model_events::ModelEvent;
use crate::terminal::shell::ShellType;

/// The `PATH` every hook in these tests runs with. The fixture scripts need only POSIX tools.
const HOOK_PATH: &str = "/usr/bin:/bin";

/// How long a test waits for real hook processes. Generous, because process spawn can stall on
/// a loaded machine; a passing run takes milliseconds.
const PROCESS_WAIT: Duration = Duration::from_secs(30);

const SAMPLE_RM_REASON: &str = "This project's hook does not allow rm -rf. Remove files one at a time, or ask the user to do it.";
const SAMPLE_PUSH_REASON: &str = "This project's hook does not allow force pushes. Push without \
                                  --force, or ask the user to do it.";

/// A temporary directory holding hook directories and the `HOME` hooks run with.
struct Sandbox {
    _dir: tempfile::TempDir,
    base: PathBuf,
    home: PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let base = dunce::canonicalize(dir.path()).expect("canonical tempdir");
        let home = base.join("home");
        fs::create_dir_all(&home).expect("mkdir home");
        Self {
            _dir: dir,
            base,
            home,
        }
    }

    fn dir(&self, name: &str) -> PathBuf {
        let path = self.base.join(name);
        fs::create_dir_all(&path).expect("mkdir");
        path
    }
}

fn write_script(path: &Path, body: &str) {
    fs::write(path, format!("#!/bin/sh\n{body}\n")).expect("write script");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("chmod");
}

/// Writes `<dir>/hooks.json` binding `./<event>.sh` (with `body`) to each of `events`.
fn write_hook_dir(dir: &Path, events: &[(&str, &str, Value)]) -> PathBuf {
    let mut hooks = serde_json::Map::new();
    for (event, body, definition) in events {
        let script = format!("{event}.sh");
        write_script(&dir.join(&script), body);
        let mut definition = definition.clone();
        definition["command"] = json!(format!("./{script}"));
        hooks.insert((*event).to_owned(), json!([definition]));
    }
    let hooks_file = dir.join("hooks.json");
    fs::write(
        &hooks_file,
        json!({ "version": 1, "hooks": hooks }).to_string(),
    )
    .expect("write hooks.json");
    hooks_file
}

/// Replaces the test doubles' runner with the real process runner.
fn install_real_runner(app: &mut App, home: &Path) {
    let home = home.to_path_buf();
    HookRuntime::handle(app).update(app, |runtime, _| {
        runtime.set_runner(Arc::new(
            CommandHookRunner::new(Arc::new(|| Some(OsString::from(HOOK_PATH))), Vec::new())
                .with_home(Some(home)),
        ));
    });
}

/// Records the process as an Oz run the server dispatched, the only kind of run that loads hooks.
fn record_oz_run(app: &mut App) {
    AppExecutionMode::handle(app).update(app, |mode, ctx| {
        *mode = AppExecutionMode::new(ExecutionMode::Sdk, false, ctx);
    });
    HookRuntime::handle(app).update(app, |runtime, _| {
        runtime.set_run(Some(RunIdentity {
            metadata: RunMetadata {
                run_id: "e2e-run".to_owned(),
                parent_run_id: None,
                task_id: Some("e2e-run".to_owned()),
            },
            surface: RunSurface::Oz,
        }));
    });
}

/// An empty root view for tests that exercise window-scoped execution profiles.
#[derive(Default)]
struct TestRootView;

impl Entity for TestRootView {
    type Event = ();
}

impl View for TestRootView {
    fn ui_name() -> &'static str {
        "HookE2eTestRootView"
    }

    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        Empty::new().finish()
    }
}

impl TypedActionView for TestRootView {
    type Action = ();
}

fn set_execute_commands(app: &mut App, terminal_view_id: EntityId, setting: ActionPermission) {
    AIExecutionProfilesModel::handle(app).update(app, |profiles, ctx| {
        let profile_id = profiles
            .active_profile(Some(terminal_view_id), ctx)
            .id()
            .clone();
        profiles.set_execute_commands(&profile_id, &setting, ctx);
    });
}

/// Loads the hook directory holding `hooks_file` as a worker-form source, straight into the
/// registry, for tests that drive the action model without a driver.
fn load_worker_hooks(app: &mut App, hooks_file: &Path) {
    let source = ExplicitHookSource::from_directory(
        HookFormat::Cursor,
        &HookDirectory::Worker {
            directory: hooks_file
                .parent()
                .expect("hooks.json has a parent")
                .to_path_buf(),
        },
        HookSourceKind::FactoryAgent,
        HookSourcePolicy::default(),
    );
    let failures = HookRegistry::handle(app)
        .update(app, |registry, _| {
            load_explicit_sources(std::slice::from_ref(&source), registry)
        })
        .expect("the registry was not finalized yet");
    assert!(failures.is_empty(), "the hook file loads: {failures:?}");
}

/// An Oz run in a test app with a real hook runner.
fn oz_run(app: &mut App, setting: ActionPermission) -> (Sandbox, HookTestApp) {
    let harness = hook_test_app(app);
    give_session_a_shell(app, &harness);
    let sandbox = Sandbox::new();
    install_real_runner(app, &sandbox.home);
    record_oz_run(app);
    app.add_window(WindowStyle::NotStealFocus, |_| TestRootView);
    set_execute_commands(app, harness.terminal_view_id, setting);
    (sandbox, harness)
}

/// Another native conversation on the harness's surface, so each case starts with an empty queue.
fn new_conversation(app: &mut App, harness: &HookTestApp) -> AIConversationId {
    let conversation = AIConversation::new(false, false);
    let conversation_id = conversation.id();
    let terminal_view_id = harness.terminal_view_id;
    app.update(|ctx| {
        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
            history.restore_conversations(terminal_view_id, vec![conversation], ctx);
        });
    });
    conversation_id
}

fn queue(
    app: &mut App,
    model: &ModelHandle<BlocklistAIActionModel>,
    conversation_id: AIConversationId,
    actions: Vec<AIAgentAction>,
) {
    model.update(app, |model, ctx| {
        model.queue_actions(actions, conversation_id, ctx)
    });
}

async fn wait_for(app: &mut App, what: &str, mut done: impl FnMut(&mut App) -> bool) {
    let deadline = Instant::now() + PROCESS_WAIT;
    while Instant::now() < deadline {
        if done(app) {
            return;
        }
        Timer::after(Duration::from_millis(20)).await;
    }
    panic!("timed out waiting for {what}");
}

async fn finished_result(
    app: &mut App,
    model: &ModelHandle<BlocklistAIActionModel>,
    action: &AIAgentAction,
) -> AIAgentActionResultType {
    let id = action.id.clone();
    wait_for(app, &format!("a result for {id}"), |app| {
        model.read(app, |model, _| model.get_action_result(&id).is_some())
    })
    .await;
    model.read(app, |model, _| {
        model
            .get_action_result(&id)
            .expect("a result")
            .result
            .clone()
    })
}

fn hook_denied_reason(result: &AIAgentActionResultType) -> Option<&str> {
    let AIAgentActionResultType::DeniedByHook { reason } = result else {
        return None;
    };
    Some(reason)
}

fn line_count(path: &Path) -> usize {
    fs::read_to_string(path).map_or(0, |text| text.lines().count())
}

fn session_id() -> SessionId {
    SessionId::from(7)
}

/// Registers a Bash session as the harness's active session.
fn give_session_a_shell(app: &mut App, harness: &HookTestApp) {
    harness.sessions.update(app, |sessions, _| {
        let mut session = SessionInfo::new_for_test().with_id(session_id());
        session.launch_data = Some(ShellLaunchData::Executable {
            executable_path: PathBuf::from("/bin/bash"),
            shell_type: ShellType::Bash,
        });
        sessions.register_session_for_test(session);
    });
    harness.dispatcher.update(app, |dispatcher, _| {
        dispatcher.set_active_session_id(session_id());
    });
}

/// Moves the harness's shell session to `cwd`, the way its prompt would report it.
fn move_session(app: &mut App, harness: &HookTestApp, cwd: &Path) {
    let cwd = cwd.display().to_string();
    harness.dispatcher.update(app, |_, ctx| {
        ctx.emit(ModelEvent::BlockMetadataReceived(
            BlockMetadataReceivedEvent {
                block_metadata: BlockMetadata::new(Some(session_id()), Some(cwd)),
                block_index: BlockIndex::zero(),
                is_after_in_band_command: false,
                is_done_bootstrapping: true,
            },
        ));
    });
}

async fn wait_until_running(
    app: &mut App,
    model: &ModelHandle<BlocklistAIActionModel>,
    action: &AIAgentAction,
) {
    let id = action.id.clone();
    wait_for(app, &format!("{id} to start running"), |app| {
        model.read(app, |model, _| {
            model
                .get_action_status(&id)
                .is_some_and(|status| status.is_running())
        })
    })
    .await;
}

/// Ends a command the mock terminal started and will never report finished.
async fn end_running_command(
    app: &mut App,
    model: &ModelHandle<BlocklistAIActionModel>,
    conversation_id: AIConversationId,
    action: &AIAgentAction,
) {
    model.update(app, |model, ctx| {
        model.cancel_action_with_id(
            conversation_id,
            &action.id,
            CancellationReason::ManuallyCancelled,
            ctx,
        )
    });
    finished_result(app, model, action).await;
}

fn add_to_denylist(app: &mut App, harness: &HookTestApp, pattern: &str) {
    let predicate = AgentModeCommandExecutionPredicate::new_regex(pattern).unwrap();
    AIExecutionProfilesModel::handle(app).update(app, |profiles, ctx| {
        let profile_id = profiles
            .active_profile(Some(harness.terminal_view_id), ctx)
            .id()
            .clone();
        profiles.add_to_command_denylist(&profile_id, &predicate, ctx);
    });
}

fn is_denylisted(result: &AIAgentActionResultType) -> bool {
    matches!(
        result,
        AIAgentActionResultType::RequestCommandOutput(
            RequestCommandOutputResult::Denylisted { .. }
        )
    )
}

/// §11 "permission matrix under run-to-completion": an explicitly configured hook cannot bypass
/// Warp's command policy, and its asks and denials still hold when Warp would run the command.
#[test]
fn permission_matrix_run_to_completion() {
    App::test((), |mut app| async move {
        let (sandbox, harness) = oz_run(&mut app, ActionPermission::AlwaysAllow);
        let work = sandbox.dir("work");
        move_session(&mut app, &harness, &work);
        let policy = sandbox.dir("run-policy");
        load_worker_hooks(
            &mut app,
            &write_hook_dir(
                &policy,
                &[(
                    "beforeShellExecution",
                    r#"input=$(cat)
printf '%s\n' "$input" >> "$PWD/inputs.log"
case "$input" in
  *'"command":"deploy'*)
    printf '{"permission":"ask","agent_message":"Deploys need a person to approve them."}' ;;
  *'"command":"release'*)
    : > "$PWD/release-started"
    i=0
    while [ ! -f "$PWD/release-go" ] && [ "$i" -lt 600 ]; do sleep 0.05; i=$((i + 1)); done
    printf '{"permission":"deny","agent_message":"Releases are frozen."}' ;;
  *'"command":"git push --force origin main"'*)
    printf '{"permission":"deny","agent_message":"This project'\''s hook does not allow force pushes. Push without --force, or ask the user to do it."}' ;;
  *) printf '{"permission":"allow"}' ;;
esac"#,
                    json!({}),
                )],
            ),
        );
        let model = action_model(&mut app, &harness);

        // The explicit hook allows the command, and Warp runs it under the active profile.
        let conversation = new_conversation(&mut app, &harness);
        let make = shell_action("make-before", "make docs");
        queue(&mut app, &model, conversation, vec![make.clone()]);
        wait_until_running(&mut app, &model, &make).await;
        end_running_command(&mut app, &model, conversation, &make).await;

        // Adding the same command to Warp's denylist still refuses it.
        add_to_denylist(&mut app, &harness, "make .*");
        let conversation = new_conversation(&mut app, &harness);
        let make = shell_action("make", "make docs");
        queue(&mut app, &model, conversation, vec![make.clone()]);
        let result = finished_result(&mut app, &model, &make).await;
        assert!(is_denylisted(&result), "{result:?}");
        assert!(
            fs::read_to_string(policy.join("inputs.log"))
                .expect("the explicit hook ran")
                .contains("make docs"),
            "the hook saw the command and allowed it"
        );

        // A headless `ask`: nobody can answer, so the action is denied with the reason.
        let conversation = new_conversation(&mut app, &harness);
        let deploy = shell_action("deploy", "deploy production");
        queue(&mut app, &model, conversation, vec![deploy.clone()]);
        let result = finished_result(&mut app, &model, &deploy).await;
        let reason = hook_denied_reason(&result).expect("the ask became a denial");
        assert!(reason.starts_with(APPROVAL_UNAVAILABLE_PREFIX), "{reason}");
        assert!(
            reason.contains("Deploys need a person to approve them."),
            "{reason}"
        );

        // The user approves while the hook is still deciding: the deny still wins.
        let conversation = new_conversation(&mut app, &harness);
        let release = shell_action("release", "release v2");
        queue(&mut app, &model, conversation, vec![release.clone()]);
        let started = policy.join("release-started");
        wait_for(&mut app, "the release hook to start", |_| started.exists()).await;
        model.update(&mut app, |model, ctx| {
            model.execute_action(&release.id, conversation, ctx)
        });
        fs::write(policy.join("release-go"), "").expect("release the hook");
        let result = finished_result(&mut app, &model, &release).await;
        assert_eq!(hook_denied_reason(&result), Some("Releases are frozen."));

        // When the profile asks, a headless run refuses the command until the conversation opts
        // into auto-approval.
        set_execute_commands(
            &mut app,
            harness.terminal_view_id,
            ActionPermission::AlwaysAsk,
        );
        let asking = new_conversation(&mut app, &harness);
        let build = shell_action("build-asking", "npm run build");
        queue(&mut app, &model, asking, vec![build.clone()]);
        let result = finished_result(&mut app, &model, &build).await;
        assert!(is_denylisted(&result), "{result:?}");

        let auto_approving = new_conversation(&mut app, &harness);
        app.update(|ctx| {
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                history.toggle_autoexecute_override(&auto_approving, harness.terminal_view_id, ctx);
            });
        });
        let build = shell_action("build", "npm run build");
        queue(&mut app, &model, auto_approving, vec![build.clone()]);
        wait_until_running(&mut app, &model, &build).await;
        end_running_command(&mut app, &model, auto_approving, &build).await;

        // Auto-approval does not override the explicit hook's denial.
        let push = shell_action("push", "git push --force origin main");
        queue(&mut app, &model, auto_approving, vec![push.clone()]);
        let result = finished_result(&mut app, &model, &push).await;
        assert_eq!(hook_denied_reason(&result), Some(SAMPLE_PUSH_REASON));
    });
}

/// §11 "batch denial with cwd changes between actions": three shell commands in one batch; the
/// second moves the shell into another directory, while the finalized explicit source remains
/// active for the third.
#[test]
fn batch_denial_with_cwd_changes() {
    App::test((), |mut app| async move {
        let (sandbox, harness) = oz_run(&mut app, ActionPermission::AlwaysAllow);
        let outer = sandbox.dir("outer");
        let nested = outer.join("nested");
        fs::create_dir_all(&nested).expect("mkdir nested");
        move_session(&mut app, &harness, &outer);
        let policy = sandbox.dir("run-policy");
        load_worker_hooks(
            &mut app,
            &write_hook_dir(
                &policy,
                &[(
                    "beforeShellExecution",
                    r#"input=$(cat)
printf '%s\n' "$input" >> "$PWD/inputs.log"
case "$input" in
  *'"command":"rm -rf build"'*)
    printf '{"permission":"deny","agent_message":"This project'\''s hook does not allow rm -rf. Remove files one at a time, or ask the user to do it."}' ;;
  *'"command":"ls"'*)
    printf '{"permission":"deny","agent_message":"The explicit policy allows no listing."}' ;;
  *) printf '{"permission":"allow"}' ;;
esac"#,
                    json!({}),
                )],
            ),
        );
        let model = action_model(&mut app, &harness);

        let delete = shell_action("delete", "rm -rf build");
        let enter = shell_action("enter", "cd nested");
        let list = shell_action("list", "ls");
        queue(
            &mut app,
            &model,
            harness.conversation_id,
            vec![delete.clone(), enter.clone(), list.clone()],
        );
        let result = finished_result(&mut app, &model, &delete).await;
        assert_eq!(hook_denied_reason(&result), Some(SAMPLE_RM_REASON));

        // The explicit source allows `cd`; the shell reports its new directory before the test
        // ends the mock command.
        wait_until_running(&mut app, &model, &enter).await;
        move_session(&mut app, &harness, &nested);
        end_running_command(&mut app, &model, harness.conversation_id, &enter).await;

        let result = finished_result(&mut app, &model, &list).await;
        assert_eq!(
            hook_denied_reason(&result),
            Some("The explicit policy allows no listing."),
            "the third command was judged by the same finalized source"
        );
        let inputs = fs::read_to_string(policy.join("inputs.log")).expect("the hook ran");
        assert!(
            inputs.contains(r#""command":"cd nested""#) && inputs.contains(r#""command":"ls""#),
            "the explicit source did not see both commands: {inputs}"
        );
    });
}

type RunExit = futures::channel::oneshot::Receiver<SDKConversationOutputStatus>;

/// A driver started on `harness`'s terminal as an Oz run whose execution configuration selected
/// `sources`, with `repositories` cloned under `working_dir`. Returns the driver, which must stay
/// alive for the run to, and the run's exit receiver, or why the run did not start.
fn try_start_driver(
    app: &mut App,
    harness: &ControllerHarness,
    working_dir: &Path,
    sources: Vec<ConfiguredHookSource>,
    repositories: &[SourceRepo],
) -> (ModelHandle<AgentDriver>, Result<RunExit, AgentDriverError>) {
    let working_dir = working_dir.to_path_buf();
    let terminal = harness.terminal.clone();
    let driver = app.add_model(|ctx| {
        let terminal_driver = TerminalDriver::create_from_existing_view(terminal, ctx);
        AgentDriver::new_for_test(working_dir, terminal_driver, ctx)
    });
    let run_exit = driver.update(app, |driver, ctx| {
        driver.start_run_with_hooks_for_test(
            RunHooks {
                sources,
                surface: RunSurface::Oz,
            },
            repositories,
            ctx,
        )
    });
    (driver, run_exit)
}

/// [`try_start_driver`] with hook directories on the worker and no repositories, for a run whose
/// sources all load.
fn start_driver(
    app: &mut App,
    harness: &ControllerHarness,
    working_dir: &Path,
    hook_dirs: Vec<PathBuf>,
) -> (ModelHandle<AgentDriver>, RunExit) {
    let sources = hook_dirs
        .iter()
        .map(|dir| worker_entry(dir, HookSourceKind::FactoryAgent, false))
        .collect();
    let (driver, run_exit) = try_start_driver(app, harness, working_dir, sources, &[]);
    (driver, run_exit.expect("the run's hook sources load"))
}

/// A worker-form entry, as the execution configuration names one.
fn worker_entry(dir: &Path, kind: HookSourceKind, strict: bool) -> ConfiguredHookSource {
    ConfiguredHookSource {
        format: HookFormat::Cursor,
        location: ConfiguredHookLocation::Worker {
            path: dir.display().to_string(),
        },
        kind,
        policy: HookSourcePolicy {
            strict,
            ..Default::default()
        },
    }
}

/// §11 "stop versus Oz idle exit", through the agent driver's own completion logic: a stop hook
/// that queues a follow-up never lets the run finish between the stop hook and the follow-up's
/// request.
#[test]
fn stop_versus_idle_exit() {
    App::test((), |mut app| async move {
        let harness = controller_harness(&mut app);
        let sandbox = Sandbox::new();
        install_real_runner(&mut app, &sandbox.home);
        record_oz_run(&mut app);
        let stop_dir = sandbox.dir("stop-hooks");
        write_hook_dir(
            &stop_dir,
            &[(
                "stop",
                r#"cat > /dev/null
count=$(cat "$PWD/count" 2>/dev/null || echo 0)
count=$((count + 1))
echo "$count" > "$PWD/count"
if [ "$count" -eq 1 ]; then
  i=0
  while [ ! -f "$PWD/go" ] && [ "$i" -lt 600 ]; do sleep 0.05; i=$((i + 1)); done
  printf '{"followup_message":"Now summarize what you found."}'
fi"#,
                json!({}),
            )],
        );
        let (_driver, mut run_exit) =
            start_driver(&mut app, &harness, &sandbox.base, vec![stop_dir.clone()]);

        let (conversation_id, stream) = harness.mock_stream(&mut app);
        let (_recorder, statuses) = record_statuses(&mut app, conversation_id);
        finish_turn_with_text(&mut app, conversation_id, &stream, "First answer.");
        let count = stop_dir.join("count");
        wait_for(&mut app, "the stop hook to start", |_| count.exists()).await;
        let_background_work_settle(&mut app).await;
        assert!(
            run_exit.try_recv().expect("the run is alive").is_none(),
            "the run did not end while the stop hook ran"
        );
        assert_eq!(
            conversation_status(&app, conversation_id),
            Some(ConversationStatus::InProgress)
        );

        fs::write(stop_dir.join("go"), "").expect("let the stop hook answer");
        wait_for(&mut app, "the follow-up's request", |app| {
            exchange_count(app, conversation_id) == 2
        })
        .await;
        assert!(
            run_exit.try_recv().expect("the run is alive").is_none(),
            "the run did not end between the stop hook and the follow-up's request"
        );
        let seen = statuses.lock().unwrap().clone();
        assert!(
            !seen.contains(&ConversationStatus::Success),
            "the conversation never looked finished: {seen:?}"
        );
        assert_eq!(line_count(&count), 1);
    });
}

/// §11 "duplicate session.start": creating a conversation and starting a driver run fire nothing;
/// the conversation's first request fires `session.start` once, a later prompt fires none, and a
/// conversation that already has exchanges (as a restored one does) fires none.
#[test]
fn duplicate_session_start() {
    App::test((), |mut app| async move {
        let harness = controller_harness(&mut app);
        let sandbox = Sandbox::new();
        install_real_runner(&mut app, &sandbox.home);
        record_oz_run(&mut app);
        let session_dir = sandbox.dir("session-hooks");
        write_hook_dir(
            &session_dir,
            &[(
                "sessionStart",
                r#"input=$(cat)
printf '%s\n' "$input" >> "$PWD/session-starts.log"
printf '{"env":{"E2E_SESSION":"yes"}}'"#,
                json!({}),
            )],
        );
        let starts = session_dir.join("session-starts.log");

        let terminal_id = harness.terminal_id();
        let conversation_id = app.update(|ctx| {
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                history.start_new_conversation(terminal_id, false, false, false, ctx)
            })
        });
        let _run = start_driver(&mut app, &harness, &sandbox.base, vec![session_dir.clone()]);
        let_background_work_settle(&mut app).await;
        assert_eq!(
            line_count(&starts),
            0,
            "neither creation nor the run fires it"
        );

        harness.send_prompt_in(&mut app, conversation_id, "hello");
        wait_for(&mut app, "the first request", |app| {
            exchange_count(app, conversation_id) == 1
        })
        .await;
        assert_eq!(line_count(&starts), 1);
        let input: Value = serde_json::from_str(
            fs::read_to_string(&starts)
                .expect("log")
                .lines()
                .next()
                .expect("one line"),
        )
        .expect("JSON");
        assert_eq!(input["hook_event_name"], "sessionStart");
        assert_eq!(input["session_id"], conversation_id.to_string());

        harness.update_controller(&mut app, |controller, ctx| {
            controller.cancel_conversation_progress(
                conversation_id,
                CancellationReason::ManuallyCancelled,
                ctx,
            );
        });
        harness.send_prompt_in(&mut app, conversation_id, "again");
        wait_for(&mut app, "the second request", |app| {
            exchange_count(app, conversation_id) == 2
        })
        .await;
        let_background_work_settle(&mut app).await;
        assert_eq!(
            line_count(&starts),
            1,
            "a later prompt does not start a session"
        );

        // A conversation that arrives with exchanges, as a restored or resumed one does.
        let restored = AIConversation::new(false, false);
        let restored_id = restored.id();
        app.update(|ctx| {
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                history.restore_conversations(terminal_id, vec![restored], ctx);
            });
        });
        harness.mock_request(&mut app, restored_id);
        harness.update_controller(&mut app, |controller, ctx| {
            controller.cancel_conversation_progress(
                restored_id,
                CancellationReason::ManuallyCancelled,
                ctx,
            );
        });
        harness.send_prompt_in(&mut app, restored_id, "picking up where we left off");
        wait_for(&mut app, "the restored conversation's request", |app| {
            exchange_count(app, restored_id) == 2
        })
        .await;
        let_background_work_settle(&mut app).await;
        assert_eq!(
            line_count(&starts),
            1,
            "a conversation with exchanges fires none"
        );
    });
}

fn reasoning_message(task_id: &str, text: &str) -> Message {
    Message {
        id: uuid::Uuid::new_v4().to_string(),
        task_id: task_id.to_owned(),
        request_id: "request".to_owned(),
        timestamp: Some(prost_types::Timestamp::from(SystemTime::now())),
        message: Some(MessageKind::AgentReasoning(AgentReasoning {
            reasoning: text.to_owned(),
            finished_duration: Some(prost_types::Duration {
                seconds: 1,
                nanos: 0,
            }),
        })),
        ..Default::default()
    }
}

/// §11 "once-per-message response observations": a stream of text, reasoning, a tool call, and
/// text runs the audit hook twice for `afterAgentResponse` and once for `afterAgentThought`.
#[test]
fn once_per_message_response_observations() {
    App::test((), |mut app| async move {
        let harness = controller_harness(&mut app);
        let sandbox = Sandbox::new();
        install_real_runner(&mut app, &sandbox.home);
        let hooks = sandbox.dir("observers");
        let audit = r#"input=$(cat)
printf '%s\n' "$input" >> "$PWD/audit.log""#;
        load_worker_hooks(
            &mut app,
            &write_hook_dir(
                &hooks,
                &[
                    ("afterAgentResponse", audit, json!({})),
                    ("afterAgentThought", audit, json!({})),
                ],
            ),
        );

        let (conversation_id, stream) = harness.mock_stream(&mut app);
        let task_id = root_task_id(&app, conversation_id);
        stream.update(&mut app, |stream, ctx| {
            stream.emit_response_event_for_test(stream_init_event(), ctx);
            stream.emit_response_event_for_test(
                create_task_with_messages(&task_id, vec![text_message(&task_id, "First part.")]),
                ctx,
            );
            stream.emit_response_event_for_test(
                add_messages_event(
                    &task_id,
                    vec![
                        reasoning_message(&task_id, "Thinking it over."),
                        shell_tool_call_message(&task_id, "ls"),
                        text_message(&task_id, "Second part."),
                    ],
                ),
                ctx,
            );
            stream.emit_response_event_for_test(done_event(), ctx);
            stream.emit_after_stream_finished_for_test(ctx);
        });

        let log = hooks.join("audit.log");
        wait_for(&mut app, "three observer runs", |_| line_count(&log) >= 3).await;
        let_background_work_settle(&mut app).await;
        let events = fs::read_to_string(&log)
            .expect("log")
            .lines()
            .map(|line| {
                let input: Value = serde_json::from_str(line).expect("one JSON object per line");
                (
                    input["hook_event_name"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                    input["text"].as_str().unwrap_or_default().to_owned(),
                )
            })
            .collect::<Vec<_>>();
        let mut responses = events
            .iter()
            .filter(|(event, _)| event == "afterAgentResponse")
            .map(|(_, text)| text.as_str())
            .collect::<Vec<_>>();
        responses.sort_unstable();
        assert_eq!(responses, vec!["First part.", "Second part."], "{events:?}");
        let thoughts = events
            .iter()
            .filter(|(event, _)| event == "afterAgentThought")
            .map(|(_, text)| text.as_str())
            .collect::<Vec<_>>();
        assert_eq!(thoughts, vec!["Thinking it over."], "{events:?}");
    });
}

/// §11 "deny reason reaches the wire": explicit shell and non-shell denials carry the script's
/// text in the generic `denied_by_hook` result and emit a `Denied` notice.
#[test]
fn deny_reason_reaches_the_wire() {
    App::test((), |mut app| async move {
        let (sandbox, harness) = oz_run(&mut app, ActionPermission::AlwaysAllow);
        let work = sandbox.dir("work");
        move_session(&mut app, &harness, &work);
        load_worker_hooks(
            &mut app,
            &write_hook_dir(
                &sandbox.dir("tool-policy"),
                &[
                    (
                        "beforeShellExecution",
                        r#"cat > /dev/null
printf '{"permission":"deny","agent_message":"This project'\''s hook does not allow rm -rf. Remove files one at a time, or ask the user to do it."}'"#,
                        json!({ "matcher": "rm -rf" }),
                    ),
                    (
                        "preToolUse",
                        r#"cat > /dev/null
printf '{"permission":"deny","agent_message":"Project setup is handled by the platform team."}'"#,
                        json!({ "matcher": "InitProject" }),
                    ),
                ],
            ),
        );
        let model = action_model(&mut app, &harness);

        let rm = shell_action("rm", "rm -rf build");
        queue(&mut app, &model, harness.conversation_id, vec![rm.clone()]);
        let result = finished_result(&mut app, &model, &rm).await;
        assert_eq!(hook_denied_reason(&result), Some(SAMPLE_RM_REASON));
        let wire =
            api::request::input::user_inputs::user_input::Input::try_from(AIAgentActionResult {
                id: rm.id.clone(),
                task_id: rm.task_id.clone(),
                result,
            })
            .expect("a hook denial reaches the wire");
        let api::request::input::user_inputs::user_input::Input::ToolCallResult(wire) = wire else {
            panic!("a tool call result on the wire");
        };
        assert_eq!(
            wire.result,
            Some(api::request::input::tool_call_result::Result::DeniedByHook(
                api::DeniedByHook {
                    reason: SAMPLE_RM_REASON.to_owned(),
                }
            ))
        );

        let conversation = new_conversation(&mut app, &harness);
        let init = init_project_action("init");
        queue(&mut app, &model, conversation, vec![init.clone()]);
        let result = finished_result(&mut app, &model, &init).await;
        assert_eq!(
            result,
            AIAgentActionResultType::DeniedByHook {
                reason: "Project setup is handled by the platform team.".to_owned(),
            }
        );
        let wire =
            api::request::input::user_inputs::user_input::Input::try_from(AIAgentActionResult {
                id: init.id.clone(),
                task_id: init.task_id.clone(),
                result,
            })
            .expect("a hook denial reaches the wire");
        let api::request::input::user_inputs::user_input::Input::ToolCallResult(wire) = wire else {
            panic!("a tool call result on the wire");
        };
        assert_eq!(
            wire.result,
            Some(api::request::input::tool_call_result::Result::DeniedByHook(
                api::DeniedByHook {
                    reason: "Project setup is handled by the platform team.".to_owned(),
                }
            ))
        );
        let notices = harness.hooks.messenger.notices();
        assert!(
            notices.iter().any(|notice| {
                notice.kind == HookNoticeKind::Denied
                    && notice.action_id.as_ref() == Some(&init.id)
                    && notice.text == "Project setup is handled by the platform team."
            }),
            "{notices:?}"
        );
    });
}

/// Records each notice and diagnostic as the Oz messenger would write it in `Ndjson` output, using
/// the messenger's own record writers.
#[derive(Default)]
struct NdjsonRecords(Mutex<Vec<u8>>);

impl NdjsonRecords {
    fn records(&self) -> Vec<Value> {
        let bytes = self.0.lock().unwrap().clone();
        String::from_utf8(bytes)
            .expect("UTF-8 output")
            .lines()
            .map(|line| serde_json::from_str(line).expect("each line is one JSON record"))
            .collect()
    }
}

impl HookMessenger for NdjsonRecords {
    fn notify(&self, notice: HookNotice) {
        write_notice(
            &notice,
            None,
            OutputFormat::Ndjson,
            &mut *self.0.lock().unwrap(),
        )
        .expect("write notice");
    }

    fn diagnostic(&self, diagnostic: HookDiagnostic) {
        write_diagnostic(
            &diagnostic,
            OutputFormat::Ndjson,
            &mut *self.0.lock().unwrap(),
        )
        .expect("write diagnostic");
    }
}

/// §11 "Oz messenger output": in `Ndjson` output a hook denial is one `hook_notice` record and a
/// hook file that does not parse is one `hook_diagnostic` record, each a line of JSON.
#[test]
fn oz_messenger_output() {
    App::test((), |mut app| async move {
        let harness = controller_harness(&mut app);
        let sandbox = Sandbox::new();
        install_real_runner(&mut app, &sandbox.home);
        record_oz_run(&mut app);
        let records = Arc::new(NdjsonRecords::default());
        let messenger: Arc<dyn HookMessenger> = records.clone();
        HookRuntime::handle(&app).update(&mut app, |runtime, _| {
            runtime.set_messenger(messenger);
        });
        let policy = sandbox.dir("policy");
        write_hook_dir(
            &policy,
            &[(
                "beforeShellExecution",
                r#"cat > /dev/null
printf '{"agent_message":"No deletions in this run."}'
exit 2"#,
                json!({}),
            )],
        );
        let broken = sandbox.dir("broken");
        fs::write(broken.join("hooks.json"), "{ not json").expect("write broken hooks.json");

        let _run = start_driver(&mut app, &harness, &sandbox.base, vec![policy, broken]);
        let (conversation_id, stream) = harness.mock_stream(&mut app);
        let task_id = root_task_id(&app, conversation_id);
        stream.update(&mut app, |stream, ctx| {
            stream.emit_response_event_for_test(stream_init_event(), ctx);
            stream.emit_response_event_for_test(
                create_task_with_messages(
                    &task_id,
                    vec![shell_tool_call_message(&task_id, "rm -rf build")],
                ),
                ctx,
            );
            stream.emit_response_event_for_test(done_event(), ctx);
            stream.emit_after_stream_finished_for_test(ctx);
        });
        let has_denial = |records: &NdjsonRecords| {
            records
                .records()
                .iter()
                .any(|record| record["type"] == "hook_notice" && record["kind"] == "denied")
        };
        wait_for(&mut app, "the denial record", |_| has_denial(&records)).await;
        let_background_work_settle(&mut app).await;

        let records = records.records();
        let denials = records
            .iter()
            .filter(|record| record["type"] == "hook_notice" && record["kind"] == "denied")
            .collect::<Vec<_>>();
        assert_eq!(denials.len(), 1, "{records:?}");
        assert_eq!(denials[0]["text"], "No deletions in this run.");
        assert_eq!(
            denials[0]["local_conversation_id"],
            conversation_id.to_string()
        );
        let diagnostics = records
            .iter()
            .filter(|record| record["type"] == "hook_diagnostic")
            .collect::<Vec<_>>();
        assert_eq!(diagnostics.len(), 1, "{records:?}");
        assert_eq!(diagnostics[0]["severity"], "error");
        assert!(
            diagnostics[0]["source"]
                .as_str()
                .is_some_and(|source| source.contains("broken")),
            "{records:?}"
        );
    });
}

const FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../crates/ai/src/hooks/fixtures"
);

const REVIEWER_DEPLOY_REASON: &str =
    "Reviewers do not deploy. Hand the change to the release agent.";

/// Copies checked-in fixture `name` to `to`, with its scripts executable.
fn copy_fixture(name: &str, to: &Path) {
    fn copy_tree(from: &Path, to: &Path) {
        fs::create_dir_all(to).expect("mkdir");
        for entry in fs::read_dir(from).expect("read fixture dir") {
            let entry = entry.expect("dir entry");
            let target = to.join(entry.file_name());
            if entry.file_type().expect("file type").is_dir() {
                copy_tree(&entry.path(), &target);
            } else {
                fs::copy(entry.path(), &target).expect("copy fixture file");
                if target
                    .extension()
                    .is_some_and(|extension| extension == "sh")
                {
                    fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).expect("chmod");
                }
            }
        }
    }
    copy_tree(&Path::new(FIXTURES).join(name), to);
}

fn acme_repo(name: &str) -> SourceRepo {
    SourceRepo::new(CodeForge::GitHub, "acme".to_owned(), name.to_owned())
}

/// A repository-form entry for `acme/<repo>`, as the execution configuration names one.
fn repository_entry(
    repo: &str,
    path: &str,
    kind: HookSourceKind,
    policy: HookSourcePolicy,
) -> ConfiguredHookSource {
    ConfiguredHookSource {
        format: HookFormat::Cursor,
        location: ConfiguredHookLocation::Repository {
            repo: acme_repo(repo),
            path: path.to_owned(),
        },
        kind,
        policy,
    }
}

/// The factory layout fixture checked out the way a run clones its repositories: the factory
/// definition repository and the `api` repository side by side in the run's working directory.
struct FactoryRun {
    harness: ControllerHarness,
    sandbox: Sandbox,
    records: Arc<NdjsonRecords>,
    working_dir: PathBuf,
}

impl FactoryRun {
    fn new(app: &mut App) -> Self {
        let harness = controller_harness(app);
        let sandbox = Sandbox::new();
        install_real_runner(app, &sandbox.home);
        record_oz_run(app);
        let records = Arc::new(NdjsonRecords::default());
        let messenger: Arc<dyn HookMessenger> = records.clone();
        HookRuntime::handle(app).update(app, |runtime, _| runtime.set_messenger(messenger));
        let working_dir = sandbox.dir("work");
        copy_fixture(
            "factory_layout/acme-factory",
            &working_dir.join("acme-factory"),
        );
        copy_fixture("sample_project", &working_dir.join("api"));
        Self {
            harness,
            sandbox,
            records,
            working_dir,
        }
    }

    /// The hook list `factory.yaml` and `agents/reviewer/agent.md` declare, as the server sends
    /// it: the agent's entries first, then the factory's.
    fn declared_sources() -> Vec<ConfiguredHookSource> {
        Self::declared_sources_with(HookOnAsk::FailRun)
    }

    /// [`Self::declared_sources`], with the reviewer's entry set to `reviewer_on_ask`.
    fn declared_sources_with(reviewer_on_ask: HookOnAsk) -> Vec<ConfiguredHookSource> {
        vec![
            repository_entry(
                "api",
                ".cursor",
                HookSourceKind::FactoryAgent,
                HookSourcePolicy::default(),
            ),
            repository_entry(
                "acme-factory",
                "agents/reviewer/hooks",
                HookSourceKind::FactoryAgent,
                HookSourcePolicy {
                    strict: false,
                    on_ask: reviewer_on_ask,
                },
            ),
            repository_entry(
                "acme-factory",
                "hooks",
                HookSourceKind::FactoryWide,
                HookSourcePolicy {
                    strict: true,
                    on_ask: HookOnAsk::Deny,
                },
            ),
        ]
    }

    fn start(
        &self,
        app: &mut App,
        sources: Vec<ConfiguredHookSource>,
    ) -> (ModelHandle<AgentDriver>, Result<RunExit, AgentDriverError>) {
        try_start_driver(
            app,
            &self.harness,
            &self.working_dir,
            sources,
            &[acme_repo("acme-factory"), acme_repo("api")],
        )
    }

    /// Streams one shell tool call with id `call_id` into a new conversation and returns the
    /// conversation.
    fn stream_shell_call(&self, app: &mut App, call_id: &str, command: &str) -> AIConversationId {
        let (conversation_id, stream) = self.harness.mock_stream(app);
        let task_id = root_task_id(app, conversation_id);
        let mut call = shell_tool_call_message(&task_id, command);
        if let Some(MessageKind::ToolCall(tool_call)) = &mut call.message {
            tool_call.tool_call_id = call_id.to_owned();
        }
        stream.update(app, |stream, ctx| {
            stream.emit_response_event_for_test(stream_init_event(), ctx);
            stream
                .emit_response_event_for_test(create_task_with_messages(&task_id, vec![call]), ctx);
            stream.emit_response_event_for_test(done_event(), ctx);
            stream.emit_after_stream_finished_for_test(ctx);
        });
        conversation_id
    }

    async fn result_for(&self, app: &mut App, call_id: &str) -> AIAgentActionResult {
        let model = self
            .harness
            .terminal
            .read(app, |view, _| view.ai_action_model().clone());
        let id = AIAgentActionId::from(call_id.to_owned());
        wait_for(app, &format!("a result for {call_id}"), |app| {
            model.read(app, |model, _| model.get_action_result(&id).is_some())
        })
        .await;
        model.read(app, |model, _| {
            AIAgentActionResult::clone(model.get_action_result(&id).expect("a result"))
        })
    }

    fn records_of(&self, kind: &str) -> Vec<Value> {
        self.records
            .records()
            .into_iter()
            .filter(|record| record["type"] == kind)
            .collect()
    }

    /// The `hook_diagnostic` records for sources that failed.
    fn load_errors(&self) -> Vec<Value> {
        self.records_of("hook_diagnostic")
            .into_iter()
            .filter(|record| record["severity"] == "error")
            .collect()
    }
}

fn denied_by_hook_on_the_wire(result: AIAgentActionResult) -> Option<String> {
    let WireInput::ToolCallResult(wire) = WireInput::try_from(result).ok()? else {
        return None;
    };
    let Some(WireResult::DeniedByHook(denied)) = wire.result else {
        return None;
    };
    Some(denied.reason)
}

/// Rung 4 of the testing ladder with the factory layout: an execution configuration that names
/// a factory definition repository's hook directories and a second repository's unchanged
/// `.cursor` directory. The second repository's root-relative hook really runs and denies, its
/// reason reaches the wire as `denied_by_hook`, and the factory's own hook runs as well.
#[test]
fn a_factory_run_denies_through_another_repositorys_cursor_hooks() {
    App::test((), |mut app| async move {
        let run = FactoryRun::new(&mut app);
        let (_driver, started) = run.start(&mut app, FactoryRun::declared_sources());
        let _run_exit = started.expect("every declared source loads");
        // Nothing about these sources is reported: they all load, and the sample's `sessionStart`
        // note that model context is not delivered yet only predicts what a hook might do, so it
        // stays in the log.
        let diagnostics = run.records_of("hook_diagnostic");
        assert!(diagnostics.is_empty(), "{diagnostics:?}");

        run.stream_shell_call(&mut app, "rm-call", "rm -rf build");
        let result = run.result_for(&mut app, "rm-call").await;
        assert_eq!(
            denied_by_hook_on_the_wire(result).as_deref(),
            Some(SAMPLE_RM_REASON)
        );
        let denials = run
            .records_of("hook_notice")
            .into_iter()
            .filter(|record| record["kind"] == "denied")
            .collect::<Vec<_>>();
        assert_eq!(denials.len(), 1, "{denials:?}");
        assert_eq!(denials[0]["text"], SAMPLE_RM_REASON);
        let factory_log =
            fs::read_to_string(run.working_dir.join("acme-factory/.factory-shell.log"))
                .expect("the factory's hook ran from its repository root");
        assert!(factory_log.contains("rm -rf build"), "{factory_log}");

        run.stream_shell_call(&mut app, "deploy-call", "deploy production");
        let result = run.result_for(&mut app, "deploy-call").await;
        assert_eq!(
            denied_by_hook_on_the_wire(result).as_deref(),
            Some(REVIEWER_DEPLOY_REASON)
        );
    });
}

/// A strict entry that cannot be found stops the run before it starts, and the failure names only
/// that entry.
#[test]
fn a_missing_strict_entry_stops_the_run() {
    App::test((), |mut app| async move {
        let run = FactoryRun::new(&mut app);
        let mut sources = FactoryRun::declared_sources();
        sources.push(repository_entry(
            "acme-factory",
            "agents/missing/hooks",
            HookSourceKind::FactoryWide,
            HookSourcePolicy {
                strict: true,
                on_ask: HookOnAsk::Deny,
            },
        ));
        let (_driver, started) = run.start(&mut app, sources);
        let Err(AgentDriverError::HookSourcesUnavailable { details }) = started else {
            panic!("the missing strict entry should stop the run");
        };
        assert_eq!(details.len(), 1, "{details:?}");
        assert!(details[0].contains("agents/missing/hooks"), "{details:?}");
        let diagnostics = run.load_errors();
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    });
}

/// An entry that is not strict and cannot be found is one `hook_diagnostic` error record, and the
/// run goes on with the entries that loaded.
#[test]
fn a_missing_entry_that_is_not_strict_is_reported_and_the_run_goes_on() {
    App::test((), |mut app| async move {
        let run = FactoryRun::new(&mut app);
        let missing = run.sandbox.base.join("not-on-this-worker");
        let mut sources = FactoryRun::declared_sources();
        sources.push(worker_entry(&missing, HookSourceKind::FactoryWide, false));
        let (_driver, started) = run.start(&mut app, sources);
        let _run_exit = started.expect("a missing lenient entry does not stop the run");
        let diagnostics = run.load_errors();
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(diagnostics[0]["severity"], "error");
        assert!(
            diagnostics[0]["source"]
                .as_str()
                .is_some_and(|source| source.contains("not-on-this-worker")),
            "{diagnostics:?}"
        );

        run.stream_shell_call(&mut app, "rm-call", "rm -rf build");
        let result = run.result_for(&mut app, "rm-call").await;
        assert_eq!(
            denied_by_hook_on_the_wire(result).as_deref(),
            Some(SAMPLE_RM_REASON),
            "the entries that loaded still apply"
        );
    });
}

/// The reviewer's own entry is set to `fail_run`, so when its hook asks before a publish and
/// nobody can answer, the action is denied and the run's conversation fails.
#[test]
fn an_ask_from_a_fail_run_entry_ends_the_factory_run() {
    App::test((), |mut app| async move {
        let run = FactoryRun::new(&mut app);
        let (_driver, started) = run.start(&mut app, FactoryRun::declared_sources());
        let _run_exit = started.expect("every declared source loads");

        let conversation_id = run.stream_shell_call(&mut app, "publish-call", "publish v2");
        let result = run.result_for(&mut app, "publish-call").await;
        let reason = denied_by_hook_on_the_wire(result).expect("the ask became a denial");
        assert!(reason.starts_with(APPROVAL_UNAVAILABLE_PREFIX), "{reason}");
        assert!(
            reason.contains("Publishing needs a person to approve it."),
            "{reason}"
        );
        wait_for(&mut app, "the run's conversation to fail", |app| {
            conversation_status(app, conversation_id) == Some(ConversationStatus::Error)
        })
        .await;
    });
}

/// The control for the test above: with the reviewer's entry set to `deny`, its hook's ask still
/// denies the publish with the same reason, and the run's conversation does not fail.
#[test]
fn an_ask_from_a_deny_entry_is_denied_and_the_run_goes_on() {
    App::test((), |mut app| async move {
        let run = FactoryRun::new(&mut app);
        let (_driver, started) =
            run.start(&mut app, FactoryRun::declared_sources_with(HookOnAsk::Deny));
        let _run_exit = started.expect("every declared source loads");

        let conversation_id = run.stream_shell_call(&mut app, "publish-call", "publish v2");
        let result = run.result_for(&mut app, "publish-call").await;
        let reason = denied_by_hook_on_the_wire(result).expect("the ask became a denial");
        assert!(reason.starts_with(APPROVAL_UNAVAILABLE_PREFIX), "{reason}");
        let_background_work_settle(&mut app).await;
        assert_ne!(
            conversation_status(&app, conversation_id),
            Some(ConversationStatus::Error)
        );
    });
}
