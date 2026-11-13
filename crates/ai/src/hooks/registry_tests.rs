use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tempfile::TempDir;

use super::*;
use crate::hooks::model::{HookBoundary, HookSourceKind};
use crate::hooks::test_util::{
    RecordingAdapter, active_binding, test_binding, test_source_id, test_source_id_of_kind,
};

const VALID: &str =
    r#"{ "version": 1, "hooks": { "beforeShellExecution": [ { "command": "./hook.sh" } ] } }"#;
const VALID_TWO: &str = r#"{ "version": 1, "hooks": { "beforeShellExecution": [
    { "command": "./hook.sh" }, { "command": "./other.sh", "matcher": "rm" } ] } }"#;
const MALFORMED: &str = "{ not json";

fn inert() -> TrustDecision {
    TrustDecision::Inert {
        reason: "test".to_owned(),
    }
}

/// An update for a source that only exists by name. Project sources still get a root, so they
/// activate; a named explicit source has no directory and does not.
fn update<'a>(
    source: &HookSourceId,
    bytes: &'a str,
    project_root: Option<&Path>,
    trust: TrustDecision,
) -> SourceUpdate<'a> {
    SourceUpdate {
        source: source.clone(),
        format: HookFormat::Cursor,
        bytes: bytes.as_bytes(),
        project_root: project_root.map(Path::to_path_buf),
        trust,
    }
}

fn project_source(name: &str) -> HookSourceId {
    test_source_id_of_kind(name, HookSourceKind::ProjectCursorFile)
}

fn source_names(bindings: &[ActiveBinding]) -> Vec<String> {
    bindings
        .iter()
        .map(|active| active.binding.id.source.to_string())
        .collect()
}

fn tool_pre_bindings(registry: &HookRegistry, cwd: &str) -> Vec<ActiveBinding> {
    registry.bindings_for(
        HookBoundary::ToolPre,
        Some(Path::new(cwd)),
        HookExecutionHost::Local,
    )
}

fn messages(registry: &HookRegistry, severity: DiagnosticSeverity) -> Vec<String> {
    registry
        .snapshot()
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == severity)
        .map(|diagnostic| diagnostic.message.clone())
        .collect()
}

fn errors(registry: &HookRegistry) -> Vec<String> {
    messages(registry, DiagnosticSeverity::Error)
}

fn warnings(registry: &HookRegistry) -> Vec<String> {
    messages(registry, DiagnosticSeverity::Warning)
}

/// A source on disk with a `hook.sh` beside it, so relative commands and protected paths resolve
/// against real files. A project file lives at `<root>/.cursor/hooks.json`; an explicit source
/// lives at `<root>/hooks.json`.
struct SourceOnDisk {
    _dir: TempDir,
    root: PathBuf,
    hooks_file: PathBuf,
    kind: HookSourceKind,
    bytes: Vec<u8>,
}

impl SourceOnDisk {
    fn new(kind: HookSourceKind, hooks_json: &str) -> Self {
        let dir = TempDir::new().expect("tempdir");
        let root = dunce::canonicalize(dir.path()).expect("canonical root");
        let hooks_file = match kind {
            HookSourceKind::ProjectCursorFile => {
                fs::create_dir_all(root.join(".cursor")).expect("mkdir .cursor");
                root.join(".cursor/hooks.json")
            }
            HookSourceKind::RunDelivered
            | HookSourceKind::FactoryAgent
            | HookSourceKind::FactoryWide => root.join("hooks.json"),
        };
        fs::write(&hooks_file, hooks_json).expect("write hooks.json");
        fs::write(root.join("hook.sh"), "#!/bin/sh\nexit 0\n").expect("write hook.sh");
        Self {
            _dir: dir,
            root,
            hooks_file,
            kind,
            bytes: hooks_json.as_bytes().to_vec(),
        }
    }

    fn project(hooks_json: &str) -> Self {
        Self::new(HookSourceKind::ProjectCursorFile, hooks_json)
    }

    fn source(&self) -> HookSourceId {
        HookSourceId {
            kind: self.kind,
            location: HookSourceLocation::CanonicalPath(self.hooks_file.clone()),
        }
    }

    fn project_root(&self) -> Option<PathBuf> {
        match self.kind {
            HookSourceKind::ProjectCursorFile => Some(self.root.clone()),
            HookSourceKind::RunDelivered
            | HookSourceKind::FactoryAgent
            | HookSourceKind::FactoryWide => None,
        }
    }

