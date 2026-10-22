//! End-to-end hook tests through the agent loop: the action model, the controller, and the
//! agent driver, with the real `HookRuntime` wired to the real process runner and the checked-in
//! Cursor fixtures. Each test is named after the Phase 1 test contract it proves
//! (architecture.md §11).
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
    APPROVAL_UNAVAILABLE_PREFIX, ExplicitHookSource, ExplicitSourceScope, HookBoundary,
    HookDiagnostic, HookExecutionHost, HookFormat, HookMessenger, HookNotice, HookNoticeKind,
    HookRegistry, HookSourceKind, PROJECT_CURSOR_HOOKS_PATH, RunMetadata, load_explicit_sources,
};
use instant::Instant;
use repo_metadata::DirectoryWatcher;
use repo_metadata::repositories::{
    DetectedRepositories, DetectedRepositoriesEvent, RepoDetectionSource,
};
use serde_json::{Value, json};
use warp_cli::agent::OutputFormat;
use warp_core::execution_mode::{AppExecutionMode, ExecutionMode};
use warp_multi_agent_api::message::{AgentReasoning, Message as MessageKind};
use warp_multi_agent_api::{self as api, Message};
use warp_util::standardized_path::StandardizedPath;
use warpui::r#async::Timer;
use warpui::elements::Empty;
use warpui::platform::WindowStyle;
use warpui::{
    App, AppContext, Element, Entity, EntityId, ModelHandle, SingletonEntity, TypedActionView, View,
};
use watcher::BulkFilesystemWatcherEvent;

use super::testing::{
    ControllerHarness, HookTestApp, action_model, controller_harness, conversation_status,
    create_task_with_messages, done_event, exchange_count, finish_turn_with_text, hook_test_app,
    init_project_action, let_background_work_settle, record_statuses, root_task_id,
    set_working_directory, shell_action, shell_tool_call_message, stream_init_event, text_message,
};
use super::{HookRuntime, RunIdentity, RunSurface};
use crate::ai::agent::conversation::{AIConversation, AIConversationId, ConversationStatus};
use crate::ai::agent::{AIAgentAction, AIAgentActionResultType, CancellationReason};
use crate::ai::agent_sdk::driver::hooks::{HookRunOptions, write_diagnostic, write_notice};
use crate::ai::agent_sdk::driver::terminal::TerminalDriver;
use crate::ai::agent_sdk::driver::{AgentDriver, SDKConversationOutputStatus};
use crate::ai::blocklist::{BlocklistAIActionModel, BlocklistAIHistoryModel};
use crate::ai::execution_profiles::ActionPermission;
use crate::ai::execution_profiles::profiles::AIExecutionProfilesModel;
use crate::features::FeatureFlag;
use crate::settings::AgentModeCommandExecutionPredicate;

const FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../crates/ai/src/hooks/fixtures"
);

/// The `PATH` every hook in these tests runs with. The fixture scripts need only POSIX tools.
const HOOK_PATH: &str = "/usr/bin:/bin";

/// How long a test waits for real hook processes. Generous, because process spawn can stall on
/// a loaded machine; a passing run takes milliseconds.
const PROCESS_WAIT: Duration = Duration::from_secs(30);

const SAMPLE_RM_REASON: &str = "This project's hook does not allow rm -rf. Remove files one at a time, or ask the user to do it.";

/// A temporary directory holding fixture copies, hook directories, and the `HOME` hooks run with.
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

    /// Copies `fixtures/<name>` to `<sandbox>/<as_name>` and makes its scripts executable.
    fn copy_fixture(&self, name: &str, as_name: &str) -> PathBuf {
        let root = self.base.join(as_name);
        copy_tree(&Path::new(FIXTURES).join(name), &root);
        root
    }

    fn dir(&self, name: &str) -> PathBuf {
        let path = self.base.join(name);
        fs::create_dir_all(&path).expect("mkdir");
        path
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
            if target
                .extension()
                .is_some_and(|extension| extension == "sh")
            {
                fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).expect("chmod");
            }
        }
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

