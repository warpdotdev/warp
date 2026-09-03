use std::path::PathBuf;
use std::time::Duration;

use warp_core::SessionId;
use warpui::App;

use super::{
    BUILD_SILENCE_THRESHOLD, DevContainerBuildOperation, DevContainerBuildPhase,
    DevContainerBuildStatus, silence_subtitle, silence_watch_delay,
};
use crate::terminal::view::dev_container::registry::DevContainerBuildKey;

fn key() -> DevContainerBuildKey {
    DevContainerBuildKey {
        workspace_folder: PathBuf::from("/tmp/ws"),
        config_file: PathBuf::from("/tmp/ws/.devcontainer/devcontainer.json"),
    }
}

#[test]
fn new_operation_starts_in_build_running() {
    let op = DevContainerBuildOperation::new(key());
    assert_eq!(op.phase(), DevContainerBuildPhase::Build);
    assert_eq!(op.status(), DevContainerBuildStatus::Running);
    assert_eq!(op.attempt_id(), 1);
}

#[test]
fn tombstone_rejects_late_completions_for_the_same_attempt() {
    App::test((), |mut app| async move {
        let op = app.add_model(|_| DevContainerBuildOperation::new(key()));
        let (operation_id, attempt_id) =
            op.read(&app, |op, _ctx| (op.operation_id(), op.attempt_id()));
        op.update(&mut app, |op, ctx| {
            op.tombstone(ctx);
        });
        op.read(&app, |op, _ctx| {
            assert!(!op.is_current_attempt(operation_id, attempt_id));
        });
    });
}

#[test]
fn retry_clears_remote_server_session_id() {
    App::test((), |mut app| async move {
        let op = app.add_model(|_| DevContainerBuildOperation::new(key()));
        op.update(&mut app, |op, ctx| {
            op.set_remote_server_session_id(Some(SessionId::from(9)), ctx);
            op.fail(DevContainerBuildPhase::Staging, ctx);
            op.begin_retry(ctx);
        });
        op.read(&app, |op, _ctx| {
            assert_eq!(op.remote_server_session_id(), None);
        });
    });
}

#[test]
fn retry_increments_attempt_and_resets_running() {
    App::test((), |mut app| async move {
        let op = app.add_model(|_| DevContainerBuildOperation::new(key()));
        let (first_attempt, first_id) =
            op.read(&app, |op, _ctx| (op.attempt_id(), op.operation_id()));
        op.update(&mut app, |op, ctx| {
            op.fail(DevContainerBuildPhase::Preflight, ctx);
            op.begin_retry(ctx);
        });
        op.read(&app, |op, _ctx| {
            assert_eq!(op.operation_id(), first_id);
            assert_eq!(op.attempt_id(), first_attempt + 1);
            assert_eq!(op.phase(), DevContainerBuildPhase::Build);
            assert_eq!(op.status(), DevContainerBuildStatus::Running);
            assert!(op.is_current_attempt(first_id, first_attempt + 1));
            assert!(!op.is_current_attempt(first_id, first_attempt));
        });
    });
}

#[test]
fn failed_setup_retains_retry_and_close() {
    App::test((), |mut app| async move {
        let op = app.add_model(|_| DevContainerBuildOperation::new(key()));
        op.update(&mut app, |op, ctx| {
            op.fail(DevContainerBuildPhase::Staging, ctx);
        });
        op.read(&app, |op, _ctx| {
            assert_eq!(op.status(), DevContainerBuildStatus::Failed);
            assert_eq!(op.phase(), DevContainerBuildPhase::Staging);
            assert!(op.shows_retry());
            assert!(op.shows_close());
        });
    });
}

#[test]
fn silence_subtitle_is_none_below_threshold() {
    assert_eq!(silence_subtitle(Duration::from_secs(0)), None);
    assert_eq!(
        silence_subtitle(BUILD_SILENCE_THRESHOLD - Duration::from_secs(1)),
        None
    );
}

#[test]
fn silence_subtitle_names_elapsed_minutes_at_threshold() {
    assert_eq!(
        silence_subtitle(BUILD_SILENCE_THRESHOLD).as_deref(),
        Some("No output for 2m")
    );
    assert_eq!(
        silence_subtitle(Duration::from_secs(180)).as_deref(),
        Some("No output for 3m")
    );
}

#[test]
fn running_build_shows_close_without_retry() {
    let op = DevContainerBuildOperation::new(key());
    assert!(op.shows_close());
    assert!(!op.shows_retry());
    assert_eq!(op.header_secondary(), "");
}

#[test]
fn failed_build_clears_silence_subtitle() {
    App::test((), |mut app| async move {
        let op = app.add_model(|_| DevContainerBuildOperation::new(key()));
        op.update(&mut app, |op, ctx| {
            op.fail(DevContainerBuildPhase::Build, ctx);
        });
        op.read(&app, |op, _ctx| {
            assert!(op.shows_retry());
            assert!(op.shows_close());
            assert_eq!(op.header_secondary(), "");
        });
    });
}

#[test]
fn output_shortly_before_initial_wake_defers_subtitle_to_last_output_plus_threshold() {
    let first_wake = BUILD_SILENCE_THRESHOLD;
    let output_at = first_wake - Duration::from_secs(10);
    let elapsed_at_first_wake = first_wake - output_at;
    assert_eq!(elapsed_at_first_wake, Duration::from_secs(10));
    assert_eq!(silence_subtitle(elapsed_at_first_wake), None);
    assert_eq!(
        silence_watch_delay(elapsed_at_first_wake),
        Duration::from_secs(110)
    );
    assert_eq!(
        first_wake + silence_watch_delay(elapsed_at_first_wake),
        output_at + BUILD_SILENCE_THRESHOLD
    );
}

#[test]
fn output_shortly_after_threshold_clears_subtitle_and_rearms_remaining() {
    assert_eq!(
        silence_subtitle(BUILD_SILENCE_THRESHOLD).as_deref(),
        Some("No output for 2m")
    );
    let elapsed_after_fresh_output = Duration::from_secs(5);
    assert_eq!(silence_subtitle(elapsed_after_fresh_output), None);
    assert_eq!(
        silence_watch_delay(elapsed_after_fresh_output),
        Duration::from_secs(115)
    );
}

#[test]
fn output_wake_channel_coalesces_to_one_pending_signal() {
    let op = DevContainerBuildOperation::new(key());
    let tx = op.output_tx();
    let rx = op.output_rx();
    while rx.try_recv().is_ok() {}
    for _ in 0..32 {
        let _ = tx.try_send(());
    }
    assert!(rx.try_recv().is_ok());
    assert!(rx.try_recv().is_err());
}
