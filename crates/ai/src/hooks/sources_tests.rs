use std::path::{Path, PathBuf};

use super::*;
use crate::skills::parse_skills_dirs_value;

#[test]
fn hook_dirs_are_split_trimmed_and_resolved_like_skill_dirs() {
    let value = " /a/hooks , ,relative/hooks,~/shared ";
    assert_eq!(
        parse_hook_dirs_value(value),
        parse_skills_dirs_value(value)
    );
    assert!(parse_hook_dirs_value(" , ,").is_empty());

    assert_eq!(
        resolve_hook_dirs(
            Path::new("/run/cwd"),
            vec![PathBuf::from("hooks"), PathBuf::from("/abs/hooks")],
        ),
        vec![PathBuf::from("/run/cwd/hooks"), PathBuf::from("/abs/hooks")]
    );
}

#[test]
fn configured_directories_become_run_delivered_cursor_sources() {
    assert_eq!(
        sources_for_hook_dirs(&[PathBuf::from("/a"), PathBuf::from("/b")]),
        vec![
            ExplicitHookSource {
                path: PathBuf::from("/a/hooks.json"),
                format: HookFormat::Cursor,
                scope: ExplicitSourceScope::Run,
            },
            ExplicitHookSource {
                path: PathBuf::from("/b/hooks.json"),
                format: HookFormat::Cursor,
                scope: ExplicitSourceScope::Run,
            },
        ]
    );
}

#[test]
fn scopes_map_to_source_priority_kinds() {
    assert_eq!(
        ExplicitSourceScope::Run.source_kind(),
        HookSourceKind::RunDelivered
    );
    assert_eq!(
        ExplicitSourceScope::FactoryAgent.source_kind(),
        HookSourceKind::FactoryAgent
    );
    assert_eq!(
        ExplicitSourceScope::FactoryWide.source_kind(),
        HookSourceKind::FactoryWide
    );
}

#[cfg(feature = "local_fs")]
mod loading {
    use std::fs;
    use std::sync::Arc;

    use tempfile::TempDir;

    use super::*;
    use crate::hooks::model::{
        DiagnosticSeverity, HookBoundary, HookExecutionHost, HookSourceKind,
    };
    use crate::hooks::registry::HookRegistry;

    const VALID: &str =
        r#"{ "version": 1, "hooks": { "beforeShellExecution": [ { "command": "./hook.sh" } ] } }"#;

    fn hook_dir(hooks_json: &str) -> TempDir {
        let dir = TempDir::new().expect("tempdir");
        fs::write(dir.path().join("hooks.json"), hooks_json).expect("write");
        dir
    }

    fn source(dir: &TempDir, scope: ExplicitSourceScope) -> ExplicitHookSource {
        ExplicitHookSource {
            path: dir.path().join("hooks.json"),
            format: HookFormat::Cursor,
            scope,
        }
    }

    #[test]
    fn configured_sources_activate_and_finalize_without_a_trust_decision() {
        let run = hook_dir(VALID);
        let factory = hook_dir(VALID);
        let mut registry = HookRegistry::new();

        let failures = load_explicit_sources(
            &[
                source(&run, ExplicitSourceScope::Run),
                source(&factory, ExplicitSourceScope::FactoryAgent),
            ],
            &mut registry,
        );

        assert!(failures.is_empty(), "{failures:?}");
        assert!(registry.is_finalized());
        let bindings =
            registry.bindings_for(HookBoundary::ToolPre, HookExecutionHost::Local);
        assert_eq!(bindings.len(), 2);
        assert_eq!(
            bindings[0].binding.id.source.kind,
            HookSourceKind::FactoryAgent
        );
        assert_eq!(
            bindings[1].working_directory,
            dunce::canonicalize(run.path()).expect("canonical")
        );
    }

    #[test]
    fn an_empty_resolved_set_is_finalized() {
        let mut registry = HookRegistry::new();
        assert!(load_explicit_sources(&[], &mut registry).is_empty());
        assert!(registry.is_finalized());
        assert!(registry.snapshot().is_empty());
    }

    #[test]
    fn missing_and_invalid_sources_are_reported_before_finalization() {
        let missing_dir = TempDir::new().expect("tempdir");
        let invalid = hook_dir("{ not json");
        let mut registry = HookRegistry::new();

        let failures = load_explicit_sources(
            &[
                ExplicitHookSource {
                    path: missing_dir.path().join("hooks.json"),
                    format: HookFormat::Cursor,
                    scope: ExplicitSourceScope::Run,
                },
                source(&invalid, ExplicitSourceScope::FactoryWide),
            ],
            &mut registry,
        );

        assert_eq!(failures.len(), 2, "{failures:?}");
        assert!(registry.is_finalized());
        assert!(registry.snapshot().is_empty());
        assert!(
            failures
                .iter()
                .all(|failure| failure.severity == DiagnosticSeverity::Error)
        );
    }

    #[test]
    fn a_finalized_set_rejects_later_sources_and_keeps_its_snapshot() {
        let initial = hook_dir(VALID);
        let later = hook_dir(VALID);
        let mut registry = HookRegistry::new();
        assert!(
            load_explicit_sources(
                &[source(&initial, ExplicitSourceScope::Run)],
                &mut registry
            )
            .is_empty()
        );
        let snapshot = registry.snapshot();

        let failures = load_explicit_sources(
            &[source(&later, ExplicitSourceScope::FactoryAgent)],
            &mut registry,
        );

        assert_eq!(failures.len(), 1);
        assert_eq!(registry.snapshot().revision, snapshot.revision);
        assert_eq!(registry.snapshot().bindings.len(), 1);
    }

    #[test]
    fn editing_a_configuration_after_finalization_does_not_change_the_snapshot() {
        let dir = hook_dir(VALID);
        let mut registry = HookRegistry::new();
        assert!(
            load_explicit_sources(&[source(&dir, ExplicitSourceScope::Run)], &mut registry)
                .is_empty()
        );
        let before = registry.snapshot();

        fs::write(
            dir.path().join("hooks.json"),
            r#"{ "version": 1, "hooks": {} }"#,
        )
        .expect("rewrite");

        let after = registry.snapshot();
        assert!(Arc::ptr_eq(&before, &after));
        assert_eq!(after.bindings.len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn a_canonical_source_selected_twice_runs_once_at_the_higher_priority() {
        let dir = hook_dir(VALID);
        let link_dir = TempDir::new().expect("tempdir");
        std::os::unix::fs::symlink(dir.path(), link_dir.path().join("linked")).expect("symlink");
        let mut registry = HookRegistry::new();

        let failures = load_explicit_sources(
            &[
                source(&dir, ExplicitSourceScope::Run),
                ExplicitHookSource {
                    path: link_dir.path().join("linked/hooks.json"),
                    format: HookFormat::Cursor,
                    scope: ExplicitSourceScope::FactoryAgent,
                },
            ],
            &mut registry,
        );

        assert!(failures.is_empty(), "{failures:?}");
        assert_eq!(registry.snapshot().bindings.len(), 1);
        assert_eq!(
            registry.snapshot().bindings[0].binding.id.source.kind,
            HookSourceKind::FactoryAgent
        );
    }
}
