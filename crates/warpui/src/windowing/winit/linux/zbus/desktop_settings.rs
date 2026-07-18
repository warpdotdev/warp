//! Provides an application-agnostic D-Bus client for retrieving the
//! desktop environment's appearance settings.

use std::ops::Deref as _;
use std::time::Duration;

use futures::StreamExt as _;
use warp_errors::report_error;
use winit::event_loop::EventLoopProxy;
use zbus::{proxy, zvariant};

use crate::platform::SystemTheme;
use crate::r#async::executor::Background;
use crate::r#async::{block_on, FutureExt as _};
use crate::windowing::winit::app::CustomEvent;

const COLOR_SCHEME_SETTINGS_NAMESPACE: &str = "org.freedesktop.appearance";
const COLOR_SCHEME_SETTINGS_KEY: &str = "color-scheme";

/// Values used by the desktop environment to encode the user's
/// system color scheme preference.
#[derive(Debug, Default, serde::Deserialize, serde::Serialize, zbus::zvariant::Type, PartialEq)]
enum SystemColorScheme {
    #[default]
    NoPreference = 0,
    Dark = 1,
    Light = 2,
}

impl From<&u32> for SystemColorScheme {
    fn from(value: &u32) -> SystemColorScheme {
        match value {
            0 => SystemColorScheme::NoPreference,
            1 => SystemColorScheme::Dark,
            2 => SystemColorScheme::Light,
            _ => SystemColorScheme::NoPreference,
        }
    }
}

impl From<&str> for SystemColorScheme {
    fn from(value: &str) -> SystemColorScheme {
        match value {
            "prefer-dark" => SystemColorScheme::Dark,
            "prefer-light" => SystemColorScheme::Light,
            _ => SystemColorScheme::NoPreference,
        }
    }
}

impl From<&zvariant::Str<'_>> for SystemColorScheme {
    fn from(value: &zvariant::Str) -> SystemColorScheme {
        SystemColorScheme::from(value.as_str())
    }
}

impl From<&zvariant::Value<'_>> for SystemColorScheme {
    fn from(value: &zvariant::Value) -> SystemColorScheme {
        match value {
            zvariant::Value::U32(u) => SystemColorScheme::from(u),
            zvariant::Value::Str(s) => SystemColorScheme::from(s),
            zvariant::Value::Value(boxed_v) => match boxed_v.downcast_ref::<u32>() {
                Ok(v) => SystemColorScheme::from(&v),
                Err(err) => {
                    report_error!(
                        anyhow::Error::new(err).context(
                            "D-Bus inner variant could not be converted to SystemThemePreference"
                        ),
                        extra: {
                            "signature" => ?value.value_signature(),
                            "value" => ?value
                        }
                    );
                    SystemColorScheme::NoPreference
                }
            },
            _ => {
                report_error!(
                    "D-Bus outer variant could not be converted to SystemThemePreference",
                    extra: {
                        "signature" => ?value.value_signature(),
                        "value" => ?value
                    }
                );
                SystemColorScheme::NoPreference
            }
        }
    }
}

impl From<&zvariant::OwnedValue> for SystemColorScheme {
    fn from(owned_value: &zvariant::OwnedValue) -> SystemColorScheme {
        SystemColorScheme::from(owned_value.deref())
    }
}

impl From<SystemColorScheme> for SystemTheme {
    fn from(os_value: SystemColorScheme) -> SystemTheme {
        match os_value {
            SystemColorScheme::Dark => SystemTheme::Dark,
            SystemColorScheme::Light => SystemTheme::Light,
            SystemColorScheme::NoPreference => SystemTheme::default(),
        }
    }
}

/// A D-Bus client for connecting to the desktop settings.
#[proxy(
    interface = "org.freedesktop.portal.Settings",
    default_service = "org.freedesktop.portal.Desktop",
    default_path = "/org/freedesktop/portal/desktop"
)]
trait DesktopSettings {
    fn read(&self, namespace: &str, key: &str) -> zbus::fdo::Result<zvariant::OwnedValue>;

    #[zbus(signal)]
    fn setting_changed(
        &self,
        interface_name: &str,
        setting_name: &str,
        new_setting_value: zvariant::Value<'_>,
    ) -> zbus::fdo::Result<()>;
}

/// Sets up a background task to listen to desktop settings change events sent
/// over dbus and inject events into the winit EventLoop accordingly.
pub fn watch_desktop_settings_changes(
    event_proxy: EventLoopProxy<CustomEvent>,
    background: &Background,
) {
    background
        .spawn(async move {
            if let Err(err) = watch_desktop_settings_changes_internal(event_proxy).await {
                log::warn!(
                    "Encountered error while watching for desktop settings change events: {err:#}"
                );
            }
        })
        .detach();
}

async fn watch_desktop_settings_changes_internal(
    event_proxy: EventLoopProxy<CustomEvent>,
) -> zbus::Result<()> {
    let connection = zbus::Connection::session().await?;
    let desktop_settings_proxy = DesktopSettingsProxy::new(&connection).await?;
    let mut stream = desktop_settings_proxy.receive_setting_changed().await?;
    while let Some(msg) = stream.next().await {
        let Ok(args) = msg.args() else {
            log::warn!("appearance settings signal should have arguments");
            continue;
        };
        // As of now, we are only interested in system color scheme changes.
        // In the future, we may check for other types of signals.
        if let (&COLOR_SCHEME_SETTINGS_NAMESPACE, &COLOR_SCHEME_SETTINGS_KEY) =
            (args.interface_name(), args.setting_name())
        {
            let _ = event_proxy.send_event(CustomEvent::SystemThemeChanged);
        }
    }
    Ok(())
}

/// Retrieves the system color scheme, blocking for up to 200ms to get the
/// value via dbus.
pub fn get_system_theme() -> Result<SystemTheme, zbus::Error> {
    block_on(async {
        query_system_theme_from_dbus()
            .with_timeout(Duration::from_millis(200))
            .await
            .unwrap_or_else(|_| {
                Err(zbus::Error::from(zbus::fdo::Error::TimedOut(
                    "Failed to get a response within 200ms".to_owned(),
                )))
            })
    })
}

/// Queries the current D-Bus session bus to get the system color scheme.
async fn query_system_theme_from_dbus() -> Result<SystemTheme, zbus::Error> {
    let client_conn = zbus::Connection::session().await?;
    let settings_proxy = DesktopSettingsProxy::new(&client_conn).await?;
    let owned_val = settings_proxy
        .read(COLOR_SCHEME_SETTINGS_NAMESPACE, COLOR_SCHEME_SETTINGS_KEY)
        .await?;
    Ok(SystemColorScheme::from(&owned_val).into())
}
