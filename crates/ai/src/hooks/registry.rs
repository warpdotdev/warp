use std::path::PathBuf;
use std::sync::Arc;

use warpui_core::{Entity, SingletonEntity};

use crate::hooks::adapters::{DeclaredBinding, HookConfigError, HookFormatAdapter};
#[cfg(feature = "local_fs")]
use crate::hooks::adapters::{ParsedHookConfig, adapter_for};
#[cfg(feature = "local_fs")]
use crate::hooks::model::{
    DiagnosticSeverity, HookSourceId, HookSourceLocation, validate_capabilities,
};
use crate::hooks::model::{HookBoundary, HookDiagnostic, HookExecutionHost, HookSourcePolicy};
#[cfg(feature = "local_fs")]
use crate::hooks::sources::ExplicitHookSource;

/// A binding from a configured source that is ready to run.
#[derive(Clone)]
pub struct ActiveBinding {
    pub binding: DeclaredBinding,
    pub adapter: Arc<dyn HookFormatAdapter>,
    pub host: HookExecutionHost,
    /// The directory the hook command runs in and resolves relative paths against.
    pub working_directory: PathBuf,
    /// The project directory the hook is told it runs for, when its source fixes one.
    pub project_dir: Option<PathBuf>,
    pub policy: HookSourcePolicy,
}

impl std::fmt::Debug for ActiveBinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActiveBinding")
            .field("binding", &self.binding)
            .field("host", &self.host)
            .field("working_directory", &self.working_directory)
            .field("project_dir", &self.project_dir)
            .field("policy", &self.policy)
            .finish_non_exhaustive()
    }
}

/// An immutable view of the active hook set. Dispatch uses one snapshot for a boundary firing.
#[derive(Debug, Default)]
pub struct RegistrySnapshot {
    pub revision: u64,
    pub bindings: Vec<ActiveBinding>,
    pub diagnostics: Vec<HookDiagnostic>,
}

impl RegistrySnapshot {
    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }

    /// Bindings for `boundary` on `host`, sorted by source priority with the source's own order
    /// preserved within a priority.
    pub fn bindings_for(
        &self,
        boundary: HookBoundary,
        host: HookExecutionHost,
    ) -> Vec<ActiveBinding> {
        let mut matching = self
            .bindings
            .iter()
            .filter(|active| active.binding.boundary == boundary && active.host == host)
            .cloned()
            .collect::<Vec<_>>();
        matching.sort_by_key(|active| active.binding.id.source.kind.priority());
        matching
    }

    /// Whether any binding exists for `boundary` at all, before matcher filtering.
    pub fn has_bindings_for(&self, boundary: HookBoundary) -> bool {
        self.bindings
            .iter()
            .any(|active| active.binding.boundary == boundary)
    }
}

/// Holds the bindings selected for this worker and freezes them before hooks may execute.
pub struct HookRegistry {
    snapshot: Arc<RegistrySnapshot>,
    finalized: bool,
}

impl Default for HookRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl HookRegistry {
    pub fn new() -> Self {
        Self {
            snapshot: Arc::new(RegistrySnapshot::default()),
            finalized: false,
        }
    }

    pub fn snapshot(&self) -> Arc<RegistrySnapshot> {
        self.snapshot.clone()
    }

    pub fn revision(&self) -> u64 {
        self.snapshot.revision
    }

    /// Whether the resolved source set has been frozen for this worker run.
    pub fn is_finalized(&self) -> bool {
        self.finalized
    }

    /// Freezes the resolved source set. A worker process finalizes exactly once, including when
    /// the resolved set is empty.
    pub fn finalize(&mut self) -> Result<u64, HookConfigError> {
        if self.finalized {
            return Err(HookConfigError::Invalid(
                "the resolved hook source set is already finalized".to_owned(),
            ));
        }
        self.finalized = true;
        Ok(self.snapshot.revision)
    }

    pub fn bindings_for(
        &self,
        boundary: HookBoundary,
        host: HookExecutionHost,
    ) -> Vec<ActiveBinding> {
        self.snapshot.bindings_for(boundary, host)
    }

    /// Replaces the active set with the given bindings, bumps the revision, and finalizes.
    #[cfg(any(test, feature = "test-util"))]
    pub fn install_snapshot_for_test(
        &mut self,
        bindings: Vec<ActiveBinding>,
        diagnostics: Vec<HookDiagnostic>,
    ) -> u64 {
        let revision = self.stage_snapshot_for_test(bindings, diagnostics);
        self.finalized = true;
        revision
    }

    /// Replaces the active set with the given bindings and bumps the revision without
    /// finalizing, as the loader does before it has finished.
    #[cfg(any(test, feature = "test-util"))]
    pub fn stage_snapshot_for_test(
        &mut self,
        bindings: Vec<ActiveBinding>,
        diagnostics: Vec<HookDiagnostic>,
    ) -> u64 {
        let revision = self.snapshot.revision + 1;
        self.snapshot = Arc::new(RegistrySnapshot {
            revision,
            bindings,
            diagnostics,
        });
        revision
    }
}

