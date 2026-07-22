use std::sync::LazyLock;

use ipc::ServerBuilder;
use parking_lot::Mutex;
use warp_core::channel::ChannelState;
use warp_errors::report_error;
use warpui::{Entity, ModelContext, SingletonEntity};
use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
use windows::Win32::System::Threading::CreateMutexW;
use windows::core::Error;

use super::service_impl::UriServiceImpl;

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

/// The single-instance mutex handle. Lives for the process lifetime.
///
/// It's a complex type. Breaking it down:
/// * LazyLock - This type lets us go from un-initialized to initialized without `mut` and _not_
///   vice-versa.
/// * Mutex - Gives us interior mutability. Unlike `RefCell` it can be used in statics since it is
///   Sync. We don't actually need to access it on other threads though.
/// * Result - CreateMutexW might fail for reasons other than another process holding the lock. In
///   those cases, we store the error type.
/// * Option - `Some` if we are the sole instance, `None` if another instance holds the lock.
static SOLE_INSTANCE_MUTEX: LazyLock<Mutex<Result<Option<MutexHandle>, Error>>> =
    LazyLock::new(|| Mutex::new(try_create_mutex()));

pub(super) fn uri_named_pipe_name() -> String {
    format!("Warp{:?}_URI_CHANNEL", ChannelState::channel())
}

fn try_create_mutex() -> Result<Option<MutexHandle>, Error> {
    // Scope this lock to the specific user session.
    // https://learn.microsoft.com/en-us/windows/win32/termserv/kernel-object-namespaces
    // > "client processes can use the "Local\" prefix to explicitly create an object in their
    //   session namespace"
    //
    // NOTE: This lock name must stay in sync with `AppMutexName` in
    // `script/windows/windows-installer.iss`, which the installer uses to detect whether Warp is
    // running.
    let name = format!("Local\\Warp{:?}_SingleInstance", ChannelState::channel())
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<u16>>();
    let handle = unsafe { CreateMutexW(None, true, windows::core::PCWSTR(name.as_ptr())) };

    // https://learn.microsoft.com/en-us/windows/win32/api/synchapi/nf-synchapi-createmutexw#return-value
    let already_exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    handle
        .inspect_err(|err| {
            report_error!(
                anyhow::anyhow!("{err:#}").context("Failed to create single-instance mutex")
            );
        })
        .map(|handle| {
            if already_exists {
                // Another instance already owns this mutex. Close our duplicate handle.
                unsafe {
                    let _ = CloseHandle(handle);
                }
                None
            } else {
                Some(MutexHandle(handle))
            }
        })
}

/// A singleton model that is responsible for ensuring there is only one instance of Warp running.
/// Uses a Windows named mutex (via `CreateMutexW`) which is a kernel object automatically cleaned
/// up by the OS when all handles are closed, including on crash.
pub(super) struct SingleInstanceManager {
    _server: Option<ipc::Server>,
}

impl SingleInstanceManager {
    /// Attempts to upgrade the current Warp instance to the "main" instance (i.e. the one that
    /// holds the named mutex). This function enforces that a URI server is created iff the mutex
    /// is held.
    pub(super) fn new(ctx: &mut ModelContext<Self>) -> Self {
        if let Ok(None) | Err(_) = &*SOLE_INSTANCE_MUTEX.lock() {
            return Self { _server: None };
        }

        let (tx, rx) = async_channel::unbounded();
        let server = match ServerBuilder::default()
            .with_fixed_address(uri_named_pipe_name())
            .with_service(UriServiceImpl::new(tx))
            .build_and_run(ctx.background_executor())
        {
            Ok((server, _)) => {
                ctx.spawn_stream_local(
                    rx,
                    |_single_instance_manager, event, ctx| {
                        for uri in event {
                            crate::uri::handle_incoming_uri(&uri, ctx);
                        }
                    },
                    |_, _| {},
                );
                server
            }
            Err(err) => {
                report_error!(
                    anyhow::anyhow!("{err:#}").context("Failed to initialize UriService Server")
                );
                // If we failed to create a server, we can't receive URI requests so we drop the
                // lock.
                *SOLE_INSTANCE_MUTEX.lock() = Ok(None);
                return Self { _server: None };
            }
        };

        Self {
            _server: Some(server),
        }
    }

    /// Returns whether or not this process should be treated as the main instance of Warp.
    ///
    /// NOTE: If an unexpected error occurs, we return `true` since it's better to open a second
    /// instance than to fail to create a first instance.
    pub(super) fn is_sole_running_instance() -> Result<bool, Error> {
        SOLE_INSTANCE_MUTEX
            .lock()
            .as_ref()
            .map(|handle| handle.is_some())
            .map_err(Clone::clone)
    }
}

impl Entity for SingleInstanceManager {
    type Event = ();
}

impl SingletonEntity for SingleInstanceManager {}