    fn update(&self, trust: TrustDecision) -> SourceUpdate<'_> {
        SourceUpdate {
            source: self.source(),
            format: HookFormat::Cursor,
            bytes: &self.bytes,
            project_root: self.project_root(),
            trust,
        }
    }

    fn update_with<'a>(&self, bytes: &'a str, trust: TrustDecision) -> SourceUpdate<'a> {
        SourceUpdate {
            source: self.source(),
            format: HookFormat::Cursor,
            bytes: bytes.as_bytes(),
            project_root: self.project_root(),
            trust,
        }
    }
}

#[test]
fn a_new_registry_has_an_empty_snapshot_at_revision_zero() {
    let registry = HookRegistry::new();
    let snapshot = registry.snapshot();
    assert_eq!(snapshot.revision, 0);
    assert!(snapshot.is_empty());
    assert!(snapshot.diagnostics.is_empty());
    assert!(snapshot.protected_paths.is_empty());
    assert!(registry.pinned().is_none());
    assert!(tool_pre_bindings(&registry, "/repo").is_empty());
}

#[test]
fn installing_a_snapshot_bumps_the_revision_and_replaces_the_set() {
    let adapter: Arc<dyn HookFormatAdapter> = Arc::new(RecordingAdapter::new());
    let mut registry = HookRegistry::new();
    let first = registry.install_snapshot_for_test(
        vec![active_binding(
            test_binding(test_source_id("a"), HookBoundary::ToolPre, 0, None),
            adapter.clone(),
            BindingScope::Everywhere,
        )],
        Vec::new(),
    );
    assert_eq!(first, 1);
    assert_eq!(registry.snapshot().bindings.len(), 1);

    let second = registry.install_snapshot_for_test(Vec::new(), Vec::new());
    assert_eq!(second, 2);
    assert_eq!(registry.revision(), 2);
    assert!(registry.snapshot().is_empty());
}

#[test]
fn a_reload_that_fails_keeps_the_last_valid_bindings_and_marks_the_source_stale() {
    let source = project_source("/repo/.cursor/hooks.json");
    let root = Path::new("/repo");
    let mut registry = HookRegistry::new();

    let first = registry.upsert_source(update(&source, VALID, Some(root), TrustDecision::Activate));
    assert_eq!(first, 1);
    assert_eq!(tool_pre_bindings(&registry, "/repo/src").len(), 1);
    let status = registry.source_status(&source).expect("known source");
    assert!(status.loaded && !status.stale);
    assert_eq!(status.active_bindings, 1);

    let second = registry.upsert_source(update(
        &source,
        MALFORMED,
        Some(root),
        TrustDecision::Activate,
    ));
    assert_eq!(second, 1, "the active set did not change");
    assert_eq!(registry.generation(), 2, "but a snapshot was published");
    assert_eq!(
        tool_pre_bindings(&registry, "/repo/src").len(),
        1,
        "the previous bindings stay in effect"
    );
    assert!(registry.source_status(&source).expect("known source").stale);
    let errors = errors(&registry);
    assert_eq!(errors.len(), 1);
    assert!(
        errors[0].contains("previous valid configuration stays in effect"),
        "{errors:?}"
    );

    let third = registry.upsert_source(update(
        &source,
        VALID_TWO,
        Some(root),
        TrustDecision::Activate,
    ));
    assert_eq!(third, 2);
    assert_eq!(registry.generation(), 3);
    assert_eq!(tool_pre_bindings(&registry, "/repo/src").len(), 2);
    assert!(!registry.source_status(&source).expect("known source").stale);
    assert!(self::errors(&registry).is_empty());
}

#[test]
fn a_source_whose_first_load_fails_is_inactive_with_an_error() {
    let source = project_source("/repo/.cursor/hooks.json");
    let mut registry = HookRegistry::new();
    let revision = registry.upsert_source(update(
        &source,
        MALFORMED,
        Some(Path::new("/repo")),
        TrustDecision::Activate,
    ));
    assert_eq!(revision, 1, "the root became a project");
    assert!(registry.snapshot().is_empty());
    let status = registry.source_status(&source).expect("known source");
    assert!(!status.loaded && !status.stale);
    let errors = errors(&registry);
    assert_eq!(errors.len(), 1);
    assert!(errors[0].contains("is not active"), "{errors:?}");
    assert_eq!(
        registry.snapshot().project_roots,
        vec![PathBuf::from("/repo")],
        "a project with a broken file is still that directory's project"
    );
}

