use std::sync::Arc;

use warpui::r#async::executor::Background;

use crate::auth::auth_state::AuthState;
use crate::send_telemetry_on_executor;
use crate::server::telemetry::{DownloadSource, TelemetryEvent};

/// Determine the Warp download method (if possible) and send a telemetry event reporting that
/// method
pub fn determine_and_report(auth_state: Arc<AuthState>, executor: Arc<Background>) {
    let telemetry_executor = executor.clone();
    executor
        .spawn(async move {
            let download_source = check_download_source().await;

            send_telemetry_on_executor!(
                auth_state,
                TelemetryEvent::DownloadSource(download_source),
                telemetry_executor
            );
        })
        .detach();
}

/// Try to determine what method was used to download Warp. Currently, on macOS, we only support
/// two download methods:
///
/// 1. The default download from our website.
/// 2. Via `homebrew`
///
/// To determine if Warp was installed with Homebrew, we run `brew list --cask warp`. That command
/// will return an failure error code if Warp was not installed with Homebrew. It will also fail
/// to launch entirely if Homebrew isn't installed. In either of those cases, we treat the download
/// as being from the Warp website.
#[cfg(target_os = "macos")]
async fn check_download_source() -> DownloadSource {
    use std::env;
    use std::process::Stdio;

    // By default when launching an app, the PATH is very limited and doesn't include the locations
    // into which Homebrew installs itself. To make sure we can accurately call `brew`, we need to
    // update the PATH to include the possible Homebrew locations. Currently, the Homebrew
    // installer can install into two paths, depending on the architecture (see
    // https://github.com/Homebrew/install/blob/5e7f30635a945f475a557240f006973c81c71324/install.sh#L153-L158
    // for details):
    //
    // * /opt/homebrew/bin
    // * /usr/local/bin
    let mut new_path = String::from("/opt/homebrew/bin:/usr/local/bin:");

    if let Ok(existing) = env::var("PATH") {
        new_path.push_str(&existing);
    }

    let result = command::r#async::Command::new("brew")
        .args(["list", "--cask", "warp"])
        .env("HOMEBREW_NO_AUTO_UPDATE", "1")
        .env("PATH", new_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await;

    match result {
        Ok(status) if status.success() => DownloadSource::Homebrew,
        _ => DownloadSource::Website,
    }
}

#[cfg(not(target_os = "macos"))]
async fn check_download_source() -> DownloadSource {
    DownloadSource::Website
}
