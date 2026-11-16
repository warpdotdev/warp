use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::Path;

use build_cache::metadata::write_cache_metadata;
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
fn substituted_remote_uses_target_cache_identity_and_source_checkout() {
    let remote = SourceRepo::new(
        CodeForge::GitHub,
        "warpdotdev".to_owned(),
        "warp-for-benchmarks".to_owned(),
    );
    let mapped = repository_cache_source(&remote, "warp", Path::new("/work"));
    assert_eq!(mapped.name, "warpdotdev/warp-for-benchmarks");
    assert_eq!(mapped.identity.repo, "warp-for-benchmarks");
    assert_eq!(mapped.cwd, Path::new("/work/warp"));
}

#[test]
fn source_repo_maps_to_canonical_identity_and_checkout() {
    let repo = SourceRepo::new(
        CodeForge::GitLab,
        "Platform/Backend".to_owned(),
        "API".to_owned(),
    );
    let mapped = repository_cache_source(&repo, "API", Path::new("/work"));
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

#[test]
fn final_snapshot_includes_caller_usage_with_or_without_build_mounts() {
    let root = tempfile::tempdir().unwrap();
    let usage = || {
        vec![build_cache::metadata::CacheUsage {
            path: "git-mirrors".into(),
            cache_framework: Some("git".to_owned()),
            mount_target: Vec::new(),
        }]
    };
    write_cache_metadata(root.path(), usage()).unwrap();
    let path = root.path().join(".ns/cache-metadata.json");
    let document: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(document["version"], 1);
    assert!(document["updated_at"].as_str().unwrap().ends_with('Z'));
    assert_eq!(
        document["user_request"],
        serde_json::json!({
            "git-mirrors": {"source": "warp", "cache_framework": "git", "mount_target": []}
        })
    );
    write_cache_metadata(
        root.path(),
        [
            vec![build_cache::metadata::CacheUsage {
                path: "repos/key/target".into(),
                cache_framework: Some("rust".to_owned()),
                mount_target: vec!["/work/target".to_owned()],
            }],
            usage(),
        ]
        .concat(),
    )
    .unwrap();
    let document: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(document["user_request"].as_object().unwrap().len(), 2);
    assert_eq!(
        document["user_request"]["repos/key/target"]["mount_target"],
        serde_json::json!(["/work/target"])
    );
    write_cache_metadata(root.path(), Vec::new()).unwrap();
    let document: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(document["user_request"], serde_json::json!({}));
}
