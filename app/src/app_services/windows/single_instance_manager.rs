use std::sync::{Arc, LazyLock};

use async_channel::Receiver;
use ipc::ServerBuilder;
use parking_lot::Mutex;
use url::Url;
use warp_core::channel::ChannelState;
use warp_errors::report_error;
use warpui::r#async::executor::Background;
use warpui::{Entity, ModelContext, SingletonEntity};
use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
use windows::Win32::System::Threading::{CreateMutexW, OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE};
use windows::core::{Error, PCWSTR};

use super::service_impl::UriServiceImpl;

/// How many forwarded hand-offs to hold while the app finishes initializing.
///
/// Bounded because the listener accepts connections from the moment the claim is taken, long
/// before anything drains it. Overflow is reported back to the sender, which then handles its own
/// startup arguments, so the cost of this being too small is a duplicate window rather than lost
/// work or unbounded growth.
const MAX_BUFFERED_HANDOFFS: usize = 32;

/// RAII wrapper around a Windows mutex HANDLE that closes it on drop.
struct MutexHandle(HANDLE);

// SAFETY: Windows kernel mutexes are valid to use from any thread. For example it says here:
// https://learn.microsoft.com/en-us/windows/win32/api/synchapi/nf-synchapi-createmutexw#remarks
// > "Any thread of the calling process can specify the mutex-object handle in a call to one of the
//   wait functions"
// The [`HANDLE`] is not Send or Sync b/c it's a common type used to point to a variety of Windows
// kernel objects, many of which are not safe to access from other threads.
unsafe impl Send for MutexHandle {}
unsafe impl Sync for MutexHandle {}

impl Drop for MutexHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// The role this process plays in single-instance enforcement, resolved on first access and fixed
/// for the process lifetime.
static INSTANCE_ROLE: LazyLock<Mutex<Result<InstanceRole, Error>>> =
    LazyLock::new(|| Mutex::new(claim_sole_instance()));

enum InstanceRole {
    /// This process owns the single-instance claim and is listening for hand-offs.
    Sole(SoleInstance),
    /// Another process owns the claim, so this launch hands its startup arguments over instead of
    /// starting a second GUI.
    Secondary,
    /// This process could not listen for hand-offs, so it never took the claim. It runs as a full
    /// instance, but later launches will not find it and will start their own.
    Undiscoverable,
}

/// The resources that make this process the sole instance, held for the process lifetime.
struct SoleInstance {
    server: Option<ipc::Server>,
    /// Drives the server's accept loop. The claim is taken before there is an `AppContext` to
    /// borrow an executor from, so the claim owns one.
    executor: Option<Arc<Background>>,
    /// Declared after the listener so that the claim outlives it: releasing the mutex while the
    /// pipe is still bound advertises a free role to a launch that then cannot bind, leaving it
    /// running with neither the claim nor a listener.
    _mutex: MutexHandle,
    forwarded_uris: Receiver<Vec<Url>>,
}

impl Drop for SoleInstance {
    /// Field order alone is not enough: dropping an [`ipc::Server`] detaches its tasks rather than
    /// stopping them, so the pipe would outlive the drop. Wait for the listener to actually go
    /// away before the mutex is dropped and the role becomes available again.
    ///
    /// This runs only where the claim is dropped explicitly. A process exiting normally never gets
    /// here, because the claim lives in a `static`, and the OS reclaims both objects in an order
    /// this code does not control.
    ///
    /// Waiting is blocking, so it must not run on the listener's own single-threaded executor: that
    /// worker would be parked here instead of polling the tasks it just aborted.
    fn drop(&mut self) {
        if let Some(server) = self.server.take() {
            warpui::r#async::block_on(server.shutdown());
        }
        self.executor.take();
    }
}

/// A bound URI listener, before it is known whether this process may keep it.
struct UriListener {
    server: ipc::Server,
    executor: Arc<Background>,
    forwarded_uris: Receiver<Vec<Url>>,
}

pub(super) fn uri_named_pipe_name() -> String {
    format!("Warp{:?}_URI_CHANNEL", ChannelState::channel())
}

/// The name of the single-instance mutex.
///
/// NOTE: This name must stay in sync with `AppMutexName` in
/// `script/windows/windows-installer.iss`, which the installer uses to detect whether Warp is
/// running.
fn single_instance_mutex_name() -> String {
    // Scope this lock to the specific user session.
    // https://learn.microsoft.com/en-us/windows/win32/termserv/kernel-object-namespaces
    // > "client processes can use the "Local\" prefix to explicitly create an object in their
    //   session namespace"
    format!("Local\\Warp{:?}_SingleInstance", ChannelState::channel())
}

fn to_nul_terminated_utf16(value: &str) -> Vec<u16> {
    value
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<u16>>()
}