/// Writes a project's own `.cursor/hooks.json` binding `.cursor/<event>.sh` for each of `events`.
fn write_project_hooks(root: &Path, events: &[(&str, &str)]) {
    let cursor = root.join(".cursor");
    fs::create_dir_all(&cursor).expect("mkdir");
    let mut hooks = serde_json::Map::new();
    for (event, body) in events {
        write_script(&cursor.join(format!("{event}.sh")), body);
        hooks.insert(
            (*event).to_owned(),
            json!([{ "command": format!(".cursor/{event}.sh") }]),
        );
    }
    fs::write(
        root.join(PROJECT_CURSOR_HOOKS_PATH),
        json!({ "version": 1, "hooks": hooks }).to_string(),
    )
    .expect("write project hooks.json");
}

/// Replaces the test doubles' runner with the real process runner.
fn install_real_runner(app: &mut App, home: &Path) {
    let home = home.to_path_buf();
    HookRuntime::handle(app).update(app, |runtime, _| {
        runtime.set_runner(Arc::new(
            CommandHookRunner::new(Arc::new(|| Some(OsString::from(HOOK_PATH))))
                .with_home(Some(home)),
        ));
    });
}

/// Records the process as a `warp agent run` from a command line, the surface where a project's
/// own hook file can activate: the execution mode a headless run has, and the run's identity.
fn record_command_line_run(app: &mut App) {
    AppExecutionMode::handle(app).update(app, |mode, ctx| {
        *mode = AppExecutionMode::new(ExecutionMode::Sdk, false, ctx);
    });
    HookRuntime::handle(app).update(app, |runtime, _| {
        runtime.set_run(Some(RunIdentity {
            metadata: RunMetadata {
                run_id: "e2e-run".to_owned(),
                parent_run_id: None,
                task_id: None,
            },
            surface: RunSurface::CommandLine,
        }));
    });
}

/// An empty root view, so a test can open the window the posture read resolves a team through.
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

/// Announces `root` the way terminal navigation detects a repository.
fn detect_root(app: &mut App, root: &Path) {
    let canonical = StandardizedPath::from_local_canonicalized(root).expect("canonical");
    app.update(|ctx| {
        let repository = DirectoryWatcher::handle(ctx)
            .update(ctx, |watcher, ctx| watcher.add_directory(canonical, ctx))
            .expect("watched directory");
        DetectedRepositories::handle(ctx).update(ctx, |_, ctx| {
            ctx.emit(DetectedRepositoriesEvent::DetectedGitRepo {
                repository,
                source: RepoDetectionSource::TerminalNavigation,
            });
        });
    });
}

fn load_hook_dir(app: &mut App, hooks_file: &Path) {
    let source = ExplicitHookSource {
        path: hooks_file.to_path_buf(),
        format: HookFormat::Cursor,
        scope: ExplicitSourceScope::Run,
    };
    let failures = app.update(|ctx| {
        let mut failures = Vec::new();
        HookRegistry::change(&HookRegistry::handle(ctx), ctx, |registry| {
            failures = load_explicit_sources(std::slice::from_ref(&source), registry);
            registry.revision()
        });
        failures
    });
    assert!(failures.is_empty(), "the hook file loads: {failures:?}");
}

fn pin(app: &mut App, terminal_view_id: EntityId) {
    HookRuntime::handle(app).update(app, |runtime, ctx| {
        runtime.pin_for_run(Some(terminal_view_id), ctx);
    });
    assert!(app.read(|ctx| HookRegistry::as_ref(ctx).pinned().is_some()));
}

