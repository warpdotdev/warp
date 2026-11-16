use super::*;
use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};

fn idle_terminal(app: &mut warpui::App) -> ViewHandle<TerminalView> {
    initialize_app_for_terminal_view(app);
    let terminal = add_window_with_terminal(app, None);
    terminal.update(app, |view, _| {
        view.is_login_shell_bootstrapped = true;
        *view.model.lock() = TerminalModel::mock(None, None);
    });
    terminal
}

#[test]
fn planned_replacement_completes_without_sharing_or_an_ai_action() {
    warpui::App::test((), |mut app| async move {
        let _respawn_disabled = FeatureFlag::CloudAgentShellRespawn.override_enabled(false);
        let terminal = idle_terminal(&mut app);
        let original_model = terminal.read(&app, |view, _| view.model.clone());
        let mut receiver = terminal.update(&mut app, |view, ctx| {
            view.replace_shell("/run".into(), HashMap::new(), ctx)
        });
        assert!(receiver.try_recv().unwrap().is_none());
        terminal.update(&mut app, |view, ctx| {
            let id = view.pending_shell_replacement.as_ref().unwrap().0;
            assert!(view.pending_cloud_shell_recovery.is_none());
            assert!(!view.model.lock().is_shared_ambient_agent_session());
            view.complete_shell_replacement(id, Ok(()), ctx);
        });
        receiver.await.unwrap().unwrap();
        terminal.read(&app, |view, _| {
            assert!(Arc::ptr_eq(&original_model, &view.model));
        });
        assert!(!original_model.lock().is_read_only());
    });
}

#[test]
fn planned_replacement_rejects_a_running_command() {
    warpui::App::test((), |mut app| async move {
        let terminal = idle_terminal(&mut app);
        let receiver = terminal.update(&mut app, |view, ctx| {
            view.model
                .lock()
                .simulate_long_running_block("sleep 10", "");
            view.replace_shell("/run".into(), HashMap::new(), ctx)
        });
        let error = receiver.await.unwrap().unwrap_err().to_string();
        assert_eq!(error, "cannot replace the shell: a command is running");
        terminal.read(&app, |view, _| {
            assert!(view.pending_shell_replacement.is_none());
        });
    });
}

#[test]
fn stale_completion_does_not_finish_a_second_replacement() {
    warpui::App::test((), |mut app| async move {
        let terminal = idle_terminal(&mut app);
        let (first, first_id) = terminal.update(&mut app, |view, ctx| {
            let receiver = view.replace_shell("/run".into(), HashMap::new(), ctx);
            let id = view.pending_shell_replacement.as_ref().unwrap().0;
            view.complete_shell_replacement(id, Ok(()), ctx);
            (receiver, id)
        });
        first.await.unwrap().unwrap();
        let mut second = terminal.update(&mut app, |view, ctx| {
            let receiver = view.replace_shell("/run".into(), HashMap::new(), ctx);
            view.complete_shell_replacement(first_id, Err(anyhow::anyhow!("stale timeout")), ctx);
            receiver
        });
        assert!(second.try_recv().unwrap().is_none());
        terminal.update(&mut app, |view, ctx| {
            let id = view.pending_shell_replacement.as_ref().unwrap().0;
            assert_ne!(id, first_id);
            view.complete_shell_replacement(id, Ok(()), ctx);
        });
        second.await.unwrap().unwrap();
    });
}

#[test]
fn replacement_exit_returns_error_without_recovery_guidance() {
    warpui::App::test((), |mut app| async move {
        let terminal = idle_terminal(&mut app);
        let receiver = terminal.update(&mut app, |view, ctx| {
            let receiver = view.replace_shell("/run".into(), HashMap::new(), ctx);
            view.model.lock().prepare_shell_replacement();
            view.handle_terminal_event(
                &ModelEvent::ShellExitObserved {
                    status: ObservedExitStatus::Code(1),
                },
                ctx,
            );
            assert!(view.pending_cloud_shell_recovery.is_none());
            assert!(view.model.lock().is_read_only());
            receiver
        });
        let error = receiver.await.unwrap().unwrap_err().to_string();
        assert_eq!(
            error,
            "shell exited during planned replacement (exit code 1)"
        );
    });
}
