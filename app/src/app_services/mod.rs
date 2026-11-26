//! Functionality relating to services that the application provides
//! to the host system.
//!
//! For example, on macOS, this module sets up integrations with
//! Finder such that the user can open a new Warp tab or window
//! in a given directory.

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
pub mod linux;
#[cfg(target_os = "macos")]
pub mod mac;
#[cfg(windows)]
pub mod windows;

#[cfg(any(target_os = "macos", windows))]
use serde::{Deserialize, Serialize};
#[cfg(any(target_os = "macos", windows))]
use url::Url;
#[cfg(any(
    target_os = "macos",
    target_os = "linux",
    target_os = "freebsd",
    windows
))]
use warp_core::channel::{Channel, ChannelState};
#[cfg(any(
    target_os = "macos",
    target_os = "linux",
    target_os = "freebsd",
    windows
))]
use warp_core::execution_mode::{AppExecutionMode, ExecutionMode};
use warpui::AppContext;
#[cfg(any(
    target_os = "macos",
    target_os = "linux",
    target_os = "freebsd",
    windows
))]
use warpui::SingletonEntity as _;

#[cfg(any(target_os = "macos", windows))]
use crate::external_session::ExternalSessionLaunchRequest;

/// Payload forwarded from a secondary launcher process to an existing GUI instance.
#[cfg(any(target_os = "macos", windows))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StartupForwardRequest {
    Urls(Vec<Url>),
    ExternalSession(ExternalSessionLaunchRequest),
}

#[cfg(any(
    target_os = "macos",
    target_os = "linux",
    target_os = "freebsd",
    windows
))]
pub(crate) fn should_run_single_instance_server(ctx: &AppContext) -> bool {
    let mode = AppExecutionMode::as_ref(ctx);
    mode.execution_mode() == ExecutionMode::App
        && !mode.is_headless()
        && ChannelState::channel() != Channel::Integration
}

#[cfg(any(target_os = "macos", windows))]
pub(crate) fn dispatch_forwarded_request(request: StartupForwardRequest, ctx: &mut AppContext) {
    match request {
        StartupForwardRequest::Urls(urls) => {
            for url in urls {
                crate::uri::handle_incoming_uri(&url, ctx);
            }
        }
        StartupForwardRequest::ExternalSession(request) => {
            crate::external_session::open_external_session(request, ctx);
        }
    }
}

pub fn init(ctx: &mut AppContext) {
    log::info!("Initializing app services");

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    linux::init(ctx);
    #[cfg(target_os = "macos")]
    mac::init(ctx);
    #[cfg(windows)]
    windows::init(ctx);
}

pub fn teardown(ctx: &mut AppContext) {
    log::info!("Tearing down app services...");

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    linux::teardown(ctx);
    #[cfg(target_os = "macos")]
    mac::teardown(ctx);
    #[cfg(not(any(target_os = "linux", target_os = "freebsd", target_os = "macos")))]
    let _ = ctx;
}
