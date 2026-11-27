use super::*;

fn app_summary(sessions_keep_running: bool) -> UnsavedStateSummary<'static> {
    UnsavedStateSummary {
        scope: QuitScope::App,
        total_long_running_commands: 2,
        windows_with_long_running_commands: 1,
        tabs_with_long_running_commands: 1,
        terminal_sessions: Vec::new(),
        shared_sessions: 0,
        unsaved_code_changes: false,
        sessions_keep_running,
    }
}

#[test]
fn warning_mentions_background_only_for_persistent_sessions() {
    assert!(
        app_summary(true)
            .warning_text()
            .contains("They will keep running in the background.")
    );
    assert!(
        !app_summary(false)
            .warning_text()
            .contains("keep running in the background")
    );
}

#[test]
fn sessions_without_running_commands_do_not_keep_running() {
    let summary = RunningSessionSummary::new(&[]);

    assert!(!sessions_keep_running_after_quit(
        &QuitScope::App,
        true,
        &summary
    ));
}
