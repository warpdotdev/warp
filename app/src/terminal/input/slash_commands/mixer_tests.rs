//! Regression tests for [`build_slash_command_mixer`].
//!
//! These verify that the saved-prompt (Agent Mode workflow) async source is opt-in:
//! the TUI slash menu (`include_saved_prompts = false`) never surfaces
//! `SavedPrompt` rows, while the GUI slash menu (`include_saved_prompts = true`)
//! does when matching workflows exist.

use std::time::Duration;

use warp_core::execution_mode::{AppExecutionMode, ExecutionMode};
use warpui::r#async::Timer;
use warpui::{App, AppContext, Entity, ModelHandle, SingletonEntity as _};

use super::{SlashCommandMixer, build_slash_command_mixer, slash_command_query};
use crate::appearance::Appearance;
use crate::auth::AuthStateProvider;
use crate::auth::auth_manager::AuthManager;
use crate::cloud_object::model::persistence::CloudModel;
use crate::cloud_object::{CloudObject, CloudObjectMetadata, CloudObjectPermissions, Owner};
use crate::network::NetworkStatus;
use crate::search::SyncDataSource;
use crate::search::data_source::{Query, QueryResult};
use crate::search::mixer::DataSourceRunErrorWrapper;
use crate::server::ids::SyncId;
use crate::server::server_api::ServerApiProvider;
use crate::settings::manager::SettingsManager;
use crate::settings::{AISettings, init_and_register_user_preferences};
use crate::terminal::input::slash_commands::AcceptSlashCommandOrSavedPrompt;
use crate::user_config::WarpConfig;
use crate::workflows::workflow::Workflow;
use crate::workflows::{CloudWorkflow, CloudWorkflowModel};
use crate::workspaces::user_workspaces::UserWorkspaces;

/// A minimal sync data source that produces no results, used as both the
/// "primary" (static-commands) source and the zero-state source in the mixer
/// tests. The tests are concerned only with whether the saved-prompt async
/// source is registered, so the sync sources are intentionally empty stubs.
struct EmptySyncSource;

impl Entity for EmptySyncSource {
    type Event = ();
}

impl SyncDataSource for EmptySyncSource {
    type Action = AcceptSlashCommandOrSavedPrompt;

    fn run_query(
        &self,
        _query: &Query,
        _app: &AppContext,
    ) -> Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper> {
        Ok(Vec::new())
    }
}

fn mock_permissions() -> CloudObjectPermissions {
    CloudObjectPermissions {
        owner: Owner::mock_current_user(),
        guests: Vec::new(),
        permissions_last_updated_ts: None,
        anyone_with_link: None,
    }
}

/// Constructs an active (non-trashed) Agent Mode workflow with the given name
/// and query, owned by the mock current user and placed at the top level
/// (no parent folder) so `breadcrumbs` resolves to the personal space.
fn agent_mode_workflow(name: &str, query: &str) -> CloudWorkflow {
    CloudWorkflow::new(
        SyncId::ServerId(1.into()),
        CloudWorkflowModel::new(Workflow::AgentMode {
            name: name.to_owned(),
            query: query.to_owned(),
            description: None,
            arguments: Vec::new(),
        }),
        CloudObjectMetadata::mock(),
        mock_permissions(),
    )
}