#[test]
fn a_file_whose_only_load_failed_is_still_protected() {
    let project = SourceOnDisk::project(MALFORMED);
    let mut registry = HookRegistry::new();
    registry.upsert_source(project.update(TrustDecision::Activate));
    assert!(registry.snapshot().is_empty());
    assert_eq!(
        registry.protected_paths(),
        std::slice::from_ref(&project.hooks_file)
    );
}

#[test]
fn a_known_root_without_a_hook_file_is_its_own_project() {
    let outer = project_source("/work/.cursor/hooks.json");
    let mut registry = HookRegistry::new();
    registry.upsert_source(update(
        &outer,
        VALID,
        Some(Path::new("/work")),
        TrustDecision::Activate,
    ));
    assert_eq!(tool_pre_bindings(&registry, "/work/inner/src").len(), 1);

    let revision = registry.add_project_root(Path::new("/work/inner"));
    assert_eq!(revision, 2);
    assert_eq!(
        registry.add_project_root(Path::new("/work/inner")),
        2,
        "a known root changes nothing"
    );
    assert!(
        tool_pre_bindings(&registry, "/work/inner/src").is_empty(),
        "the inner project has no hooks of its own and the outer project's do not reach in"
    );
    assert_eq!(tool_pre_bindings(&registry, "/work/src").len(), 1);
    assert_eq!(
        registry.snapshot().project_roots,
        vec![PathBuf::from("/work"), PathBuf::from("/work/inner")]
    );

    let inner = project_source("/work/inner/.cursor/hooks.json");
    registry.upsert_source(update(
        &inner,
        MALFORMED,
        Some(Path::new("/work/inner")),
        TrustDecision::Activate,
    ));
    assert!(
        tool_pre_bindings(&registry, "/work/inner/src").is_empty(),
        "a broken inner file does not hand the directory to the outer project"
    );

    registry.remove_source(&inner);
    let revision = registry.remove_project_root(Path::new("/work/inner"));
    assert_eq!(revision, registry.revision());
    assert_eq!(tool_pre_bindings(&registry, "/work/inner/src").len(), 1);
    assert_eq!(
        registry.remove_project_root(Path::new("/work/inner")),
        revision,
        "an unknown root changes nothing"
    );
}

#[test]
fn a_root_found_after_pinning_does_not_become_a_project_until_the_next_run() {
    let outer = project_source("/work/.cursor/hooks.json");
    let mut registry = HookRegistry::new();
    registry.upsert_source(update(
        &outer,
        VALID,
        Some(Path::new("/work")),
        TrustDecision::Activate,
    ));
    registry.pin(CommandExecutionPosture::AutoApproves);

    let revision = registry.add_project_root(Path::new("/work/inner"));
    assert_eq!(revision, 1, "the active set did not change");
    assert_eq!(registry.generation(), 3);
    assert_eq!(
        tool_pre_bindings(&registry, "/work/inner/src").len(),
        1,
        "the outer project's hooks keep applying where they did when the run started"
    );
    assert_eq!(
        registry.snapshot().project_roots,
        vec![PathBuf::from("/work")]
    );
    assert!(
        warnings(&registry)
            .iter()
            .any(|warning| warning.contains("the project was found after the run started")),
        "{:?}",
        warnings(&registry)
    );
    assert_eq!(
        registry.remove_project_root(Path::new("/work")),
        1,
        "the project set does not shrink after pinning either"
    );
    assert_eq!(tool_pre_bindings(&registry, "/work/src").len(), 1);
}

#[test]
fn a_read_failure_behaves_like_a_failed_parse() {
    let source = project_source("/repo/.cursor/hooks.json");
    let root = Path::new("/repo");
    let mut registry = HookRegistry::new();
    registry.upsert_source(update(&source, VALID, Some(root), TrustDecision::Activate));
    let revision = registry.record_read_failure(&source, "permission denied");
    assert_eq!(
        revision, 1,
        "the last valid bindings stay, so the revision does too"
    );
    assert_eq!(registry.generation(), 2);
    assert_eq!(tool_pre_bindings(&registry, "/repo").len(), 1);
    assert!(registry.source_status(&source).expect("known source").stale);

    let fresh = project_source("/other/.cursor/hooks.json");
    registry.record_read_failure(&fresh, "permission denied");
    let status = registry.source_status(&fresh).expect("known source");
    assert!(!status.loaded && !status.stale);
    assert_eq!(errors(&registry).len(), 2);
}

