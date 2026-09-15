use futures_lite::StreamExt;
use winit::event_loop::EventLoopProxy;
use zbus::proxy;

use crate::r#async::executor::Background;
use crate::windowing::winit::app::CustomEvent;

/// A zbus proxy for receiving network status signals from `NetworkManager`.
#[proxy(
    interface = "org.freedesktop.NetworkManager",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager",
    gen_blocking = false
)]
trait NetworkManager {
    #[zbus(signal)]
    fn state_changed(&self, state: u32) -> zbus::Result<()>;
}

/// Sets up a background task to listen to changes to network status.
pub fn watch_network_status_changed(
    event_proxy: EventLoopProxy<CustomEvent>,
    background: &Background,
) {
    background
        .spawn(async move {
            if let Err(err) = watch_network_status_changed_internal(event_proxy).await {
                log::warn!("Encountered error while watching for network status events: {err:#}");
            }
        })
        .detach();
}

async fn watch_network_status_changed_internal(
    event_proxy: EventLoopProxy<CustomEvent>,
) -> zbus::Result<()> {
    let connection = zbus::Connection::system().await?;
    let network_manager_proxy = NetworkManagerProxy::new(&connection).await?;
    let mut state_changed_stream = network_manager_proxy.receive_state_changed().await?;
    while let Some(msg) = state_changed_stream.next().await {
        if let Ok(args) = msg.args() {
            // NetworkManager's connectivity check can report `NM_STATE_CONNECTED_LOCAL` or
            // `NM_STATE_CONNECTED_SITE` when tun interfaces misroute its probes, even though the app
            // can still reach the internet. `NM_STATE_UNKNOWN` is also documented as an unknown
            // state, not as disconnected.
            // See also: https://networkmanager.dev/docs/api/1.32.10/nm-dbus-types.html
            match args.state {
                0 | 50 | 60 | 70 => {
                    let _ = event_proxy.send_event(CustomEvent::InternetConnected);
                }
                _ => {
                    let _ = event_proxy.send_event(CustomEvent::InternetDisconnected);
                }
            }
        }
    }
    Ok(())
}
