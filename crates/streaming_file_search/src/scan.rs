//! Candidate producers for the streaming file search engine.
//!
//! For git repositories, tracked and untracked files are enumerated with
//! `git ls-files -z --cached --others --exclude-standard` (fast even on
//! 100k+ file repositories) while a parallel [`ignore`] walk enumerates
//! directories. For non-git directories a single parallel walk enumerates
//! both files and directories. All candidates are streamed into the nucleo
//! [`Injector`] as they are discovered.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf, MAIN_SEPARATOR};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, SystemTime};

use command::blocking::Command;
use ignore::{WalkBuilder, WalkState};
use nucleo::Injector;

use crate::FileCandidate;

/// Shared state between a running scan and its [`ScanHandle`].
struct ScanState {
    cancelled: AtomicBool,
    complete: Mutex<bool>,
    condvar: Condvar,
}

impl ScanState {
    fn new() -> Self {
        Self {
            cancelled: AtomicBool::new(false),
            complete: Mutex::new(false),
            condvar: Condvar::new(),
        }
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    fn mark_complete(&self) {
        *self.complete.lock().expect("scan completion lock poisoned") = true;
        self.condvar.notify_all();
    }
}

/// Handle to a background filesystem scan.
pub(crate) struct ScanHandle {
    state: Arc<ScanState>,
}

impl ScanHandle {
    /// Signals producer threads to stop. Threads exit at the next candidate
    /// boundary; this does not block.
    pub(crate) fn cancel(&self) {
        self.state.cancelled.store(true, Ordering::Relaxed);
    }

    pub(crate) fn is_complete(&self) -> bool {
        *self
            .state
            .complete
            .lock()
            .expect("scan completion lock poisoned")
    }

    /// Blocks until the scan finishes or `timeout` elapses, whichever comes
    /// first. Used for the synchronous burst on engine construction.
    pub(crate) fn wait_for_completion(&self, timeout: Duration) {
        let complete = self
            .state
            .complete
            .lock()
            .expect("scan completion lock poisoned");
        let _guard = self
            .state
            .condvar
            .wait_timeout_while(complete, timeout, |complete| !*complete)
            .expect("scan completion lock poisoned");
    }
}

/// Returns the mtime of the repository's `.git/index`, used to detect when a
/// completed scan has gone stale. Falls back to the `.git` path itself (e.g.
/// for worktrees, where `.git` is a file pointing at the real git dir).
/// Returns `None` for non-git roots.
pub(crate) fn git_index_mtime(root: &Path) -> Option<SystemTime> {
    let git_path = root.join(".git");
    let index_mtime = std::fs::metadata(git_path.join("index")).and_then(|meta| meta.modified());
    index_mtime
        .or_else(|_| std::fs::metadata(&git_path).and_then(|meta| meta.modified()))
        .ok()
}

/// Starts producer threads that stream candidates under `root` into
/// `injector`. Returns immediately; use the returned [`ScanHandle`] to wait,
/// poll completion, or cancel.
pub(crate) fn start_scan(root: PathBuf, injector: Injector<FileCandidate>) -> ScanHandle {
    let state = Arc::new(ScanState::new());
    let handle = ScanHandle {
        state: state.clone(),
    };

    let spawn_result = std::thread::Builder::new()
        .name("file-search-scan".to_string())
        .spawn(move || {
            let is_git_repo = root.join(".git").exists();
            if is_git_repo {
                let git_thread = std::thread::Builder::new()
                    .name("file-search-git-ls-files".to_string())
                    .spawn({
                        let root = root.clone();
                        let injector = injector.clone();
                        let state = state.clone();
                        move || stream_git_files(&root, &injector, &state)
                    });
                // Directories are not covered by `git ls-files`; walk them in
                // parallel with the git enumeration.
                walk(&root, &injector, &state, /* directories_only */ true);
                match git_thread {
                    Ok(git_thread) => {
                        let _ = git_thread.join();
                    }
                    Err(err) => {
                        log::warn!("Failed to spawn git ls-files thread: {err:#}");
                        // Fall back to walking files as well so the engine
                        // still produces candidates.
                        walk(&root, &injector, &state, /* directories_only */ false);
                    }
                }
            } else {
                walk(&root, &injector, &state, /* directories_only */ false);
            }
            state.mark_complete();
        });
    if let Err(err) = spawn_result {
        log::warn!("Failed to spawn file search scan thread: {err:#}");
        // The scan never ran; mark it complete so callers do not wait on it.
        handle.state.mark_complete();
    }

    handle
}

/// Streams tracked and untracked (non-ignored) files from `git ls-files`.
fn stream_git_files(root: &Path, injector: &Injector<FileCandidate>, state: &ScanState) {
    let mut child = match Command::new("git")
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])
        .current_dir(root)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .stdin(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(err) => {
            log::warn!("Failed to spawn git ls-files: {err:#}");
            return;
        }
    };

    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return;
    };

    let reader = BufReader::new(stdout);
    for entry in reader.split(b'\0') {
        if state.is_cancelled() {
            let _ = child.kill();
            break;
        }
        let Ok(bytes) = entry else {
            break;
        };
        if bytes.is_empty() {
            continue;
        }
        let mut relative_path = String::from_utf8_lossy(&bytes).into_owned();
        // `git ls-files` always emits `/`-separated paths; normalize to the
        // platform separator to match the eager index's path format.
        if MAIN_SEPARATOR != '/' {
            relative_path = relative_path.replace('/', std::path::MAIN_SEPARATOR_STR);
        }
        push_candidate(injector, relative_path, false);
    }
    let _ = child.wait();
}

/// Walks `root` in parallel, respecting gitignore rules, streaming candidates
/// into `injector`. With `directories_only`, only directory entries are
/// emitted (used for git repositories, where files come from `git ls-files`).
fn walk(
    root: &Path,
    injector: &Injector<FileCandidate>,
    state: &ScanState,
    directories_only: bool,
) {
    let walker = WalkBuilder::new(root)
        // Include hidden entries (the eager index does too); `.git` itself is
        // filtered out below.
        .hidden(false)
        .follow_links(false)
        .filter_entry(|entry| entry.file_name() != ".git")
        .build_parallel();

    walker.run(|| {
        let injector = injector.clone();
        let root = root.to_path_buf();
        Box::new(move |entry| {
            if state.is_cancelled() {
                return WalkState::Quit;
            }
            let Ok(entry) = entry else {
                return WalkState::Continue;
            };
            let is_directory = entry.file_type().is_some_and(|ft| ft.is_dir());
            if directories_only && !is_directory {
                return WalkState::Continue;
            }
            let Ok(relative) = entry.path().strip_prefix(&root) else {
                return WalkState::Continue;
            };
            if relative.as_os_str().is_empty() {
                // The root itself.
                return WalkState::Continue;
            }
            let mut relative_path = relative.to_string_lossy().into_owned();
            if is_directory && !relative_path.ends_with(MAIN_SEPARATOR) {
                relative_path.push(MAIN_SEPARATOR);
            }
            push_candidate(&injector, relative_path, is_directory);
            WalkState::Continue
        })
    });
}

fn push_candidate(injector: &Injector<FileCandidate>, relative_path: String, is_directory: bool) {
    injector.push(
        FileCandidate {
            relative_path,
            is_directory,
        },
        |candidate, columns| {
            columns[0] = candidate.relative_path.as_str().into();
        },
    );
}
