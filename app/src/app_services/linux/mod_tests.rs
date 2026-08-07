use warpui::App;

use super::*;

#[test]
fn skips_dbus_application_service_when_not_desktop_owner() {
    App::test((), |mut app| async move {
        app.update(|ctx| init(false, ctx));

        assert!(
            !app.read(|ctx| ctx.has_singleton_model::<DBusServiceHost>()),
            "a launch mode that cannot present the desktop GUI must not take the desktop D-Bus application name"
        );

        // Teardown runs for every launch mode, including those that never
        // registered the service.
        app.update(teardown);
    });
}

#[test]
fn registers_dbus_application_service_for_the_desktop_owner() {
    App::test((), |mut app| async move {
        app.update(|ctx| init(true, ctx));

        assert!(
            app.read(|ctx| ctx.has_singleton_model::<DBusServiceHost>()),
            "the primary desktop instance must host the D-Bus application service"
        );

        // The service task is left to be cancelled when the test app drops:
        // `teardown` blocks on the background task, which is not something to
        // do from inside the test executor. Bus-less runners are fine either
        // way, since the connection failure only logs.
    });
}
