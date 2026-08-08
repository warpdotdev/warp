use std::time::Duration;

use warpui::App;
use warpui::r#async::Timer;

use super::{REFRESHING_CREDENTIALS_MESSAGE, resolve_default_warping_text};
use crate::ai::blocklist::ResponseStreamId;
use crate::ai::blocklist::block::view_impl::common::LOAD_OUTPUT_MESSAGE;
use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};

/// The status bar waits before showing the refresh text, cancels a pending delay
/// when the refresh ends, and clears already-visible text immediately.
#[test]
fn credential_refresh_display_delays_and_clears() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let (controller, status_bar) = terminal.read(&app, |view, ctx| {
            let status_bar = view
                .input()
                .read(ctx, |input, _| input.agent_status_bar().clone());
            (view.ai_controller().clone(), status_bar)
        });
        let stream_id = ResponseStreamId::new_for_test();

        controller.update(&mut app, |controller, ctx| {
            controller.set_credential_refresh_waiting_for_test(stream_id.clone(), true, ctx);
        });
        status_bar.update(&mut app, |status_bar, ctx| {
            status_bar.latest_response_stream_id = Some(stream_id.clone());
            status_bar.update_credential_refresh_display(ctx);
            assert!(!status_bar.credential_refresh_text_visible);
            assert!(status_bar.credential_refresh_delay_handle.is_some());
        });

        // A fast refresh must end before the delayed text becomes visible.
        Timer::after(Duration::from_millis(100)).await;
        controller.update(&mut app, |controller, ctx| {
            controller.set_credential_refresh_waiting_for_test(stream_id.clone(), false, ctx);
        });
        status_bar.update(&mut app, |status_bar, ctx| {
            status_bar.update_credential_refresh_display(ctx);
            assert!(!status_bar.credential_refresh_text_visible);
            assert!(status_bar.credential_refresh_delay_handle.is_none());
        });
        Timer::after(Duration::from_millis(250)).await;
        status_bar.read(&app, |status_bar, _| {
            assert!(!status_bar.credential_refresh_text_visible);
        });

        // A refresh that outlives the delay must show the alternate text.
        controller.update(&mut app, |controller, ctx| {
            controller.set_credential_refresh_waiting_for_test(stream_id.clone(), true, ctx);
        });
        status_bar.update(&mut app, |status_bar, ctx| {
            status_bar.update_credential_refresh_display(ctx);
        });
        Timer::after(Duration::from_millis(350)).await;
        status_bar.read(&app, |status_bar, _| {
            assert!(status_bar.credential_refresh_text_visible);
            assert!(status_bar.credential_refresh_delay_handle.is_none());
        });

        // Completion, timeout/failure, and cancellation all clear controller
        // waiting state and must therefore hide the text synchronously.
        controller.update(&mut app, |controller, ctx| {
            controller.set_credential_refresh_waiting_for_test(stream_id.clone(), false, ctx);
        });
        status_bar.update(&mut app, |status_bar, ctx| {
            status_bar.update_credential_refresh_display(ctx);
            assert!(!status_bar.credential_refresh_text_visible);
            assert!(status_bar.credential_refresh_delay_handle.is_none());
        });
    });
}

/// When the 300 ms delay has elapsed during a request-blocking GEAP refresh,
/// the status bar must show "Refreshing Gemini Enterprise credentials...".
#[test]
fn blocked_credential_refresh_uses_refreshing_text() {
    assert_eq!(
        resolve_default_warping_text(true, None),
        REFRESHING_CREDENTIALS_MESSAGE
    );
}

/// A background refresh (credential_refresh_text_visible = false) must keep
/// the default "Warping..." text so it is never shown for proactive refreshes.
#[test]
fn non_blocking_credential_refresh_keeps_default_warping_text() {
    assert_eq!(
        resolve_default_warping_text(false, None),
        LOAD_OUTPUT_MESSAGE
    );
}

/// The credential-refresh text must take precedence over the fallback-model
/// text ("Warping with Claude…") when both could apply simultaneously.
#[test]
fn credential_refresh_text_takes_precedence_over_fallback_text() {
    assert_eq!(
        resolve_default_warping_text(true, Some("Warping with another model.")),
        REFRESHING_CREDENTIALS_MESSAGE
    );
}
