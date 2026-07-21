use tauri_winrt_notification::Toast;
use winit::event_loop::EventLoopProxy;

use crate::WindowId;
use crate::notification::NotificationSendError;
use crate::windowing::winit::app::CustomEvent;
use crate::windowing::winit::notifications::NotificationInfo;

pub(super) async fn send_notification(
    notification_info: NotificationInfo,
    window_id: WindowId,
    proxy: EventLoopProxy<CustomEvent>,
) {
    let NotificationInfo {
        notification_content,
        on_error,
    } = notification_info;

    let powershell_app_id = Toast::POWERSHELL_APP_ID.to_string();
    let app_id = unsafe { fetch_windows_app_id() }
        .ok()
        .unwrap_or(powershell_app_id);
    let proxy_clone = proxy.clone();
    let toast = Toast::new(&app_id)
        .title(notification_content.title())
        .text1(notification_content.body())
        .on_activated(move |_activated_arguments| {
            let _ = proxy_clone
                .send_event(CustomEvent::FocusWindow { window_id })
                .map_err(|err| {
                    log::warn!("Unable to focus window after event loop closed: {err:?}");
                });
            Ok(())
        });

    if let Err(err) = toast.show() {
        let error = NotificationSendError::Other {
            error_message: err.to_string(),
        };

        let _ = proxy.send_event(CustomEvent::UpdateUIApp(Box::new(|ctx| {
            on_error(error, ctx);
        })));
    }
}

unsafe fn fetch_windows_app_id() -> Result<String, anyhow::Error> {
    unsafe {
        let app_id_pwstr = windows::Win32::UI::Shell::GetCurrentProcessExplicitAppUserModelID()
            .map_err(|win_err| {
                log::warn!("error retrieving Win32 AppUserModel ID: {win_err:?}");
                anyhow::anyhow!(win_err)
            })?;
        Ok(app_id_pwstr.to_string()?)
    }
}