/// A command-line run in a test app: the hook doubles replaced by the real runner, a window to
/// read the posture through, and the profile's execute-commands setting at `setting`.
fn command_line_run(app: &mut App, setting: ActionPermission) -> (Sandbox, HookTestApp) {
    let harness = hook_test_app(app);
    let sandbox = Sandbox::new();
    install_real_runner(app, &sandbox.home);
    record_command_line_run(app);
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

async fn wait_until_blocked(
    app: &mut App,
    model: &ModelHandle<BlocklistAIActionModel>,
    action: &AIAgentAction,
) {
    let id = action.id.clone();
    wait_for(app, &format!("{id} to wait for confirmation"), |app| {
        model.read(app, |model, _| {
            model
                .get_action_status(&id)
                .is_some_and(|status| status.is_blocked())
        })
    })
    .await;
}

fn hook_denied_reason(result: &AIAgentActionResultType) -> Option<&str> {
    match result {
        AIAgentActionResultType::RequestCommandOutput(RequestCommandOutputResult::HookDenied {
            reason,
            ..
        }) => Some(reason),
        _ => None,
    }
}

fn line_count(path: &Path) -> usize {
    fs::read_to_string(path).map_or(0, |text| text.lines().count())
}

/// The sample project, copied into the sandbox and detected as the run's repository.
fn sample_project(app: &mut App, sandbox: &Sandbox, harness: &HookTestApp) -> PathBuf {
    let root = sandbox.copy_fixture("sample_project", "sample");
    detect_root(app, &root);
    set_working_directory(app, harness, &root);
    root
}

/// §11 "permission matrix under run-to-completion": on a command-line run, a hook's deny holds
/// when the conversation auto-approves and when the user approves; a headless `ask` denies with
/// the approval-unavailable reason; and a denylisted command stays behind Warp's confirmation even
/// though the hook allowed it.
#[test]
fn permission_matrix_run_to_completion() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let (sandbox, harness) = command_line_run(&mut app, ActionPermission::AlwaysAllow);
        let root = sample_project(&mut app, &sandbox, &harness);
        let policy = sandbox.dir("run-policy");
        load_hook_dir(
            &mut app,
            &write_hook_dir(
                &policy,
                &[(
                    "beforeShellExecution",
                    r#"input=$(cat)
case "$input" in
  *'"command":"deploy'*)
    printf '{"permission":"ask","agent_message":"Deploys need a person to approve them."}' ;;
  *'"command":"release'*)
    : > "$PWD/release-started"
    i=0
    while [ ! -f "$PWD/release-go" ] && [ "$i" -lt 600 ]; do sleep 0.05; i=$((i + 1)); done
    printf '{"permission":"deny","agent_message":"Releases are frozen."}' ;;
  *) printf '{"permission":"allow"}' ;;
esac"#,
                    json!({}),
                )],
            ),
        );
        pin(&mut app, harness.terminal_view_id);
        assert_eq!(
            app.read(|ctx| HookRegistry::as_ref(ctx).snapshot())
                .bindings_for(HookBoundary::ToolPre, Some(&root), HookExecutionHost::Local)
                .len(),
            2,
            "the project file activated on the command line beside the run's policy"
        );
        let model = action_model(&mut app, &harness);
        app.update(|ctx| {
            AIExecutionProfilesModel::handle(ctx).update(ctx, |profiles, ctx| {
                let profile_id = profiles
                    .active_profile(Some(harness.terminal_view_id), ctx)
                    .id()
                    .clone();
                profiles.add_to_command_denylist(
                    &profile_id,
                    &AgentModeCommandExecutionPredicate::new_regex("curl .*").unwrap(),
                    ctx,
                );
            });
        });

        // A denylisted command: both hooks allow it, and the denylist still refuses it, as it does
        // in any headless run.
        let conversation = new_conversation(&mut app, &harness);
        let curl = shell_action("curl", "curl https://example.com");
        queue(&mut app, &model, conversation, vec![curl.clone()]);
        let result = finished_result(&mut app, &model, &curl).await;
        assert!(
            matches!(
                result,
                AIAgentActionResultType::RequestCommandOutput(
                    RequestCommandOutputResult::Denylisted { .. }
                )
            ),
            "{result:?}"
        );
        assert!(
            fs::read_to_string(root.join(".hook-last-shell-input.json"))
                .expect("the project hook ran")
                .contains("curl https://example.com"),
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

        // The conversation auto-approves everything: the project's deny still holds.
        let conversation = new_conversation(&mut app, &harness);
        app.update(|ctx| {
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                history.toggle_autoexecute_override(&conversation, harness.terminal_view_id, ctx);
            });
        });
        let rm = shell_action("rm", "rm -rf build");
        queue(&mut app, &model, conversation, vec![rm.clone()]);
        let result = finished_result(&mut app, &model, &rm).await;
        assert_eq!(hook_denied_reason(&result), Some(SAMPLE_RM_REASON));
    });
}