/// Acquires the single-instance mutex, returning `None` when another process already holds it.
fn try_acquire_mutex(name: &str) -> Result<Option<MutexHandle>, Error> {
    let name = to_nul_terminated_utf16(name);
    let handle = unsafe { CreateMutexW(None, true, PCWSTR(name.as_ptr())) };

    // https://learn.microsoft.com/en-us/windows/win32/api/synchapi/nf-synchapi-createmutexw#return-value
    let already_exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    handle
        .inspect_err(|err| {
            report_error!(
                anyhow::Error::new(err.clone()).context("Failed to create single-instance mutex")
            );
        })
        .map(|handle| {
            if already_exists {
                unsafe {
                    let _ = CloseHandle(handle);
                }
                None
            } else {
                Some(MutexHandle(handle))
            }
        })
}

/// Tests whether the single-instance mutex exists without acquiring it.
#[cfg(test)]
fn mutex_exists(name: &str) -> bool {
    let name = to_nul_terminated_utf16(name);
    let handle = unsafe { OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, false, PCWSTR(name.as_ptr())) };
    match handle {
        Ok(handle) => {
            unsafe {
                let _ = CloseHandle(handle);
            }
            true
        }
        Err(_) => false,
    }
}

/// Binds the URI pipe and starts accepting on it.
fn bind_uri_listener(pipe_name: &str) -> anyhow::Result<UriListener> {
    let executor = Arc::new(Background::new(1, |_| "uri-server".to_owned()));
    let (tx, forwarded_uris) = async_channel::bounded(MAX_BUFFERED_HANDOFFS);
    let (server, _) = ServerBuilder::default()
        .with_fixed_address(pipe_name.to_owned())
        .with_service(UriServiceImpl::new(tx))
        .build_and_run(executor.clone())
        .map_err(anyhow::Error::new)?;
    Ok(UriListener {
        server,
        executor,
        forwarded_uris,
    })
}

fn claim_sole_instance() -> Result<InstanceRole, Error> {
    claim_instance(&single_instance_mutex_name(), &uri_named_pipe_name())
}

/// Determines this process's [`InstanceRole`] by acquiring the mutex and then binding the pipe.
///
/// These are two kernel objects and no ordering makes acquiring them atomic, so a concurrent
/// launch can always observe one without the other. This order is chosen because of which
/// conclusion the in-between state leads to: with the mutex first, a launch that arrives mid-claim
/// decides "an instance exists" and waits out its connect budget, where the reverse order would
/// have it decide "no instance" and start a duplicate immediately. Binding here rather than later
/// during GUI initialization is what keeps that wait to the gap between these two statements.
///
/// Releasing the mutex when the bind fails follows the same reasoning: a claim that this process
/// cannot serve is worse than no claim at all.
fn claim_instance(mutex_name: &str, pipe_name: &str) -> Result<InstanceRole, Error> {
    let Some(mutex) = try_acquire_mutex(mutex_name)? else {
        return Ok(InstanceRole::Secondary);
    };

    match bind_uri_listener(pipe_name) {
        Ok(listener) => Ok(InstanceRole::Sole(SoleInstance {
            server: Some(listener.server),
            executor: Some(listener.executor),
            _mutex: mutex,
            forwarded_uris: listener.forwarded_uris,
        })),
        Err(err) => {
            report_error!(err.context("Failed to initialize UriService Server"));
            drop(mutex);
            Ok(InstanceRole::Undiscoverable)
        }
    }
}

/// A singleton model that is responsible for ensuring there is only one instance of Warp running.
/// Uses a Windows named mutex (via `CreateMutexW`) which is a kernel object automatically cleaned
/// up by the OS when all handles are closed, including on crash.
pub(super) struct SingleInstanceManager {}

impl SingleInstanceManager {
    /// Starts handling the URIs that later launches have handed to this process, including any
    /// that arrived while the app was still initializing.
    pub(super) fn new(ctx: &mut ModelContext<Self>) -> Self {
        let forwarded_uris = match &*INSTANCE_ROLE.lock() {
            Ok(InstanceRole::Sole(sole_instance)) => sole_instance.forwarded_uris.clone(),
            Ok(InstanceRole::Secondary | InstanceRole::Undiscoverable) | Err(_) => {
                return Self {};
            }
        };

        ctx.spawn_stream_local(
            forwarded_uris,
            |_single_instance_manager, uris, ctx| {
                for uri in uris {
                    crate::uri::handle_incoming_uri(&uri, ctx);
                }
            },
            |_, _| {},
        );

        Self {}
    }

    /// Whether another instance of Warp holds the single-instance claim, meaning this launch should
    /// hand its startup arguments to that instance instead of starting a second GUI.
    ///
    /// `Undiscoverable` reports `false`: that process is not the sole instance, but there is no
    /// other instance to hand over to either, so it has to launch.
    pub(super) fn has_existing_instance() -> Result<bool, Error> {
        match &*INSTANCE_ROLE.lock() {
            Ok(InstanceRole::Secondary) => Ok(true),
            Ok(InstanceRole::Sole(_) | InstanceRole::Undiscoverable) => Ok(false),
            Err(err) => Err(err.clone()),
        }
    }
}

impl Entity for SingleInstanceManager {
    type Event = ();
}

impl SingletonEntity for SingleInstanceManager {}

#[cfg(test)]
#[path = "single_instance_manager_tests.rs"]
mod tests;
