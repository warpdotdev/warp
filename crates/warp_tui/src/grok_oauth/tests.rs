use uuid::Uuid;
use warpui_core::App;

use super::{
    CALLBACK_FAILURE_MESSAGE, MANUAL_FAILURE_MESSAGE, TuiGrokOAuthController, TuiGrokOAuthPhase,
};

fn test_controller() -> TuiGrokOAuthController {
    TuiGrokOAuthController {
        active_attempt_id: Some(Uuid::new_v4()),
        manual_exchange: None,
        cancellation: None,
        phase: TuiGrokOAuthPhase::Waiting { manual_error: None },
        callback_error: None,
    }
}

#[test]
fn callback_and_manual_failures_are_sanitized() {
    App::test((), |mut app| async move {
        let controller = app.add_model(|_| test_controller());
        controller.update(&mut app, |controller, ctx| {
            let attempt_id = controller.active_attempt_id.unwrap();
            controller.phase = TuiGrokOAuthPhase::ExchangingManualCode;
            controller.handle_callback_result(
                attempt_id,
                Err(anyhow::anyhow!("raw-callback-secret")),
                ctx,
            );
            assert_eq!(
                controller.callback_error.as_deref(),
                Some(CALLBACK_FAILURE_MESSAGE)
            );
            assert!(controller.is_exchanging());

            controller.handle_manual_result(
                attempt_id,
                Err(anyhow::anyhow!("raw-manual-secret")),
                ctx,
            );
            assert_eq!(controller.error(), Some(CALLBACK_FAILURE_MESSAGE));

            controller.phase = TuiGrokOAuthPhase::Waiting { manual_error: None };
            controller.callback_error = None;
            controller.handle_manual_result(
                attempt_id,
                Err(anyhow::anyhow!("another-raw-secret")),
                ctx,
            );
            assert_eq!(controller.error(), Some(MANUAL_FAILURE_MESSAGE));
        });
    });
}

#[test]
fn cancellation_ignores_stale_callback_results() {
    App::test((), |mut app| async move {
        let controller = app.add_model(|_| test_controller());
        controller.update(&mut app, |controller, ctx| {
            let attempt_id = controller.active_attempt_id.unwrap();
            controller.cancel(ctx);
            controller.handle_callback_result(
                attempt_id,
                Err(anyhow::anyhow!("stale-raw-secret")),
                ctx,
            );
            assert!(!controller.is_active());
            assert!(controller.callback_error.is_none());
            assert!(controller.error().is_none());
        });
    });
}
