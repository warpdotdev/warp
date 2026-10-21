use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use indexmap::IndexMap;
use warpui_core::{Entity, ModelHandle, SingletonEntity, UpdateModel};

use crate::hooks::adapters::{
    DeclaredBinding, HookConfigError, HookFormatAdapter, ParsedHookConfig, adapter_for,
};
use crate::hooks::model::{
    DiagnosticSeverity, HOOK_TIMEOUT_CEILING, HookBoundary, HookDiagnostic, HookExecutionHost,
    HookFormat, HookSourceId, HookSourceKind, HookSourceLocation, validate_capabilities,
};
use crate::hooks::runner::working_directory_for;
use crate::hooks::trust::{CommandExecutionPosture, TrustDecision};

/// Which working directories a binding applies to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindingScope {
    /// Only operations whose nearest project root is this one.
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
    /// The root of every project file that has loaded, whether or not it is active. The nearest
    /// one that contains a working directory is that directory's project.
    pub project_roots: Vec<PathBuf>,
    /// Files the agent may not edit without approval: every source file the registry knows and
    /// every script a definition names that could be resolved to a file on disk.
    pub protected_paths: Vec<PathBuf>,
}

impl RegistrySnapshot {
    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }

    /// Bindings for `boundary` that apply at `cwd` on `host`, sorted by source priority with
    /// each source's own order preserved within a priority. Project bindings apply only when
    /// their root is the nearest project root containing `cwd`; run-delivered and factory
    /// bindings apply everywhere.
    pub fn bindings_for(
        &self,
        boundary: HookBoundary,
        cwd: Option<&Path>,
        host: HookExecutionHost,
    ) -> Vec<ActiveBinding> {
        let project_root = cwd.and_then(|cwd| self.nearest_project_root(cwd));
        let mut matching = self
            .bindings
            .iter()
            .filter(|active| active.binding.boundary == boundary && active.host == host)
            .filter(|active| match &active.scope {
                BindingScope::Everywhere => true,
                BindingScope::Root(root) => project_root.is_some_and(|nearest| nearest == root),
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

    /// The deepest project root that contains `cwd`. Roots are canonical paths, so `cwd` is
    /// canonicalized too when it exists; a path that cannot be resolved is compared as given.
    pub fn nearest_project_root(&self, cwd: &Path) -> Option<&Path> {
        if self.project_roots.is_empty() {
            return None;
        }
        let canonical = dunce::canonicalize(cwd).ok();
        let cwd = canonical.as_deref().unwrap_or(cwd);
        self.project_roots
            .iter()
            .filter(|root| cwd.starts_with(root))
            .max_by_key(|root| root.components().count())
            .map(PathBuf::as_path)
    }
}

/// One load of a source: the bytes a loader read and the facts the registry needs to scope and
/// trust them.
#[derive(Debug, Clone)]
pub struct SourceUpdate<'a> {
    pub source: HookSourceId,
    pub format: HookFormat,
    pub bytes: &'a [u8],
    /// For a project file, the detected root it was found under. Its bindings apply only when
    /// that root is the nearest project root, and its relative commands resolve against it. A
    /// file reached from several roots is one source that applies under each of them.
    pub project_root: Option<PathBuf>,
    pub trust: TrustDecision,
}

/// What the registry knows about one source, for callers that report state rather than run
/// hooks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceStatus {
    /// Whether a valid configuration has ever loaded from the source.
    pub loaded: bool,
    /// Whether the latest load failed while an earlier valid one stays in effect.
    pub stale: bool,
    pub trust: TrustDecision,
    /// Whether the source belongs to the set pinned when the run started.
    pub pinned: bool,
    /// How many of the source's definitions are active right now.
    pub active_bindings: usize,
}

/// The run the active set was pinned for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PinnedRun {
    /// The command execution posture the trust decisions were made under. It is read once, when
    /// the run starts; a profile change afterwards does not re-trust anything.
    pub posture: CommandExecutionPosture,
    /// The revision the active set had when it was pinned.
    pub revision: u64,
}