/// §11 "batch denial with cwd changes between actions": three shell commands in one batch; the
/// second moves the shell into a nested repository with its own hook file, and the third is
/// judged by that file.
#[test]
fn batch_denial_with_cwd_changes() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let (sandbox, harness) = command_line_run(&mut app, ActionPermission::AlwaysAllow);
        let outer = sample_project(&mut app, &sandbox, &harness);
        let nested = outer.join("nested");
        write_project_hooks(
            &nested,
            &[(
                "beforeShellExecution",
                r#"cat > /dev/null
printf '{"permission":"deny","agent_message":"The nested project allows no shell commands."}'"#,
            )],
        );
        detect_root(&mut app, &nested);
        pin(&mut app, harness.terminal_view_id);
        let model = action_model(&mut app, &harness);
        // The mock terminal has no shell type, so Warp cannot classify a command it would run on
        // its own, and a headless run refuses such a command outright. With the hooks pinned, the
        // process goes back to the interactive mode, where Warp asks instead, so the test can
        // approve the `cd` and have it start. The pinned project hooks stay active.
        AppExecutionMode::handle(&app).update(&mut app, |mode, ctx| {
            *mode = AppExecutionMode::new(ExecutionMode::App, false, ctx);
        });

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

        // The outer project allows `cd`, and once approved it runs. The mock terminal never reports
        // it finished, so the test moves the session the way the shell's prompt would and then
        // ends the command.
        wait_until_blocked(&mut app, &model, &enter).await;
        model.update(&mut app, |model, ctx| {
            model.execute_action(&enter.id, harness.conversation_id, ctx)
        });
        let enter_id = enter.id.clone();
        wait_for(&mut app, "the cd to start running", |app| {
            model.read(app, |model, _| {
                model
                    .get_action_status(&enter_id)
                    .is_some_and(|status| status.is_running())
            })
        })
        .await;
        set_working_directory(&mut app, &harness, &nested);
        model.update(&mut app, |model, ctx| {
            model.cancel_action_with_id(
                harness.conversation_id,
                &enter.id,
                CancellationReason::ManuallyCancelled,
                ctx,
            )
        });

        let result = finished_result(&mut app, &model, &list).await;
        assert_eq!(
            hook_denied_reason(&result),
            Some("The nested project allows no shell commands."),
            "the third command was judged by the nested project's file"
        );
        let last_outer_input = fs::read_to_string(outer.join(".hook-last-shell-input.json"))
            .expect("the outer hook ran");
        assert!(
            last_outer_input.contains(r#""command":"cd nested""#),
            "the outer hook never saw the third command: {last_outer_input}"
        );
    });
}

/// A driver started on `harness`'s terminal with `hook_dirs` as its run's hook directories, after
/// the hook sources load. Returns the driver, which must stay alive for the run to, and the run's
/// exit receiver.
fn start_driver(
    app: &mut App,
    harness: &ControllerHarness,
    working_dir: &Path,
    hook_dirs: Vec<PathBuf>,
) -> (
    ModelHandle<AgentDriver>,
    futures::channel::oneshot::Receiver<SDKConversationOutputStatus>,
) {
    let working_dir = working_dir.to_path_buf();
    let terminal = harness.terminal.clone();
    let driver = app.add_model(|ctx| {
        let terminal_driver = TerminalDriver::create_from_existing_view(terminal, ctx);
        AgentDriver::new_for_test(working_dir, terminal_driver, ctx)
    });
    let run_exit = driver
        .update(app, |driver, ctx| {
            driver.start_run_with_hooks_for_test(
                HookRunOptions {
                    hook_dirs,
                    ..Default::default()
                },
                ctx,
            )
        })
        .expect("the run's hook sources load");
    (driver, run_exit)
}

/// §11 "stop versus Oz idle exit", through the agent driver's own completion logic: a stop hook
/// that queues a follow-up never lets the run finish between the stop hook and the follow-up's
/// request.
#[test]
fn stop_versus_idle_exit() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = controller_harness(&mut app);
        let sandbox = Sandbox::new();
        install_real_runner(&mut app, &sandbox.home);
        record_command_line_run(&mut app);
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
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = controller_harness(&mut app);
        let sandbox = Sandbox::new();
        install_real_runner(&mut app, &sandbox.home);
        record_command_line_run(&mut app);
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

