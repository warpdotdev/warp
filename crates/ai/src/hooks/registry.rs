use std::path::{Path, PathBuf};
use std::sync::Arc;

use warpui_core::{Entity, SingletonEntity};

use crate::hooks::adapters::{DeclaredBinding, HookFormatAdapter};
#[cfg(feature = "local_fs")]
use crate::hooks::adapters::{HookConfigError, ParsedHookConfig, adapter_for};
#[cfg(feature = "local_fs")]
use crate::hooks::model::{
    DiagnosticSeverity, HookFormat, HookSourceId, HookSourceKind, HookSourceLocation,
    validate_capabilities,
};
use crate::hooks::model::{HookBoundary, HookDiagnostic, HookExecutionHost};

/// Which working directories a binding applies to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindingScope {
    /// Only operations whose working directory is inside this root.
    Root(PathBuf),
    /// Every operation on the host, regardless of working directory.
    Everywhere,
}

/// A binding whose source passed the trust policy and is ready to run.
#[derive(Clone)]
pub struct ActiveBinding {
    pub binding: DeclaredBinding,
    pub adapter: Arc<dyn HookFormatAdapter>,
    pub scope: BindingScope,
    pub host: HookExecutionHost,
    /// The directory the hook command runs in and resolves relative paths against.
    pub working_directory: PathBuf,
}

impl std::fmt::Debug for ActiveBinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActiveBinding")
            .field("binding", &self.binding)
            .field("scope", &self.scope)
            .field("host", &self.host)
            .field("working_directory", &self.working_directory)
            .finish_non_exhaustive()
    }
}

/// An immutable view of the active hook set. Dispatch pins one snapshot for the whole of a
/// boundary firing so a reload in the middle cannot change which hooks ran.
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

    /// Bindings for `boundary` that apply at `cwd` on `host`, sorted by source priority with the
    /// source's own order preserved within a priority.
    pub fn bindings_for(
        &self,
        boundary: HookBoundary,
        cwd: Option<&Path>,
        host: HookExecutionHost,
    ) -> Vec<ActiveBinding> {
        let mut matching = self
            .bindings
            .iter()
            .filter(|active| active.binding.boundary == boundary && active.host == host)
            .filter(|active| match &active.scope {
                BindingScope::Everywhere => true,
                BindingScope::Root(root) => cwd.is_some_and(|cwd| cwd.starts_with(root)),
            })
            .cloned()
            .collect::<Vec<_>>();
        matching.sort_by_key(|active| active.binding.id.source.kind.priority());
        matching
    }

    /// Whether any binding exists for `boundary` at all, before scope and matcher filtering.
    pub fn has_bindings_for(&self, boundary: HookBoundary) -> bool {
        self.bindings
            .iter()
            .any(|active| active.binding.boundary == boundary)
    }
}

/// Holds the current snapshot of active bindings. A loader replaces the snapshot wholesale;
/// discovery and trust are not wired yet.
pub struct HookRegistry {
    snapshot: Arc<RegistrySnapshot>,
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
        }
    }

    pub fn snapshot(&self) -> Arc<RegistrySnapshot> {
        self.snapshot.clone()
    }

    pub fn revision(&self) -> u64 {
        self.snapshot.revision
    }

    pub fn bindings_for(
        &self,
        boundary: HookBoundary,
        cwd: Option<&Path>,
        host: HookExecutionHost,
    ) -> Vec<ActiveBinding> {
        self.snapshot.bindings_for(boundary, cwd, host)
    }

    /// Replaces the active set with the given bindings and bumps the revision.
    #[cfg(any(test, feature = "test-util"))]
    pub fn install_snapshot_for_test(
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

/// Loading one named file. This is the one explicit way to activate a source until discovery,
/// trust, and run-delivered sources arrive; it consults no trust policy and applies the file
/// everywhere on the local host.
#[cfg(feature = "local_fs")]
impl HookRegistry {
    /// Reads `path`, parses it with the adapter for `format`, and publishes a new snapshot in
    /// which this file's bindings replace any loaded from the same file before. Relative commands
    /// in the file resolve against the file's directory.
    pub fn install_explicit_source(
        &mut self,
        path: &Path,
        format: HookFormat,
        kind: HookSourceKind,
    ) -> Result<(), HookConfigError> {
        let unreadable = |error: std::io::Error| {
            HookConfigError::Unreadable(format!("{}: {error}", path.display()))
        };
        let canonical_path = std::fs::canonicalize(path).map_err(unreadable)?;
        let bytes = std::fs::read(&canonical_path).map_err(unreadable)?;
        let working_directory =
            canonical_path
                .parent()
                .map(Path::to_path_buf)
                .ok_or_else(|| {
                    HookConfigError::Unreadable(format!(
                        "{} has no parent directory",
                        path.display()
                    ))
                })?;
        let adapter = adapter_for(format).ok_or_else(|| {
            HookConfigError::Invalid(format!("no adapter is registered for {format:?}"))
        })?;
        let source = HookSourceId {
            kind,
            location: HookSourceLocation::CanonicalPath(canonical_path),
        };
        let config = adapter.parse(&bytes, &source)?;
        let (bindings, diagnostics) = activate(&adapter, &config, &working_directory);
        self.replace_source(&source, bindings, diagnostics);
        Ok(())
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
            .filter(|active| active.binding.id.source != *source)
            .cloned()
            .collect::<Vec<_>>();
        next_bindings.extend(bindings);
        let mut next_diagnostics = previous
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.source != *source)
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

/// Turns a parsed source into bindings that apply everywhere on the local host and run in
/// `working_directory`, plus the source's parse warnings and one warning per capability the
/// boundary cannot honor yet.
#[cfg(feature = "local_fs")]
fn activate(
    adapter: &Arc<dyn HookFormatAdapter>,
    config: &ParsedHookConfig,
    working_directory: &Path,
) -> (Vec<ActiveBinding>, Vec<HookDiagnostic>) {
    let mut diagnostics = config.warnings.clone();
    let mut bindings = Vec::new();
    for binding in adapter.bindings(config) {
        for warning in validate_capabilities(binding.boundary, &binding.capabilities) {
            diagnostics.push(HookDiagnostic {
                source: config.source.clone(),
                severity: DiagnosticSeverity::Warning,
                message: format!("{}: {}", binding.id, warning.message),
            });
        }
        bindings.push(ActiveBinding {
            binding,
            adapter: adapter.clone(),
            scope: BindingScope::Everywhere,
            host: HookExecutionHost::Local,
            working_directory: working_directory.to_path_buf(),
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
