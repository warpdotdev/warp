use std::path::PathBuf;

use crate::code::lsp_logs::custom_relative_log_path;

#[test]
fn custom_relative_log_path_keys_by_name_and_workspace_hash() {
    let path = custom_relative_log_path("ruby-lsp", &PathBuf::from("/tmp/repo"));
    let as_str = path.to_string_lossy();
    // First component is the descriptor name (the per-server bucket).
    assert!(
        as_str.starts_with("ruby-lsp/"),
        "expected `ruby-lsp/` prefix, got {as_str}"
    );
    // Trailing component is `<hash>.log`.
    assert!(
        as_str.ends_with(".log"),
        "expected `.log` suffix, got {as_str}"
    );
}

#[test]
fn custom_relative_log_path_differs_per_workspace() {
    // Same descriptor in two workspaces should produce different log
    // paths so the per-server bucket doesn't get collisions.
    let a = custom_relative_log_path("ruby-lsp", &PathBuf::from("/tmp/repo-a"));
    let b = custom_relative_log_path("ruby-lsp", &PathBuf::from("/tmp/repo-b"));
    assert_ne!(
        a, b,
        "different workspaces must produce different log paths"
    );
}

#[test]
fn custom_relative_log_path_differs_per_descriptor() {
    // Same workspace, two different descriptor names → different bucket.
    let a = custom_relative_log_path("ruby-lsp", &PathBuf::from("/tmp/repo"));
    let b = custom_relative_log_path("solargraph", &PathBuf::from("/tmp/repo"));
    assert_ne!(a, b);
}