#[test]
fn after_pinning_a_changed_project_file_is_reported_and_not_applied() {
    let source = project_source("/repo/.cursor/hooks.json");
    let root = Path::new("/repo");
    let mut registry = HookRegistry::new();
    registry.upsert_source(update(&source, VALID, Some(root), TrustDecision::Activate));
    let pinned_at = registry.pin(CommandExecutionPosture::AutoApproves);
    assert_eq!(pinned_at, 1, "pinning does not change the active set");
    assert_eq!(registry.generation(), 2);
    assert_eq!(
        registry.pinned(),
        Some(PinnedRun {
            posture: CommandExecutionPosture::AutoApproves,
            revision: 1,
        })
    );
    assert!(
        registry
            .source_status(&source)
            .expect("known source")
            .pinned
    );

    let revision = registry.upsert_source(update(
        &source,
        VALID_TWO,
        Some(root),
        TrustDecision::Activate,
    ));
    assert_eq!(
        revision, 1,
        "a decision keyed by the revision still holds: the hooks that can run did not change"
    );
    assert_eq!(registry.generation(), 3, "the change was published");
    assert_eq!(
        tool_pre_bindings(&registry, "/repo").len(),
        1,
        "the pinned binding set is unchanged"
    );
    let warnings = warnings(&registry);
    assert_eq!(warnings.len(), 1);
    assert!(
        warnings[0].contains("changed after the run started"),
        "{warnings:?}"
    );

    // A second identical change does not pile up diagnostics; it is still published.
    let again = registry.upsert_source(update(
        &source,
        VALID_TWO,
        Some(root),
        TrustDecision::Activate,
    ));
    assert_eq!(again, 1);
    assert_eq!(registry.generation(), 4);
    assert_eq!(self::warnings(&registry).len(), 1);
}

#[test]
fn after_pinning_a_new_project_file_is_protected_and_waits_for_the_next_run() {
    let project = SourceOnDisk::project(VALID);
    let mut registry = HookRegistry::new();
    registry.pin(CommandExecutionPosture::AutoApproves);
    let revision = registry.upsert_source(project.update(TrustDecision::Activate));
    assert_eq!(
        revision, 0,
        "nothing can run, so the revision has never moved"
    );
    assert_eq!(registry.generation(), 2);
    assert!(registry.snapshot().is_empty());
    let status = registry.source_status(&project.source()).expect("known");
    assert!(status.loaded && !status.pinned);
    assert!(matches!(status.trust, TrustDecision::Inert { .. }));
    let mut expected = vec![project.hooks_file.clone(), project.root.join("hook.sh")];
    expected.sort();
    assert_eq!(registry.protected_paths(), expected.as_slice());
    assert!(
        registry.snapshot().project_roots.is_empty(),
        "the project set was pinned with the run"
    );
    assert!(
        warnings(&registry)
            .iter()
            .any(|warning| warning.contains("takes effect at the next run start")),
        "{:?}",
        warnings(&registry)
    );

    registry.set_trust(&project.source(), TrustDecision::Activate);
    assert!(
        registry.snapshot().is_empty(),
        "nothing activates it this run"
    );
}

#[test]
fn after_pinning_a_new_explicit_source_is_an_error() {
    let run = SourceOnDisk::new(HookSourceKind::RunDelivered, VALID);
    let mut registry = HookRegistry::new();
    registry.pin(CommandExecutionPosture::AutoApproves);
    registry.upsert_source(run.update(TrustDecision::Activate));
    assert!(registry.snapshot().is_empty());
    let errors = errors(&registry);
    assert_eq!(errors.len(), 1);
    assert!(
        errors[0].contains("explicit sources must be loaded before the run starts"),
        "{errors:?}"
    );
    assert_eq!(registry.protected_paths().len(), 2);
}

#[test]
fn after_pinning_an_explicit_source_cannot_change() {
    let run = SourceOnDisk::new(HookSourceKind::RunDelivered, VALID);
    let mut registry = HookRegistry::new();
    registry.upsert_source(run.update(TrustDecision::Activate));
    registry.pin(CommandExecutionPosture::AutoApproves);
    let revision = registry.upsert_source(run.update_with(VALID_TWO, TrustDecision::Activate));
    assert_eq!(revision, 1);
    assert_eq!(registry.generation(), 3);
    assert_eq!(registry.snapshot().bindings.len(), 1);
    assert!(
        warnings(&registry)[0].contains("cannot change after it starts"),
        "{:?}",
        warnings(&registry)
    );
}