/// Loading one configured file. Explicit sources are selected by the resolved run configuration,
/// so they require no source-location trust decision and apply everywhere on the local host.
#[cfg(feature = "local_fs")]
impl HookRegistry {
    /// Reads `source`'s configuration file, parses it with the adapter for its format, and
    /// publishes a new snapshot in which this file's bindings replace any loaded from the same
    /// file before. The bindings run in the source's working directory under its policy. This
    /// neither merges duplicate selections nor finalizes; production code loads through
    /// [`crate::hooks::load_explicit_sources`].
    pub(crate) fn install_explicit_source(
        &mut self,
        source: &ExplicitHookSource,
    ) -> Result<(), HookConfigError> {
        if self.finalized {
            return Err(HookConfigError::Invalid(
                "explicit hook sources must be loaded before the resolved set is finalized"
                    .to_owned(),
            ));
        }
        let path = &source.config_file;
        let unreadable = |error: std::io::Error| {
            HookConfigError::Unreadable(format!("{}: {error}", path.display()))
        };
        let canonical_path = dunce::canonicalize(path).map_err(unreadable)?;
        let bytes = std::fs::read(&canonical_path).map_err(unreadable)?;
        let format = source.format;
        let adapter = adapter_for(format).ok_or_else(|| {
            HookConfigError::Invalid(format!("no adapter is registered for {format:?}"))
        })?;
        let source_id = HookSourceId {
            kind: source.kind,
            location: HookSourceLocation::CanonicalPath(canonical_path),
        };
        let config = adapter.parse(&bytes, &source_id)?;
        let (bindings, diagnostics) = activate(&adapter, &config, source);
        self.replace_source(&source_id, bindings, diagnostics);
        Ok(())
    }

    /// Records a source that the resolved configuration named but the loader could not install.
    pub(crate) fn record_load_failure(
        &mut self,
        source: HookSourceId,
        message: impl Into<String>,
    ) -> HookDiagnostic {
        let diagnostic = HookDiagnostic {
            source: source.clone(),
            severity: DiagnosticSeverity::Error,
            message: message.into(),
        };
        self.record_diagnostic(diagnostic.clone());
        diagnostic
    }

    /// Adds a diagnostic about a source without changing any bindings.
    pub(crate) fn record_diagnostic(&mut self, diagnostic: HookDiagnostic) {
        debug_assert!(!self.finalized);
        let previous = &self.snapshot;
        let mut diagnostics = previous.diagnostics.clone();
        diagnostics.push(diagnostic);
        self.snapshot = Arc::new(RegistrySnapshot {
            revision: previous.revision + 1,
            bindings: previous.bindings.clone(),
            diagnostics,
        });
    }

    /// Publishes a snapshot in which `source`'s bindings and diagnostics are the given ones and
    /// every other source is unchanged.
    fn replace_source(
        &mut self,
        source: &HookSourceId,
        bindings: Vec<ActiveBinding>,
        diagnostics: Vec<HookDiagnostic>,
    ) {
        let previous = &self.snapshot;
        let mut next_bindings = previous
            .bindings
            .iter()
            .filter(|active| active.binding.id.source.location != source.location)
            .cloned()
            .collect::<Vec<_>>();
        next_bindings.extend(bindings);
        let mut next_diagnostics = previous
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.source.location != source.location)
            .cloned()
            .collect::<Vec<_>>();
        next_diagnostics.extend(diagnostics);
        self.snapshot = Arc::new(RegistrySnapshot {
            revision: previous.revision + 1,
            bindings: next_bindings,
            diagnostics: next_diagnostics,
        });
    }
}

/// Turns a parsed source into bindings that apply everywhere on the local host and run where
/// `source` says, plus the source's parse warnings and one `Info` diagnostic per capability the
/// boundary cannot honor yet. Those are `Info` because they only predict what a hook might emit;
/// `Warning` is for something that has already changed what the source does.
#[cfg(feature = "local_fs")]
fn activate(
    adapter: &Arc<dyn HookFormatAdapter>,
    config: &ParsedHookConfig,
    source: &ExplicitHookSource,
) -> (Vec<ActiveBinding>, Vec<HookDiagnostic>) {
    let mut diagnostics = config.warnings.clone();
    let mut bindings = Vec::new();
    for binding in adapter.bindings(config) {
        for warning in validate_capabilities(binding.boundary, &binding.capabilities) {
            diagnostics.push(HookDiagnostic {
                source: config.source.clone(),
                severity: DiagnosticSeverity::Info,
                message: format!("{}: {}", binding.id, warning.message),
            });
        }
        bindings.push(ActiveBinding {
            binding,
            adapter: adapter.clone(),
            host: HookExecutionHost::Local,
            working_directory: source.working_directory.clone(),
            project_dir: source.project_dir.clone(),
            policy: source.policy,
        });
    }
    (bindings, diagnostics)
}

/// Emitted when a loader installs a new snapshot.
pub enum HookRegistryEvent {
    SnapshotChanged { revision: u64 },
}

impl Entity for HookRegistry {
    type Event = HookRegistryEvent;
}

impl SingletonEntity for HookRegistry {}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
