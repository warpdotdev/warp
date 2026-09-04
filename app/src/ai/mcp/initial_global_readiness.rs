use std::collections::HashSet;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

use regex::Regex;
use strum::IntoEnumIterator;
use uuid::Uuid;
use warp_util::sync::Condition;

use super::MCPProvider;

static HOME_SUBDIR_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([^/]+)/[^/]+$").expect("Regex is valid"));

/// Returns the subdirectory under the home directory that needs its own directory watcher,
/// inferred from the provider's home config path. Matches paths that are exactly one directory
/// deep (e.g. `.codex/config.toml` → `.codex`). Returns `None` when the config file lives
/// directly in the home dir (e.g. `.claude.json`).
pub(crate) fn home_subdir_to_watch(provider: MCPProvider) -> Option<PathBuf> {
    let path_str = provider.home_config_path().to_str()?;
    HOME_SUBDIR_REGEX
        .captures(path_str)
        .and_then(|caps| caps.get(1))
        .map(|m| PathBuf::from(m.as_str()))
}

/// Late-subscriber-safe latch for the one-time initial global file-based MCP scan.
///
/// [`Condition`] is set exactly once when the scan settles. Waiters that subscribe after
/// that still observe completion immediately, with the frozen auto-start UUID list.
#[derive(Clone, Debug)]
pub struct InitialGlobalMcpReadiness {
    complete: Condition,
    result: Arc<Mutex<Option<Vec<Uuid>>>>,
}

impl InitialGlobalMcpReadiness {
    pub fn pending() -> Self {
        Self {
            complete: Condition::new(),
            result: Arc::new(Mutex::new(None)),
        }
    }

    pub fn complete_empty() -> Self {
        let latch = Self::pending();
        latch.complete(Vec::new());
        latch
    }

    /// Freeze the wait set and wake every waiter. Idempotent.
    pub fn complete(&self, wait_server_uuids: Vec<Uuid>) {
        let mut result = self.result.lock().unwrap_or_else(|err| err.into_inner());
        if result.is_some() {
            return;
        }
        *result = Some(wait_server_uuids);
        drop(result);
        self.complete.set();
    }

    #[cfg(test)]
    pub fn result(&self) -> Option<Vec<Uuid>> {
        self.result
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    pub fn is_complete(&self) -> bool {
        self.complete.is_set()
    }

    pub fn wait(&self) -> impl Future<Output = Vec<Uuid>> + use<> {
        let complete = self.complete.clone();
        let result = Arc::clone(&self.result);
        async move {
            complete.wait().await;
            result
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .clone()
                .unwrap_or_default()
        }
    }
}

impl Default for InitialGlobalMcpReadiness {
    fn default() -> Self {
        Self::pending()
    }
}

/// Global home-config sources owed by the one-time startup scan, plus whether
/// completion has already been emitted.
///
/// Continuous filesystem watching is independent of this set: a source is
/// removed exactly once it produces a first terminal parse outcome.
#[derive(Clone, Debug, Default)]
pub struct InitialGlobalScanCohort {
    pending: HashSet<(PathBuf, MCPProvider)>,
    emitted: bool,
}

impl InitialGlobalScanCohort {
    pub fn from_pending(pending: HashSet<(PathBuf, MCPProvider)>) -> Self {
        Self {
            pending,
            emitted: false,
        }
    }

    pub fn contains(&self, source: &(PathBuf, MCPProvider)) -> bool {
        self.pending.contains(source)
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    #[cfg(test)]
    pub fn has_emitted(&self) -> bool {
        self.emitted
    }

    /// Remove `source` if it was still owed.
    pub fn remove(&mut self, source: &(PathBuf, MCPProvider)) -> bool {
        self.pending.remove(source)
    }

    /// Mark completion if every owed source has settled. Returns whether the caller
    /// should emit the completion event.
    pub fn try_complete(&mut self) -> bool {
        if self.emitted || !self.pending.is_empty() {
            return false;
        }
        self.emitted = true;
        true
    }
}

#[derive(Debug, Default)]
pub(crate) struct InitialGlobalScanPlan {
    pub pending: HashSet<(PathBuf, MCPProvider)>,
    pub direct_parses: Vec<(PathBuf, PathBuf, MCPProvider)>,
    pub watch_subdirs: Vec<(PathBuf, PathBuf)>,
}

/// Pure description of which global home-config sources the startup scan owes, and
/// whether each should be read by a direct parse or by watching an existing subdir.
///
/// `subdir_is_present` is injected so tests can assert cohort membership without
/// constructing a live watcher whose queued scans race with construction.
pub(crate) fn plan_initial_global_scan(
    home_dir: Option<PathBuf>,
    warp_config: Option<(PathBuf, PathBuf)>,
    subdir_is_present: impl Fn(&Path) -> bool,
) -> InitialGlobalScanPlan {
    let mut plan = InitialGlobalScanPlan::default();
    if let Some((config_path, root_path)) = warp_config {
        plan.pending
            .insert((config_path.clone(), MCPProvider::Warp));
        plan.direct_parses
            .push((config_path, root_path, MCPProvider::Warp));
    }
    let Some(home_dir) = home_dir else {
        return plan;
    };
    for provider in MCPProvider::iter() {
        if provider == MCPProvider::Warp {
            continue;
        }
        let config_path = home_dir.join(provider.home_config_path());
        plan.pending.insert((config_path.clone(), provider));
        match home_subdir_to_watch(provider) {
            None => {
                plan.direct_parses
                    .push((config_path, home_dir.clone(), provider));
            }
            Some(subdir) => {
                let subdir_path = home_dir.join(&subdir);
                if subdir_is_present(&subdir_path) {
                    if !plan
                        .watch_subdirs
                        .iter()
                        .any(|(path, _)| path == &subdir_path)
                    {
                        plan.watch_subdirs.push((subdir_path, home_dir.clone()));
                    }
                } else {
                    plan.direct_parses
                        .push((config_path, home_dir.clone(), provider));
                }
            }
        }
    }
    plan
}

#[cfg(test)]
#[path = "initial_global_readiness_tests.rs"]
mod tests;
