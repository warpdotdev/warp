use std::cell::RefCell;
use std::rc::Rc;

use session_sharing_protocol::sharer::SessionRetentionReason;
use warpui::App;

use super::TerminalDriver;
use crate::terminal::shared_session::SharedSessionStatus;
use crate::terminal::view::Event;
use crate::test_util::add_window_with_terminal;
use crate::test_util::terminal::initialize_app_for_terminal_view;

#[test]
fn extend_shared_session_retention_emits_event_for_active_sharer() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal_view = add_window_with_terminal(&mut app, None);
        let terminal_driver =
            app.update(|ctx| TerminalDriver::create_from_existing_view(terminal_view.clone(), ctx));
        let emitted_reasons = Rc::new(RefCell::new(Vec::new()));

        app.update(|ctx| {
            let emitted_reasons = emitted_reasons.clone();
            ctx.subscribe_to_view(&terminal_view, move |_, event, _| {
                if let Event::ExtendSessionRetention { reason } = event {
                    emitted_reasons.borrow_mut().push(*reason);
                }
            });
        });

        terminal_driver.update(&mut app, |driver, ctx| {
            driver.extend_shared_session_retention(SessionRetentionReason::SetupFailed, ctx);
        });

        assert!(
            emitted_reasons.borrow().is_empty(),
            "retention should not be extended before session sharing is active"
        );

        terminal_view.update(&mut app, |view, _| {
            view.model
                .lock()
                .set_shared_session_status(SharedSessionStatus::ActiveSharer);
        });

        terminal_driver.update(&mut app, |driver, ctx| {
            driver.extend_shared_session_retention(SessionRetentionReason::SetupFailed, ctx);
        });

        let emitted_reasons = emitted_reasons.borrow();
        assert_eq!(emitted_reasons.len(), 1);
        assert!(matches!(
            emitted_reasons[0],
            SessionRetentionReason::SetupFailed
        ));
    });
}
