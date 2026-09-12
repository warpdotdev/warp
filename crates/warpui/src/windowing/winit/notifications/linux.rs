use std::io;
use std::path::PathBuf;

use futures::FutureExt;
use winit::event_loop::EventLoopProxy;

use crate::WindowId;
use crate::notification::NotificationSendError;
use crate::windowing::winit::app::CustomEvent;
use crate::windowing::winit::notifications::NotificationInfo;

fn notify_rust_exe_name(current_exe: io::Result<PathBuf>) -> Result<String, String> {
    let path = current_exe.map_err(|err| err.to_string())?;
    path.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
        .ok_or_else(|| "current executable path is missing a UTF-8 file name".to_string())
}

pub(super) async fn send_notification(
    notification_info: NotificationInfo,
    _window_id: WindowId,
    proxy: EventLoopProxy<CustomEvent>,
) {
    let NotificationInfo {
        notification_content,
        on_error,
    } = notification_info;

    // notify-rust's Notification::new() unwraps std::env::current_exe(), which panics when
    // /proc/self/exe is unavailable. Preflight the same lookup so we can fail through on_error.
    if let Err(error_message) = notify_rust_exe_name(std::env::current_exe()) {
        let error = NotificationSendError::Other { error_message };
        let _ = proxy.send_event(CustomEvent::UpdateUIApp(Box::new(|ctx| {
            on_error(error, ctx);
        })));
        return;
    }

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

#[cfg(test)]
#[path = "linux_tests.rs"]
mod tests;
