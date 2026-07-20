use futures::FutureExt;
use winit::event_loop::EventLoopProxy;

use crate::WindowId;
use crate::notification::NotificationSendError;
use crate::windowing::winit::app::CustomEvent;
use crate::windowing::winit::notifications::NotificationInfo;

pub(super) async fn send_notification(
    notification_info: NotificationInfo,
    _window_id: WindowId,
    proxy: EventLoopProxy<CustomEvent>,
) {
    let NotificationInfo {
        notification_content,
        on_error,
    } = notification_info;

    let mut notification = notify_rust::Notification::new();
    notification
        .summary(notification_content.title())
        .body(notification_content.body());

    notification
        .show_async()
        .then(|handle| async move {
            match handle {
                Ok(handle) => {
                    // The call to on_close blocks until the notification is closed, so make the blocking
                    // call on its own thread in the `blocking` crate threadpool to avoid starving the shared
                    // background executor.
                    blocking::unblock(move || {
                        // Without the on_close handler, the notification will fail to appear.
                        handle.on_close(|reason| log::info!("Notification closed via {reason:?}"))
                    })
                    .await;
                }
                Err(err) => {
                    // Always consider the error to be a `NotificationSendError::Other`.
                    // Dbus does not report if a notification couldn't be shown because
                    // the application didn't have permissions, so we can never return a
                    // `NotificationSendError::PermissionDenied` error.
                    let error = NotificationSendError::Other {
                        error_message: err.to_string(),
                    };

                    let _ = proxy.send_event(CustomEvent::UpdateUIApp(Box::new(|ctx| {
                        on_error(error, ctx);
                    })));
                }
            }
        })
        .await
}
