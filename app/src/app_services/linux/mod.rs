use std::collections::HashMap;

use futures_util::FutureExt as _;
use itertools::Itertools as _;
use warp_errors::{report_error, report_if_error};
use warpui::r#async::executor::BackgroundTask;
use warpui::{AppContext, SingletonEntity};
use zbus::{interface, proxy, zvariant};

use crate::channel::ChannelState;

/// Initializes application services.
pub fn init(ctx: &mut AppContext) {
    ctx.add_singleton_model(DBusServiceHost::new);
}

/// Tears down application services.
pub fn teardown(ctx: &mut AppContext) {
    DBusServiceHost::handle(ctx).update(ctx, |service_host, _| {
        service_host.terminate();
    });
}

/// Attempts to forward startup arguments to an existing instance of the
/// application.
///
/// Returns Ok if an existing instance exists and was reachable.
#[cfg(feature = "release_bundle")]
pub fn pass_startup_args_to_existing_instance(
    args: &warp_cli::AppArgs,
) -> Result<(), StartupArgsForwardingError> {
    if args.finish_update {
        return Err(StartupArgsForwardingError::IgnoredAfterAutoUpdate);
    }

    warpui::r#async::block_on(async {
        let conn = zbus::Connection::session().await?;
        let proxy = ExistingApplicationProxy::builder(&conn)
            .destination(DBusServiceHost::well_known_name())?
            .path(DBusServiceHost::application_service_path())?
            .build()
            .await?;
        let mut open_new_url;
        let mut url_refs = args.urls.iter().map(AsRef::as_ref).collect_vec();
        // If there are no URLs on the command line, send one to open a new
        // window using the same current working directory as this process.
        if url_refs.is_empty() {
            open_new_url = format!("{}://action/new_window", ChannelState::url_scheme());
            if let Ok(current_dir) = std::env::current_dir() {
                open_new_url.push_str(&format!("?path={}", current_dir.display()));
            }
            url_refs.push(&open_new_url);
        }
        proxy.open(&url_refs, HashMap::new()).await?;

        // Make sure we close the connection and clean up resources, to avoid
        // leaving behind file descriptors that will interfere with the terminal
        // server spawn process.
        let _ = conn.close().await;

        Ok(())
    })
}

#[derive(Debug, thiserror::Error)]
#[cfg(feature = "release_bundle")]
pub enum StartupArgsForwardingError {
    /// There's no instance of Warp already running.
    #[error("no existing instance found to forward args to")]
    NoExistingInstance,
    /// This instance was launched after an auto-update and should not forward
    /// arguments to the old (terminating) instance.
    #[error("should not forward args after an auto-update")]
    IgnoredAfterAutoUpdate,
    /// An unknown D-Bus error occurred.
    #[error("unknown dbus error")]
    Unknown(zbus::Error),
}

#[cfg(feature = "release_bundle")]
impl From<zbus::fdo::Error> for StartupArgsForwardingError {
    fn from(value: zbus::fdo::Error) -> Self {
        // While ServiceUnknown usually means that D-Bus doesn't know how to
        // _launch_ something to handle your message, in our case, we're not
        // registering a service, so this really means that Warp is not already
        // running.
        if matches!(value, zbus::fdo::Error::ServiceUnknown(_)) {
            StartupArgsForwardingError::NoExistingInstance
        } else {
            StartupArgsForwardingError::Unknown(zbus::Error::FDO(Box::new(value)))
        }
    }
}

#[cfg(feature = "release_bundle")]
impl From<zbus::Error> for StartupArgsForwardingError {
    fn from(value: zbus::Error) -> Self {
        match value {
            zbus::Error::FDO(err) => (*err).into(),
            err => StartupArgsForwardingError::Unknown(err),
        }
    }
}

enum ApplicationServiceEvent {
    Open { uris: Vec<String> },
}

/// A structure providing an implementation of the org.freedesktop.Application
/// D-Bus service.
struct ApplicationService {
    tx: async_channel::Sender<ApplicationServiceEvent>,
}