/// §11 "stale registry revision": the hook file changes while a pre-tool hook is deciding, so its
/// verdict is dropped and the hook runs once more against the new configuration. Covered twice:
/// through a run-delivered file loaded again before the run is pinned, which changes the active
/// hooks and moves the revision; and through the project file watcher after the run is pinned,
/// which leaves the active hooks and the revision as they were, so the verdict in flight stands
/// and the hook does not run again.
#[test]
fn stale_registry_revision() {
    const SLOW_DENY: &str = r#"cat > /dev/null
echo run >> "$WARP_PROJECT_DIR/runs.log"
runs=$(wc -l < "$WARP_PROJECT_DIR/runs.log" | tr -d ' ')
if [ "$runs" -eq 1 ]; then
  i=0
  while [ ! -f "$WARP_PROJECT_DIR/go" ] && [ "$i" -lt 600 ]; do sleep 0.05; i=$((i + 1)); done
fi
printf '{"permission":"deny","agent_message":"denied on run %s"}' "$runs""#;

    // A run-delivered file reloaded before the run is pinned.
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let (sandbox, harness) = command_line_run(&mut app, ActionPermission::AlwaysAllow);
        let work = sandbox.dir("work");
        set_working_directory(&mut app, &harness, &work);
        let policy = sandbox.dir("run-policy");
        let hooks_file = write_hook_dir(&policy, &[("beforeShellExecution", SLOW_DENY, json!({}))]);
        load_hook_dir(&mut app, &hooks_file);
        let model = action_model(&mut app, &harness);

        let action = shell_action("ls", "ls");
        queue(
            &mut app,
            &model,
            harness.conversation_id,
            vec![action.clone()],
        );
        let runs = work.join("runs.log");
        wait_for(&mut app, "the first hook run", |_| line_count(&runs) == 1).await;
        let before = app.read(|ctx| HookRegistry::as_ref(ctx).revision());
        fs::write(
            &hooks_file,
            json!({ "version": 1, "hooks": { "beforeShellExecution": [
                { "command": "./beforeShellExecution.sh", "timeout": 20 }
            ] } })
            .to_string(),
        )
        .expect("rewrite hooks.json");
        load_hook_dir(&mut app, &hooks_file);
        assert!(app.read(|ctx| HookRegistry::as_ref(ctx).revision()) > before);
        fs::write(work.join("go"), "").expect("let the first run answer");

        let result = finished_result(&mut app, &model, &action).await;
        assert_eq!(
            hook_denied_reason(&result),
            Some("denied on run 2"),
            "the first verdict was dropped and the hook ran again"
        );
        let_background_work_settle(&mut app).await;
        assert_eq!(line_count(&runs), 2, "exactly one more run");
    });

    // A project file the watcher reloads after the run is pinned.
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let (sandbox, harness) = command_line_run(&mut app, ActionPermission::AlwaysAllow);
        let root = sandbox.dir("project");
        write_project_hooks(&root, &[("beforeShellExecution", SLOW_DENY)]);
        detect_root(&mut app, &root);
        set_working_directory(&mut app, &harness, &root);
        pin(&mut app, harness.terminal_view_id);
        let model = action_model(&mut app, &harness);

        let action = shell_action("ls", "ls");
        queue(
            &mut app,
            &model,
            harness.conversation_id,
            vec![action.clone()],
        );
        let runs = root.join("runs.log");
        wait_for(&mut app, "the first hook run", |_| line_count(&runs) == 1).await;
        let before = app.read(|ctx| HookRegistry::as_ref(ctx).snapshot());
        let hooks_file = root.join(PROJECT_CURSOR_HOOKS_PATH);
        fs::write(&hooks_file, "{ \"version\": 1, \"hooks\": {} }").expect("rewrite");
        let watcher = app
            .read(|ctx| HookRuntime::as_ref(ctx).config_watcher_for_test())
            .expect("the detected root started the watcher");
        watcher.update(&mut app, |watcher, ctx| {
            watcher.handle_fs_event(
                &BulkFilesystemWatcherEvent {
                    modified: [hooks_file.clone()].into_iter().collect(),
                    ..Default::default()
                },
                ctx,
            )
        });
        let after = app.read(|ctx| HookRegistry::as_ref(ctx).snapshot());
        assert!(
            after.generation > before.generation,
            "the change was published"
        );
        assert_eq!(
            after.revision, before.revision,
            "the active hooks did not change, so the revision did not either"
        );
        assert!(
            after
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("after the run started")),
            "{:?}",
            after.diagnostics
        );
        fs::write(root.join("go"), "").expect("let the first run answer");

        let result = finished_result(&mut app, &model, &action).await;
        assert_eq!(
            hook_denied_reason(&result),
            Some("denied on run 1"),
            "the verdict in flight stands"
        );
        let_background_work_settle(&mut app).await;
        assert_eq!(line_count(&runs), 1, "the hook did not run again");
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
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = controller_harness(&mut app);
        let sandbox = Sandbox::new();
        install_real_runner(&mut app, &sandbox.home);
        let hooks = sandbox.dir("observers");
        let audit = r#"input=$(cat)
