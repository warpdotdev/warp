#[cfg(feature = "local_fs")]
pub mod command;
pub mod env;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use futures::future::BoxFuture;

use crate::hooks::model::{HookBindingId, HookSourceId, HookSourceKind, HookSourceLocation};

/// Returns the login shell's PATH so hook processes can find user-installed tools, or `None` when
/// the PATH has not been captured yet and the runner should fall back to its own process PATH.
pub type PathCapture = Arc<dyn Fn() -> Option<OsString> + Send + Sync>;

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
    /// Whether `stdout` is only the start of what the hook printed because it went over the
    /// runner's limit. Adapters treat a cut-off response as malformed output.
    pub stdout_truncated: bool,
    /// The end of stderr, bounded so it can be logged safely.
    pub stderr_tail: String,
    pub duration: Duration,
}

/// The directory a hook from `source` runs in, and therefore the one a relative command resolves
/// against: the project root for a project file, the directory that holds the file for a
/// run-delivered or factory source. `None` when neither is known; such a binding must not
/// activate, because the runner never falls back to its own working directory.
pub fn working_directory_for(
    source: &HookSourceId,
    project_root: Option<&Path>,
) -> Option<PathBuf> {
    let declaring_dir = match &source.location {
        HookSourceLocation::CanonicalPath(path) => path.parent().map(Path::to_path_buf),
        HookSourceLocation::Named(_) => None,
    };
    match source.kind {
        HookSourceKind::ProjectCursorFile => project_root.map(Path::to_path_buf).or_else(|| {
            // A project file lives at `<root>/.cursor/hooks.json`, so the root is the parent of
            // the directory that holds it.
            declaring_dir
                .as_deref()
                .and_then(Path::parent)
                .map(Path::to_path_buf)
        }),
        HookSourceKind::RunDelivered
        | HookSourceKind::FactoryAgent
        | HookSourceKind::FactoryWide => declaring_dir,
    }
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
            stdout_truncated: false,
            stderr_tail: String::new(),
            duration: Duration::ZERO,
        }))
    }
}

#[cfg(test)]
#[path = "mod_test.rs"]
mod tests;
