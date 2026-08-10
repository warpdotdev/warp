use std::fs;
use std::path::Path;

use serde_json::json;
use tempfile::tempdir;

use super::*;
use crate::plugins::manifest::MANIFEST_SCHEMA_1_0_0;

/// Writes a plugin package with `manifest_name` under `search_root`, in a directory named
/// `directory_name` so tests can prove the directory name never influences precedence.
fn write_package(search_root: &Path, directory_name: &str, manifest_name: &str) {
    let root = search_root.join(directory_name);
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("plugin.json"),
        json!({ "$schema": MANIFEST_SCHEMA_1_0_0, "name": manifest_name }).to_string(),
    )
    .unwrap();
}

fn candidate(
    root: &Path,
    scope: PluginScopeId,
    kind: PluginSourceKind,
    identity: &str,
) -> PluginCandidate {
    PluginCandidate {
        root: root.to_path_buf(),
        scope,
        source: PluginSourceId::new(kind, identity),
    }
}

#[test]
fn user_search_roots_cover_agents_and_the_channel_aware_warp_home() {
    let roots = user_search_roots();
    assert!(
        roots.iter().any(
            |root| root.path.ends_with(Path::new(".agents").join("plugins"))
                && root.source.kind == PluginSourceKind::AgentsDirectory
        ),
        "expected a ~/.agents/plugins root, got {roots:?}"
    );
    // The Warp root follows the channel-aware home config directory rather than a hard-coded
    // `.warp`, so a dev or profile build does not read the stable channel's plugins.
    if let Some(warp_config_dir) = warp_core::paths::warp_home_config_dir() {
        assert!(
            roots
                .iter()
                .any(|root| root.path == warp_config_dir.join("plugins")),
            "expected {}/plugins, got {roots:?}",
            warp_config_dir.display()
        );
    }
    assert!(roots.iter().all(|root| root.scope == PluginScopeId::User));
}

/// A bare `<repo-root>/plugins` is deliberately not a repository search root.
#[test]
fn repository_search_roots_are_only_the_two_provider_directories() {
    let roots = repository_search_roots(Path::new("/repo"));
    let paths: Vec<String> = roots
        .iter()
        .map(|root| root.path.to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        paths,
        vec![
            Path::new("/repo/.agents/plugins").to_string_lossy(),
            Path::new("/repo/.warp/plugins").to_string_lossy(),
        ]
    );
    assert!(
        roots
            .iter()
            .all(|root| root.scope == PluginScopeId::Repository)
    );
}

/// §25: only immediate children of a search root are candidates.
#[test]
fn only_immediate_children_are_candidates() {
    let temp = tempdir().unwrap();
    let search_root = temp.path().join(".agents").join("plugins");
    write_package(&search_root, "one", "one");
    write_package(&search_root.join("nested"), "two", "two");
    fs::write(search_root.join("README.md"), "not a plugin").unwrap();

    let root = PluginSearchRoot {
        path: search_root.clone(),
        scope: PluginScopeId::Repository,
        source: PluginSourceId::new(PluginSourceKind::AgentsDirectory, "/repo"),
    };
    let candidates = scan_search_root(&root);
    let names: Vec<String> = candidates
        .iter()
        .map(|candidate| {
            candidate
                .root
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(names, vec!["nested".to_owned(), "one".to_owned()]);

    // `nested` is scanned as a candidate but has no manifest of its own, so it is not a plugin
    // and the package one level deeper is never reached.
    let resolved = resolve_active_packages(candidates);
    assert_eq!(resolved.active.keys().collect::<Vec<_>>(), vec!["one"]);
}

#[test]
fn a_missing_search_root_yields_no_candidates() {
    let temp = tempdir().unwrap();
    let root = PluginSearchRoot {
        path: temp.path().join("does-not-exist"),
        scope: PluginScopeId::User,
        source: PluginSourceId::new(PluginSourceKind::AgentsDirectory, "/home/.agents"),
    };
    assert!(scan_search_root(&root).is_empty());
}

/// §7: same-name packages shadow as complete packages, in the documented order.
#[test]
fn same_name_packages_shadow_in_precedence_order() {
    let temp = tempdir().unwrap();
    let repo_agents = temp.path().join("repo/.agents/plugins");
    let repo_warp = temp.path().join("repo/.warp/plugins");
    let user_agents = temp.path().join("home/.agents/plugins");
    let user_warp = temp.path().join("home/.warp/plugins");
    for (search_root, directory) in [
        (&repo_agents, "a"),
        (&repo_warp, "b"),
        (&user_agents, "c"),
        (&user_warp, "d"),
    ] {
        write_package(search_root, directory, "acme-tools");
    }

    let candidates = vec![
        candidate(
            &user_warp.join("d"),
            PluginScopeId::User,
            PluginSourceKind::WarpDirectory,
            "/home/.warp",
        ),
        candidate(
            &user_agents.join("c"),
            PluginScopeId::User,
            PluginSourceKind::AgentsDirectory,
            "/home/.agents",
        ),
        candidate(
            &repo_warp.join("b"),
            PluginScopeId::Repository,
            PluginSourceKind::WarpDirectory,
            "/repo",
        ),
        candidate(
            &repo_agents.join("a"),
            PluginScopeId::Repository,
            PluginSourceKind::AgentsDirectory,
            "/repo",
        ),
    ];

    let resolved = resolve_active_packages(candidates);
    let winner = resolved.get("acme-tools").unwrap();
    assert_eq!(winner.instance.scope, PluginScopeId::Repository);
    assert_eq!(
        winner.instance.source.kind,
        PluginSourceKind::AgentsDirectory
    );
    assert_eq!(resolved.shadowed.len(), 3);
    // A shadowed package stays visible in diagnostics, naming the source that won.
    assert!(
        resolved
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == PluginDiagnosticCode::PluginShadowed)
            .count()
            == 3
    );
}

/// Shadowing replaces a package outright: components never merge across packages.
#[test]
fn shadowing_replaces_the_whole_package_rather_than_merging_components() {
    let temp = tempdir().unwrap();
    let repo = temp.path().join("repo/.agents/plugins");
    let user = temp.path().join("home/.agents/plugins");

    // The repository package has only a skill; the user package has only an MCP server. If the
    // two merged, the winner would expose the user package's server.
    write_package(&repo, "acme", "acme-tools");
    let skill_dir = repo.join("acme").join("skills").join("deploy");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: deploy\ndescription: Deploy\n---\n\n# Deploy\n",
    )
    .unwrap();

    write_package(&user, "acme", "acme-tools");
    fs::write(
        user.join("acme").join("mcp.json"),
        json!({
            "$schema": crate::plugins::mcp::MCP_SCHEMA_1_0_0,
            "mcpServers": { "registry": { "type": "stdio", "command": "server" } },
        })
        .to_string(),
    )
    .unwrap();

    let resolved = resolve_active_packages(vec![
        candidate(
            &repo.join("acme"),
            PluginScopeId::Repository,
            PluginSourceKind::AgentsDirectory,
            "/repo",
        ),
        candidate(
            &user.join("acme"),
            PluginScopeId::User,
            PluginSourceKind::AgentsDirectory,
            "/home/.agents",
        ),
    ]);

    let winner = resolved.get("acme-tools").unwrap();
    assert_eq!(winner.skills.len(), 1);
    assert!(
        winner.mcp_servers.is_empty(),
        "the shadowed package must not contribute components"
    );
}

