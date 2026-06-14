//! Integration test for the custom LSP enable→spawn plumbing.
//!
//! Verifies the end-to-end pipeline: a `[[editor.language_servers]]`
//! entry in `settings.toml` is parsed into `LanguageServersSettings`,
//! `resolve_server_for_path` matches a file by descriptor `filetypes`,
//! and `PersistedWorkspace::enable_and_spawn_lsp_server` registers the
//! server in `LspManagerModel`.
//!
//! The descriptor uses `sleep` as its `command` so the spawned process
//! never speaks LSP — we are testing plumbing, not protocol. The
//! assertion stops at "server registered with `ServerKey::Custom`"
//! rather than waiting for `LspState::Available`, which would require
//! a mock that responds to the `initialize` handshake.

use std::path::PathBuf;
use std::time::Duration;

use lsp::{LspManagerModel, ServerKey};
use settings::Setting as _;
use warp::features::FeatureFlag;
use warp::integration_testing::lsp::{
    resolve_server_for_path, CodeFooterView, EnablementState, PersistedWorkspace, ResolvedLspServer,
};
use warp::integration_testing::step::new_step_with_default_assertions;
use warp::integration_testing::terminal::wait_until_bootstrapped_single_pane_for_tab;
use warp::settings::LanguageServersSettings;
use warpui_core::integration::TestStep;
use warpui_core::{async_assert, SingletonEntity};

use super::{new_builder, Builder};

const DESCRIPTOR_NAME: &str = "test_lsp";
const FILE_EXT: &str = "test_lsp_marker";

fn settings_toml() -> String {
    format!(
        "[[editor.language_servers]]\n\
         name = \"{DESCRIPTOR_NAME}\"\n\
         command = \"sleep\"\n\
         args = [\"3600\"]\n\
         filetypes = [{{ pattern = \"*.{FILE_EXT}\" }}]\n"
    )
}

fn workspace_root() -> PathBuf {
    std::env::temp_dir().join("warp-test-custom-lsp-workspace")
}

fn file_path() -> PathBuf {
    workspace_root().join(format!("file.{FILE_EXT}"))
}

