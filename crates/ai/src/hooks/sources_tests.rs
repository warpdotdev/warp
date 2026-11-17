use std::path::PathBuf;

use super::*;
use crate::hooks::model::HookOnAsk;

#[test]
fn a_source_from_a_repository_directory_runs_from_the_repository_root() {
    let source = ExplicitHookSource::from_directory(
        HookFormat::Cursor,
        &HookDirectory::Repository {
            root: PathBuf::from("/work/api"),
            directory: PathBuf::from("/work/api/.cursor"),
        },
        HookSourceKind::FactoryAgent,
        HookSourcePolicy {
            strict: true,
            on_ask: HookOnAsk::FailRun,
        },
    );
    assert_eq!(
        source,
        ExplicitHookSource {
            config_file: PathBuf::from("/work/api/.cursor/hooks.json"),
            format: HookFormat::Cursor,
            kind: HookSourceKind::FactoryAgent,
            working_directory: PathBuf::from("/work/api"),
            project_dir: Some(PathBuf::from("/work/api")),
            policy: HookSourcePolicy {
                strict: true,
                on_ask: HookOnAsk::FailRun,
            },
        }
    );
}

#[test]
fn a_source_from_a_worker_directory_runs_from_that_directory() {
    let source = ExplicitHookSource::from_directory(
        HookFormat::Cursor,
        &HookDirectory::Worker {
            directory: PathBuf::from("/etc/acme/warp-hooks"),
        },
        HookSourceKind::FactoryWide,
        HookSourcePolicy::default(),
    );
    assert_eq!(
        source.config_file,
        PathBuf::from("/etc/acme/warp-hooks/hooks.json")
    );
    assert_eq!(
        source.working_directory,
        PathBuf::from("/etc/acme/warp-hooks")
    );
    assert_eq!(source.project_dir, None);
}

#[test]
fn the_stricter_policy_wins_each_setting() {
    let lax = HookSourcePolicy::default();
    let strict_only = HookSourcePolicy {
        strict: true,
        on_ask: HookOnAsk::Deny,
    };
    let fail_only = HookSourcePolicy {
        strict: false,
        on_ask: HookOnAsk::FailRun,
    };
    assert_eq!(lax.stricter(lax), lax);
    assert_eq!(
        strict_only.stricter(fail_only),
        HookSourcePolicy {
            strict: true,
            on_ask: HookOnAsk::FailRun,
        }
    );
    assert_eq!(fail_only.stricter(lax), fail_only);
}

#[cfg(feature = "local_fs")]
mod loading {
    use std::fs;
    use std::path::Path;
    use std::sync::Arc;

    use tempfile::TempDir;

    use super::*;
    use crate::hooks::model::{
        DiagnosticSeverity, HookBoundary, HookExecutionHost, HookSourceKind,
    };
    use crate::hooks::registry::HookRegistry;
    use crate::hooks::test_util::{repository_source, worker_source};

    const VALID: &str =
        r#"{ "version": 1, "hooks": { "beforeShellExecution": [ { "command": "./hook.sh" } ] } }"#;

    fn hook_dir(hooks_json: &str) -> TempDir {
        let dir = TempDir::new().expect("tempdir");
        fs::write(dir.path().join("hooks.json"), hooks_json).expect("write");
        dir
    }

    fn canonical(path: &Path) -> PathBuf {
        dunce::canonicalize(path).expect("canonical")
    }

    fn strict() -> HookSourcePolicy {
        HookSourcePolicy {
            strict: true,
            on_ask: HookOnAsk::Deny,
        }
    }

    fn load(sources: &[ExplicitHookSource], registry: &mut HookRegistry) -> Vec<SourceLoadFailure> {
        load_explicit_sources(sources, registry).expect("not finalized yet")
    }

    #[test]
    fn configured_sources_activate_and_finalize_without_a_trust_decision() {
        let agent = hook_dir(VALID);
        let factory = hook_dir(VALID);
        let mut registry = HookRegistry::new();

        let failures = load(
            &[
                worker_source(
                    factory.path(),
                    HookSourceKind::FactoryWide,
                    HookSourcePolicy::default(),
                ),
                worker_source(agent.path(), HookSourceKind::FactoryAgent, strict()),
            ],
            &mut registry,
        );

        assert!(failures.is_empty(), "{failures:?}");
        assert!(registry.is_finalized());
        let bindings = registry.bindings_for(HookBoundary::ToolPre, HookExecutionHost::Local);
        assert_eq!(bindings.len(), 2);
        assert_eq!(
            bindings[0].binding.id.source.kind,
            HookSourceKind::FactoryAgent
        );
        assert_eq!(bindings[0].policy, strict());
        assert_eq!(bindings[1].policy, HookSourcePolicy::default());
        assert_eq!(bindings[1].working_directory, factory.path());
        assert_eq!(bindings[1].project_dir, None);
    }

