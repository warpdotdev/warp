//! Hook sources selected by the resolved agent or factory configuration.

use std::path::{Path, PathBuf};

use crate::hooks::model::{HookFormat, HookSourceKind};

/// Extra hook directories for a run, as a comma-separated list of paths. Each directory holds a
/// `hooks.json`. Relative entries are resolved against the run's working directory.
pub const WARP_HOOK_DIRS_ENV: &str = "WARP_HOOK_DIRS";

const HOOKS_FILE_NAME: &str = "hooks.json";

/// Which part of the resolved configuration selected an explicit source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExplicitSourceScope {
    /// Named by the run's own configuration.
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

/// A hook configuration file the resolved run configuration selected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExplicitHookSource {
    pub path: PathBuf,
    pub format: HookFormat,
    pub scope: ExplicitSourceScope,
}

/// Parses a `WARP_HOOK_DIRS` value. Entries are split on commas and trimmed, blank entries are
/// dropped, and a leading `~` is expanded to the home directory.
pub fn parse_hook_dirs_value(value: &str) -> Vec<PathBuf> {
    value
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(|entry| PathBuf::from(shellexpand::tilde(entry).into_owned()))
        .collect()
}

/// Resolves relative hook directories against the run's working directory.
pub fn resolve_hook_dirs(base: &Path, dirs: Vec<PathBuf>) -> Vec<PathBuf> {
    dirs.into_iter()
        .map(|dir| {
            if dir.is_absolute() {
                dir
            } else {
                base.join(dir)
            }
        })
        .collect()
}

/// Converts configured hook directories to their `hooks.json` sources.
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
    use super::ExplicitHookSource;
    use crate::hooks::model::{
        DiagnosticSeverity, HookDiagnostic, HookSourceId, HookSourceKind, HookSourceLocation,
    };
    use crate::hooks::registry::HookRegistry;

    /// Loads and freezes the complete explicit source list selected for a worker run.
    ///
    /// The returned errors let the caller decide whether the run may continue with the valid
    /// subset. The registry is finalized even when the list is empty or contains errors.
    pub fn load_explicit_sources(
        sources: &[ExplicitHookSource],
        registry: &mut HookRegistry,
    ) -> Vec<HookDiagnostic> {
        if registry.is_finalized() {
            return vec![finalization_diagnostic(
                "the resolved hook source set is already finalized",
            )];
        }

        let mut failures = Vec::new();
        for source in sources {
            let source_id = source_id(source);
            match registry.install_explicit_source(
                &source.path,
                source.format,
                source.scope.source_kind(),
            ) {
                Ok(()) => {
                    failures.extend(
                        registry
                            .snapshot()
                            .diagnostics
                            .iter()
                            .filter(|diagnostic| {
                                diagnostic.source == source_id
                                    && diagnostic.severity == DiagnosticSeverity::Error
                            })
                            .cloned(),
                    );
                }
                Err(error) => {
                    let diagnostic = registry.record_load_failure(
                        source_id,
                        format!("could not load configured hook source: {error}"),
                    );
                    failures.push(diagnostic);
                }
            }
        }

        if let Err(error) = registry.finalize() {
            failures.push(finalization_diagnostic(error.to_string()));
        }
        failures
    }

    fn source_id(source: &ExplicitHookSource) -> HookSourceId {
        let location = dunce::canonicalize(&source.path)
            .map(HookSourceLocation::CanonicalPath)
            .unwrap_or_else(|_| HookSourceLocation::Named(source.path.display().to_string()));
        HookSourceId {
            kind: source.scope.source_kind(),
            location,
        }
    }

    fn finalization_diagnostic(message: impl Into<String>) -> HookDiagnostic {
        HookDiagnostic {
            source: HookSourceId {
                kind: HookSourceKind::RunDelivered,
                location: HookSourceLocation::Named("resolved hook configuration".to_owned()),
            },
            severity: DiagnosticSeverity::Error,
            message: message.into(),
        }
    }
}

#[cfg(test)]
#[path = "sources_tests.rs"]
mod tests;