printf '%s\n' "$input" >> "$PWD/audit.log""#;
        load_hook_dir(
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
                super::testing::add_messages_event(
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

/// Which hook kinds are active for a shell command at the sample project's root after a
/// command-line run pins with the profile at `setting`, and the project file's diagnostics.
fn posture_case(setting: ActionPermission) -> (Vec<HookSourceKind>, Vec<String>) {
    let observed = Arc::new(Mutex::new(None));
    let result = observed.clone();
    App::test((), move |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let (sandbox, harness) = command_line_run(&mut app, setting);
        let root = sample_project(&mut app, &sandbox, &harness);
        load_hook_dir(&mut app, &root.join("run_hooks/hooks.json"));
        pin(&mut app, harness.terminal_view_id);
        let snapshot = app.read(|ctx| HookRegistry::as_ref(ctx).snapshot());
        let kinds = snapshot
            .bindings_for(HookBoundary::ToolPre, Some(&root), HookExecutionHost::Local)
            .iter()
            .map(|active| active.binding.id.source.kind)
            .collect();
        let project_file = root.join(PROJECT_CURSOR_HOOKS_PATH);
        let diagnostics = snapshot
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.source.to_string() == project_file.to_string_lossy())
            .map(|diagnostic| diagnostic.message.clone())
            .collect();
        *result.lock().unwrap() = Some((kinds, diagnostics));
    });
    let observed = observed.lock().unwrap().take();
    observed.expect("the case ran")
}

/// §11 "posture rule activation": on the command line a project's own hook file activates only
/// when the profile already runs commands without asking, and the diagnostic says why when it
/// does not; a hook directory delivered with the run activates either way.
#[test]
fn posture_rule_activation() {
    let (kinds, _) = posture_case(ActionPermission::AlwaysAllow);
    assert_eq!(
        kinds,
        vec![
            HookSourceKind::RunDelivered,
            HookSourceKind::ProjectCursorFile
        ]
    );

    let (kinds, diagnostics) = posture_case(ActionPermission::AlwaysAsk);
    assert_eq!(kinds, vec![HookSourceKind::RunDelivered]);
    assert!(
        diagnostics
            .iter()
            .any(|message| message.contains("loaded but not active")),
        "{diagnostics:?}"
    );
}

/// The project-file diagnostic `posture_case` reported, checked separately so a failure names
/// the reason text.
#[test]
fn an_inert_project_file_says_it_waits_for_a_profile_that_runs_commands() {
    let (_, diagnostics) = posture_case(ActionPermission::AgentDecides);
    let inert = diagnostics
        .iter()
        .find(|message| message.contains("loaded but not active"))
        .unwrap_or_else(|| panic!("no inert diagnostic in {diagnostics:?}"));
    assert!(inert.contains("approv"), "{inert}");
}