#[test]
fn after_pinning_removing_or_re_trusting_a_source_keeps_its_bindings() {
    let source = project_source("/repo/.cursor/hooks.json");
    let root = Path::new("/repo");
    let mut registry = HookRegistry::new();
    registry.upsert_source(update(&source, VALID, Some(root), TrustDecision::Activate));
    registry.pin(CommandExecutionPosture::AutoApproves);

    let revision = registry.remove_source(&source);
    assert_eq!(revision, 1);
    assert_eq!(registry.generation(), 3);
    assert_eq!(tool_pre_bindings(&registry, "/repo").len(), 1);

    let revision = registry.set_trust(&source, inert());
    assert_eq!(revision, 1);
    assert_eq!(registry.generation(), 4);
    assert_eq!(tool_pre_bindings(&registry, "/repo").len(), 1);

    let revision = registry.remove_source_root(&source, root);
    assert_eq!(revision, 1);
    assert_eq!(registry.generation(), 5);
    assert_eq!(tool_pre_bindings(&registry, "/repo").len(), 1);
    assert!(
        warnings(&registry)
            .iter()
            .any(|warning| warning.contains("was removed after the run started")),
        "{:?}",
        warnings(&registry)
    );
}

#[test]
fn pinning_twice_changes_nothing() {
    let mut registry = HookRegistry::new();
    let first = registry.pin(CommandExecutionPosture::RequiresApproval);
    let second = registry.pin(CommandExecutionPosture::AutoApproves);
    assert_eq!(first, second);
    assert_eq!(
        registry.pinned().map(|pinned| pinned.posture),
        Some(CommandExecutionPosture::RequiresApproval)
    );
}

/// Every change publishes a snapshot; the revision moves with the hooks that can run and never
/// goes back.
#[test]
fn the_generation_moves_on_every_change_and_the_revision_with_the_active_set() {
    let source = project_source("/repo/.cursor/hooks.json");
    let root = Path::new("/repo");
    let mut registry = HookRegistry::new();
    let mut changes = Vec::new();
    let mut record = |registry: &HookRegistry, active_set_changed: bool| {
        changes.push((
            registry.revision(),
            registry.generation(),
            active_set_changed,
        ));
    };
    registry.upsert_source(update(&source, VALID, Some(root), TrustDecision::Activate));
    record(&registry, true);
    registry.upsert_source(update(
        &source,
        MALFORMED,
        Some(root),
        TrustDecision::Activate,
    ));
    record(&registry, false);
    registry.set_trust(&source, inert());
    record(&registry, true);
    registry.record_read_failure(&source, "gone");
    record(&registry, false);
    registry.remove_source(&source);
    record(&registry, false);
    registry.pin(CommandExecutionPosture::Unknown);
    record(&registry, false);

    let mut last = (0, 0);
    for (revision, generation, active_set_changed) in changes {
        assert_eq!(generation, last.1 + 1, "every change is published");
        if active_set_changed {
            assert_eq!(revision, last.0 + 1, "at generation {generation}");
        } else {
            assert_eq!(revision, last.0, "at generation {generation}");
        }
        last = (revision, generation);
    }
    assert_eq!(registry.snapshot().revision, last.0);
    assert_eq!(registry.snapshot().generation, last.1);
}

#[test]
fn changing_an_unknown_source_does_not_publish_anything() {
    let mut registry = HookRegistry::new();
    assert_eq!(registry.remove_source(&test_source_id("nope")), 0);
    assert_eq!(registry.set_trust(&test_source_id("nope"), inert()), 0);
    assert_eq!(
        registry.remove_source_root(&test_source_id("nope"), Path::new("/x")),
        0
    );
    assert_eq!(registry.generation(), 0);
}

/// Reloading a file whose definitions did not change, or whose only change is a diagnostic,
/// keeps a decision keyed by the revision valid.
#[test]
fn a_reload_with_the_same_definitions_keeps_the_revision() {
    let source = project_source("/repo/.cursor/hooks.json");
    let root = Path::new("/repo");
    let mut registry = HookRegistry::new();
    registry.upsert_source(update(&source, VALID, Some(root), TrustDecision::Activate));
    let same_again =
        registry.upsert_source(update(&source, VALID, Some(root), TrustDecision::Activate));
    assert_eq!(same_again, 1);
    assert_eq!(registry.generation(), 2);
}

