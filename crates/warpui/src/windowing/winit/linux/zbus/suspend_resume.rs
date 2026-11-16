use futures_lite::StreamExt;
use winit::event_loop::EventLoopProxy;
use zbus::proxy;

use crate::r#async::executor::Background;
use crate::windowing::winit::app::CustomEvent;

/// A zbus proxy for receiving PrepareForSleep signals from systemd-logind.
#[proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1",
    gen_blocking = false
)]
trait LoginManager {
    #[zbus(signal)]
    fn prepare_for_sleep(&self, start: bool) -> zbus::Result<()>;
}

/// Sets up a background task to listen to suspend/resume events sent over dbus
/// and inject events into the winit EventLoop accordingly.
pub fn watch_suspend_resume_changes(
    event_proxy: EventLoopProxy<CustomEvent>,
    background: &Background,
) {
    background
        .spawn(async move {
            if let Err(err) = watch_suspend_resume_changes_internal(event_proxy).await {
                log::warn!(
                    "Encountered error while watching for system suspend/resume events: {err:#}"
                );
            }
        })
        .detach();
}

async fn watch_suspend_resume_changes_internal(
    event_proxy: EventLoopProxy<CustomEvent>,
) -> zbus::Result<()> {
    let connection = zbus::Connection::system().await?;
    let login_manager_proxy = LoginManagerProxy::new(&connection).await?;
    let mut stream = login_manager_proxy.receive_prepare_for_sleep().await?;
    while let Some(msg) = stream.next().await {
        if let Ok(args) = msg.args() {
            if args.start {
                let _ = event_proxy.send_event(CustomEvent::AboutToSleep);
            } else {
                let _ = event_proxy.send_event(CustomEvent::ResumedFromSleep);
            }
        }
    }
    Ok(())
}