/// §42: two equally ranked sources are an ambiguity, not a race decided by directory order.
#[test]
fn equally_ranked_sources_are_ambiguous_and_neither_loads() {
    let temp = tempdir().unwrap();
    let first = temp.path().join("repo-one/.agents/plugins");
    let second = temp.path().join("repo-two/.agents/plugins");
    write_package(&first, "acme", "acme-tools");
    write_package(&second, "acme", "acme-tools");

    let resolved = resolve_active_packages(vec![
        candidate(
            &first.join("acme"),
            PluginScopeId::Repository,
            PluginSourceKind::AgentsDirectory,
            "/repo-one",
        ),
        candidate(
            &second.join("acme"),
            PluginScopeId::Repository,
            PluginSourceKind::AgentsDirectory,
            "/repo-two",
        ),
    ]);

    assert!(resolved.get("acme-tools").is_none());
    assert_eq!(resolved.ambiguous["acme-tools"].len(), 2);
    let diagnostic = resolved
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == PluginDiagnosticCode::PluginAmbiguous)
        .unwrap();
    // The message has to name both candidates for the user to be able to act on it.
    assert!(diagnostic.reason.contains("repo-one"));
    assert!(diagnostic.reason.contains("repo-two"));
}

/// Precedence keys on the manifest name, so two differently named directories that declare the
/// same plugin still collide, and one directory name never wins by being alphabetically first.
#[test]
fn precedence_keys_on_the_manifest_name_not_the_directory_name() {
    let temp = tempdir().unwrap();
    let repo = temp.path().join("repo/.agents/plugins");
    let user = temp.path().join("home/.agents/plugins");
    write_package(&repo, "zzz-directory", "acme-tools");
    write_package(&user, "aaa-directory", "acme-tools");

    let resolved = resolve_active_packages(vec![
        candidate(
            &user.join("aaa-directory"),
            PluginScopeId::User,
            PluginSourceKind::AgentsDirectory,
            "/home/.agents",
        ),
        candidate(
            &repo.join("zzz-directory"),
            PluginScopeId::Repository,
            PluginSourceKind::AgentsDirectory,
            "/repo",
        ),
    ]);

    let winner = resolved.get("acme-tools").unwrap();
    assert!(winner.root.ends_with("zzz-directory"));
}

/// An invalid package is reported and skipped without affecting the valid ones around it.
#[test]
fn an_invalid_candidate_does_not_block_the_others() {
    let temp = tempdir().unwrap();
    let search_root = temp.path().join(".agents").join("plugins");
    write_package(&search_root, "good", "good-plugin");
    let broken = search_root.join("broken");
    fs::create_dir_all(&broken).unwrap();
    fs::write(broken.join("plugin.json"), "{ not json").unwrap();

    let root = PluginSearchRoot {
        path: search_root,
        scope: PluginScopeId::Repository,
        source: PluginSourceId::new(PluginSourceKind::AgentsDirectory, "/repo"),
    };
    let resolved = resolve_active_packages(scan_search_root(&root));

    assert_eq!(
        resolved.active.keys().collect::<Vec<_>>(),
        vec!["good-plugin"]
    );
    assert_eq!(
        resolved.diagnostics[0].code,
        PluginDiagnosticCode::ManifestInvalidJson
    );
}

#[test]
fn precedence_rank_is_lowest_for_the_repository_agents_directory() {
    assert_eq!(
        precedence_rank(
            &PluginScopeId::Repository,
            PluginSourceKind::AgentsDirectory
        ),
        (
            PluginScopeId::Repository.scope_rank(),
            PluginSourceKind::AgentsDirectory.provider_rank()
        )
    );
    assert!(
        precedence_rank(
            &PluginScopeId::Repository,
            PluginSourceKind::AgentsDirectory
        ) < precedence_rank(&PluginScopeId::User, PluginSourceKind::WarpDirectory)
    );
}