#[test]
fn bindings_for_orders_by_precedence_and_picks_the_nearest_root() {
    let outer = project_source("/work/.cursor/hooks.json");
    let inner = project_source("/work/inner/.cursor/hooks.json");
    let run = SourceOnDisk::new(HookSourceKind::RunDelivered, VALID);
    let factory_wide = SourceOnDisk::new(HookSourceKind::FactoryWide, VALID);
    let factory_agent = SourceOnDisk::new(HookSourceKind::FactoryAgent, VALID);
    let mut registry = HookRegistry::new();
    registry.upsert_source(run.update(TrustDecision::Activate));
    registry.upsert_source(update(
        &outer,
        VALID,
        Some(Path::new("/work")),
        TrustDecision::Activate,
    ));
    registry.upsert_source(factory_wide.update(TrustDecision::Activate));
    registry.upsert_source(update(
        &inner,
        VALID,
        Some(Path::new("/work/inner")),
        TrustDecision::Activate,
    ));
    registry.upsert_source(factory_agent.update(TrustDecision::Activate));

    let run_name = run.source().to_string();
    let factory_wide_name = factory_wide.source().to_string();
    let factory_agent_name = factory_agent.source().to_string();
    let explicit = vec![
        factory_agent_name.clone(),
        factory_wide_name.clone(),
        run_name.clone(),
    ];

    let mut under_outer = explicit.clone();
    under_outer.push("/work/.cursor/hooks.json".to_owned());
    assert_eq!(
        source_names(&tool_pre_bindings(&registry, "/work/src")),
        under_outer
    );

    let mut under_inner = explicit.clone();
    under_inner.push("/work/inner/.cursor/hooks.json".to_owned());
    assert_eq!(
        source_names(&tool_pre_bindings(&registry, "/work/inner/src")),
        under_inner,
        "the inner root shadows the outer one"
    );

    assert_eq!(
        source_names(&tool_pre_bindings(&registry, "/elsewhere")),
        explicit,
        "a cwd outside every root gets only explicit sources"
    );
    assert_eq!(
        source_names(&registry.bindings_for(HookBoundary::ToolPre, None, HookExecutionHost::Local)),
        explicit
    );
    assert!(
        registry
            .bindings_for(
                HookBoundary::ToolResult,
                Some(Path::new("/work")),
                HookExecutionHost::Local
            )
            .is_empty()
    );
    assert_eq!(
        registry.snapshot().project_roots,
        vec![PathBuf::from("/work"), PathBuf::from("/work/inner")]
    );
}

#[test]
fn bindings_for_keeps_array_order_within_a_source() {
    let run = SourceOnDisk::new(HookSourceKind::RunDelivered, VALID_TWO);
    let mut registry = HookRegistry::new();
    registry.upsert_source(run.update(TrustDecision::Activate));
    let ids = registry
        .bindings_for(HookBoundary::ToolPre, None, HookExecutionHost::Local)
        .into_iter()
        .map(|active| active.binding.id.to_string())
        .collect::<Vec<_>>();
    let source = run.source().to_string();
    assert_eq!(
        ids,
        vec![
            format!("{source}#beforeShellExecution[0]"),
            format!("{source}#beforeShellExecution[1]")
        ]
    );
    assert_eq!(registry.snapshot().bindings[0].working_directory, run.root);
}

#[test]
fn a_project_file_runs_in_its_root() {
    let project = SourceOnDisk::project(VALID);
    let mut registry = HookRegistry::new();
    registry.upsert_source(project.update(TrustDecision::Activate));
    let bindings = tool_pre_bindings(&registry, project.root.join("src").to_str().unwrap());
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0].working_directory, project.root);
    assert_eq!(bindings[0].scope, BindingScope::Root(project.root.clone()));
}

#[test]
fn an_inert_source_contributes_no_bindings_but_is_reported() {
    let source = project_source("/repo/.cursor/hooks.json");
    let root = Path::new("/repo");
    let mut registry = HookRegistry::new();
    registry.upsert_source(update(&source, VALID, Some(root), inert()));
    assert!(registry.snapshot().is_empty());
    assert!(
        messages(&registry, DiagnosticSeverity::Info)
            .iter()
            .any(|message| message.contains("loaded but not active: test"))
    );
    assert_eq!(
        registry.snapshot().project_roots,
        vec![PathBuf::from("/repo")],
        "an inert project still counts as the project for its root"
    );

    registry.set_trust(&source, TrustDecision::Activate);
    assert_eq!(tool_pre_bindings(&registry, "/repo").len(), 1);
    registry.set_trust(&source, inert());
    assert!(registry.snapshot().is_empty());
}

