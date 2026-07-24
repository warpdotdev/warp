use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::Path;

use cloud_object_models::{CodeForge, SourceRepo};
use warp_isolation_platform::IsolationPlatformType;

use super::{build_export_command, repository_cache_source, should_setup_cache};
use crate::terminal::shell::ShellType;

#[test]
fn gate_matrix_requires_namespace_and_nonempty_root() {
    let root = OsStr::new("/cache/build");
    assert!(should_setup_cache(
        Some(IsolationPlatformType::Namespace),
        Some(root)
    ));
    assert!(!should_setup_cache(None, Some(root)));
    assert!(!should_setup_cache(
        Some(IsolationPlatformType::Docker),
        Some(root)
    ));
    assert!(!should_setup_cache(
        Some(IsolationPlatformType::Namespace),
        None
    ));
    assert!(!should_setup_cache(
        Some(IsolationPlatformType::Namespace),
        Some(OsStr::new(""))
    ));
}

#[test]
fn source_repo_maps_to_canonical_identity_and_checkout() {
    let repo = SourceRepo::new(
        CodeForge::GitLab,
        "Platform/Backend".to_owned(),
        "API".to_owned(),
    );
    let mapped = repository_cache_source(&repo, Path::new("/work"));
    assert_eq!(mapped.name, "Platform/Backend/API");
    assert_eq!(mapped.identity.forge_host, "gitlab.com");
    assert_eq!(mapped.identity.owner, "platform/backend");
    assert_eq!(mapped.identity.repo, "api");
    assert_eq!(mapped.cwd, Path::new("/work/API"));
}

#[test]
fn export_commands_use_active_shell_syntax_and_escaping() {
    let environment = BTreeMap::from([
        ("A_VAR".to_owned(), "a value".to_owned()),
        ("QUOTE".to_owned(), "it's quoted".to_owned()),
    ]);
    assert_eq!(
        build_export_command(&environment, ShellType::Bash),
        "export A_VAR='a value'; export QUOTE='it'\"'\"'s quoted'"
    );
    assert_eq!(
        build_export_command(&environment, ShellType::Zsh),
        "export A_VAR='a value'; export QUOTE='it'\"'\"'s quoted'"
    );
    assert_eq!(
        build_export_command(&environment, ShellType::Fish),
        "set -gx A_VAR 'a value'; set -gx QUOTE 'it\\'s quoted'"
    );
    assert_eq!(
        build_export_command(&environment, ShellType::PowerShell),
        "$env:A_VAR = 'a value'; $env:QUOTE = 'it''s quoted'"
    );
}
