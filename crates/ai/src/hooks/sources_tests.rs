use std::path::{Path, PathBuf};

use super::*;
use crate::skills::parse_skills_dirs_value;

#[test]
fn hook_dirs_are_split_on_commas_trimmed_and_blank_entries_dropped() {
    let dirs = parse_hook_dirs_value(" /a/hooks , ,/b/hooks,, relative/hooks ,");
    assert_eq!(
        dirs,
        vec![
            PathBuf::from("/a/hooks"),
            PathBuf::from("/b/hooks"),
            PathBuf::from("relative/hooks"),
        ]
    );
    assert!(parse_hook_dirs_value("").is_empty());
    assert!(parse_hook_dirs_value(" , ,").is_empty());
}

#[test]
fn a_leading_tilde_expands_to_the_home_directory() {
    let dirs = parse_hook_dirs_value("~/hooks");
    let home = dirs::home_dir().expect("home directory");
    assert_eq!(dirs, vec![home.join("hooks")]);
}

/// The two variables share one grammar, so a value means the same thing to both.
#[test]
fn hook_dirs_parse_exactly_like_skill_dirs() {
    for value in [
        "",
        " ",
        "/a,/b",
        " /a , /b ",
        "~/x,relative,,/abs/path",
        "a,b,c",
        "~",
    ] {
        assert_eq!(
            parse_hook_dirs_value(value),
            parse_skills_dirs_value(value),
            "{value:?}"
        );
    }
}

#[test]
fn relative_hook_dirs_resolve_against_the_base_and_absolute_ones_pass_through() {
    let resolved = resolve_hook_dirs(
        Path::new("/run/cwd"),
        vec![PathBuf::from("hooks"), PathBuf::from("/abs/hooks")],
    );
    assert_eq!(
        resolved,
        vec![PathBuf::from("/run/cwd/hooks"), PathBuf::from("/abs/hooks")]
    );
}

#[test]
fn each_hook_dir_names_its_hooks_json_as_a_run_delivered_cursor_source() {
    let sources = sources_for_hook_dirs(&[PathBuf::from("/a"), PathBuf::from("/b")]);
    assert_eq!(
        sources,
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
fn scopes_map_to_their_source_kinds() {
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

    use tempfile::TempDir;

    use super::*;
    use crate::hooks::model::{
        DiagnosticSeverity, HookBoundary, HookExecutionHost, HookSourceId, HookSourceLocation,
    };
    use crate::hooks::registry::HookRegistry;
    use crate::hooks::trust::TrustDecision;

    const VALID: &str =
        r#"{ "version": 1, "hooks": { "beforeShellExecution": [ { "command": "./hook.sh" } ] } }"#;

    fn hook_dir(hooks_json: &str) -> TempDir {
        let dir = TempDir::new().expect("tempdir");
        fs::write(dir.path().join("hooks.json"), hooks_json).expect("write");
        dir
    }

    #[test]
    fn explicit_sources_activate_without_consulting_the_trust_table() {
        let dir = hook_dir(VALID);
        let factory = hook_dir(VALID);
        let mut registry = HookRegistry::new();
        let failures = load_explicit_sources(
            &[
                ExplicitHookSource {
                    path: dir.path().join("hooks.json"),
                    format: HookFormat::Cursor,
                    scope: ExplicitSourceScope::Run,
                },
                ExplicitHookSource {
                    path: factory.path().join("hooks.json"),
                    format: HookFormat::Cursor,
                    scope: ExplicitSourceScope::FactoryAgent,
                },
            ],
            &mut registry,
        );
        assert!(failures.is_empty());
        let bindings = registry.bindings_for(
            HookBoundary::ToolPre,
            Some(Path::new("/anywhere")),
            HookExecutionHost::Local,
        );
        assert_eq!(bindings.len(), 2);
        assert_eq!(
            bindings[0].binding.id.source.kind,
            HookSourceKind::FactoryAgent,
            "the factory agent definition sorts first"
        );
        assert_eq!(
            bindings[1].working_directory,
            dunce::canonicalize(dir.path()).expect("canonical"),
            "a run-delivered source runs in its own directory"
        );
        for binding in &bindings {
            let status = registry
                .source_status(&binding.binding.id.source)
                .expect("known");
            assert_eq!(status.trust, TrustDecision::Activate);
        }
    }

    #[test]
    fn a_missing_explicit_source_is_a_diagnostic_rather_than_a_panic() {
        let dir = TempDir::new().expect("tempdir");
        let missing = dir.path().join("hooks.json");
        let mut registry = HookRegistry::new();
        let failures = load_explicit_sources(
            &sources_for_hook_dirs(&[dir.path().to_path_buf()]),
            &mut registry,
        );
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].severity, DiagnosticSeverity::Error);
        assert_eq!(
            failures[0].source,
            HookSourceId {
                kind: HookSourceKind::RunDelivered,
                location: HookSourceLocation::Named(missing.display().to_string()),
            }
        );
        assert!(registry.snapshot().is_empty());
        assert_eq!(registry.snapshot().diagnostics, failures);
    }

    #[test]
    fn an_invalid_explicit_source_loads_as_an_inactive_source_with_an_error() {
        let dir = hook_dir("{ not json");
        let mut registry = HookRegistry::new();
        let failures = load_explicit_sources(
            &sources_for_hook_dirs(&[dir.path().to_path_buf()]),
            &mut registry,
        );
        assert!(
            failures.is_empty(),
            "a parse error is the registry's diagnostic, not a read failure"
        );
        assert!(registry.snapshot().is_empty());
        assert_eq!(registry.snapshot().diagnostics.len(), 1);
        assert_eq!(
            registry.snapshot().diagnostics[0].severity,
            DiagnosticSeverity::Error
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_source_named_twice_through_a_symlink_loads_once() {
        let dir = hook_dir(VALID);
        let link_dir = TempDir::new().expect("tempdir");
        std::os::unix::fs::symlink(dir.path(), link_dir.path().join("linked")).expect("symlink");
        let mut registry = HookRegistry::new();
        load_explicit_sources(
            &sources_for_hook_dirs(&[dir.path().to_path_buf(), link_dir.path().join("linked")]),
            &mut registry,
        );
        assert_eq!(registry.snapshot().bindings.len(), 1);
        assert_eq!(registry.revision(), 2);
    }
}