#[cfg(unix)]
#[test]
fn a_file_reached_through_a_symlink_is_one_source_and_two_files_are_two() {
    let run = SourceOnDisk::new(HookSourceKind::RunDelivered, VALID);
    let link_dir = TempDir::new().expect("tempdir");
    let link = link_dir.path().join("hooks.json");
    std::os::unix::fs::symlink(&run.hooks_file, &link).expect("symlink");

    let mut registry = HookRegistry::new();
    for path in [&run.hooks_file, &link] {
        let canonical = dunce::canonicalize(path).expect("canonical");
        registry.upsert_source(SourceUpdate {
            source: HookSourceId {
                kind: HookSourceKind::RunDelivered,
                location: HookSourceLocation::CanonicalPath(canonical),
            },
            format: HookFormat::Cursor,
            bytes: &fs::read(path).expect("read"),
            project_root: None,
            trust: TrustDecision::Activate,
        });
    }
    assert_eq!(registry.generation(), 2);
    assert_eq!(registry.revision(), 1, "the second load changed nothing");
    assert_eq!(
        registry.snapshot().bindings.len(),
        1,
        "the symlink and its target are one source"
    );

    let copy = SourceOnDisk::new(HookSourceKind::RunDelivered, VALID);
    registry.upsert_source(copy.update(TrustDecision::Activate));
    assert_eq!(
        registry.snapshot().bindings.len(),
        2,
        "identical definitions in two files are two bindings"
    );
}

#[test]
fn a_file_delivered_with_the_run_and_found_as_a_project_file_runs_once() {
    let project = SourceOnDisk::project(VALID);
    let explicit = HookSourceId {
        kind: HookSourceKind::RunDelivered,
        location: HookSourceLocation::CanonicalPath(project.hooks_file.clone()),
    };
    let mut registry = HookRegistry::new();
    registry.upsert_source(project.update(TrustDecision::Activate));
    registry.upsert_source(SourceUpdate {
        source: explicit.clone(),
        format: HookFormat::Cursor,
        bytes: &project.bytes,
        project_root: None,
        trust: TrustDecision::Activate,
    });

    let cwd = project.root.join("src");
    let bindings = tool_pre_bindings(&registry, cwd.to_str().unwrap());
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0].binding.id.source, explicit);
    assert_eq!(
        registry
            .source_status(&project.source())
            .expect("known")
            .active_bindings,
        0
    );
    assert!(
        messages(&registry, DiagnosticSeverity::Info)
            .iter()
            .any(|message| message.contains("also delivered with the run"))
    );

    registry.remove_source(&explicit);
    assert_eq!(
        registry
            .source_status(&project.source())
            .expect("known")
            .active_bindings,
        1,
        "the project file runs again once it is the only source for the file"
    );
}

#[test]
fn a_project_file_reached_from_two_roots_applies_under_each() {
    let source = project_source("/shared/hooks.json");
    let mut registry = HookRegistry::new();
    registry.upsert_source(update(
        &source,
        VALID,
        Some(Path::new("/work/a")),
        TrustDecision::Activate,
    ));
    registry.upsert_source(update(
        &source,
        VALID,
        Some(Path::new("/work/b")),
        TrustDecision::Activate,
    ));
    assert_eq!(tool_pre_bindings(&registry, "/work/a/src").len(), 1);
    assert_eq!(tool_pre_bindings(&registry, "/work/b/src").len(), 1);
    assert_eq!(
        tool_pre_bindings(&registry, "/work/a/src")[0].working_directory,
        PathBuf::from("/work/a")
    );

    registry.remove_source_root(&source, Path::new("/work/a"));
    assert!(tool_pre_bindings(&registry, "/work/a/src").is_empty());
    assert_eq!(tool_pre_bindings(&registry, "/work/b/src").len(), 1);
    registry.remove_source_root(&source, Path::new("/work/b"));
    assert!(registry.source_status(&source).is_none());
}

#[test]
fn protected_paths_cover_the_source_file_and_every_script_that_resolves() {
    let project = SourceOnDisk::project(
        r#"{ "version": 1, "hooks": { "beforeShellExecution": [
            { "command": "./hook.sh --strict" },
            { "command": "npx some-linter" },
            { "command": "./missing.sh" },
            { "command": "bash scripts/check.sh --fix" },
            { "command": "node ./hooks/format.js" },
            { "command": "python3 hooks/audit.py" },
            { "command": "python3 hooks/missing.py" }
        ] } }"#,
    );
    fs::create_dir_all(project.root.join("scripts")).expect("mkdir");
    fs::create_dir_all(project.root.join("hooks")).expect("mkdir");
    for script in ["scripts/check.sh", "hooks/format.js", "hooks/audit.py"] {
        fs::write(project.root.join(script), "").expect("write");
    }
    let mut registry = HookRegistry::new();
    registry.upsert_source(project.update(TrustDecision::Activate));
    let mut expected = vec![
        project.hooks_file.clone(),
        project.root.join("hook.sh"),
        project.root.join("scripts/check.sh"),
        project.root.join("hooks/format.js"),
        project.root.join("hooks/audit.py"),
    ];
    expected.sort();
    assert_eq!(registry.protected_paths(), expected.as_slice());
    assert_eq!(registry.snapshot().bindings.len(), 7);
}

