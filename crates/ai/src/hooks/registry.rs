use std::collections::{BTreeSet, HashSet};
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
use crate::hooks::sources::PROJECT_CURSOR_HOOKS_PATH;
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

/// An immutable view of the active hook set, known project roots, and protected paths at one
/// revision.
#[derive(Debug, Default)]
pub struct RegistrySnapshot {
    pub revision: u64,
    pub bindings: Vec<ActiveBinding>,
    pub diagnostics: Vec<HookDiagnostic>,
    /// Every project root the registry knows, whether or not it has a hook file. The nearest one
    /// that contains a working directory is that directory's project; a project with no hook
    /// file has no project hooks, and an enclosing project's hooks do not reach into it.
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

/// Load, trust, and pin state of one registered source.
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

/// How the latest load of a source went.
enum LoadState {
    /// Heard about, never loaded.
    Unloaded,
    Loaded(ParsedSource),
    /// The latest load failed; the earlier valid parse stays in effect.
    Stale {
        last_valid: ParsedSource,
        failure: HookDiagnostic,
    },
    /// No load has ever succeeded.
    Failed(HookDiagnostic),
}

impl LoadState {
    /// The state after a load that did not produce a valid parse.
    fn after_failed_load(self, source: &HookSourceId, error: &str) -> Self {
        match self {
            Self::Loaded(last_valid) | Self::Stale { last_valid, .. } => Self::Stale {
                failure: load_failure(source, true, error),
                last_valid,
            },
            Self::Unloaded | Self::Failed(_) => Self::Failed(load_failure(source, false, error)),
        }
    }

    fn last_valid(&self) -> Option<&ParsedSource> {
        match self {
            Self::Loaded(parsed)
            | Self::Stale {
                last_valid: parsed, ..
            } => Some(parsed),
            Self::Unloaded | Self::Failed(_) => None,
        }
    }

    fn failure(&self) -> Option<&HookDiagnostic> {
        match self {
            Self::Stale { failure, .. } | Self::Failed(failure) => Some(failure),
            Self::Unloaded | Self::Loaded(_) => None,
        }
    }

    fn is_stale(&self) -> bool {
        matches!(self, Self::Stale { .. })
    }
}

struct SourceState {
    /// Roots a project file was found under. Empty for explicit sources.
    project_roots: BTreeSet<PathBuf>,
    load: LoadState,
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
            load: LoadState::Unloaded,
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

    /// The state of a source first seen after the run was pinned. It is parsed, so its files
    /// are protected, but nothing can make it active in this run.
    fn found_after_pin(kind: HookSourceKind) -> Self {
        let reason = match kind {
            HookSourceKind::ProjectCursorFile => {
                "it was found after the run started and takes effect at the next run start"
            }
            HookSourceKind::RunDelivered
            | HookSourceKind::FactoryAgent
            | HookSourceKind::FactoryWide => {
                "explicit sources must be loaded before the run starts"
            }
        };
        Self::new(TrustDecision::Inert {
            reason: reason.to_owned(),
        })
    }

    /// Records the outcome of one load.
    fn apply(&mut self, source: &HookSourceId, parsed: Result<ParsedSource, HookConfigError>) {
        match parsed {
            Ok(parsed) => self.load = LoadState::Loaded(parsed),
            Err(error) => self.apply_load_failure(source, &error.to_string()),
        }
    }

    /// Records that the source could not be read or parsed.
    fn apply_load_failure(&mut self, source: &HookSourceId, message: &str) {
        let load = std::mem::replace(&mut self.load, LoadState::Unloaded);
        self.load = load.after_failed_load(source, message);
    }

