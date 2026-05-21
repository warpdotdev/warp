//! Module to display system desktop notifications through the winit windowing backend.

use crate::platform::NotificationInfo;
use crate::platform::{RequestNotificationPermissionsCallback, SendNotificationErrorCallback};
use crate::windowing::winit::app::CustomEvent;
use crate::{notification, WindowId};
use winit::event_loop::EventLoopProxy;

#[cfg_attr(any(target_os = "linux", target_os = "freebsd"), path = "linux.rs")]
#[cfg_attr(target_os = "windows", path = "windows.rs")]
#[cfg_attr(target_family = "wasm", path = "wasm.rs")]
mod imp;

#[cfg(target_family = "wasm")]
pub(super) use imp::request_notification_permissions;

pub async fn send_notification(
    notification_info: NotificationInfo,
    window_id: WindowId,
    event_loop_proxy: EventLoopProxy<CustomEvent>,
) {
    imp::send_notification(notification_info, window_id, event_loop_proxy).await
}

pub(super) fn request_desktop_notification_permissions(
    on_completion: RequestNotificationPermissionsCallback,
    event_loop_proxy: &EventLoopProxy<CustomEvent>,
) {
    let _ = event_loop_proxy.send_event(CustomEvent::RequestNotificationPermissions(Box::new(
        |outcome, ctx| on_completion(outcome, ctx),
    )));
}

pub(super) fn send_desktop_notification(
    notification_content: notification::UserNotification,
    window_id: WindowId,
    on_error: SendNotificationErrorCallback,
    event_loop_proxy: &EventLoopProxy<CustomEvent>,
) {
    use crate::platform::NotificationInfo;

    let _ = event_loop_proxy.send_event(CustomEvent::SendNotification {
        window_id,
        notification_info: NotificationInfo {
            notification_content,
            on_error,
        },
    });
}