#[test]
fn protected_paths_resolve_explicit_source_scripts_against_the_declaring_directory() {
    let run = SourceOnDisk::new(HookSourceKind::RunDelivered, VALID);
    let mut registry = HookRegistry::new();
    registry.upsert_source(run.update(TrustDecision::Activate));
    let mut expected = vec![run.hooks_file.clone(), run.root.join("hook.sh")];
    expected.sort();
    assert_eq!(registry.protected_paths(), expected.as_slice());
}

#[test]
fn protected_paths_include_inert_and_stale_sources() {
    let project = SourceOnDisk::project(VALID);
    let mut registry = HookRegistry::new();
    registry.upsert_source(project.update(inert()));
    assert_eq!(registry.protected_paths().len(), 2);
    registry.upsert_source(project.update_with(MALFORMED, inert()));
    assert_eq!(registry.protected_paths().len(), 2);
}

#[test]
fn protected_paths_are_recomputed_when_a_source_goes_away() {
    let project = SourceOnDisk::project(VALID);
    let mut registry = HookRegistry::new();
    registry.upsert_source(project.update(TrustDecision::Activate));
    assert_eq!(registry.protected_paths().len(), 2);
    registry.remove_source(&project.source());
    assert!(registry.protected_paths().is_empty());
}

#[test]
fn a_named_explicit_source_with_no_directory_does_not_activate() {
    let source = test_source_id("run");
    let mut registry = HookRegistry::new();
    registry.upsert_source(update(&source, VALID, None, TrustDecision::Activate));
    assert!(registry.snapshot().is_empty());
    assert!(
        errors(&registry)[0].contains("has no directory to run its hooks in"),
        "{:?}",
        errors(&registry)
    );
    assert!(registry.protected_paths().is_empty());
}

#[test]
fn resolve_timeout_returns_the_declared_or_default_value_and_rejects_values_above_the_ceiling() {
    let default = test_binding(test_source_id("s"), HookBoundary::ToolPre, 0, None);
    assert_eq!(
        resolve_timeout(&default),
        Ok(HookBoundary::ToolPre.default_timeout())
    );

    let mut declared = default.clone();
    declared.timeout = Duration::from_secs(5);
    assert_eq!(resolve_timeout(&declared), Ok(Duration::from_secs(5)));

    let mut at_ceiling = default.clone();
    at_ceiling.timeout = HOOK_TIMEOUT_CEILING;
    assert_eq!(resolve_timeout(&at_ceiling), Ok(HOOK_TIMEOUT_CEILING));

    let mut above = default;
    above.timeout = HOOK_TIMEOUT_CEILING + Duration::from_secs(1);
    let error = resolve_timeout(&above).expect_err("above the ceiling");
    assert!(error.contains("above the 600s limit"), "{error}");
}

#[test]
fn a_definition_above_the_timeout_ceiling_is_skipped_with_a_diagnostic() {
    let run = SourceOnDisk::new(
        HookSourceKind::RunDelivered,
        &json!({
            "version": 1,
            "hooks": { "beforeShellExecution": [
                { "command": "./slow.sh", "timeout": 601 },
                { "command": "./ok.sh", "timeout": 600 }
            ] }
        })
        .to_string(),
    );
    let mut registry = HookRegistry::new();
    registry.upsert_source(run.update(TrustDecision::Activate));
    let bindings = registry.bindings_for(HookBoundary::ToolPre, None, HookExecutionHost::Local);
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0].binding.command, "./ok.sh");
    let errors = errors(&registry);
    assert_eq!(errors.len(), 1);
    assert!(
        errors[0].contains("beforeShellExecution[0]") && errors[0].contains("skipped"),
        "{errors:?}"
    );
}

#[test]
fn has_bindings_for_reports_the_boundary_before_scope_filtering() {
    let source = project_source("/repo/.cursor/hooks.json");
    let mut registry = HookRegistry::new();
    registry.upsert_source(update(
        &source,
        VALID,
        Some(Path::new("/repo")),
        TrustDecision::Activate,
    ));
    assert!(registry.snapshot().has_bindings_for(HookBoundary::ToolPre));
    assert!(!registry.snapshot().has_bindings_for(HookBoundary::TurnStop));
}
