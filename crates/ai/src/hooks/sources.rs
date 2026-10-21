//! Sources named by whoever configured the run: hook directories passed through `WARP_HOOK_DIRS`,
//! `hooks` entries in the run's configuration file, and factory definitions. They are approved by
//! construction, so they activate without consulting the trust policy.

use std::path::{Path, PathBuf};

use crate::hooks::model::{HookFormat, HookSourceKind};

/// Extra hook directories for a run, as a comma-separated list of paths. Each directory holds a
/// `hooks.json`. Relative entries are resolved against the run's working directory.
pub const WARP_HOOK_DIRS_ENV: &str = "WARP_HOOK_DIRS";

/// The configuration file inside a hook directory or a project's `.cursor` directory.
pub const HOOKS_FILE_NAME: &str = "hooks.json";

/// Where a project keeps its Cursor hooks, relative to the project root.
pub const PROJECT_CURSOR_HOOKS_PATH: &str = ".cursor/hooks.json";

/// Who delivered an explicit source, which decides its merge priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExplicitSourceScope {
    /// Named by the run's own configuration: `WARP_HOOK_DIRS` or the configuration file.
    Run,
    /// A factory agent's own definition.
    FactoryAgent,
    /// A factory-wide definition shared by its agents.
    FactoryWide,
}

impl ExplicitSourceScope {
    pub fn source_kind(self) -> HookSourceKind {
        match self {
            Self::Run => HookSourceKind::RunDelivered,
            Self::FactoryAgent => HookSourceKind::FactoryAgent,
            Self::FactoryWide => HookSourceKind::FactoryWide,
        }
    }
}

/// A hook configuration file the run was told to load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExplicitHookSource {
    pub path: PathBuf,
    pub format: HookFormat,
    pub scope: ExplicitSourceScope,
}

/// Parses a `WARP_HOOK_DIRS` value. Entries are split on commas and trimmed, blank entries are
/// dropped, and a leading `~` is expanded to the home directory. Relative entries are returned as
/// written; see [`resolve_hook_dirs`].
pub fn parse_hook_dirs_value(value: &str) -> Vec<PathBuf> {
    value
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(|entry| PathBuf::from(shellexpand::tilde(entry).into_owned()))
        .collect()
}

/// Resolves relative hook directories against `base`, the run's working directory, so their
/// meaning does not depend on the process's own current directory.
pub fn resolve_hook_dirs(base: &Path, dirs: Vec<PathBuf>) -> Vec<PathBuf> {
    dirs.into_iter()
        .map(|dir| if dir.is_absolute() { dir } else { base.join(dir) })
        .collect()
}

/// The `hooks.json` inside each hook directory, as run-delivered Cursor sources.
pub fn sources_for_hook_dirs(dirs: &[PathBuf]) -> Vec<ExplicitHookSource> {
    dirs.iter()
        .map(|dir| ExplicitHookSource {
            path: dir.join(HOOKS_FILE_NAME),
            format: HookFormat::Cursor,
            scope: ExplicitSourceScope::Run,
        })
        .collect()
}

#[cfg(feature = "local_fs")]
pub use loader::load_explicit_sources;

#[cfg(feature = "local_fs")]
mod loader {
    use std::fs;

    use super::ExplicitHookSource;
    use crate::hooks::model::{HookDiagnostic, HookSourceId, HookSourceLocation};
    use crate::hooks::registry::{HookRegistry, SourceUpdate};
    use crate::hooks::trust::TrustDecision;

    /// Reads each explicit source and activates it. A missing or unreadable file is recorded in
    /// the registry as a failed load and returned, so the caller can decide whether the run may
    /// continue without it. A file reached through two paths is one source.
    pub fn load_explicit_sources(
        sources: &[ExplicitHookSource],
        registry: &mut HookRegistry,
    ) -> Vec<HookDiagnostic> {
        let mut failures = Vec::new();
        for source in sources {
            let kind = source.scope.source_kind();
            let contents = dunce::canonicalize(&source.path)
                .and_then(|canonical| fs::read(&canonical).map(|bytes| (canonical, bytes)));
            match contents {
                Ok((canonical, bytes)) => {
                    registry.upsert_source(SourceUpdate {
                        source: HookSourceId {
                            kind,
                            location: HookSourceLocation::CanonicalPath(canonical),
                        },
                        format: source.format,
                        bytes: &bytes,
                        project_root: None,
                        trust: TrustDecision::Activate,
                    });
                }
                Err(error) => {
                    let id = HookSourceId {
                        kind,
                        location: HookSourceLocation::Named(source.path.display().to_string()),
                    };
                    registry.record_read_failure(&id, &error.to_string());
                    failures.extend(
                        registry
                            .snapshot()
                            .diagnostics
                            .iter()
                            .filter(|diagnostic| diagnostic.source == id)
                            .cloned(),
                    );
                }
            }
        }
        failures
    }
}

#[cfg(test)]
#[path = "sources_tests.rs"]
mod tests;