    fn diagnostics(&self, source: &HookSourceId) -> Vec<HookDiagnostic> {
        let mut diagnostics = Vec::new();
        if let Some(parsed) = self.load.last_valid() {
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
        diagnostics.extend(self.load.failure().cloned());
        diagnostics.extend(self.changes_after_pin.iter().cloned());
        diagnostics
    }
}

/// Where one loaded source's bindings apply and the directory its hooks run in.
struct Placement {
    scope: BindingScope,
    /// `None` when the source has no directory to run in, which keeps its definitions inactive.
    working_directory: Option<PathBuf>,
}

/// A project file applies under each root it was found under and runs in that root; an explicit
/// source applies everywhere and runs in the directory that holds it.
fn placements(source: &HookSourceId, state: &SourceState) -> Vec<Placement> {
    match source.kind {
        HookSourceKind::ProjectCursorFile => state
            .project_roots
            .iter()
            .map(|root| Placement {
                scope: BindingScope::Root(root.clone()),
                working_directory: working_directory_for(source, Some(root)),
            })
            .collect(),
        HookSourceKind::RunDelivered
        | HookSourceKind::FactoryAgent
        | HookSourceKind::FactoryWide => {
            vec![Placement {
                scope: BindingScope::Everywhere,
                working_directory: working_directory_for(source, None),
            }]
        }
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

/// The script a command names, when it can be resolved to a file relative to `dir`: the first
/// word (`./check.sh --strict`), or the second word when the first names a program rather than a
/// path (`bash scripts/check.sh`, `node hooks/format.js`). A command whose words resolve to no
/// file, such as `npx foo`, is not protected. Quoted paths with spaces are not resolved.
fn script_path(command: &str, dir: &Path) -> Option<PathBuf> {
    let mut words = command.split_whitespace();
    let first_word = words.next()?;
    let resolve = |word: &str| {
        dunce::canonicalize(dir.join(word))
            .ok()
            .filter(|path| path.is_file())
    };
    if let Some(script) = resolve(first_word) {
        return Some(script);
    }
    let names_a_program = !first_word.contains(['/', '\\']);
    if names_a_program {
        return words.next().and_then(resolve);
    }
    None
}

fn canonical_source_path(source: &HookSourceId) -> Option<&Path> {
    match &source.location {
        HookSourceLocation::CanonicalPath(path) => Some(path.as_path()),
        HookSourceLocation::Named(_) => None,
    }
}

/// The source a project root's hook file would be, so a diagnostic about the root itself has a
/// source to hang on.
fn root_source(root: &Path) -> HookSourceId {
    HookSourceId {
        kind: HookSourceKind::ProjectCursorFile,
        location: HookSourceLocation::CanonicalPath(root.join(PROJECT_CURSOR_HOOKS_PATH)),
    }
}

/// Emitted whenever the snapshot changes, including reload attempts that left the active bindings
/// as they were.
pub enum HookRegistryEvent {
    SnapshotChanged { revision: u64 },
}

/// Holds every known source and publishes immutable snapshots of the bindings that may run.
///
/// Each change, including a failed reload, publishes a new snapshot with a higher revision, so a
/// decision keyed by revision is invalidated whenever a source changes. Every mutator returns the
/// resulting revision. A source whose reload fails keeps its last valid configuration in effect
/// and is marked stale. Once the run is pinned, changes to sources and to the set of project
/// roots are reported and never applied.
pub struct HookRegistry {
    snapshot: Arc<RegistrySnapshot>,
    sources: IndexMap<HookSourceId, SourceState>,
    /// Every project root a watcher announced or a project file was loaded under.
    project_roots: BTreeSet<PathBuf>,
    /// Roots announced after the run was pinned. They are reported and never become projects in
    /// this run.
    roots_after_pin: Vec<HookDiagnostic>,
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
            project_roots: BTreeSet::new(),
            roots_after_pin: Vec::new(),
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
            loaded: state.load.last_valid().is_some(),
            stale: state.load.is_stale(),
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

    /// Makes `root` a project the registry knows, with or without a hook file, so a working
    /// directory under it belongs to that project and not to an enclosing one.
    pub fn add_project_root(&mut self, root: &Path) -> u64 {
        if self.project_roots.contains(root) {
            return self.revision;
        }
        if self.pinned.is_some() {
            let message = format!(
                "{}: the project was found after the run started; it becomes a project at the \
                 next run start",
                root.display()
            );
            if !self
                .roots_after_pin
                .iter()
                .any(|existing| existing.message == message)
            {
                self.roots_after_pin.push(HookDiagnostic {
                    source: root_source(root),
                    severity: DiagnosticSeverity::Warning,
                    message,
                });
            }
            return self.rebuild();
        }
        self.project_roots.insert(root.to_path_buf());
        self.rebuild()
    }

    /// Forgets `root` as a project.
    pub fn remove_project_root(&mut self, root: &Path) -> u64 {
        if self.pinned.is_some() || !self.project_roots.remove(root) {
            return self.revision;
        }
        self.rebuild()
    }

    /// Loads or reloads one source from the bytes a loader read. A source first seen after the
    /// run is pinned is still parsed so its files are protected, but stays inert.
    pub fn upsert_source(&mut self, update: SourceUpdate<'_>) -> u64 {
        let SourceUpdate {
            source,
            format,
            bytes,
            project_root,
            trust,
        } = update;
        if self.pinned.is_some() {
            if self.sources.contains_key(&source) {
                self.note_change_after_pin(&source, "changed");
                return self.rebuild();
            }
            let parsed = parse_source(&source, format, bytes);
            let mut state = SourceState::found_after_pin(source.kind);
            state.project_roots.extend(project_root);
            state.apply(&source, parsed);
            self.sources.insert(source.clone(), state);
            self.note_change_after_pin(&source, "was found");
            return self.rebuild();
        }

        let parsed = parse_source(&source, format, bytes);
        self.project_roots.extend(project_root.clone());
        let state = self
            .sources
            .entry(source.clone())
            .or_insert_with(|| SourceState::new(trust.clone()));
        state.project_roots.extend(project_root);
        state.trust = trust;
        state.apply(&source, parsed);
        self.rebuild()
    }

    /// Records that `source` could not be read, treating it like a failed parse.
    pub fn record_read_failure(&mut self, source: &HookSourceId, message: &str) -> u64 {
        if self.pinned.is_some() {
            self.note_change_after_pin(source, "could not be read");
            return self.rebuild();
        }
        self.sources
            .entry(source.clone())
            .or_insert_with(SourceState::unloaded)
            .apply_load_failure(source, message);
        self.rebuild()
    }

    /// Forgets `source`.
    pub fn remove_source(&mut self, source: &HookSourceId) -> u64 {
        let Some(state) = self.sources.get(source) else {
            return self.revision;
        };
        if self.pinned.is_some() && state.pinned {
            self.note_change_after_pin(source, "was removed");
            return self.rebuild();
        }
        self.sources.shift_remove(source);
        self.rebuild()
    }

    /// Stops applying a project file under `root`, forgetting the source once no root is left.
    /// The root itself stays a project; see [`Self::remove_project_root`].
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

    /// Replaces the trust decision for `source` without reloading it.
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

    /// Fixes the active set and the set of projects for the rest of the run. A second call
    /// changes nothing.
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

    /// Records a change to `source` after the run was pinned. A source in the pinned set keeps
    /// its hooks and gets a warning; a project file found late waits for the next run; an
    /// explicit source that was not loaded before the run started cannot run in it at all,
    /// which is an error the loader reports.
    fn note_change_after_pin(&mut self, source: &HookSourceId, what: &str) {
        let state = self
            .sources
            .entry(source.clone())
            .or_insert_with(|| SourceState::found_after_pin(source.kind));
        let (severity, outcome) = match (state.pinned, source.kind) {
            (true, HookSourceKind::ProjectCursorFile) => (
                DiagnosticSeverity::Warning,
                "its hooks stay as they were when the run started",
            ),
            (
                true,
                HookSourceKind::RunDelivered
                | HookSourceKind::FactoryAgent
                | HookSourceKind::FactoryWide,
            ) => (
                DiagnosticSeverity::Warning,
                "sources delivered with the run cannot change after it starts",
            ),
            (false, HookSourceKind::ProjectCursorFile) => (
                DiagnosticSeverity::Warning,
                "it takes effect at the next run start",
            ),
            (
                false,
                HookSourceKind::RunDelivered
                | HookSourceKind::FactoryAgent
                | HookSourceKind::FactoryWide,
            ) => (
                DiagnosticSeverity::Error,
                "explicit sources must be loaded before the run starts, so it is not active",
            ),
        };
        let message = format!("{source}: {what} after the run started; {outcome}");
        if !state
            .changes_after_pin
            .iter()
            .any(|existing| existing.message == message)
        {
            state.changes_after_pin.push(HookDiagnostic {
                source: source.clone(),
                severity,
                message,
            });
        }
    }

    /// Canonical paths of explicit sources that have loaded. When the same file is also found as
    /// a project file, only the explicit source runs.
    fn loaded_explicit_files(&self) -> HashSet<&Path> {
        self.sources
            .iter()
            .filter(|(source, state)| {
                state.load.last_valid().is_some()
                    && match source.kind {
                        HookSourceKind::ProjectCursorFile => false,
                        HookSourceKind::RunDelivered
                        | HookSourceKind::FactoryAgent
                        | HookSourceKind::FactoryWide => true,
                    }
            })
            .filter_map(|(source, _)| canonical_source_path(source))
            .collect()
    }

    /// Publishes a new snapshot from the current source states.
    fn rebuild(&mut self) -> u64 {
        self.revision += 1;
        let mut bindings = Vec::new();
        let mut diagnostics = Vec::new();
        let mut protected_paths = BTreeSet::new();
        let explicit_files = self.loaded_explicit_files();

        for (source, state) in &self.sources {
            diagnostics.extend(state.diagnostics(source));
            // A file the registry knows is a hook file even when its contents never loaded.
            if let Some(path) = canonical_source_path(source) {
                protected_paths.insert(path.to_path_buf());
            }
            let Some(parsed) = state.load.last_valid() else {
                continue;
            };
            let shadowed_by_explicit = source.kind == HookSourceKind::ProjectCursorFile
                && canonical_source_path(source).is_some_and(|path| explicit_files.contains(path));
            if shadowed_by_explicit {
                diagnostics.push(HookDiagnostic {
                    source: source.clone(),
                    severity: DiagnosticSeverity::Info,
                    message: format!(
                        "{source}: also delivered with the run, so it runs once, as that source"
                    ),
                });
            }
            for placement in placements(source, state) {
                let Some(working_directory) = placement.working_directory else {
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
                if state.trust != TrustDecision::Activate || shadowed_by_explicit {
                    continue;
                }
                bindings.extend(parsed.bindings.iter().map(|binding| ActiveBinding {
                    binding: binding.clone(),
                    adapter: parsed.adapter.clone(),
                    scope: placement.scope.clone(),
                    host: HookExecutionHost::Local,
                    working_directory: working_directory.clone(),
                }));
            }
        }
        diagnostics.extend(self.roots_after_pin.iter().cloned());

        #[cfg(any(test, feature = "test-util"))]
        {
            bindings.extend(self.installed_for_test.0.iter().cloned());
            diagnostics.extend(self.installed_for_test.1.iter().cloned());
        }

        self.snapshot = Arc::new(RegistrySnapshot {
            revision: self.revision,
            bindings,
            diagnostics,
            project_roots: self.project_roots.iter().cloned().collect(),
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