    #[test]
    fn a_repository_source_runs_from_the_repository_root_and_names_it_as_the_project() {
        let repo = TempDir::new().expect("tempdir");
        let cursor = repo.path().join(".cursor");
        fs::create_dir_all(&cursor).expect("mkdir");
        fs::write(cursor.join("hooks.json"), VALID).expect("write");
        let mut registry = HookRegistry::new();

        let failures = load(
            &[repository_source(
                repo.path(),
                &cursor,
                HookSourceKind::FactoryAgent,
                HookSourcePolicy::default(),
            )],
            &mut registry,
        );

        assert!(failures.is_empty(), "{failures:?}");
        let bindings = registry.bindings_for(HookBoundary::ToolPre, HookExecutionHost::Local);
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].working_directory, repo.path());
        assert_eq!(bindings[0].project_dir.as_deref(), Some(repo.path()));
    }

    #[test]
    fn an_empty_resolved_set_is_finalized() {
        let mut registry = HookRegistry::new();
        assert!(load(&[], &mut registry).is_empty());
        assert!(registry.is_finalized());
        assert!(registry.snapshot().is_empty());
    }

    #[test]
    fn each_failure_carries_its_own_sources_strict_setting() {
        let missing_dir = TempDir::new().expect("tempdir");
        let invalid = hook_dir("{ not json");
        let mut registry = HookRegistry::new();

        let failures = load(
            &[
                worker_source(missing_dir.path(), HookSourceKind::FactoryAgent, strict()),
                worker_source(
                    invalid.path(),
                    HookSourceKind::FactoryWide,
                    HookSourcePolicy::default(),
                ),
            ],
            &mut registry,
        );

        assert_eq!(
            failures
                .iter()
                .map(|failure| failure.strict)
                .collect::<Vec<_>>(),
            vec![true, false],
            "{failures:?}"
        );
        assert!(registry.is_finalized());
        assert!(registry.snapshot().is_empty());
        assert!(
            failures
                .iter()
                .all(|failure| failure.diagnostic.severity == DiagnosticSeverity::Error)
        );
    }

    /// One source failing does not stop the others: the valid one still loads, each failure
    /// carries its own source's `strict`, and the set is finalized.
    #[test]
    fn a_valid_source_loads_between_failing_ones() {
        let missing_dir = TempDir::new().expect("tempdir");
        let valid = hook_dir(VALID);
        let invalid = hook_dir("{ not json");
        let mut registry = HookRegistry::new();

        let failures = load(
            &[
                worker_source(missing_dir.path(), HookSourceKind::FactoryAgent, strict()),
                worker_source(
                    valid.path(),
                    HookSourceKind::FactoryAgent,
                    HookSourcePolicy::default(),
                ),
                worker_source(
                    invalid.path(),
                    HookSourceKind::FactoryWide,
                    HookSourcePolicy::default(),
                ),
            ],
            &mut registry,
        );

        assert_eq!(
            failures
                .iter()
                .map(|failure| failure.strict)
                .collect::<Vec<_>>(),
            vec![true, false],
            "{failures:?}"
        );
        assert!(registry.is_finalized());
        let snapshot = registry.snapshot();
        assert_eq!(snapshot.bindings.len(), 1);
        assert_eq!(
            snapshot.bindings[0].binding.id.source.location,
            crate::hooks::model::HookSourceLocation::CanonicalPath(canonical(
                &valid.path().join("hooks.json")
            ))
        );
    }

    /// A file that cannot be resolved has no canonical identity, so two entries naming it are not
    /// merged: each reports its own failure with its own `strict`. A strict copy still fails the
    /// run, which is the stricter-wins outcome; the cost is a repeated diagnostic.
    #[test]
    fn two_entries_for_one_missing_file_each_report_a_failure() {
        let missing_dir = TempDir::new().expect("tempdir");
        let mut registry = HookRegistry::new();

        let failures = load(
            &[
                worker_source(
                    missing_dir.path(),
                    HookSourceKind::FactoryAgent,
                    HookSourcePolicy::default(),
                ),
                worker_source(missing_dir.path(), HookSourceKind::FactoryWide, strict()),
            ],
            &mut registry,
        );

        assert_eq!(
            failures
                .iter()
                .map(|failure| failure.strict)
                .collect::<Vec<_>>(),
            vec![false, true],
            "{failures:?}"
        );
        assert!(registry.is_finalized());
    }

    #[test]
    fn a_rejected_definition_is_a_failure_of_its_source() {
        let dir = hook_dir(
            r#"{ "version": 1, "hooks": { "beforeShellExecution": [ { "timeout": 5 } ] } }"#,
        );
        let mut registry = HookRegistry::new();

        let failures = load(
            &[worker_source(
                dir.path(),
                HookSourceKind::FactoryAgent,
                strict(),
            )],
            &mut registry,
        );

        assert_eq!(failures.len(), 1, "{failures:?}");
        assert!(failures[0].strict);
    }

    #[test]
    fn a_finalized_set_rejects_a_second_load_and_keeps_its_snapshot() {
        let initial = hook_dir(VALID);
        let later = hook_dir(VALID);
        let mut registry = HookRegistry::new();
        assert!(
            load(
                &[worker_source(
                    initial.path(),
                    HookSourceKind::FactoryAgent,
                    HookSourcePolicy::default(),
                )],
                &mut registry,
            )
            .is_empty()
        );
        let snapshot = registry.snapshot();

        let result = load_explicit_sources(
            &[worker_source(
                later.path(),
                HookSourceKind::FactoryAgent,
                HookSourcePolicy::default(),
            )],
            &mut registry,
        );

        assert!(result.is_err());
        assert_eq!(registry.snapshot().revision, snapshot.revision);
        assert_eq!(registry.snapshot().bindings.len(), 1);
    }

    #[test]
    fn editing_a_configuration_after_finalization_does_not_change_the_snapshot() {
        let dir = hook_dir(VALID);
        let mut registry = HookRegistry::new();
        assert!(
            load(
                &[worker_source(
                    dir.path(),
                    HookSourceKind::FactoryAgent,
                    HookSourcePolicy::default(),
                )],
                &mut registry,
            )
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

    /// An agent entry and a factory entry name the same directory: it runs once, under the agent
    /// entry, with the factory entry's stricter settings.
    #[test]
    fn a_file_selected_by_agent_and_factory_runs_once_under_the_agent_with_the_stricter_policy() {
        let dir = hook_dir(VALID);
        let mut registry = HookRegistry::new();

        let failures = load(
            &[
                worker_source(
                    dir.path(),
                    HookSourceKind::FactoryAgent,
                    HookSourcePolicy::default(),
                ),
                worker_source(
                    dir.path(),
                    HookSourceKind::FactoryWide,
                    HookSourcePolicy {
                        strict: true,
                        on_ask: HookOnAsk::FailRun,
                    },
                ),
            ],
            &mut registry,
        );

        assert!(failures.is_empty(), "{failures:?}");
        let snapshot = registry.snapshot();
        assert_eq!(snapshot.bindings.len(), 1);
        let binding = &snapshot.bindings[0];
        assert_eq!(binding.binding.id.source.kind, HookSourceKind::FactoryAgent);
        assert_eq!(
            binding.policy,
            HookSourcePolicy {
                strict: true,
                on_ask: HookOnAsk::FailRun,
            }
        );
        assert!(
            snapshot.diagnostics.is_empty(),
            "agreeing duplicates are not reported: {:?}",
            snapshot.diagnostics
        );
    }

    /// Two entries at the same level reach one file from different locations: the first entry's
    /// location is used and the disagreement is reported.
    #[test]
    fn a_file_selected_twice_from_different_locations_uses_the_first_and_warns() {
        let repo = TempDir::new().expect("tempdir");
        let cursor = repo.path().join(".cursor");
        fs::create_dir_all(&cursor).expect("mkdir");
        fs::write(cursor.join("hooks.json"), VALID).expect("write");
        let mut registry = HookRegistry::new();

        let failures = load(
            &[
                repository_source(
                    repo.path(),
                    &cursor,
                    HookSourceKind::FactoryAgent,
                    HookSourcePolicy::default(),
                ),
                worker_source(&cursor, HookSourceKind::FactoryAgent, strict()),
            ],
            &mut registry,
        );

        assert!(failures.is_empty(), "{failures:?}");
        let snapshot = registry.snapshot();
        assert_eq!(snapshot.bindings.len(), 1);
        assert_eq!(snapshot.bindings[0].working_directory, repo.path());
        assert_eq!(snapshot.bindings[0].policy, strict());
        assert_eq!(snapshot.diagnostics.len(), 1, "{:?}", snapshot.diagnostics);
        let warning = &snapshot.diagnostics[0];
        assert_eq!(warning.severity, DiagnosticSeverity::Warning);
        assert!(
            warning.message.contains(&repo.path().display().to_string())
                && warning.message.contains(&cursor.display().to_string()),
            "{}",
            warning.message
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_file_reached_through_a_link_is_the_same_source() {
        let dir = hook_dir(VALID);
        let link_dir = TempDir::new().expect("tempdir");
        let linked = link_dir.path().join("linked");
        std::os::unix::fs::symlink(dir.path(), &linked).expect("symlink");
        let mut registry = HookRegistry::new();

        let failures = load(
            &[
                worker_source(
                    &linked,
                    HookSourceKind::FactoryAgent,
                    HookSourcePolicy::default(),
                ),
                worker_source(
                    dir.path(),
                    HookSourceKind::FactoryWide,
                    HookSourcePolicy::default(),
                ),
            ],
            &mut registry,
        );

        assert!(failures.is_empty(), "{failures:?}");
        let snapshot = registry.snapshot();
        assert_eq!(snapshot.bindings.len(), 1);
        assert_eq!(
            snapshot.bindings[0].binding.id.source.location,
            crate::hooks::model::HookSourceLocation::CanonicalPath(canonical(
                &dir.path().join("hooks.json")
            ))
        );
        assert!(
            snapshot.diagnostics.is_empty(),
            "a link to the same directory is not a different location: {:?}",
            snapshot.diagnostics
        );
    }
}