pub fn test_custom_lsp_enable_registers_server_in_manager() -> Builder {
    FeatureFlag::SettingsFile.set_enabled(true);

    new_builder()
        .with_setup(move |_utils| {
            // Ensure the fake workspace dir exists so `command.current_dir`
            // in the spawn path doesn't fail before the server is even
            // registered.
            std::fs::create_dir_all(workspace_root()).expect("should create fake workspace dir");

            let path = warp::settings::user_preferences_toml_file_path();
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("should create config dir");
            }
            std::fs::write(&path, settings_toml()).expect("should write settings.toml");
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        // Step 1: confirm settings parsed the descriptor on startup so we
        // know the settings→model path works before exercising spawn.
        .with_step(
            new_step_with_default_assertions("Custom descriptor parsed from settings")
                .add_named_assertion("LanguageServersSettings contains test_lsp", |app, _| {
                    app.read(|ctx| {
                        let settings = LanguageServersSettings::as_ref(ctx);
                        let descriptors = &settings.language_servers.value().0;
                        async_assert!(
                            descriptors.iter().any(|d| d.name == DESCRIPTOR_NAME),
                            "test_lsp descriptor should be parsed from settings.toml"
                        )
                    })
                }),
        )
        // Step 2: drive the enable+spawn path and assert the manager
        // ends up holding a Custom-keyed server. The action is
        // idempotent — repeated assertion polls just hit the manager's
        // "already registered" early-return.
        .with_step(
            TestStep::new("Enable custom LSP and verify manager registers it")
                .set_timeout(Duration::from_secs(15))
                .add_named_assertion("manager holds Custom(test_lsp) server", |app, _| {
                    app.update(|ctx| {
                        let Some(resolved) = resolve_server_for_path(&file_path(), ctx) else {
                            return;
                        };
                        if !matches!(resolved, ResolvedLspServer::Custom(_)) {
                            return;
                        }
                        PersistedWorkspace::handle(ctx).update(ctx, |ws, ctx| {
                            ws.enable_and_spawn_lsp_server(
                                &workspace_root(),
                                &resolved,
                                file_path(),
                                ctx,
                            );
                        });
                    });

                    app.read(|ctx| {
                        let manager = LspManagerModel::as_ref(ctx);
                        let has_custom = manager
                            .servers_for_workspace(&workspace_root())
                            .map(|servers| {
                                servers.iter().any(|server| {
                                    matches!(
                                        server.as_ref(ctx).key(),
                                        ServerKey::Custom(name) if name == DESCRIPTOR_NAME
                                    )
                                })
                            })
                            .unwrap_or(false);
                        async_assert!(
                            has_custom,
                            "LspManagerModel should contain Custom({DESCRIPTOR_NAME}) after enable"
                        )
                    })
                }),
        )
}

// --- Override-of-built-in regression test ---
//
// Built-in `LSPServerType::GoPls` claims `*.go` via the `LanguageId` map.
// Without the fix in `CodeFooterView::new`, opening a `.go` file with a
// custom descriptor that also matches `*.go` would let built-in
// install-status detection run after the custom-first resolve, stomping
// the button label from "Enable <custom-name>" back to the built-in's
// `binary_name()` ("gopls"). This test locks the fix: after constructing
// the footer for a custom-overriding `.go` path, `lsp_repo_status` stays
// `None` (built-in detection skipped) and the button label still reflects
// the custom descriptor's `name`.

const OVERRIDE_DESCRIPTOR_NAME: &str = "test_go_lsp";

fn override_settings_toml() -> String {
    format!(
        "[[editor.language_servers]]\n\
         name = \"{OVERRIDE_DESCRIPTOR_NAME}\"\n\
         command = \"sleep\"\n\
         args = [\"3600\"]\n\
         filetypes = [{{ pattern = \"*.go\" }}]\n"
    )
}

fn override_workspace_root() -> PathBuf {
    std::env::temp_dir().join("warp-test-custom-lsp-override")
}

fn override_file_path() -> PathBuf {
    override_workspace_root().join("main.go")
}

pub fn test_custom_lsp_override_yields_custom_label_in_footer() -> Builder {
    FeatureFlag::SettingsFile.set_enabled(true);

    new_builder()
        .with_setup(move |_utils| {
            std::fs::create_dir_all(override_workspace_root())
                .expect("should create fake workspace dir");

            let path = warp::settings::user_preferences_toml_file_path();
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("should create config dir");
            }
            std::fs::write(&path, override_settings_toml())
                .expect("should write settings.toml");
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        // Sanity: settings parsed and resolver picks the custom over the
        // built-in `GoPls` mapping for `*.go`.
        .with_step(
            new_step_with_default_assertions("Custom override descriptor wins resolve for *.go")
                .add_named_assertion(
                    "resolver returns Custom(test_go_lsp) for main.go",
                    |app, _| {
                        app.read(|ctx| {
                            let resolved = resolve_server_for_path(&override_file_path(), ctx);
                            let matched = matches!(
                                resolved,
                                Some(ResolvedLspServer::Custom(ref d))
                                    if d.name == OVERRIDE_DESCRIPTOR_NAME
                            );
                            async_assert!(
                                matched,
                                "expected Custom({OVERRIDE_DESCRIPTOR_NAME}), got {:?}",
                                resolved.as_ref().map(|r| match r {
                                    ResolvedLspServer::BuiltIn(t) => format!("BuiltIn({t:?})"),
                                    ResolvedLspServer::Custom(d) => format!("Custom({})", d.name),
                                })
                            )
                        })
                    },
                ),
        )
        // The actual regression check: construct the footer for the
        // custom-overriding path and verify (a) `lsp_repo_status` is
        // `None` — built-in install detection was skipped — and (b) the
        // enable button is labeled with the custom descriptor's name,
        // not the built-in's `binary_name()`.
        .with_step(
            TestStep::new("Footer for custom-overriding *.go path uses custom name")
                .set_timeout(Duration::from_secs(15))
                .add_named_assertion(
                    "lsp_repo_status is None and button labeled with custom name",
                    |app, window_id| {
                        let footer = app.add_typed_action_view(window_id, |ctx| {
                            CodeFooterView::new(override_file_path(), ctx)
                        });

                        let (status_ok, label) = footer.read(app, |footer, ctx| {
                            let status_ok = footer.is_single_file_without_builtin_status();
                            let label = footer
                                .enable_lsp_button()
                                .map(|btn| btn.read(ctx, |b, _| b.label().to_owned()));
                            (status_ok, label)
                        });

                        let expected_label = format!("Enable {OVERRIDE_DESCRIPTOR_NAME}");
                        let label_ok = label.as_deref() == Some(expected_label.as_str());

                        async_assert!(
                            status_ok && label_ok,
                            "expected SingleFile-with-no-builtin-status and label=Some({expected_label:?}); got status_ok={status_ok} label={label:?}",
                        )
                    },
                ),
        )
}

const PERSIST_DESCRIPTOR_NAME: &str = "persist_test_lsp";

fn persist_settings_toml() -> String {
    format!(
        "[[editor.language_servers]]\n\
         name = \"{PERSIST_DESCRIPTOR_NAME}\"\n\
         command = \"sleep\"\n\
         args = [\"3600\"]\n\
         filetypes = [{{ pattern = \"*.{FILE_EXT}\" }}]\n"
    )
}

fn persist_workspace_root() -> PathBuf {
    std::env::temp_dir().join("warp-test-custom-lsp-persist")
}

fn persist_file_path() -> PathBuf {
    persist_workspace_root().join(format!("file.{FILE_EXT}"))
}

/// Regression test for product.md invariant 13: a custom LSP server enabled
/// for a workspace stays enabled across an app restart. Enables a custom server
/// in a fresh workspace, then asserts it is still enabled after the persisted
/// state is reloaded.
pub fn test_custom_lsp_enablement_survives_reload() -> Builder {
    FeatureFlag::SettingsFile.set_enabled(true);

    new_builder()
        .with_setup(move |_utils| {
            std::fs::create_dir_all(persist_workspace_root())
                .expect("should create fake workspace dir");

            let path = warp::settings::user_preferences_toml_file_path();
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("should create config dir");
            }
            std::fs::write(&path, persist_settings_toml()).expect("should write settings.toml");
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            TestStep::new("Enable custom LSP in a fresh workspace, then assert it persists")
                .set_timeout(Duration::from_secs(20))
                .add_named_assertion(
                    "custom enablement is durably persisted as Yes across reload",
                    |app, _| {
                        // Enable the custom server for this workspace.
                        app.update(|ctx| {
                            let Some(resolved) = resolve_server_for_path(&persist_file_path(), ctx)
                            else {
                                return;
                            };
                            if !matches!(resolved, ResolvedLspServer::Custom(_)) {
                                return;
                            }
                            PersistedWorkspace::handle(ctx).update(ctx, |ws, ctx| {
                                ws.enable_and_spawn_lsp_server(
                                    &persist_workspace_root(),
                                    &resolved,
                                    persist_file_path(),
                                    ctx,
                                );
                            });
                        });

                        // Read the persisted state the way a fresh launch would.
                        let persisted = warp::sqlite_testing::persisted_custom_lsp_enablement(
                            &persist_workspace_root(),
                            PERSIST_DESCRIPTOR_NAME,
                        );
                        async_assert!(
                            persisted == Some(EnablementState::Yes),
                            "custom enablement should survive reload; read back {persisted:?}"
                        )
                    },
                ),
        )
}
