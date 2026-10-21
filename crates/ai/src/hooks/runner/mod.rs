use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use futures::future::BoxFuture;

use crate::hooks::model::HookBindingId;

/// Captures the login shell's PATH so hook processes can find user-installed tools. Supplied by
/// the application, which owns the shell state.
pub type PathCapture = fn() -> Option<OsString>;

/// One request to run a hook process.
#[derive(Debug, Clone)]
pub struct HookInvocation {
    pub binding: HookBindingId,
    pub command: String,
    pub cwd: PathBuf,
    pub env: Vec<(OsString, OsString)>,
    /// The JSON payload, written whole and then closed. Payload bytes are never placed in the
    /// command line.
    pub stdin: Vec<u8>,
    pub timeout: Duration,
    /// Set by the caller when the result is no longer wanted; the runner stops the process.
    pub cancel: Arc<AtomicBool>,
}

/// How a hook process ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessExit {
    Code(i32),
    Signal(i32),
    TimedOut,
    Cancelled,
    SpawnFailed(String),
}

/// What a hook process did, as seen by the runner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookProcessOutcome {
    pub exit: ProcessExit,
    pub stdout: Vec<u8>,
    /// The end of stderr, bounded so it can be logged safely.
    pub stderr_tail: String,
    pub duration: Duration,
}

/// Runs hook processes. Implementations own spawning, deadlines, output limits, and cleanup.
pub trait HookRunner: Send + Sync {
    fn run(&self, invocation: HookInvocation) -> BoxFuture<'static, HookProcessOutcome>;
}

/// A runner for builds and targets that cannot spawn processes. Every invocation fails to spawn,
/// which the adapters' failure policies then handle.
pub struct UnavailableRunner;

impl HookRunner for UnavailableRunner {
    fn run(&self, invocation: HookInvocation) -> BoxFuture<'static, HookProcessOutcome> {
        log::warn!(
            "hook {} was not run: no hook runner is available in this build",
            invocation.binding
        );
        Box::pin(futures::future::ready(HookProcessOutcome {
            exit: ProcessExit::SpawnFailed("no hook runner is available in this build".to_owned()),
            stdout: Vec::new(),
            stderr_tail: String::new(),
            duration: Duration::ZERO,
        }))
    }
}
