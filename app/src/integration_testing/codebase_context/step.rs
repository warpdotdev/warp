use std::path::PathBuf;
use std::time::Duration;

use ai::index::full_source_code_embedding::manager::CodebaseIndexManager;
use settings::Setting;
use warpui::integration::{AssertionOutcome, StepData, TestStep};
use warpui::{async_assert, App, ReadModel, SingletonEntity, UpdateModel, WindowId};

use crate::integration_testing::step::new_step_with_default_assertions;
use crate::settings::CodeSettings;
use crate::workspace::ActiveSession;

const SYNC_DEFAULT_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const CWD_DATA_KEY: &str = "cwd";

fn active_session_cwd(app: &App, window_id: WindowId) -> Option<PathBuf> {
    app.read_model(&ActiveSession::handle(app), |active_session, _ctx| {
        active_session
            .path_if_local(window_id)
            .map(|path| path.to_path_buf())
    })
}

/// Attempts to sync the git repo at the current working directory of the active
/// session.
///
/// Assumes that the active session is in a git repo. If not, the step will
/// pass.
pub fn sync_current_codebase_index() -> TestStep {
    new_step_with_default_assertions("Sync current codebase index")
        .set_timeout(SYNC_DEFAULT_TIMEOUT)
        .add_assertion(move |app, _window_id| {
            app.read_model(&CodeSettings::handle(app), |code_settings, _ctx| {
                async_assert!(
                    *code_settings.codebase_context_enabled.value(),
                    "Codebase context should be enabled"
                )
            })
        })
        .add_assertion(move |app, window_id| {
            let Some(cwd) = active_session_cwd(app, window_id) else {
                return AssertionOutcome::failure(
                    "Expected active session to have a cwd".to_string(),
                );
            };
            let Ok(canonicalized_path) = dunce::canonicalize(&cwd) else {
                return AssertionOutcome::failure(format!(
                    "Failed to canonicalize repo path: {}",
                    cwd.display()
                ));
            };

            // Kick off codebase indexing at the current directory.
            app.update_model(&CodebaseIndexManager::handle(app), |manager, ctx| {
                manager.index_directory(canonicalized_path.clone(), ctx);
            });

            AssertionOutcome::SuccessWithData(StepData::new(
                CWD_DATA_KEY.to_string(),
                canonicalized_path,
            ))
        })
        .add_named_assertion_with_data_from_prior_step(
            "Assert that codebase index has been synced",
            |app, _window_id, step_data_map| {
                let cwd = step_data_map
                    .get::<String, PathBuf>(CWD_DATA_KEY.into())
                    .expect("No cwd");

                let status = app.read_model(&CodebaseIndexManager::handle(app), |manager, ctx| {
                    manager.get_codebase_index_status_for_path(cwd, ctx)
                });
                match status {
                    Some(status) => {
                        async_assert!(
                            status.has_synced_version()
                                && status.last_sync_successful().unwrap_or(false),
                            "Codebase index for {} should be synced and successful",
                            cwd.display()
                        )
                    }
                    // If the index hasn't been created, then we are not in a git repo.
                    // Mark as success to avoid failing evals that are not in git repos.
                    None => AssertionOutcome::Success,
                }
            },
        )
}
