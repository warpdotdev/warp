use std::time::Duration;

use warp_core::ui::appearance::Appearance;
use warpui::platform::WindowStyle;
use warpui::{App, View};

use super::{DismissibleToast, DismissibleToastStack, ToastLink};

#[derive(Debug, Clone, PartialEq)]
struct TestAction;

/// A toast carrying more than one link (e.g. the download-success toast's
/// `View in Finder | New Session` links) should lay out without panicking.
#[test]
fn test_multi_link_toast_can_layout() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let (_, view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            DismissibleToastStack::<TestAction>::new(Duration::from_secs(5))
        });

        // An empty stack renders nothing, but should not panic.
        view.read(&app, |view, ctx| view.render(ctx));

        view.update(&mut app, |view, ctx| {
            let toast = DismissibleToast::success("file.csv was downloaded to /tmp.".to_string())
                .with_link(
                    ToastLink::new("View in Finder".to_string()).with_onclick_action(TestAction),
                )
                .with_link(
                    ToastLink::new("New Session".to_string()).with_onclick_action(TestAction),
                );
            view.add_persistent_toast(toast, ctx);
        });

        // Rendering a toast with two links plus a separator should not panic.
        view.read(&app, |view, ctx| view.render(ctx));
    })
}
