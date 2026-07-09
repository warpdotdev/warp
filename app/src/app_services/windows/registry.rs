use std::ffi::OsString;

use warp_core::channel::ChannelState;
use warp_errors::report_error;
use windows_registry::{CURRENT_USER, HSTRING};

pub(super) fn register_uri_handler() {
    // To change the settings for the user, changes must be made under
    // HKEY_CURRENT_USER\Software\Classes instead of under HKEY_CLASSES_ROOT since only an
    // administrator can modify it. It gets merged into HKEY_CLASSES_ROOT later.
    let Ok(classes_key) = CURRENT_USER.open("Software\\Classes") else {
        report_error!("Failed to get current_user\\software\\classes");
        return;
    };

    // The Windows Registry entry for Warp (assuming the channel is WarpLocal):
    // warplocal
    //   (Default) = "WarpLocal"
    //   URL Protocol = ""
    //   DefaultIcon
    //      (Default) = "{path_to_channel_icon},0" TODO(CORE-2860): Add icon file path here.
    //   shell
    //      open
    //         command
    //            (Default) = "{path_to_executable}" "%0"
    let uri_scheme = ChannelState::url_scheme();
    match classes_key.create(uri_scheme) {
        Ok(parent_key) => {
            // The empty string represents the "(Default)" value for a registry key.
            if let Err(err) = parent_key.set_string("", ChannelState::app_id().application_name()) {
                report_error!(
                    anyhow::Error::new(err).context("Could not set URI Scheme display name")
                );
                return;
            }
            if let Err(err) = parent_key.set_string("URL Protocol", "") {
                report_error!(
                    anyhow::Error::new(err).context("Could not set URI Scheme URL Protocol Key")
                );
                return;
            };

            // TODO(CORE-2861): Add the `DefaultIcon` Default value here with the file path to
            // Warp's icon once we figure out distribution on Windows.

            let command_key = match parent_key.create("shell\\open\\command") {
                Ok(command_key) => command_key,
                Err(err) => {
                    report_error!(anyhow::Error::new(err)
                        .context("Could not create shell\\open\\command key"));
                    return;
                }
            };
            let command = match std::env::current_exe() {
                Ok(path) => {
                    let mut command = OsString::new();
                    command.push("\"");
                    command.push(path.as_os_str());
                    command.push("\" \"%0\"");
                    HSTRING::from(command.as_os_str())
                }
                Err(err) => {
                    report_error!(anyhow::Error::new(err).context(
                        "Could not get path to current executable for registering URI scheme"
                    ));
                    return;
                }
            };
            // The empty string represents the "(Default)" value for a registry key.
            if let Err(err) = command_key.set_hstring("", &command) {
                report_error!(anyhow::Error::new(err)
                    .context("Could not set shell command path for URI Scheme"));
            }
        }
        Err(err) => {
            report_error!(
                anyhow::Error::new(err).context("Failed to create URI Scheme registry entry")
            );
        }
    }
}
