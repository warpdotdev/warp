//! Hook sources selected by the resolved agent or factory configuration.

use std::path::PathBuf;

use crate::hooks::adapters::{HookDirectory, source_layout};
use crate::hooks::model::{HookDiagnostic, HookFormat, HookSourceKind, HookSourcePolicy};

/// A hook configuration file the resolved configuration selected, with everything needed to load
/// it and run its hooks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExplicitHookSource {
    pub config_file: PathBuf,
    pub format: HookFormat,
    pub kind: HookSourceKind,
    /// The directory the source's commands run in and resolve relative paths against.
    pub working_directory: PathBuf,
    /// The project directory the source's hooks are told they run for, when the source fixes one.
    pub project_dir: Option<PathBuf>,
    pub policy: HookSourcePolicy,
}

impl ExplicitHookSource {
    /// The source `format` reads from `directory`, with the configuration file, working directory,
    /// and project directory that format derives from it.
    pub fn from_directory(
        format: HookFormat,
        directory: &HookDirectory,
        kind: HookSourceKind,
        policy: HookSourcePolicy,
    ) -> Self {
        let layout = source_layout(format, directory);
        Self {
            config_file: layout.config_file,
            format,
            kind,
            working_directory: layout.working_directory,
            project_dir: layout.project_dir,
            policy,
        }
    }
}

/// A selected source that could not take part in the run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLoadFailure {
    pub diagnostic: HookDiagnostic,
    /// Whether the source was strict, which makes the failure end the run before it starts.
    pub strict: bool,
}

#[cfg(feature = "local_fs")]
pub use loader::load_explicit_sources;

#[cfg(feature = "local_fs")]
mod loader {
    use std::path::{Path, PathBuf};

    use super::{ExplicitHookSource, SourceLoadFailure};
    use crate::hooks::adapters::HookConfigError;
    use crate::hooks::model::{
        DiagnosticSeverity, HookDiagnostic, HookSourceId, HookSourceLocation,
    };
    use crate::hooks::registry::HookRegistry;

    /// Loads and freezes the complete source list selected for a worker run.
    ///
    /// Entries that reach the same canonical file load once. The first entry in list order decides
    /// the source's kind, working directory, and project directory; its policy is the stricter of
    /// every such entry's. The registry is finalized even when the list is empty or some sources
    /// fail, so the returned failures let the caller decide whether the run may continue. Returns
    /// an error only when the registry was already finalized.
    pub fn load_explicit_sources(
        sources: &[ExplicitHookSource],
        registry: &mut HookRegistry,
    ) -> Result<Vec<SourceLoadFailure>, HookConfigError> {
        if registry.is_finalized() {
            return Err(HookConfigError::Invalid(
                "the resolved hook source set is already finalized".to_owned(),
            ));
        }

        let mut failures = Vec::new();
        for merged in merge_duplicates(sources) {
            let source = &merged.source;
            let source_id = source_id(source, merged.canonical.as_ref());
            match registry.install_explicit_source(source) {
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
                            .cloned()
                            .map(|diagnostic| SourceLoadFailure {
                                diagnostic,
                                strict: source.policy.strict,
                            }),
                    );
                    if !merged.other_working_directories.is_empty() {
                        registry.record_diagnostic(conflicting_locations_warning(
                            &source_id,
                            source,
                            &merged.other_working_directories,
                        ));
                    }
                }
                Err(error) => {
                    let diagnostic = registry.record_load_failure(
                        source_id,
                        format!("could not load configured hook source: {error}"),
                    );
                    failures.push(SourceLoadFailure {
                        diagnostic,
                        strict: source.policy.strict,
                    });
                }
            }
        }

        registry.finalize()?;
        Ok(failures)
    }

    /// One source to install after entries that reach the same file were merged.
    struct MergedSource {
        source: ExplicitHookSource,
        /// The canonical configuration file, when it resolves.
        canonical: Option<PathBuf>,
        /// Working directories later entries for the same file asked for that differ from the one
        /// in effect.
        other_working_directories: Vec<PathBuf>,
    }

    fn merge_duplicates(sources: &[ExplicitHookSource]) -> Vec<MergedSource> {
        let mut merged: Vec<MergedSource> = Vec::new();
        for source in sources {
            let canonical = dunce::canonicalize(&source.config_file).ok();
            let existing = canonical.as_ref().and_then(|canonical| {
                merged
                    .iter_mut()
                    .find(|entry| entry.canonical.as_ref() == Some(canonical))
            });
            match existing {
                Some(entry) => {
                    entry.source.policy = entry.source.policy.stricter(source.policy);
                    if !same_directory(&source.working_directory, &entry.source.working_directory)
                        && !entry
                            .other_working_directories
                            .contains(&source.working_directory)
                    {
                        entry
                            .other_working_directories
                            .push(source.working_directory.clone());
                    }
                }
                None => merged.push(MergedSource {
                    source: source.clone(),
                    canonical,
                    other_working_directories: Vec::new(),
                }),
            }
        }
        merged
    }

    fn same_directory(a: &Path, b: &Path) -> bool {
        let resolve = |path: &Path| dunce::canonicalize(path).unwrap_or_else(|_| path.to_owned());
        a == b || resolve(a) == resolve(b)
    }

    fn source_id(source: &ExplicitHookSource, canonical: Option<&PathBuf>) -> HookSourceId {
        let location = match canonical {
            Some(path) => HookSourceLocation::CanonicalPath(path.clone()),
            None => HookSourceLocation::Named(source.config_file.display().to_string()),
        };
        HookSourceId {
            kind: source.kind,
            location,
        }
    }

    fn conflicting_locations_warning(
        source_id: &HookSourceId,
        source: &ExplicitHookSource,
        others: &[PathBuf],
    ) -> HookDiagnostic {
        let others = others
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        HookDiagnostic {
            source: source_id.clone(),
            severity: DiagnosticSeverity::Warning,
            message: format!(
                "this hooks file was selected more than once with different locations; its hooks \
                 run from {} (also selected to run from {others})",
                source.working_directory.display()
            ),
        }
    }
}

#[cfg(test)]
#[path = "sources_tests.rs"]
mod tests;
