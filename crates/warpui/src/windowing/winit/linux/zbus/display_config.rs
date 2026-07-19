//! Provides a D-Bus client for querying Mutter's display configuration.
//!
//! This interface is only available on GNOME (Mutter) sessions; on other
//! desktop environments the service does not exist and calls will fail.

use std::collections::HashMap;
use std::time::Duration;

use zbus::{proxy, zvariant};

use crate::r#async::{block_on, FutureExt as _};

/// The connector info tuple used by Mutter to identify a physical monitor:
/// `(connector, vendor, product, serial)`.
type MonitorId = (String, String, String, String);

/// A display mode of a physical monitor:
/// `(id, width, height, refresh_rate, preferred_scale, supported_scales, properties)`.
type MonitorMode = (
    String,
    i32,
    i32,
    f64,
    f64,
    Vec<f64>,
    HashMap<String, zvariant::OwnedValue>,
);

/// A physical monitor: `(id, modes, properties)`.
type Monitor = (
    MonitorId,
    Vec<MonitorMode>,
    HashMap<String, zvariant::OwnedValue>,
);

/// A logical monitor: `(x, y, scale, transform, primary, monitors, properties)`.
type LogicalMonitor = (
    i32,
    i32,
    f64,
    u32,
    bool,
    Vec<MonitorId>,
    HashMap<String, zvariant::OwnedValue>,
);

/// The full reply of `GetCurrentState`:
/// `(serial, monitors, logical_monitors, properties)`.
type CurrentState = (
    u32,
    Vec<Monitor>,
    Vec<LogicalMonitor>,
    HashMap<String, zvariant::OwnedValue>,
);

/// A D-Bus client for connecting to Mutter's display configuration.
#[proxy(
    interface = "org.gnome.Mutter.DisplayConfig",
    default_service = "org.gnome.Mutter.DisplayConfig",
    default_path = "/org/gnome/Mutter/DisplayConfig"
)]
trait DisplayConfig {
    fn get_current_state(&self) -> zbus::fdo::Result<CurrentState>;
}

/// Retrieves the largest scale factor across all logical monitors, blocking
/// for up to 200ms to get the value via dbus.
///
/// Returns an error when the session is not running under Mutter (the
/// service only exists on GNOME) or when no logical monitors are reported.
pub fn get_max_monitor_scale() -> Result<f64, zbus::Error> {
    block_on(async {
        query_max_monitor_scale_from_dbus()
            .with_timeout(Duration::from_millis(200))
            .await
            .unwrap_or_else(|_| {
                Err(zbus::Error::from(zbus::fdo::Error::TimedOut(
                    "Failed to get a response within 200ms".to_owned(),
                )))
            })
    })
}

/// Queries the current D-Bus session bus to get the maximum logical monitor
/// scale from Mutter.
async fn query_max_monitor_scale_from_dbus() -> Result<f64, zbus::Error> {
    let client_conn = zbus::Connection::session().await?;
    let display_config_proxy = DisplayConfigProxy::new(&client_conn).await?;
    let (_serial, _monitors, logical_monitors, _properties) =
        display_config_proxy.get_current_state().await?;
    logical_monitors
        .into_iter()
        .map(|(_x, _y, scale, _transform, _primary, _monitors, _properties)| scale)
        .reduce(f64::max)
        .ok_or_else(|| zbus::Error::Failure("no logical monitors reported".to_owned()))
}
