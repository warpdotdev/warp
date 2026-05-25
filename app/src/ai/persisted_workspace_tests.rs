use std::path::Path;

use lsp::supported_servers::LSPServerType;
use lsp::LspManagerModel;
use warpui::{App, ModelHandle, SingletonEntity};

use super::{CustomLspRepoStatus, PersistedWorkspace};
use crate::test_util::settings::initialize_settings_for_tests;

/// Minimum singletons required by `PersistedWorkspace::custom_lsp_repo_status`.
fn init_app(app: &mut App) {
    initialize_settings_for_tests(app);
    app.add_singleton_model(|_| LspManagerModel::new());
    app.add_singleton_model(PersistedWorkspace::new_for_test);
}

fn persisted(app: &App) -> ModelHandle<PersistedWorkspace> {
    PersistedWorkspace::handle(app)
}

#[test]
fn custom_lsp_repo_status_disabled_when_not_enabled() {
    // A descriptor that has never been enabled for this workspace returns
    // `Disabled` regardless of whether any other server is running. This
    // is the state the footer renders the Enable CTA from.
    App::test((), |mut app| async move {
        init_app(&mut app);
        let repo_root = Path::new("/tmp/some-repo");
        let handle = persisted(&app);
        app.read(|ctx| {
            let status = handle
                .as_ref(ctx)
                .custom_lsp_repo_status(repo_root, "ruby-lsp", ctx);
            assert_eq!(status, CustomLspRepoStatus::Disabled);
        });
    });
}

#[test]
fn custom_lsp_repo_status_enabled_when_persisted_but_no_running_server() {
    // After the user enables a custom but before the manager has spawned
    // a server with that key, the status is `Enabled` (queued). This is
    // the transient state between Enable click and `LspState::Available`.
    App::test((), |mut app| async move {
        init_app(&mut app);
        let repo_root = Path::new("/tmp/some-repo");
        let handle = persisted(&app);
        handle.update(&mut app, |workspace, _| {
            workspace.enable_custom_lsp_server_for_path(repo_root, "ruby-lsp");
        });

        app.read(|ctx| {
            let status = handle
                .as_ref(ctx)
                .custom_lsp_repo_status(repo_root, "ruby-lsp", ctx);
            assert_eq!(status, CustomLspRepoStatus::Enabled);
        });
    });
}

#[test]
fn custom_lsp_repo_status_returns_disabled_after_explicit_disable() {
    // Toggling back to disabled (e.g., footer's RemoveServer action) must
    // surface `Disabled` even though the descriptor name has a prior
    // entry in the in-memory map.
    App::test((), |mut app| async move {
        init_app(&mut app);
        let repo_root = Path::new("/tmp/some-repo");
        let handle = persisted(&app);
        handle.update(&mut app, |workspace, _| {
            workspace.enable_custom_lsp_server_for_path(repo_root, "ruby-lsp");
            workspace.disable_custom_lsp_server_for_path(repo_root, "ruby-lsp");
        });

        app.read(|ctx| {
            let status = handle
                .as_ref(ctx)
                .custom_lsp_repo_status(repo_root, "ruby-lsp", ctx);
            assert_eq!(status, CustomLspRepoStatus::Disabled);
        });
    });
}

#[test]
fn has_any_enabled_lsp_server_false_when_nothing_enabled() {
    // Guards the `LspTask::Spawn` early-return: a fresh workspace with no
    // enabled servers must short-circuit before the expensive interactive
    // PATH capture.
    App::test((), |mut app| async move {
        init_app(&mut app);
        let path = Path::new("/tmp/some-repo/file.rs");
        let handle = persisted(&app);
        app.read(|ctx| {
            assert!(!handle.as_ref(ctx).has_any_enabled_lsp_server(path));
        });
    });
}

#[test]
fn has_any_enabled_lsp_server_true_for_builtin_only() {
    // Built-in enablement alone is sufficient; matches pre-custom behavior.
    App::test((), |mut app| async move {
        init_app(&mut app);
        let repo_root = Path::new("/tmp/some-repo");
        let handle = persisted(&app);
        handle.update(&mut app, |workspace, _| {
            workspace.enable_lsp_server_for_path(repo_root, LSPServerType::RustAnalyzer);
        });

        app.read(|ctx| {
            assert!(handle.as_ref(ctx).has_any_enabled_lsp_server(repo_root));
        });
    });
}

#[test]
fn has_any_enabled_lsp_server_true_for_custom_only() {
    // A workspace with only a custom server enabled (no built-ins) must
    // still report as having an enabled server so the spawn guard does
    // not short-circuit.
    App::test((), |mut app| async move {
        init_app(&mut app);
        let repo_root = Path::new("/tmp/some-repo");
        let handle = persisted(&app);
        handle.update(&mut app, |workspace, _| {
            workspace.enable_custom_lsp_server_for_path(repo_root, "jdtls");
        });

        app.read(|ctx| {
            assert!(handle.as_ref(ctx).has_any_enabled_lsp_server(repo_root));
        });
    });
}

#[test]
fn has_any_enabled_lsp_server_false_after_disabling_only_custom() {
    // Toggling the sole custom back to disabled returns the workspace to
    // the no-spawn state.
    App::test((), |mut app| async move {
        init_app(&mut app);
        let repo_root = Path::new("/tmp/some-repo");
        let handle = persisted(&app);
        handle.update(&mut app, |workspace, _| {
            workspace.enable_custom_lsp_server_for_path(repo_root, "jdtls");
            workspace.disable_custom_lsp_server_for_path(repo_root, "jdtls");
        });

        app.read(|ctx| {
            assert!(!handle.as_ref(ctx).has_any_enabled_lsp_server(repo_root));
        });
    });
}

#[test]
fn custom_lsp_repo_status_different_descriptors_tracked_separately() {
    // Enabling one descriptor must not affect a sibling descriptor's
    // status for the same workspace.
    App::test((), |mut app| async move {
        init_app(&mut app);
        let repo_root = Path::new("/tmp/some-repo");
        let handle = persisted(&app);
        handle.update(&mut app, |workspace, _| {
            workspace.enable_custom_lsp_server_for_path(repo_root, "ruby-lsp");
        });

        app.read(|ctx| {
            let ruby = handle
                .as_ref(ctx)
                .custom_lsp_repo_status(repo_root, "ruby-lsp", ctx);
            let solargraph =
                handle
                    .as_ref(ctx)
                    .custom_lsp_repo_status(repo_root, "solargraph", ctx);
            assert_eq!(ruby, CustomLspRepoStatus::Enabled);
            assert_eq!(solargraph, CustomLspRepoStatus::Disabled);
        });
    });
}
