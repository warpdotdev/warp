use futures::FutureExt;
use notify_rust::{Hint, Notification};
use winit::event_loop::EventLoopProxy;

use crate::WindowId;
use crate::notification::{NotificationSendError, UserNotification};
use crate::windowing::winit::app::CustomEvent;
use crate::windowing::winit::notifications::NotificationInfo;

pub(super) async fn send_notification(
    notification_info: NotificationInfo,
    _window_id: WindowId,
    desktop_entry: Option<String>,
    proxy: EventLoopProxy<CustomEvent>,
) {
    let NotificationInfo {
        notification_content,
        on_error,
    } = notification_info;

    build_notification(&notification_content, desktop_entry.as_deref())
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

/// Name of the freedesktop sound theme sound requested when a notification should play a sound.
const NOTIFICATION_SOUND_NAME: &str = "message-new-instant";

/// Builds the D-Bus notification for `content`, attributing it to `desktop_entry` (the app's
/// desktop file ID without the `.desktop` suffix) when one is known.
fn build_notification(content: &UserNotification, desktop_entry: Option<&str>) -> Notification {
    let mut notification = Notification::new();
    notification.summary(content.title()).body(content.body());

    if let Some(desktop_entry) = desktop_entry {
        notification.hint(Hint::DesktopEntry(desktop_entry.to_owned()));
    }

    if content.play_sound() {
        notification.sound_name(NOTIFICATION_SOUND_NAME);
    } else {
        notification.hint(Hint::SuppressSound(true));
    }

    notification
}

#[cfg(test)]
#[path = "linux_tests.rs"]
mod tests;
