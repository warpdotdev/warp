use std::sync::Arc;
use std::time::Duration;

use async_channel::{Sender, TrySendError};
use async_trait::async_trait;
use instant::Instant;
use ipc::{Client, ConnectionAddress};
use url::Url;
use warp_errors::report_error;
use warpui::r#async::executor::Background;
use warpui::r#async::{FutureExt as _, Timer};
use windows::Win32::UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow};

use super::StartupArgsForwardingError;
use super::single_instance_manager::uri_named_pipe_name;

/// How long a launch keeps trying to reach the running instance before concluding it cannot.
///
/// The listener owns one free pipe instance at a time and creates the next one after accepting, so
/// a hand-off arriving during that rollover is refused and succeeds on a later attempt. The budget
/// only has to outlast that, and stay short enough that a launch with nobody to talk to still
/// starts promptly.
const CONNECT_RETRY_BUDGET: Duration = Duration::from_millis(750);

/// How long to wait between connection attempts within [`CONNECT_RETRY_BUDGET`].
const CONNECT_RETRY_INTERVAL: Duration = Duration::from_millis(25);

/// IPC Service to respond to URIs sent to the active Warp instance.
pub(super) struct UriService {}

impl ipc::Service for UriService {
    type Request = Vec<Url>;
    type Response = ();
}

#[derive(Clone)]
pub(super) struct UriServiceImpl {
    tx: Sender<Vec<Url>>,
}

impl UriServiceImpl {
    pub(super) fn new(tx: Sender<Vec<Url>>) -> Self {
        Self { tx }
    }
}

#[async_trait]
impl ipc::ServiceImpl for UriServiceImpl {
    type Service = UriService;

    async fn handle_request(&self, request: Vec<Url>) -> Result<(), String> {
        // Never awaits on a full queue: blocking here would leave the sender waiting on an
        // instance that is not draining, which is a worse outcome than it opening its own window.
        match self.tx.try_send(request) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => {
                // Throttled because a stalled instance can refuse every hand-off it is sent, and
                // the first one already says everything the rest would.
                report_error!(
                    anyhow::anyhow!("Refused a URI hand-off because the pending queue is full"),
                    warp_errors::ReportErrorLogMode::OncePerRun
                );
                Err("the running instance has too many pending hand-offs".to_owned())
            }
            Err(TrySendError::Closed(_)) => {
                Err("the running instance is no longer accepting hand-offs".to_owned())
            }
        }
    }
}

/// Lets the running instance pull itself to the foreground when it handles the hand-off.
///
/// Windows refuses `SetForegroundWindow` from a process that is not already in the foreground and
/// only flashes its taskbar button instead, so without this grant a redirected launch is
/// indistinguishable from one that did nothing - and the user launches Warp again. This process was
/// started by whatever the user just interacted with (Explorer, a shell, a browser), so it does
/// hold the right and can pass it on. It exits immediately afterwards, and Windows revokes the
/// grant on the next user input.
fn allow_existing_instance_to_take_foreground() {
    // SAFETY: no pointer or handle arguments to keep valid; the call only adjusts which processes
    // may take the foreground.
    if let Err(err) = unsafe { AllowSetForegroundWindow(ASFW_ANY) } {
        log::warn!("Failed to grant foreground rights to the running Warp instance: {err}");
    }
}

/// Connects to the running instance's URI pipe, retrying transient failures until
/// [`CONNECT_RETRY_BUDGET`] is spent.
pub(super) async fn connect_to_sole_running_instance(
    pipe_name: &str,
    background_executor: Arc<Background>,
) -> Result<Client, StartupArgsForwardingError> {
    let deadline = Instant::now() + CONNECT_RETRY_BUDGET;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(StartupArgsForwardingError::TimedOut);
        }

        // The attempt itself is capped, so that one connect which never resolves cannot outlast
        // the budget the rest of this loop is keeping.
        let attempt = Client::connect(
            ConnectionAddress::from(pipe_name.to_owned()),
            background_executor.clone(),
        )
        .with_timeout(remaining)
        .await;

        match attempt {
            Ok(Ok(client)) => return Ok(client),
            Ok(Err(err)) if err.is_transient_connect_failure() => {
                log::debug!("Retrying connection to the running instance of Warp: {err}");
                let until_deadline = deadline.saturating_duration_since(Instant::now());
                Timer::after(CONNECT_RETRY_INTERVAL.min(until_deadline)).await;
            }
            Ok(Err(err)) => return Err(err.into()),
            Err(_) => return Err(StartupArgsForwardingError::TimedOut),
        }
    }
}

/// Forwards the given URLs to the main running instance of Warp.
pub(super) async fn forward_uri_to_sole_running_instance(
    urls: Vec<Url>,
) -> Result<(), StartupArgsForwardingError> {
    // We need to construct a new background executor because this function is
    // run before we have a `AppContext`.  We explicitly create it with
    // a single backing thread, as we don't need an entire pool of threads.
    let background_executor = Arc::new(Background::new(1, |_| "forward-uris".to_owned()));
    let client =
        connect_to_sole_running_instance(&uri_named_pipe_name(), background_executor).await?;
    allow_existing_instance_to_take_foreground();
    let uri_service_caller = ipc::service_caller::<UriService>(Arc::new(client));
    uri_service_caller.call(urls).await?;
    Ok(())
}
