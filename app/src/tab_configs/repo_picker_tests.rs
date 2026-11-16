use std::collections::HashMap;

use ai::workspace::WorkspaceMetadata;
use chrono::Utc;
use tempfile::TempDir;
use warp_core::ui::appearance::Appearance;
use warpui::platform::WindowStyle;
use warpui::{App, SingletonEntity};

use super::RepoPicker;
use crate::ai::persisted_workspace::PersistedWorkspace;
use crate::server::telemetry::context_provider::AppTelemetryContextProvider;
use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::vim_registers::VimRegisters;
use crate::workspace::sync_inputs::SyncedInputState;

/// Regression test for GH-16566: without a default, the picker displays the most recent repo,
/// so `selected_value()` must report it too, or required repo params block submission.
#[test]
fn test_selected_value_defaults_to_displayed_repo() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(AppTelemetryContextProvider::new_context_provider);
        app.add_singleton_model(|_| Appearance::mock());
        app.add_singleton_model(|_| SyncedInputState::mock());
        app.add_singleton_model(|_| VimRegisters::new());
        app.add_singleton_model(|_| KeybindingChangedNotifier::mock());

        let repo_dir = TempDir::new().expect("failed to create temp dir");
        let repo_path = repo_dir.path().to_path_buf();
        let metadata = WorkspaceMetadata {
            path: repo_path.clone(),
            navigated_ts: Some(Utc::now()),
            modified_ts: None,
            queried_ts: None,
        };
        app.add_singleton_model(|ctx| {
            PersistedWorkspace::new(vec![metadata], HashMap::new(), None, ctx)
        });
        assert!(PersistedWorkspace::handle(&app).read(&app, |workspace, _| {
            workspace.workspaces().any(|ws| ws.path == repo_path)
        }));

        let (_, picker) =
            app.add_window(WindowStyle::NotStealFocus, |ctx| RepoPicker::new(None, ctx));

        let selected = picker.read(&app, |picker, ctx| picker.selected_value(ctx));
        assert_eq!(selected, Some(repo_path.to_string_lossy().into_owned()));
    })
}