/// A source's last successful parse, kept so a later failed reload can leave it in effect.
struct ParsedSource {
    adapter: Arc<dyn HookFormatAdapter>,
    config: ParsedHookConfig,
    /// Definitions that may run, in the adapter's declared order, after the timeout ceiling was
    /// applied.
    bindings: Vec<DeclaredBinding>,
    /// Problems found while activating the parsed definitions, such as capability gaps and
    /// rejected definitions.
    diagnostics: Vec<HookDiagnostic>,
}

struct SourceState {
    /// Roots a project file was found under. Empty for explicit sources.
    project_roots: BTreeSet<PathBuf>,
    last_valid: Option<ParsedSource>,
    /// Whether the latest load failed after an earlier one succeeded.
    stale: bool,
    /// The latest load failure, kept until a load succeeds.
    failure: Option<HookDiagnostic>,
    trust: TrustDecision,
    /// Whether the source was present when the run was pinned.
    pinned: bool,
    /// Changes seen after the run was pinned. They are reported and never applied.
    changes_after_pin: Vec<HookDiagnostic>,
}

impl SourceState {
    fn new(trust: TrustDecision) -> Self {
        Self {
            project_roots: BTreeSet::new(),
            last_valid: None,
            stale: false,
            failure: None,
            trust,
            pinned: false,
            changes_after_pin: Vec::new(),
        }
    }

    /// The state of a source the registry has heard about but never loaded.
    fn unloaded() -> Self {
        Self::new(TrustDecision::Inert {
            reason: "the source has not loaded".to_owned(),
        })
    }

    fn diagnostics(&self, source: &HookSourceId) -> Vec<HookDiagnostic> {
        let mut diagnostics = Vec::new();
        if let Some(parsed) = &self.last_valid {
            diagnostics.extend(parsed.config.warnings.iter().cloned());
            diagnostics.extend(parsed.diagnostics.iter().cloned());
            if let TrustDecision::Inert { reason } = &self.trust {
                diagnostics.push(HookDiagnostic {
                    source: source.clone(),
                    severity: DiagnosticSeverity::Info,
                    message: format!("{source}: loaded but not active: {reason}"),
                });
            }
        }
        diagnostics.extend(self.failure.iter().cloned());
        diagnostics.extend(self.changes_after_pin.iter().cloned());
        diagnostics
    }
}

/// The deadline a definition runs with. Adapters fill in the boundary default when a definition
/// declares none, so this only applies the ceiling: a value above [`HOOK_TIMEOUT_CEILING`] is an
/// error and the definition is rejected rather than run with a shorter deadline.
pub fn resolve_timeout(binding: &DeclaredBinding) -> Result<Duration, String> {
    if binding.timeout > HOOK_TIMEOUT_CEILING {
        return Err(format!(
            "{}: timeout of {}s is above the {}s limit, so the definition was skipped",
            binding.id,
            binding.timeout.as_secs_f64(),
            HOOK_TIMEOUT_CEILING.as_secs()
        ));
    }
    Ok(binding.timeout)
}

/// The script a command names, when its first word resolves to a file relative to `dir`. A
/// command that starts with a program found on PATH, such as `npx foo`, resolves to nothing and
/// is not protected.
fn script_path(command: &str, dir: &Path) -> Option<PathBuf> {
    let first_word = command.split_whitespace().next()?;
    dunce::canonicalize(dir.join(first_word))
        .ok()
        .filter(|path| path.is_file())
}

/// Emitted whenever the snapshot changes, including reload attempts that left the active bindings
/// as they were.
pub enum HookRegistryEvent {
    SnapshotChanged { revision: u64 },
}