/// §11 "protected hook paths": a file edit that targets the project's hook file or one of its
/// scripts, or a run-delivered hook file, is refused with `ProtectedPath`, also in a sandboxed run.
#[test]
fn protected_hook_paths() {
    use crate::ai::blocklist::BlocklistAIPermissions;
    use crate::ai::blocklist::permissions::{FileWritePermission, FileWritePermissionDeniedReason};
    use crate::workspaces::user_workspaces::UserWorkspaces;

    for sandboxed in [false, true] {
        App::test((), move |mut app| async move {
            let _flag = FeatureFlag::AgentHooks.override_enabled(true);
            let (conversation_id, terminal_view_id) = permissions_app(&mut app, sandboxed);
            let sandbox = Sandbox::new();
            let root = sandbox.copy_fixture("sample_project", "sample");
            app.add_singleton_model(|_| HookRegistry::new());
            HookRegistry::handle(&app).update(&mut app, |registry, _| {
                let hooks_file = root.join(PROJECT_CURSOR_HOOKS_PATH);
                let bytes = fs::read(&hooks_file).expect("read");
                registry.upsert_source(ai::hooks::SourceUpdate {
                    source: ai::hooks::HookSourceId {
                        kind: HookSourceKind::ProjectCursorFile,
                        location: ai::hooks::HookSourceLocation::CanonicalPath(hooks_file),
                    },
                    format: HookFormat::Cursor,
                    bytes: &bytes,
                    project_root: Some(root.clone()),
                    trust: ai::hooks::TrustDecision::Activate,
                });
                load_explicit_sources(
                    &[ExplicitHookSource {
                        path: root.join("run_hooks/hooks.json"),
                        format: HookFormat::Cursor,
                        scope: ExplicitSourceScope::Run,
                    }],
                    registry,
                );
            });

            let write = |app: &App, path: PathBuf| {
                BlocklistAIPermissions::handle(app).read(app, |permissions, ctx| {
                    permissions.can_write_files(
                        &conversation_id,
                        &[path],
                        Some(terminal_view_id),
                        &UserWorkspaces::teamless_context_for_operation_for_test(),
                        ctx,
                    )
                })
            };
            for protected in [
                ".cursor/hooks.json",
                ".cursor/scripts/deny_dangerous.sh",
                ".cursor/scripts/audit.sh",
                "run_hooks/hooks.json",
            ] {
                let permission = write(&app, root.join(protected));
                assert!(
                    matches!(
                        permission,
                        FileWritePermission::Denied(FileWritePermissionDeniedReason::ProtectedPath)
                    ),
                    "{protected} (sandboxed: {sandboxed}): {permission:?}"
                );
            }
            let other = write(&app, root.join("README.md"));
            assert!(
                !matches!(
                    other,
                    FileWritePermission::Denied(FileWritePermissionDeniedReason::ProtectedPath)
                ),
                "an ordinary file is not protected (sandboxed: {sandboxed}): {other:?}"
            );
        });
    }
}

