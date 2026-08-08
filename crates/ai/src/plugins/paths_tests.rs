use std::fs;

use tempfile::tempdir;

use super::*;

#[test]
fn plugin_relative_form_requires_a_leading_dot_slash() {
    assert!(is_plugin_relative("./bin/server"));
    assert!(is_plugin_relative("./"));
    assert!(!is_plugin_relative("bin/server"));
    assert!(!is_plugin_relative("/bin/server"));
    assert!(!is_plugin_relative("../bin/server"));
    // The Windows spelling is not the portable form the standard defines.
    assert!(!is_plugin_relative(r".\bin\server"));
}

#[test]
fn contained_paths_resolve_inside_the_plugin_root() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("plugin");
    fs::create_dir_all(root.join("bin")).unwrap();
    fs::write(root.join("bin").join("server"), "#!/bin/sh\n").unwrap();

    let resolved = resolve_contained(&root, "./bin/server").unwrap();
    assert!(resolved.ends_with("bin/server"));
    assert!(resolved.starts_with(dunce::canonicalize(&root).unwrap()));
}

/// A path that does not exist yet is still resolvable, because a server may create its working
/// directory on first run.
#[test]
fn a_not_yet_created_path_still_resolves() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("plugin");
    fs::create_dir_all(&root).unwrap();

    let resolved = resolve_contained(&root, "./state/cache").unwrap();
    assert!(resolved.ends_with("state/cache"));
    assert!(!resolved.exists());
}

#[test]
fn parent_traversal_is_rejected_before_touching_the_filesystem() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("plugin");
    fs::create_dir_all(&root).unwrap();

    let error = resolve_contained(&root, "./../outside").unwrap_err();
    assert!(matches!(error, PluginPathError::ParentTraversal(_)));
}

#[test]
fn a_path_that_is_not_plugin_relative_is_rejected() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("plugin");
    fs::create_dir_all(&root).unwrap();

    for candidate in ["bin/server", "/etc/passwd", "../sibling"] {
        let error = resolve_contained(&root, candidate).unwrap_err();
        assert!(
            matches!(error, PluginPathError::NotPluginRelative(_)),
            "'{candidate}' should be rejected as not plugin-relative, got {error:?}"
        );
    }
}

/// §4.1: a symlink may point inside the plugin root, but a package path that resolves outside it
/// must be rejected. This is the case a lexical check alone would miss.
#[cfg(unix)]
#[test]
fn a_symlink_escaping_the_plugin_root_is_rejected() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("plugin");
    let outside = temp.path().join("outside");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("secret"), "secret").unwrap();
    std::os::unix::fs::symlink(&outside, root.join("escape")).unwrap();

    let error = resolve_contained(&root, "./escape/secret").unwrap_err();
    assert!(
        matches!(error, PluginPathError::EscapesRoot { .. }),
        "expected an escape, got {error:?}"
    );
}

#[cfg(unix)]
#[test]
fn a_symlink_that_stays_inside_the_plugin_root_is_allowed() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("plugin");
    fs::create_dir_all(root.join("real")).unwrap();
    fs::write(root.join("real").join("server"), "#!/bin/sh\n").unwrap();
    std::os::unix::fs::symlink(root.join("real"), root.join("link")).unwrap();

    let resolved = resolve_contained(&root, "./link/server").unwrap();
    assert!(resolved.ends_with("real/server"));
}

/// A symlink hidden behind a component that does not exist yet must still be followed, otherwise
/// an escape could be smuggled in under a leaf the package creates later.
#[cfg(unix)]
#[test]
fn an_escape_behind_a_missing_leaf_is_still_detected() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("plugin");
    let outside = temp.path().join("outside");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, root.join("escape")).unwrap();

    let error = resolve_contained(&root, "./escape/not-created-yet").unwrap_err();
    assert!(matches!(error, PluginPathError::EscapesRoot { .. }));
}

#[test]
fn resolve_partial_returns_the_canonical_prefix_plus_the_missing_tail() {
    let temp = tempdir().unwrap();
    let existing = temp.path().join("existing");
    fs::create_dir_all(&existing).unwrap();

    let resolved = resolve_partial(&existing.join("a").join("b")).unwrap();
    assert_eq!(
        resolved,
        dunce::canonicalize(&existing).unwrap().join("a").join("b")
    );
}