/// Holds every known source and publishes immutable snapshots of the bindings that may run.
///
/// Each change, including a failed reload, publishes a new snapshot with a higher revision, so a
/// decision keyed by revision is invalidated whenever a source changes. A source whose reload
/// fails keeps its last valid configuration in effect and is marked stale. Once the run is
/// pinned, changes to sources are reported and never applied.
pub struct HookRegistry {
    snapshot: Arc<RegistrySnapshot>,
    sources: IndexMap<HookSourceId, SourceState>,
    revision: u64,
    pinned: Option<PinnedRun>,
    /// Bindings a test installed directly. They ride along with every snapshot the sources
    /// produce, so pinning or reloading a source does not drop them.
    #[cfg(any(test, feature = "test-util"))]
    installed_for_test: (Vec<ActiveBinding>, Vec<HookDiagnostic>),
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
            sources: IndexMap::new(),
            revision: 0,
            pinned: None,
            #[cfg(any(test, feature = "test-util"))]
            installed_for_test: (Vec::new(), Vec::new()),
        }
    }

    pub fn snapshot(&self) -> Arc<RegistrySnapshot> {
        self.snapshot.clone()
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn pinned(&self) -> Option<PinnedRun> {
        self.pinned
    }

    pub fn bindings_for(
        &self,
        boundary: HookBoundary,
        cwd: Option<&Path>,
        host: HookExecutionHost,
    ) -> Vec<ActiveBinding> {
        self.snapshot.bindings_for(boundary, cwd, host)
    }

    pub fn protected_paths(&self) -> &[PathBuf] {
        &self.snapshot.protected_paths
    }

    pub fn source_status(&self, source: &HookSourceId) -> Option<SourceStatus> {
        let state = self.sources.get(source)?;
        Some(SourceStatus {
            loaded: state.last_valid.is_some(),
            stale: state.stale,
            trust: state.trust.clone(),
            pinned: state.pinned,
            active_bindings: self
                .snapshot
                .bindings
                .iter()
                .filter(|active| active.binding.id.source == *source)
                .count(),
        })
    }

    /// Applies `change` to the registry behind `handle` and, when it published a new snapshot,
    /// tells subscribers which revision it produced.
    pub fn change(
        handle: &ModelHandle<Self>,
        ctx: &mut impl UpdateModel,
        change: impl FnOnce(&mut Self) -> u64,
    ) -> u64 {
        handle.update(ctx, |registry, ctx| {
            let before = registry.revision;
            let revision = change(registry);
            if revision != before {
                ctx.emit(HookRegistryEvent::SnapshotChanged { revision });
            }
            revision
        })
    }

    /// Loads or reloads one source from the bytes a loader read. A successful parse replaces the
    /// source's last valid configuration; a failed one keeps it and marks the source stale. A
    /// source whose first load fails stays inactive with an error. After the run is pinned the
    /// change is only reported. Returns the new revision.
    pub fn upsert_source(&mut self, update: SourceUpdate<'_>) -> u64 {
        let SourceUpdate {
            source,
            format,
            bytes,
            project_root,
            trust,
        } = update;
        if self.pinned.is_some() {
            self.note_change_after_pin(&source, "changed");
            return self.rebuild();
        }

        let parsed = parse_source(&source, format, bytes);
        let state = self
            .sources
            .entry(source.clone())
            .or_insert_with(|| SourceState::new(trust.clone()));
        state.project_roots.extend(project_root);
        state.trust = trust;
        match parsed {
            Ok(parsed) => {
                state.last_valid = Some(parsed);
                state.stale = false;
                state.failure = None;
            }
            Err(error) => {
                state.stale = state.last_valid.is_some();
                state.failure = Some(load_failure(&source, state.stale, &error.to_string()));
            }
        }
        self.rebuild()
    }

    /// Records that `source` could not be read. Behaves like a failed parse: a last valid
    /// configuration stays in effect and the source is marked stale. Returns the new revision.
    pub fn record_read_failure(&mut self, source: &HookSourceId, message: &str) -> u64 {
        if self.pinned.is_some() {
            self.note_change_after_pin(source, "could not be read");
            return self.rebuild();
        }
        let state = self
            .sources
            .entry(source.clone())
            .or_insert_with(SourceState::unloaded);
        state.stale = state.last_valid.is_some();
        state.failure = Some(load_failure(source, state.stale, message));
        self.rebuild()
    }

    /// Forgets `source`. After the run is pinned, a pinned source keeps its bindings until the
    /// run ends and the removal is only reported. Returns the new revision.
    pub fn remove_source(&mut self, source: &HookSourceId) -> u64 {
        if !self.sources.contains_key(source) {
            return self.revision;
        }
        if self.pinned.is_some() && self.sources[source].pinned {
            self.note_change_after_pin(source, "was removed");
            return self.rebuild();
        }
        self.sources.shift_remove(source);
        self.rebuild()
    }

    /// Stops applying a project file under `root`. The source is forgotten once no root is left.
    /// Returns the new revision.
    pub fn remove_source_root(&mut self, source: &HookSourceId, root: &Path) -> u64 {
        let Some(state) = self.sources.get_mut(source) else {
            return self.revision;
        };
        if self.pinned.is_some() && state.pinned {
            self.note_change_after_pin(source, "was removed");
            return self.rebuild();
        }
        state.project_roots.remove(root);
        if state.project_roots.is_empty() && source.kind == HookSourceKind::ProjectCursorFile {
            self.sources.shift_remove(source);
        }
        self.rebuild()
    }

    /// Replaces the trust decision for `source` without reloading it. After the run is pinned
    /// the change is only reported. Returns the new revision.
    pub fn set_trust(&mut self, source: &HookSourceId, trust: TrustDecision) -> u64 {
        let Some(state) = self.sources.get_mut(source) else {
            return self.revision;
        };
        if self.pinned.is_some() {
            self.note_change_after_pin(source, "changed its trust decision");
            return self.rebuild();
        }
        state.trust = trust;
        self.rebuild()
    }

    /// Fixes the active set for the rest of the run. Later changes to any source bump the
    /// revision and produce a diagnostic but never change the active bindings. A second call
    /// changes nothing. Returns the new revision.
    pub fn pin(&mut self, posture: CommandExecutionPosture) -> u64 {
        if self.pinned.is_some() {
            log::warn!("hook registry was already pinned; the second pin was ignored");
            return self.revision;
        }
        for state in self.sources.values_mut() {
            state.pinned = true;
        }
        let revision = self.rebuild();
        self.pinned = Some(PinnedRun { posture, revision });
        revision
    }

    /// Replaces the test-installed bindings, which sit beside whatever the sources produce, and
    /// bumps the revision.
    #[cfg(any(test, feature = "test-util"))]
    pub fn install_snapshot_for_test(
        &mut self,
        bindings: Vec<ActiveBinding>,
        diagnostics: Vec<HookDiagnostic>,
    ) -> u64 {
        self.installed_for_test = (bindings, diagnostics);
        self.rebuild()
    }

    fn note_change_after_pin(&mut self, source: &HookSourceId, what: &str) {
        let state = self
            .sources
            .entry(source.clone())
            .or_insert_with(SourceState::unloaded);
        let outcome = if state.pinned {
            match source.kind {
                HookSourceKind::ProjectCursorFile => {
                    "its hooks stay as they were when the run started"
                }
                HookSourceKind::RunDelivered
                | HookSourceKind::FactoryAgent
                | HookSourceKind::FactoryWide => {
                    "sources delivered with the run cannot change after it starts"
                }
            }
        } else {
            "it takes effect at the next run start"
        };
        let message = format!("{source}: {what} after the run started; {outcome}");
        if !state
            .changes_after_pin
            .iter()
            .any(|existing| existing.message == message)
        {
            state.changes_after_pin.push(HookDiagnostic {
                source: source.clone(),
                severity: DiagnosticSeverity::Warning,
                message,
            });
        }
    }

    /// Publishes a new snapshot from the current source states.
    fn rebuild(&mut self) -> u64 {
        self.revision += 1;
        let mut bindings = Vec::new();
        let mut diagnostics = Vec::new();
        let mut project_roots = BTreeSet::new();
        let mut protected_paths = BTreeSet::new();

        for (source, state) in &self.sources {
            diagnostics.extend(state.diagnostics(source));
            let Some(parsed) = &state.last_valid else {
                continue;
            };
            if let HookSourceLocation::CanonicalPath(path) = &source.location {
                protected_paths.insert(path.clone());
            }
            let placements: Vec<(BindingScope, Option<PathBuf>)> = match source.kind {
                // A project file loaded without a detected root is scoped to the root its path
                // implies, the parent of its `.cursor` directory.
                HookSourceKind::ProjectCursorFile if state.project_roots.is_empty() => {
                    match working_directory_for(source, None) {
                        Some(root) => {
                            project_roots.insert(root.clone());
                            vec![(BindingScope::Root(root.clone()), Some(root))]
                        }
                        None => vec![(BindingScope::Everywhere, None)],
                    }
                }
                HookSourceKind::ProjectCursorFile => state
                    .project_roots
                    .iter()
                    .map(|root| {
                        project_roots.insert(root.clone());
                        (
                            BindingScope::Root(root.clone()),
                            working_directory_for(source, Some(root)),
                        )
                    })
                    .collect(),
                HookSourceKind::RunDelivered
                | HookSourceKind::FactoryAgent
                | HookSourceKind::FactoryWide => {
                    vec![(
                        BindingScope::Everywhere,
                        working_directory_for(source, None),
                    )]
                }
            };
            for (scope, working_directory) in placements {
                let Some(working_directory) = working_directory else {
                    diagnostics.push(HookDiagnostic {
                        source: source.clone(),
                        severity: DiagnosticSeverity::Error,
                        message: format!(
                            "{source}: has no directory to run its hooks in, so it is not active"
                        ),
                    });
                    continue;
                };
                protected_paths.extend(
                    parsed
                        .bindings
                        .iter()
                        .filter_map(|binding| script_path(&binding.command, &working_directory)),
                );
                if state.trust != TrustDecision::Activate {
                    continue;
                }
                bindings.extend(parsed.bindings.iter().map(|binding| ActiveBinding {
                    binding: binding.clone(),
                    adapter: parsed.adapter.clone(),
                    scope: scope.clone(),
                    host: HookExecutionHost::Local,
                    working_directory: working_directory.clone(),
                }));
            }
        }

        #[cfg(any(test, feature = "test-util"))]
        {
            bindings.extend(self.installed_for_test.0.iter().cloned());
            diagnostics.extend(self.installed_for_test.1.iter().cloned());
        }

        self.snapshot = Arc::new(RegistrySnapshot {
            revision: self.revision,
            bindings,
            diagnostics,
            project_roots: project_roots.into_iter().collect(),
            protected_paths: protected_paths.into_iter().collect(),
        });
        self.revision
    }
}

