use registry::register_uri_handler;
#[cfg(any(feature = "release_bundle", test))]
use thiserror::Error;
#[cfg(feature = "release_bundle")]
use warp_errors::report_error;
use warpui::AppContext;
#[cfg(feature = "release_bundle")]
use {
    service_impl::forward_uri_to_sole_running_instance,
    single_instance_manager::SingleInstanceManager, url::Url, warp_core::channel::ChannelState,
};

mod registry;
// Compiled in test builds as well as release bundles: the single-instance ordering contract is
// covered by unit tests, and the workspace test job does not enable `release_bundle`.
#[cfg(any(feature = "release_bundle", test))]
#[cfg_attr(not(feature = "release_bundle"), allow(dead_code))]
mod service_impl;
#[cfg(any(feature = "release_bundle", test))]
#[cfg_attr(not(feature = "release_bundle"), allow(dead_code))]
mod single_instance_manager;

#[derive(Error, Debug)]
#[cfg(any(feature = "release_bundle", test))]
#[cfg_attr(not(feature = "release_bundle"), allow(dead_code))]
pub enum StartupArgsForwardingError {
    #[error("should not forward arguments after an auto-update")]
    IgnoredAfterAutoUpdate,
    #[error("there is no other instance of Warp")]
    NoExistingInstance,
    #[error("failed to construct url")]
    CouldNotCreateUrl(#[from] url::ParseError),
    #[error("IPC Client failed to send message")]
    IpcError(#[from] ipc::ClientError),
    #[error("timed out reaching the running instance of Warp")]
    TimedOut,
    #[error("Win32 error")]
    WindowsError(#[from] windows::core::Error),
}

#[cfg(feature = "release_bundle")]
pub fn pass_startup_args_to_existing_instance(
    args: &warp_cli::AppArgs,
) -> Result<(), StartupArgsForwardingError> {
    if args.finish_update {
        return Err(StartupArgsForwardingError::IgnoredAfterAutoUpdate);
    }
    if !SingleInstanceManager::has_existing_instance()? {
        return Err(StartupArgsForwardingError::NoExistingInstance);
    }

    warpui::r#async::block_on(async {
        if args.urls.is_empty() {
            // If there are no URLs on the command line, send one to open a new
            // window using the same current working directory as this process.
            let mut open_new_url = format!("{}://action/new_window", ChannelState::url_scheme());
            if let Ok(current_dir) = std::env::current_dir() {
                match current_dir.into_os_string().into_string() {
                    Ok(current_dir) => open_new_url.push_str(&format!("?path={}", current_dir)),
                    Err(os_string) => {
                        report_error!(
                            "Failed to convert OsString to String",
                            extra: { "os_string" => ?os_string }
                        );
                    }
                }
            }

            let url = Url::parse(&open_new_url)?;
            forward_uri_to_sole_running_instance(vec![url]).await?
        } else {
            forward_uri_to_sole_running_instance(args.urls.clone()).await?
        }

        Ok(())
    })
}

pub(super) fn init(_ctx: &mut AppContext) {
    #[cfg(feature = "release_bundle")]
    _ctx.add_singleton_model(SingleInstanceManager::new);
    register_uri_handler();
}