/// An implementation of the org.freedesktop.Application D-Bus service.
///
/// See: https://specifications.freedesktop.org/desktop-entry-spec/1.5/ar01s08.html
#[interface(name = "org.freedesktop.Application")]
impl ApplicationService {
    /// Called when the application is started without any files to open.
    async fn activate(
        &self,
        _platform_data: HashMap<String, zvariant::Value<'_>>,
    ) -> zbus::fdo::Result<()> {
        // not yet implemented
        Ok(())
    }

    /// Called when desktop actions are activated.
    ///
    /// See: https://specifications.freedesktop.org/desktop-entry-spec/1.5/ar01s11.html
    async fn activate_action(
        &self,
        _action_name: String,
        _parameter: Vec<zvariant::Value<'_>>,
        _platform_data: HashMap<String, zvariant::Value<'_>>,
    ) -> zbus::fdo::Result<()> {
        // not yet implemented
        Ok(())
    }

    /// Called when the application is started with files.
    async fn open(
        &self,
        uris: Vec<String>,
        _platform_data: HashMap<String, zvariant::Value<'_>>,
    ) -> zbus::fdo::Result<()> {
        let _ = self.tx.send(ApplicationServiceEvent::Open { uris }).await;
        Ok(())
    }
}

// A D-Bus client for connecting to an already-running instance of Warp and
// invoking org.freedesktop.Application IPC methods.
#[proxy(
    interface = "org.freedesktop.Application",
    default_service = "dev.warp.WarpLocal",
    default_path = "/dev/warp/WarpLocal",
    gen_blocking = false
)]
trait ExistingApplication {
    fn activate(&self, platform_data: HashMap<&str, zvariant::Value<'_>>) -> zbus::fdo::Result<()>;

    fn activate_action(
        &self,
        action_name: &str,
        parameter: &[zvariant::Value<'_>],
        platform_data: HashMap<&str, zvariant::Value<'_>>,
    ) -> zbus::fdo::Result<()>;

    fn open(
        &self,
        uris: &[&str],
        platform_data: HashMap<&str, zvariant::Value<'_>>,
    ) -> zbus::fdo::Result<()>;
}

/// A singleton model that is responsible for hosting all D-Bus services
/// exposed by the application.
struct DBusServiceHost {
    server_task: Option<BackgroundTask>,
}

impl DBusServiceHost {
    fn new(ctx: &mut warpui::ModelContext<Self>) -> Self {
        let (tx, rx) = async_channel::unbounded();

        // Spawn a background task for the D-Bus server.
        let server_task = ctx.background_executor().spawn(
            async {
                let conn = zbus::connection::Builder::session()?
                    .name(Self::well_known_name())?
                    .serve_at(Self::application_service_path(), ApplicationService { tx })?
                    // Instead of having zbus spawn a thread to poll for new
                    // messages, we'll poll on our own executor.
                    .internal_executor(false)
                    .build()
                    .await?;

                loop {
                    conn.executor().tick().await;
                }
            }
            .map(|result: anyhow::Result<()>| {
                if let Err(err) = result {
                    report_error!(err
                        .context("Failed to initialize org.freedesktop.Application D-Bus service"));
                }
            }),
        );

        // Process any events that we receive over D-Bus.
        ctx.spawn_stream_local(rx, |_, event, ctx| {
            match event {
                ApplicationServiceEvent::Open { uris } => {
                    for uri in uris {
                        match url::Url::parse(&uri) {
                            Ok(uri) => crate::uri::handle_incoming_uri(&uri, ctx),
                            Err(err) => log::warn!("Failed to parse URI when handling org.freedesktop.Application/open: {err:#}"),
                        }
                    }
                },
            }
        }, |_, _| {});

        Self {
            server_task: Some(server_task),
        }
    }

    fn terminate(&mut self) {
        if let Some(server_task) = self.server_task.take() {
            server_task.abort();
            // Wait until we've torn down the dbus service.
            report_if_error!(warpui::r#async::block_on(server_task));
        }
    }

    /// Returns the D-Bus well-known name that should be used.
    fn well_known_name() -> String {
        ChannelState::app_id().to_string()
    }

    /// Returns the path under which the org.freedesktop.Application interface
    /// will be hosted.
    fn application_service_path() -> String {
        format!("/{}", Self::well_known_name().split('.').join("/"))
    }
}

impl warpui::Entity for DBusServiceHost {
    type Event = ();
}

impl SingletonEntity for DBusServiceHost {}