/// Parses `bytes` with the adapter for `format` and prepares the definitions that may run.
fn parse_source(
    source: &HookSourceId,
    format: HookFormat,
    bytes: &[u8],
) -> Result<ParsedSource, HookConfigError> {
    let adapter = adapter_for(format).ok_or_else(|| {
        HookConfigError::Invalid(format!("no adapter is registered for {format:?}"))
    })?;
    let config = adapter.parse(bytes, source)?;
    let mut diagnostics = Vec::new();
    let mut bindings = Vec::new();
    for binding in adapter.bindings(&config) {
        if let Err(message) = resolve_timeout(&binding) {
            diagnostics.push(HookDiagnostic {
                source: source.clone(),
                severity: DiagnosticSeverity::Error,
                message,
            });
            continue;
        }
        for warning in validate_capabilities(binding.boundary, &binding.capabilities) {
            diagnostics.push(HookDiagnostic {
                source: source.clone(),
                severity: DiagnosticSeverity::Warning,
                message: format!("{}: {}", binding.id, warning.message),
            });
        }
        bindings.push(binding);
    }
    Ok(ParsedSource {
        adapter,
        config,
        bindings,
        diagnostics,
    })
}

fn load_failure(source: &HookSourceId, stale: bool, error: &str) -> HookDiagnostic {
    let message = if stale {
        format!(
            "{source}: the new contents could not be loaded ({error}); the previous valid \
             configuration stays in effect"
        )
    } else {
        format!("{source}: could not be loaded and is not active: {error}")
    };
    HookDiagnostic {
        source: source.clone(),
        severity: DiagnosticSeverity::Error,
        message,
    }
}

impl Entity for HookRegistry {
    type Event = HookRegistryEvent;
}

impl SingletonEntity for HookRegistry {}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
