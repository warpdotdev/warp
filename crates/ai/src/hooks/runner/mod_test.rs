use std::path::{Path, PathBuf};

use super::*;

fn file_source(kind: HookSourceKind, path: &str) -> HookSourceId {
    HookSourceId {
        kind,
        location: HookSourceLocation::CanonicalPath(PathBuf::from(path)),
    }
}

fn named_source(kind: HookSourceKind) -> HookSourceId {
    HookSourceId {
        kind,
        location: HookSourceLocation::Named("factory:reviewer".to_owned()),
    }
}

#[test]
fn a_project_file_runs_in_the_project_root() {
    let source = file_source(
        HookSourceKind::ProjectCursorFile,
        "/work/repo/.cursor/hooks.json",
    );
    assert_eq!(
        working_directory_for(&source, Some(Path::new("/work/repo"))),
        Some(PathBuf::from("/work/repo"))
    );
}

#[test]
fn a_project_file_without_a_known_root_runs_above_its_cursor_directory() {
    let source = file_source(
        HookSourceKind::ProjectCursorFile,
        "/work/repo/.cursor/hooks.json",
    );
    assert_eq!(
        working_directory_for(&source, None),
        Some(PathBuf::from("/work/repo"))
    );
}

#[test]
fn explicit_sources_run_in_the_directory_that_holds_them_even_inside_a_project() {
    for kind in [
        HookSourceKind::RunDelivered,
        HookSourceKind::FactoryAgent,
        HookSourceKind::FactoryWide,
    ] {
        let source = file_source(kind, "/work/repo/agents/reviewer/hooks/hooks.json");
        assert_eq!(
            working_directory_for(&source, Some(Path::new("/work/repo"))),
            Some(PathBuf::from("/work/repo/agents/reviewer/hooks")),
            "{kind:?}"
        );
        assert_eq!(
            working_directory_for(&source, None),
            Some(PathBuf::from("/work/repo/agents/reviewer/hooks")),
            "{kind:?}"
        );
    }
}

#[test]
fn a_named_source_has_no_directory_unless_it_is_a_project_file_with_a_root() {
    for kind in [
        HookSourceKind::RunDelivered,
        HookSourceKind::FactoryAgent,
        HookSourceKind::FactoryWide,
    ] {
        assert_eq!(
            working_directory_for(&named_source(kind), Some(Path::new("/work/repo"))),
            None,
            "{kind:?}"
        );
    }
    assert_eq!(
        working_directory_for(
            &named_source(HookSourceKind::ProjectCursorFile),
            Some(Path::new("/work/repo"))
        ),
        Some(PathBuf::from("/work/repo"))
    );
    assert_eq!(
        working_directory_for(&named_source(HookSourceKind::ProjectCursorFile), None),
        None
    );
}
