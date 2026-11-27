use std::ffi::OsString;
use std::fs::{self, Permissions};
use std::os::unix::fs::{DirBuilderExt as _, MetadataExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_channel::Sender;
use async_trait::async_trait;
use ipc::{Client, ConnectionAddress, ServerBuilder, service_caller};
use objc2::rc::Retained;
use objc2_foundation::NSString;
use sha2::{Digest as _, Sha256};
use thiserror::Error;
use warp_core::channel::ChannelState;
use warp_core::{safe_info, safe_warn};
use warp_errors::report_error;
use warpui::r#async::executor::Background;
use warpui::{AppContext, Entity, ModelContext, SingletonEntity};

use super::{StartupForwardRequest, dispatch_forwarded_request, should_run_single_instance_server};
use crate::external_session::ExternalSessionLaunchRequest;

unsafe extern "C" {
    /// ObjC function to create and register the NSServices provider for the
    /// application.
    fn warp_register_services_provider();
}

/// Initializes application services.
pub fn init(ctx: &mut AppContext) {
    unsafe {
        warp_register_services_provider();
    }
    ctx.add_singleton_model(MacSingleInstanceServer::new);
}

/// Tears down application services.
pub fn teardown(ctx: &mut AppContext) {
    MacSingleInstanceServer::handle(ctx).update(ctx, |server, _| {
        server.teardown();
    });
}

#[derive(Error, Debug)]
pub enum StartupArgsForwardingError {
    #[error("should not forward arguments after an auto-update")]
    IgnoredAfterAutoUpdate,
    #[error("no startup arguments require single-instance forwarding")]
    NothingToForward,
    #[error("there is no other running instance of Warp")]
    NoExistingInstance,
    #[error("IPC client failed to send message")]
    IpcError(#[from] ipc::ClientError),
}

struct MacSingleInstanceService;

impl ipc::Service for MacSingleInstanceService {
    type Request = StartupForwardRequest;
    type Response = ();
}

#[derive(Clone)]
struct MacSingleInstanceServiceImpl {
    tx: Sender<StartupForwardRequest>,
}

#[async_trait]
impl ipc::ServiceImpl for MacSingleInstanceServiceImpl {
    type Service = MacSingleInstanceService;

    async fn handle_request(&self, request: StartupForwardRequest) {
        if let Err(err) = self.tx.send(request).await {
            report_error!(
                anyhow::Error::new(err).context("Error forwarding startup request to macOS app")
            );
        }
    }
}

struct MacSingleInstanceServer {
    _server: Option<ipc::Server>,
    socket_path: Option<PathBuf>,
}

impl MacSingleInstanceServer {
    fn new(ctx: &mut ModelContext<Self>) -> Self {
        if !should_run_single_instance_server(ctx) {
            return Self {
                _server: None,
                socket_path: None,
            };
        }

        let Some(socket_path) = macos_ipc_socket_path() else {
            return Self {
                _server: None,
                socket_path: None,
            };
        };

        if socket_path.exists() {
            let probe_result = warpui::r#async::block_on(async {
                let executor = Arc::new(Background::new(1, |_| "probe-mac-ipc".to_owned()));
                Client::connect(
                    ConnectionAddress::from(socket_path.to_string_lossy().into_owned()),
                    executor,
                )
                .await
            });
            if probe_result.is_ok() {
                safe_info!(
                    safe: ("Another macOS Warp instance already owns the IPC socket"),
                    full: (
                        "Another macOS Warp instance already owns IPC socket at {}",
                        socket_path.display()
                    )
                );
                return Self {
                    _server: None,
                    socket_path: None,
                };
            }
            let _ = fs::remove_file(&socket_path);
        }

        let (tx, rx) = async_channel::unbounded();
        let address = ConnectionAddress::from(socket_path.to_string_lossy().into_owned());
        match ServerBuilder::default()
            .with_fixed_address(address)
            .with_service(MacSingleInstanceServiceImpl { tx })
            .build_and_run(ctx.background_executor())
        {
            Ok((server, _)) => {
                if let Err(err) = fs::set_permissions(&socket_path, Permissions::from_mode(0o600)) {
                    safe_warn!(
                        safe: ("Failed to restrict permissions on macOS IPC socket"),
                        full: (
                            "Failed to restrict permissions on macOS IPC socket {}: {err:#}",
                            socket_path.display()
                        )
                    );
                }
                ctx.spawn_stream_local(
                    rx,
                    |_server, request, ctx| {
                        dispatch_forwarded_request(request, ctx);
                    },
                    |_, _| {},
                );
                Self {
                    _server: Some(server),
                    socket_path: Some(socket_path),
                }
            }
            Err(err) => {
                safe_warn!(
                    safe: ("Failed to bind macOS single-instance IPC socket"),
                    full: (
                        "Failed to bind macOS single-instance IPC socket {}: {err:#}",
                        socket_path.display()
                    )
                );
                Self {
                    _server: None,
                    socket_path: None,
                }
            }
        }
    }

    fn teardown(&mut self) {
        self._server = None;
        if let Some(socket_path) = self.socket_path.take() {
            let _ = fs::remove_file(socket_path);
        }
    }
}

