use wasm_bindgen_futures::JsFuture;
use winit::event_loop::EventLoopProxy;

use crate::WindowId;
use crate::notification::{NotificationSendError, RequestPermissionsOutcome};
use crate::platform::NotificationInfo;
use crate::windowing::winit::CustomEvent;
use crate::windowing::winit::app::RequestPermissionsCallback;

pub async fn send_notification(
    notification_info: NotificationInfo,
    _window_id: WindowId,
    proxy: EventLoopProxy<CustomEvent>,
) {
    let NotificationInfo {
        notification_content,
        on_error,
    } = notification_info;

    // First, we check to see if the page has permissions to send notifications. If not, we should prematurely
    // execute the on_error callback. We can't rely on the result of the web_sys::Notification constructor to
    // know whether the notification has the right permissions to actually send.
    // https://developer.mozilla.org/en-US/docs/Web/API/Notification/Notification#return_value.
    match web_sys::Notification::permission() {
        web_sys::NotificationPermission::Granted => {
            // If permissions are granted, send it! Constructing the Notification object is enough to launch it.
            let _ = web_sys::Notification::new(notification_content.title());
        }
        web_sys::NotificationPermission::Default => {
            let _ = proxy.send_event(CustomEvent::UpdateUIApp(Box::new(|ctx| {
                on_error(NotificationSendError::PermissionsNotYetGranted, ctx)
            })));
        }
        web_sys::NotificationPermission::Denied => {
            let _ = proxy.send_event(CustomEvent::UpdateUIApp(Box::new(|ctx| {
                on_error(NotificationSendError::PermissionsDenied, ctx)
            })));
        }
        _ => {
            let _ = proxy.send_event(CustomEvent::UpdateUIApp(Box::new(|ctx| {
                on_error(
                    NotificationSendError::Other {
                        error_message: "unknown notifications permissions".to_string(),
                    },
                    ctx,
                )
            })));
        }
    }
}

pub async fn request_notification_permissions(
    callback: RequestPermissionsCallback,
    proxy: EventLoopProxy<CustomEvent>,
) {
    // The web_sys request_permission method returns a Promise that resolves to a string indicating
    // whether the permissions request was granted, denied, or default.
    // See https://developer.mozilla.org/en-US/docs/Web/API/Notification/requestPermission_static.
    let Ok(permissions_request_promise) = web_sys::Notification::request_permission() else {
        let _ = proxy.send_event(CustomEvent::UpdateUIApp(Box::new(|ctx| {
            callback(
                RequestPermissionsOutcome::OtherError {
                    error_message: "Error sending notification permissions request".to_string(),
                },
                ctx,
            );
        })));
        return;
    };

    let request_outcome = match JsFuture::from(permissions_request_promise)
        .await
        .map(|r| r.as_string())
    {
        Ok(Some(user_response)) if user_response == "granted" => {
            RequestPermissionsOutcome::Accepted
        }
        // Any response besides "granted" is considered a permissions denied.
        Ok(Some(_)) => RequestPermissionsOutcome::PermissionsDenied,
        _ => RequestPermissionsOutcome::OtherError {
            error_message: "Error receiving response from notification permissions request"
                .to_string(),
        },
    };

    // When the request has completed, we execute the callback with the outcome.
    let _ = proxy.send_event(CustomEvent::UpdateUIApp(Box::new(|ctx| {
        callback(request_outcome, ctx);
    })));
}