/// The singletons the permission checks read, in the desktop app or a sandboxed SDK run, with
/// one conversation. Returns the conversation and its terminal view.
fn permissions_app(app: &mut App, sandboxed: bool) -> (AIConversationId, EntityId) {
    use warp_core::execution_mode::ExecutionMode;

    use crate::ai::active_agent_views_model::ActiveAgentViewsModel;
    use crate::ai::blocklist::BlocklistAIPermissions;
    use crate::ai::mcp::templatable_manager::TemplatableMCPServerManager;
    use crate::auth::AuthStateProvider;
    use crate::cloud_object::model::persistence::CloudModel;
    use crate::network::NetworkStatus;
    use crate::server::cloud_objects::update_manager::UpdateManager;
    use crate::server::sync_queue::SyncQueue;
    use crate::settings::PrivacySettings;
    use crate::terminal::cli_agent_sessions::CLIAgentSessionsModel;
    use crate::test_util::settings::initialize_settings_for_tests_with_mode;
    use crate::workspaces::team_tester::TeamTesterStatus;
    use crate::workspaces::user_workspaces::UserWorkspaces;
    use crate::{
        AgentNotificationsModel, GlobalResourceHandles, GlobalResourceHandlesProvider, LaunchMode,
    };

    let mode = if sandboxed {
        ExecutionMode::Sdk
    } else {
        ExecutionMode::App
    };
    initialize_settings_for_tests_with_mode(app, mode, sandboxed);
    let global_resource_handles = GlobalResourceHandles::mock(app);
    app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resource_handles));
    let history = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], vec![], &[]));
    app.add_singleton_model(|_| CLIAgentSessionsModel::new());
    app.add_singleton_model(|_| ActiveAgentViewsModel::new());
    app.add_singleton_model(AgentNotificationsModel::new);
    app.add_singleton_model(BlocklistAIPermissions::new);
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(SyncQueue::mock);
    app.add_singleton_model(|_| NetworkStatus::new());
    app.add_singleton_model(TeamTesterStatus::mock);
    app.add_singleton_model(UpdateManager::mock);
    app.add_singleton_model(CloudModel::mock);
    app.add_singleton_model(|_| TemplatableMCPServerManager::default());
    let profiles = app.add_singleton_model(|ctx| {
        AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
    });
    app.add_singleton_model(PrivacySettings::mock);
    app.add_singleton_model(UserWorkspaces::default_mock);
    let terminal_view_id = EntityId::new();
    profiles.update(app, |profiles, ctx| {
        let profile_id = profiles
            .active_profile(Some(terminal_view_id), ctx)
            .id()
            .clone();
        if sandboxed {
            profiles.apply_cli_profile_defaults_for_test(&profile_id, true, ctx);
        }
        profiles.set_apply_code_diffs(&profile_id, &ActionPermission::AlwaysAllow, ctx);
    });
    let conversation_id = history.update(app, |history, ctx| {
        history.start_new_conversation(terminal_view_id, false, false, false, ctx)
    });
    (conversation_id, terminal_view_id)
}

/// §11 "deny reason reaches the wire": the project's shell denial carries the script's own text
/// into `permission_denied.hook_denied.reason`; a tool with no text slot denied by a `preToolUse`
/// hook finishes with its cancelled result and the reason reaches only a `Denied` notice (the
/// recorded gap in §9.3).
#[test]
fn deny_reason_reaches_the_wire() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let (sandbox, harness) = command_line_run(&mut app, ActionPermission::AlwaysAllow);
        sample_project(&mut app, &sandbox, &harness);
        load_hook_dir(
            &mut app,
            &write_hook_dir(
                &sandbox.dir("tool-policy"),
                &[(
                    "preToolUse",
                    r#"cat > /dev/null
printf '{"permission":"deny","agent_message":"Project setup is handled by the platform team."}'"#,
                    json!({ "matcher": "InitProject" }),
                )],
            ),
        );
        pin(&mut app, harness.terminal_view_id);
        let model = action_model(&mut app, &harness);

        let rm = shell_action("rm", "rm -rf build");
        queue(&mut app, &model, harness.conversation_id, vec![rm.clone()]);
        let result = finished_result(&mut app, &model, &rm).await;
        let AIAgentActionResultType::RequestCommandOutput(shell_result) = result else {
            panic!("a shell result: {result:?}");
        };
        let wire = api::request::input::tool_call_result::Result::try_from(shell_result)
            .expect("a hook denial reaches the wire");
        let api::request::input::tool_call_result::Result::RunShellCommand(shell) = wire else {
            panic!("a shell result on the wire");
        };
        let Some(api::run_shell_command_result::Result::PermissionDenied(denied)) = shell.result
        else {
            panic!("a permission denial on the wire");
        };
        assert_eq!(
            denied.reason,
            Some(api::permission_denied::Reason::HookDenied(
                api::permission_denied::HookDenied {
                    reason: SAMPLE_RM_REASON.to_owned(),
                }
            ))
        );

        let conversation = new_conversation(&mut app, &harness);
        let init = init_project_action("init");
        queue(&mut app, &model, conversation, vec![init.clone()]);
        let result = finished_result(&mut app, &model, &init).await;
        assert_eq!(result, init.action.cancelled_result());
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
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = controller_harness(&mut app);
        let sandbox = Sandbox::new();
        install_real_runner(&mut app, &sandbox.home);
        record_command_line_run(&mut app);
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