/// Registers the minimal singleton set required by [`super::saved_prompts_data_source`]
/// and [`AISettings::is_any_ai_enabled`], with a [`CloudModel`] pre-populated with an
/// Agent Mode workflow so the saved-prompt async source has a matching candidate.
///
/// This mirrors the relevant subset of `register_tui_session_view_test_singletons`
/// but seeds `CloudModel` with a workflow instead of an empty mock.
fn register_singletons_with_workflow(app: &mut App, workflow: CloudWorkflow) {
    // Appearance is read by the saved-prompt snapshot for the UI font family.
    app.add_singleton_model(|_| Appearance::mock());
    // Settings infrastructure required by AISettings::register.
    app.add_singleton_model(|ctx| AppExecutionMode::new(ExecutionMode::App, false, ctx));
    app.update(init_and_register_user_preferences);
    app.add_singleton_model(|_| SettingsManager::default());
    app.add_singleton_model(WarpConfig::mock);
    app.update(|ctx| {
        warpui_extras::secure_storage::register_noop("test", ctx);
    });
    app.update(AISettings::register_and_subscribe_to_events);
    // Auth + server provider: is_any_ai_enabled checks the logged-out state.
    app.add_singleton_model(|_| NetworkStatus::new());
    app.add_singleton_model(|_| ServerApiProvider::new_for_test());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(AuthManager::new_for_test);
    // UserWorkspaces: is_ai_allowed_in_remote_sessions and breadcrumbs' owner_to_space.
    app.add_singleton_model(|ctx| {
        let (team_client, workspace_client) = {
            let provider = ServerApiProvider::as_ref(ctx);
            (provider.get_team_client(), provider.get_workspace_client())
        };
        UserWorkspaces::mock(team_client, workspace_client, Vec::new(), ctx)
    });
    // CloudModel seeded with the Agent Mode workflow so the saved-prompt source
    // has a candidate to match when the async source is registered.
    app.add_singleton_model(move |_| {
        CloudModel::new(None, vec![Box::new(workflow) as Box<dyn CloudObject>], None)
    });
}

/// Builds a slash-command mixer with the given `include_saved_prompts` flag,
/// using empty stub sync sources, and returns the model handle.
fn build_test_mixer(app: &mut App, include_saved_prompts: bool) -> ModelHandle<SlashCommandMixer> {
    let primary = app.add_model(|_| EmptySyncSource);
    app.add_model(move |ctx| {
        build_slash_command_mixer(primary, EmptySyncSource, include_saved_prompts, ctx)
    })
}

fn mixer_has_saved_prompt_row(app: &App, mixer: &ModelHandle<SlashCommandMixer>) -> bool {
    app.read(|ctx| {
        mixer.as_ref(ctx).results().iter().any(|result| {
            matches!(
                result.accept_result(),
                AcceptSlashCommandOrSavedPrompt::SavedPrompt { .. }
            )
        })
    })
}

/// The TUI slash-command mixer (`include_saved_prompts = false`) must never
/// surface `SavedPrompt` rows for a typed/fuzzy query, even when an Agent Mode
/// workflow exists that the saved-prompt source would otherwise match. The GUI
/// mixer (`include_saved_prompts = true`) must continue to surface it.
#[test]
fn tui_mixer_omits_saved_prompts_while_gui_mixer_surfaces_them() {
    App::test((), |mut app| async move {
        register_singletons_with_workflow(
            &mut app,
            agent_mode_workflow("Refactor Code", "refactor the selected code"),
        );

        let tui_mixer = build_test_mixer(&mut app, false);
        let gui_mixer = build_test_mixer(&mut app, true);

        // Run a non-empty query that fuzzy-matches the workflow so the
        // saved-prompt async source (when registered) produces a SavedPrompt row.
        tui_mixer.update(&mut app, |mixer, ctx| {
            mixer.run_query(slash_command_query("refactor"), ctx);
        });
        gui_mixer.update(&mut app, |mixer, ctx| {
            mixer.run_query(slash_command_query("refactor"), ctx);
        });

        // Wait for the async saved-prompt source to complete (well past the
        // mixer's INITIAL_RESULTS_TIMEOUT of 500ms and the per-chunk yield).
        Timer::after(Duration::from_millis(700)).await;

        assert!(
            !mixer_has_saved_prompt_row(&app, &tui_mixer),
            "TUI slash-command mixer must not surface saved-prompt rows"
        );
        assert!(
            mixer_has_saved_prompt_row(&app, &gui_mixer),
            "GUI slash-command mixer should surface the saved-prompt row when a workflow matches"
        );
    });
}