impl Drop for MacSingleInstanceServer {
    fn drop(&mut self) {
        self.teardown();
    }
}

impl Entity for MacSingleInstanceServer {
    type Event = ();
}

impl SingletonEntity for MacSingleInstanceServer {}

fn macos_ipc_socket_path() -> Option<PathBuf> {
    // macOS sockaddr_un.sun_path is 104 bytes; keep a margin so binding never fails on long home paths.
    const MAX_UNIX_SOCKET_PATH_BYTES: usize = 96;

    let run_dir = warp_core::paths::config_local_dir().join("run");
    if ensure_owner_only_dir(&run_dir).is_ok() {
        let candidate = run_dir.join("app.sock");
        if candidate.as_os_str().as_encoded_bytes().len() <= MAX_UNIX_SOCKET_PATH_BYTES {
            return Some(candidate);
        }
    }

    let uid = nix::unistd::getuid().as_raw();
    let fallback_dir = PathBuf::from(format!("/tmp/warp-{uid}"));
    if ensure_owner_only_dir(&fallback_dir).is_err() {
        return None;
    }

    let digest = Sha256::digest(ChannelState::data_domain().as_bytes());
    let short_hash = hex::encode(&digest[..8]);
    Some(fallback_dir.join(format!("{short_hash}.sock")))
}

fn ensure_owner_only_dir(dir: &Path) -> std::io::Result<()> {
    if let Some(parent) = dir.parent()
        && !parent.exists()
    {
        fs::create_dir_all(parent)?;
    }

    match fs::DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(err) => return Err(err),
    }

    let uid = nix::unistd::getuid().as_raw();
    let meta = fs::symlink_metadata(dir)?;
    if !(!meta.file_type().is_symlink() && meta.is_dir() && meta.uid() == uid) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "IPC runtime directory is not an owner-owned directory",
        ));
    }
    if meta.permissions().mode() & 0o777 != 0o700 {
        fs::set_permissions(dir, Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Forwards explicit external-session or URL launch requests to an already-running Warp instance on
/// macOS.
pub fn pass_startup_args_to_existing_instance(
    args: &warp_cli::AppArgs,
    initial_env: &[(OsString, OsString)],
) -> Result<(), StartupArgsForwardingError> {
    if args.finish_update {
        return Err(StartupArgsForwardingError::IgnoredAfterAutoUpdate);
    }

    let request = if args.external_session.has_session_options() {
        StartupForwardRequest::ExternalSession(ExternalSessionLaunchRequest::from_cli_args(
            &args.external_session,
            initial_env,
        ))
    } else if !args.urls.is_empty() {
        StartupForwardRequest::Urls(args.urls.clone())
    } else {
        return Err(StartupArgsForwardingError::NothingToForward);
    };

    let Some(socket_path) = macos_ipc_socket_path().filter(|path| path.exists()) else {
        return Err(StartupArgsForwardingError::NoExistingInstance);
    };

    warpui::r#async::block_on(async {
        let executor = Arc::new(Background::new(1, |_| "forward-mac-startup".to_owned()));
        let client = Client::connect(
            ConnectionAddress::from(socket_path.to_string_lossy().into_owned()),
            executor,
        )
        .await
        .map_err(|_| StartupArgsForwardingError::NoExistingInstance)?;
        let caller = service_caller::<MacSingleInstanceService>(Arc::new(client));
        caller.call(request).await?;
        Ok(())
    })
}

/// Returns an NSString containing the custom URL scheme that this build of the
/// application will respond to.
///
/// Called synchronously from the NSServices dispatch path in
/// `services.m::forFilesFromPasteboard:performAction:`, which wraps the body in
/// an `@autoreleasepool` block. `autorelease_return` hands the string to that
/// ambient pool, which takes ownership of it.
#[unsafe(no_mangle)]
extern "C-unwind" fn warp_services_provider_custom_url_scheme() -> *mut NSString {
    Retained::autorelease_return(NSString::from_str(ChannelState::url_scheme()))
}
